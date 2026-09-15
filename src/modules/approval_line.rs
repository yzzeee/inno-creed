//! 개인결재라인(config) CRUD — `/eap/eap102A0x`. **상신이 아니라 "상신 시 재사용할 결재선 config"**.
//! 생성→수정→삭제 왕복을 실호출로 검증하고 환경을 원복해 확정했다.
//! 인증은 헤더 서명만으로 완결(ensure_session 불필요).
//!
//! ⚠️ **결재자 객체(detailLine)는 호출자가 조립하지 않는다** — `save_line`이 empSeq 목록을 받아
//! `approver_node`로 만든다. 서버 payload에 필요한 `org_id`/`org_div`는 어떤 조회 도구도 주지
//! 않으므로(find_person·org_chart·suggest_approval_line 전부 empSeq까지만 준다) 호출자에게
//! 요구하면 맞힐 수 없다 — 그렇게 요구하던 시절 결재자 0명 라인이 "저장 성공"으로 보고돼
//! 상신이 실패했다(2026-09-15).
//! 서버가 자동 생성한 결재선은 규칙과 어긋나므로 그대로 신뢰하지 말 것 — 규칙 확인은
//! `approval_line_suggest::suggest_line`.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::client::GwClient;
use crate::util::s;

/// 저장된 개인결재라인 목록 — eap102A02(body `{}`).
/// resultData[] : `{line_id, line_nm, form_id, form_nm, proc_id, proc_nm, line_kind, form_list}`.
/// 삭제(eap102A09)가 id 배열이 아니라 **행 객체**를 요구하므로 원본(`_row`)을 보존해 같이 돌려준다
/// — 다만 그 조립은 `delete_line`이 하므로 호출자가 쓸 일은 없다.
pub async fn list_lines(c: &GwClient) -> Result<Value> {
    let d = c.call("/eap/eap102A02", &json!({})).await?;
    let arr = d.as_array().cloned().unwrap_or_default();
    let lines: Vec<Value> = arr
        .iter()
        .map(|l| {
            json!({
                "lineId": s(l, "line_id"),
                "lineName": s(l, "line_nm"),
                "formId": s(l, "form_id"),
                "formName": s(l, "form_nm"),
                "procId": s(l, "proc_id"),
                "procName": s(l, "proc_nm"),
                "lineKind": s(l, "line_kind"),
                "_row": l  // 서버가 삭제에 요구하는 행 객체 원본(delete_line이 스스로 찾아 쓴다 — 호출자는 lineId만 주면 된다)
            })
        })
        .collect();
    Ok(json!({ "kind": "approvalLines", "count": lines.len(), "lines": lines }))
}

/// 라인 단건의 결재자 구성 — eap102A05(body `{lineId, line_id}`).
/// resultData.aaData[] 결재자 객체를 **원본 그대로** 반환한다. `save_line`의 read-back 판정
/// 근거이고(결재자 수), 상신 전에 사용자에게 결재선을 보여주는 재료다 — 라인을 새로 만들 때
/// 이 객체를 베낄 필요는 없다(save_line이 empSeq만 받는다).
pub async fn read_line(c: &GwClient, line_id: &str) -> Result<Value> {
    let d = c
        .call("/eap/eap102A05", &json!({ "lineId": line_id, "line_id": line_id }))
        .await?;
    let members = d
        .get("aaData")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(json!({
        "kind": "approvalLineMembers",
        "lineId": line_id,
        "count": members.len(),
        "members": members,  // 원본 결재자 객체(user_id=empSeq, user_nm=이름, 순서=결재 순서)
        "note": "각 객체의 act_id 3000=결재/4000=합의. 라인을 새로 만들 땐 이 객체를 베끼지 말고 save_approval_line(approvers=[empSeq,…])를 쓴다."
    }))
}

/// 결재선 구성의 **금지 판정**(순수). 저장 시점(`save_line`)과 상신 시점
/// (`approval_submit::check_line_before_submit`)이 **같은 규칙을 써야** 둘이 어긋나지 않는다.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LineShape {
    /// 쓸 수 있다.
    Usable,
    /// 결재자가 없다 — 그 라인으로는 상신이 성립하지 않는다.
    Empty,
    /// 기안자 단독 — 상신 즉시 `종결`(doc_sts 90)이 되고 `cancel_approval`이 90 취소를 거부해
    /// **되돌릴 수 없는 문서가 남는다**(2026-09-15 실측, docId 148978).
    SoleDrafter,
}

/// `seqs`=결재자 empSeq 목록(순서=결재 순서), `me`=본인 empSeq.
pub(crate) fn line_shape(seqs: &[String], me: &str) -> LineShape {
    if seqs.is_empty() {
        LineShape::Empty
    } else if seqs.len() == 1 && seqs[0] == me {
        LineShape::SoleDrafter
    } else {
        LineShape::Usable
    }
}

/// 결재자 1명의 eap102A10 `detailLine` 노드를 만든다 — 호출자는 empSeq만 준다.
///
/// ⚠️ **`org_id`(=empSeq) + `org_div`("m") 한 쌍이 없으면 서버가 행을 만들고도 결재자를 0명으로
/// 저장한다**(2026-09-15 실측, 필드 이분법 7회). `insertDResult:1`로 성공을 주므로 응답만으로는
/// 구분되지 않는다 — 그래서 이 값을 호출자에게 요구하지 않고 여기서 채운다.
/// 최소 필수 5필드 = `user_id`·`co_id`·`act_id`·`org_id`·`org_div`.
/// 나머지(dept_id·duty_cd·grade_cd·이름·login_id·path_name)는 **서버가 조직도에서 채운다**.
/// (같은 규칙이 상신 경로에도 있다 — `approval_submit::norm_participant`.)
fn approver_node(emp_seq: &str, co_id: &str, seq: i64) -> Value {
    json!({
        "user_id": emp_seq,
        "org_id": emp_seq,
        "org_div": "m",
        "co_id": co_id,
        "act_id": 3000,          // 3000=결재. 합의자는 담지 않는다(상신 때 서버가 양식필수로 병합).
        "doc_line_seq": seq,     // ⚠️ 순서 3필드(1-base) — line_seq만으론 순서가 저장 안 되고
        "doc_line_m_seq": seq,   //    doc_line_seq가 null이 돼 결재 순서가 뒤섞인다(2026-08-03 실측).
        "line_seq": seq
    })
}

/// 개인결재라인 생성/수정 — eap102A10. `approvers`는 **empSeq 목록**(배열 순서 = 결재 순서).
/// `line_id`=0이면 신규, 기존 id면 수정.
///
/// 금지 규칙(둘 다 저장 자체를 막는다):
/// - **결재자 0명** — 저장 후 read-back으로 판정한다(§7.1). 신규 생성이었으면 만들어진 빈 라인을
///   되돌린다.
/// - **기안자 단독** — 결재자가 본인 1명이면 상신 즉시 `종결`(doc_sts 90)이 되고
///   `cancel_approval`은 90 취소를 거부한다(실증 범위 10·20·30) → 되돌릴 수 없는 문서가 남는다
///   (2026-09-15 실측, docId 148978). §7.2의 fail-closed와 같은 비대칭: 못 만든 라인은 웹에서
///   만들 수 있으나 종결된 문서는 되돌릴 수 없다.
///
/// ⚠️ 이건 config 저장일 뿐 **상신이 아니다**.
pub async fn save_line(
    c: &GwClient,
    line_id: i64,
    line_nm: &str,
    form_id: i64,
    proc_id: &str,
    approvers: &[String],
) -> Result<Value> {
    let seqs: Vec<String> = approvers
        .iter()
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect();
    let me = c.emp_seq();
    match line_shape(&seqs, &me) {
        LineShape::Usable => {}
        LineShape::Empty => {
            return Err(crate::error::InvalidInput::new(
                "approvers(결재자 empSeq 목록)가 비어있다 — 결재자 없는 라인으로는 상신할 수 없다. \
                 결재선 규칙은 suggest_approval_line으로 확인할 것.",
            )
            .into());
        }
        LineShape::SoleDrafter => {
            return Err(crate::error::InvalidInput::new(format!(
                "기안자 단독 결재선({me})은 만들 수 없다 — 상신 즉시 종결(doc_sts 90)되어 \
                 cancel_approval로 취소할 수 없다(실증 범위 10·20·30). 본인 아닌 결재자를 최소 \
                 1명 포함할 것. 규칙상 결재선은 suggest_approval_line으로 확인한다."
            ))
            .into());
        }
    }

    let co_id = c.comp_seq();
    let detail: Vec<Value> = seqs
        .iter()
        .enumerate()
        .map(|(i, emp)| approver_node(emp, &co_id, i as i64 + 1))
        .collect();
    let proc = if proc_id.trim().is_empty() { "1000" } else { proc_id.trim() };
    let body = json!({
        "line_id": line_id,
        "line_nm": line_nm,
        "line_kind": "10",
        "proc_id": proc,
        "detailLine": detail,
        "formList": [form_id],
        "form_id": form_id
    });
    let d = c.call("/eap/eap102A10", &body).await?;
    let created = d.get("createdLineId").cloned().unwrap_or(Value::Null);
    // 실제로 쓰인 라인 id — 신규면 서버가 발급한 것, 수정이면 넘긴 것.
    let target_id = if line_id > 0 {
        line_id.to_string()
    } else {
        match &created {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => String::new(),
        }
    };
    if target_id.is_empty() {
        return Err(anyhow!(
            "eap102A10이 createdLineId를 주지 않아 저장 결과를 확인할 수 없다: {d}"
        ));
    }

    // ── read-back(§7.1) — `insertDResult:1`을 반영으로 믿지 않는다. ──────────────
    let back = read_line(c, &target_id).await.map_err(|e| {
        anyhow!("저장은 호출됐으나 재조회(eap102A05)에 실패해 결재자를 확인하지 못했다(lineId={target_id}): {e}")
    })?;
    let saved = back.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    if saved != seqs.len() {
        // 신규 생성이었다면 결재자가 어긋난 라인을 남기지 않는다(진단 라인이 계정에 쌓이는 것을 막는다).
        let cleanup = if line_id == 0 {
            match delete_line(c, &target_id).await {
                Ok(_) => " 방금 만들어진 라인은 되돌렸다.",
                Err(_) => " ⚠️ 만들어진 라인을 되돌리지 못했으니 delete_approval_line으로 정리할 것.",
            }
        } else {
            ""
        };
        return Err(anyhow!(
            "결재자 {}명을 보냈는데 재조회에는 {saved}명이다(lineId={target_id}) — empSeq가 아닌 \
             값이 섞였거나 명부에 없는 사번일 수 있다(find_person으로 확인).{cleanup}",
            seqs.len()
        ));
    }

    Ok(json!({
        "kind": "approvalLineSaved",
        "lineId": target_id,
        "createdLineId": created,
        "approverCount": saved,
        "approvers": back.get("members").map(member_digest).unwrap_or(Value::Null),
        "verified_by_readback": true,
        "insertDResult": d.get("insertDResult").cloned().unwrap_or(Value::Null),
        "insertFormResult": d.get("insertFormResult").cloned().unwrap_or(Value::Null),
        "note": "config 저장 완료(상신 아님) — 결재자 수를 재조회로 확인했다. 이름은 서버가 조직도에서 채운 값이니 사용자에게 보여 확인받을 것."
    }))
}

/// read-back 보고용 — 결재자 객체에서 사람이 읽을 것만 추린다(순서=결재 순서).
fn member_digest(members: &Value) -> Value {
    let arr = members.as_array().cloned().unwrap_or_default();
    let out: Vec<Value> = arr
        .iter()
        .map(|m| {
            json!({
                "empSeq": s(m, "user_id"),
                "name": s(m, "user_nm"),
                "dept": s(m, "dept_nm"),
                "duty": s(m, "duty_nm"),
                "act": s(m, "act_nm")
            })
        })
        .collect();
    json!(out)
}

/// 개인결재라인 삭제 — eap102A09. 호출자는 **lineId만** 준다.
///
/// ⚠️ 서버는 id 배열을 받지 않는다(id만 넣으면 resultCode 2165) — `list_lines`의 행 객체를
/// 그대로 실어야 하므로, 그 조회·조립을 여기서 한다.
/// 삭제 후 목록 재조회로 부재를 확인한다(§7.1).
pub async fn delete_line(c: &GwClient, line_id: &str) -> Result<Value> {
    let id = line_id.trim();
    if id.is_empty() {
        return Err(crate::error::InvalidInput::new("line_id가 비어있다").into());
    }
    let row = find_row(c, id)
        .await?
        .ok_or_else(|| anyhow!("lineId={id} 인 개인결재라인이 없다 — list_approval_lines로 확인할 것"))?;
    let d = c
        .call("/eap/eap102A09", &json!({ "lineIdList": [row] }))
        .await?;
    // read-back — 목록에서 사라졌는지로 판정한다.
    let gone = match find_row(c, id).await {
        Ok(r) => r.is_none(),
        Err(_) => false,
    };
    Ok(json!({
        "kind": "approvalLineDeleted",
        "lineId": id,
        "ok": gone,
        "verified_by_readback": gone,
        "resultCount": d.get("resultCount").cloned().unwrap_or(Value::Null),
        "note": if gone {
            "삭제됨(목록 재조회로 부재 확인)."
        } else {
            "삭제를 호출했으나 재조회에 아직 남아 있다 — 반영 실패이거나 확인 실패다. list_approval_lines로 직접 볼 것."
        }
    }))
}

/// eap102A02에서 그 `line_id`의 **행 객체 원본**을 찾는다(삭제 payload·존재 확인 공용).
async fn find_row(c: &GwClient, line_id: &str) -> Result<Option<Value>> {
    let d = c.call("/eap/eap102A02", &json!({})).await?;
    Ok(d.as_array().and_then(|arr| {
        arr.iter()
            .find(|l| s(l, "line_id") == line_id)
            .cloned()
    }))
}


#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    /// ⛔ 회귀 방지: 결재자 0명과 기안자 단독은 **저장 시점에** 막아야 한다.
    /// 0명은 서버가 `insertDResult:1`로 성공을 주고(2026-09-15 실측 7회), 기안자 단독은
    /// 상신 즉시 종결(90)돼 cancel_approval이 거부한다(docId 148978) — 둘 다 응답만으로는
    /// 사고를 알 수 없어서 규칙으로 막는 것이다.
    #[test]
    fn 결재선_금지_판정은_0명과_기안자단독을_막는다() {
        let me = "3166";
        assert_eq!(line_shape(&v(&[]), me), LineShape::Empty);
        assert_eq!(line_shape(&v(&["3166"]), me), LineShape::SoleDrafter);

        // 본인 아닌 결재자가 하나라도 있으면 30(진행중)에 머물러 취소할 수 있다.
        assert_eq!(line_shape(&v(&["3081"]), me), LineShape::Usable);
        assert_eq!(line_shape(&v(&["3166", "3081"]), me), LineShape::Usable);
        assert_eq!(line_shape(&v(&["3081", "3166"]), me), LineShape::Usable);
    }

    /// ⛔ 회귀 방지: `org_id`(=empSeq) + `org_div`("m")가 빠지면 서버가 행을 만들고도 결재자를
    /// **0명으로 저장한다**(2026-09-15 필드 이분법 실측). 그래서 호출자에게 요구하지 않고
    /// 여기서 채운다 — 이 두 필드가 노드에서 사라지면 그 사고가 되돌아온다.
    #[test]
    fn 결재자_노드는_필수_5필드를_스스로_채운다() {
        let n = approver_node("2083", "1000", 1);
        assert_eq!(n["user_id"], "2083");
        assert_eq!(n["org_id"], "2083", "org_id는 empSeq와 같은 값이어야 한다");
        assert_eq!(n["org_div"], "m");
        assert_eq!(n["co_id"], "1000");
        assert_eq!(n["act_id"], 3000, "개인라인에는 결재(3000)만 담는다");
    }

    /// 배열 순서가 결재 순서다 — 순서 필드 3개를 1-base로 함께 넣어야 저장된다
    /// (line_seq만으론 doc_line_seq가 null이 돼 순서가 뒤섞임 — 2026-08-03 실측).
    #[test]
    fn 결재자_노드는_순서필드_3개를_1base로_넣는다() {
        let n = approver_node("2857", "1000", 2);
        assert_eq!(n["doc_line_seq"], 2);
        assert_eq!(n["doc_line_m_seq"], 2);
        assert_eq!(n["line_seq"], 2);
    }

    /// read-back 보고는 사람이 확인할 것(이름·부서·직책)만 추린다 — 상신 전에 사용자에게
    /// 결재선을 보여주기 위한 값이다.
    #[test]
    fn 결재자_요약은_이름과_직책을_남긴다() {
        let members = json!([{
            "user_id": "3081", "user_nm": "정선미", "dept_nm": "네이티브 플랫폼팀",
            "duty_nm": "팀장", "act_nm": "결재", "grade_cd": "910"
        }]);
        let d = member_digest(&members);
        assert_eq!(d[0]["empSeq"], "3081");
        assert_eq!(d[0]["name"], "정선미");
        assert_eq!(d[0]["duty"], "팀장");
        assert_eq!(d[0]["act"], "결재");
    }
}
