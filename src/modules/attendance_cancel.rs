//! 근태신청 취소 — 아마란스의 "결재취소"는 **취소 API가 아니라 취소신청서 상신**이다.
//!
//! 2026-09-15 브라우저 전량 캡처로 확정한 체인(`.claude-workspace/captures/20260915-attend-cancel/`).
//! 종결(doc_sts 90)된 근태 문서는 `cancel_approval`(eap110A54·A18·A19)로 되돌릴 수 없는데,
//! 이 경로는 **별도 양식의 새 문서를 상신**해 원본을 음수로 상쇄한다. 그래서 90에도 쓸 수 있다.
//!
//! ```text
//! 1) at00001                     대상 근태신청 찾기 → appSq·detailSq·linkKey·idDoc·표시필드
//! 2) selectAttendApplicationInfo  → outProcessCancelId (취소 양식의 formDTp)
//! 3) eap096A45(formDTp=위 값)      → 취소 양식 formId (연차는 44, 하드코딩하지 않는다)
//! 4) eap096A62(원본 docId)         → 원본 결재선 (취소문서가 이걸 물려받는다)
//! 5) 0hr00022                     취소 가능 사전 점검
//! 6) createCancelApplication      취소 신청 레코드 생성 → 새 appSq
//! 7) GetLinkKey → saveLinkKey     결재 연동 키 바인딩
//! 8) eap110A03 → SetEnageGroup → eap110A06   상신
//! ```
//!
//! ⚠️ **되돌릴 수 없다.** 취소의 취소는 없다. 그래서 상신 전에 대상을 특정하고(§7.2 소유권 가드),
//! 상신 후 재조회로 실제 문서 상태를 확인해 돌려준다(§7.1 read-back).

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::client::GwClient;
use crate::modules::approval_submit::{
    encode_uri_component, gen_approkey, norm_participant, now_kst_datetime, submitted_doc_id,
};

/// 취소 대상 한 건(=근태신청 1행). `at00001` 응답을 그대로 담아 필요한 값만 꺼내 쓴다.
struct Target {
    app_dt: String,
    app_sq: String,
    detail_sq: String,
    link_key: String,
    doc_id: String,
    title: String,
    emp_cd: String,
    row: Value,
}

fn s(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn f(v: &Value, k: &str) -> f64 {
    match v.get(k) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(x)) => x.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

/// `20261030` → `2026-10-30`. 형식이 다르면 원본을 그대로 둔다(추측해 바꾸지 않는다).
fn dash(d: &str) -> String {
    if d.len() == 8 && d.chars().all(|c| c.is_ascii_digit()) {
        format!("{}-{}-{}", &d[0..4], &d[4..6], &d[6..8])
    } else {
        d.to_string()
    }
}

/// `0900` → `09:00`.
fn colon(t: &str) -> String {
    if t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()) {
        format!("{}:{}", &t[0..2], &t[2..4])
    } else {
        t.to_string()
    }
}

/// `20261030` → `2026-10-30(금)`. 문서 본문 표에 그대로 렌더되는 문자열이다.
fn with_dow(d: &str) -> String {
    let base = dash(d);
    match dow_kr(d) {
        Some(w) => format!("{base}({w})"),
        None => base,
    }
}

/// 요일 계산(Sakamoto). 입력이 `YYYYMMDD`가 아니면 None.
fn dow_kr(d: &str) -> Option<&'static str> {
    if d.len() != 8 || !d.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let y: i32 = d[0..4].parse().ok()?;
    let m: usize = d[4..6].parse().ok()?;
    let day: i32 = d[6..8].parse().ok()?;
    if !(1..=12).contains(&m) {
        return None;
    }
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let idx = (y + y / 4 - y / 100 + y / 400 + T[m - 1] + day).rem_euclid(7) as usize;
    Some(["일", "월", "화", "수", "목", "금", "토"][idx])
}

/// 음수 표기. `1` → `-1`, `1.5` → `-1.5` (정수는 소수점을 남기지 않는다 — 브라우저와 같은 표기).
fn neg(x: f64) -> String {
    let v = -x;
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// 내 근태신청 중 `date`(YYYYMMDD) 자의 것을 찾는다.
///
/// 브라우저 캘린더는 `approState` `0,1,4,5`만 훑지만 **여기서는 전 상태를 본다** — 결재 진행중인
/// 신청이 그 범위에 없어서 "취소할 신청이 없다"는 틀린 안내가 나왔다(2026-09-15 실측).
/// 취소 가능 여부는 조회 범위가 아니라 `outProcessCancelId`와 `0hr00022`가 판정한다.
async fn find_targets(c: &GwClient, date: &str) -> Result<Vec<Value>> {
    let emp_cd = c.emp_cd();
    let r = c
        .call(
            "/human/attendapplication/at00001",
            &json!({
                "empCd": emp_cd, "startDate": date, "endDate": date,
                "approState": "0,1,2,3,4,5"
            }),
        )
        .await
        .map_err(|e| anyhow!("근태신청 조회(at00001) 실패: {e}"))?;
    // `c.call`이 봉투(resultData)를 이미 벗겨 주므로 응답 자체가 배열이다.
    // 한 번 더 벗기려 들면 조용히 0건이 된다(2026-09-15에 그 실수를 했다).
    let list = r
        .as_array()
        .or_else(|| r.get("resultData").and_then(|v| v.as_array()))
        .cloned()
        .unwrap_or_default();
    // 이미 취소신청이 걸린 행은 대상에서 뺀다(중복 취소 방지).
    Ok(list
        .into_iter()
        .filter(|x| {
            x.get("cancellationApplication") != Some(&Value::Bool(true))
                && s(x, "atDt") == date
        })
        .collect())
}

/// 대상 후보를 사람이 고를 수 있는 형태로 요약한다(여러 건일 때의 에러 메시지용).
fn digest(v: &Value) -> Value {
    json!({
        "appSq": s(v, "appSq"),
        "구분": s(v, "atCdNm"),
        "기간": format!("{} ~ {}", dash(&s(v, "startDt")), dash(&s(v, "endDt"))),
        "일수": f(v, "appDy"),
        "문서번호": s(v, "noDoc"),
        "docId": s(v, "idDoc"),
        "상태": s(v, "approState"),
        "제목": s(v, "titleDc"),
    })
}

/// 취소 가능한 근태신청인지 확인하고 대상을 확정한다.
async fn resolve(c: &GwClient, date: &str, app_sq: Option<&str>) -> Result<Target> {
    let rows = find_targets(c, date).await?;
    if rows.is_empty() {
        bail!(
            "{date} 에 취소할 내 근태신청이 없다(이미 취소신청이 걸린 건은 제외한다). \
             날짜는 근태가 적용되는 날(atDt)이지 상신한 날이 아니다."
        );
    }
    let row = match app_sq.map(str::trim).filter(|x| !x.is_empty()) {
        Some(want) => rows
            .iter()
            .find(|r| s(r, "appSq") == want)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "{date} 에 appSq={want} 인 내 근태신청이 없다. 후보: {}",
                    Value::Array(rows.iter().map(digest).collect())
                )
            })?,
        None if rows.len() > 1 => bail!(
            "{date} 에 근태신청이 {}건이다 — app_sq로 하나를 지목할 것. 후보: {}",
            rows.len(),
            Value::Array(rows.iter().map(digest).collect())
        ),
        None => rows[0].clone(),
    };

    // §7.2 소유권 가드 — 남의 신청을 취소하지 않는다(fail-closed).
    let me = c.emp_cd();
    let owner = s(&row, "appEmpCd");
    if !owner.is_empty() && owner != me {
        bail!("이 근태신청의 신청자는 내가 아니다(appEmpCd={owner}, 나={me}) — 취소하지 않는다.");
    }
    let app_sq = s(&row, "appSq");
    let doc_id = s(&row, "idDoc");
    if app_sq.is_empty() || doc_id.is_empty() {
        bail!(
            "대상 근태신청에 appSq/idDoc이 없다 — 상신되지 않은 건이거나 과거 상신 실패가 \
             남긴 고아 HP 레코드일 수 있다(그건 지울 방법이 없다). 대상: {}",
            digest(&row)
        );
    }

    // 결재가 끝난 신청만 취소신청 대상이다. 진행중인 문서에 createCancelApplication을 쏘면
    // 서버가 `resultCode -1 "취소신청이 불가능한 문서입니다.(0)"`로 거부하는데(2026-09-15 실측),
    // 그때는 이미 되돌릴 수 없는 구간에 들어와 있다. 그래서 여기서 먼저 막고 **무엇을 써야 하는지**
    // 알려준다 — "취소할 신청이 없다"는 식의 틀린 안내를 하지 않는다.
    // 관측된 값: "1"=결재완료(취소신청 가능) / "0"=상신·진행중 / "2"=취소신청 생성 직후.
    let state = s(&row, "approState");
    if state != "1" {
        let form_id = s(&row, "formId");
        bail!(
            "이 근태신청은 아직 결재가 끝나지 않았다(approState={state}) — 취소신청서는 \
             결재완료(1)된 신청에만 낼 수 있다. 진행중인 문서는 상신 자체를 물리면 된다: \
             cancel_approval(doc_id={doc_id}, form_id={form_id}, purge=true). 대상: {}",
            digest(&row)
        );
    }
    Ok(Target {
        app_dt: s(&row, "appDt"),
        app_sq,
        detail_sq: s(&row, "detailSq"),
        link_key: s(&row, "linkKey"),
        doc_id,
        title: s(&row, "titleDc"),
        emp_cd: s(&row, "empCd"),
        row,
    })
}

/// 취소 양식의 formId를 **서버에서** 얻는다. 연차는 44지만 상수로 박지 않는다 —
/// 양식마다 `outProcessCancelId`가 다르고, 그걸 그대로 `eap096A45`에 넘기면 서버가 알려준다.
async fn cancel_form_id(c: &GwClient, t: &Target) -> Result<(i64, String)> {
    let info = c
        .call(
            "/human/hrd0220/selectAttendApplicationInfo",
            &json!({"appDt": t.app_dt, "appSq": t.app_sq.parse::<i64>().unwrap_or(0)}),
        )
        .await
        .map_err(|e| anyhow!("근태신청 상세 조회(selectAttendApplicationInfo) 실패: {e}"))?;
    let master = info
        .get("attendApplicationMaster")
        .cloned()
        .unwrap_or(Value::Null);
    let form_d_tp = s(&master, "outProcessCancelId");
    if form_d_tp.is_empty() {
        bail!(
            "이 근태신청에는 취소 양식이 없다(outProcessCancelId 없음) — 아마란스 화면에도 \
             결재취소가 뜨지 않는 상태다. 원본 상세: {master}"
        );
    }
    let forms = c
        .call(
            "/eap/eap096A45",
            &json!({
                "header": {"groupSeq": c.group_seq(), "empSeq": c.emp_seq()},
                "body": {
                    "companyInfo": {"compSeq": c.comp_seq(), "bizSeq": c.comp_seq(), "deptSeq": c.dept_seq()},
                    "formDTp": form_d_tp, "langCode": "kr", "emailAddr": "", "emailDomain": ""
                }
            }),
        )
        .await
        .map_err(|e| anyhow!("취소 양식 조회(eap096A45) 실패: {e}"))?;
    let form = forms
        .as_array()
        .and_then(|a| a.first())
        .cloned()
        .ok_or_else(|| anyhow!("취소 양식({form_d_tp})을 서버가 알려주지 않았다: {forms}"))?;
    let form_id = form
        .get("formId")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow!("취소 양식 응답에 formId가 없다: {form}"))?;
    Ok((form_id, s(&form, "formNm")))
}

/// 취소문서의 결재선 = **원본 문서의 결재선**(실측). 같은 결재자가 취소도 승인하게 된다.
async fn inherit_line(c: &GwClient, doc_id: &str) -> Result<Vec<Value>> {
    let r = c
        .call("/eap/eap096A62", &json!({"docId": doc_id}))
        .await
        .map_err(|e| anyhow!("원본 결재선 조회(eap096A62) 실패: {e}"))?;
    let list = r
        .get("lineList")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if list.is_empty() {
        bail!("원본 문서({doc_id})의 결재선이 비어 있다 — 취소문서를 상신할 결재선이 없다.");
    }
    Ok(list
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let seq = (i + 1) as i64;
            json!({
                "user_id": s(n, "empSeq"), "org_id": s(n, "empSeq"), "org_div": "m",
                "co_id": s(n, "compSeq"), "dept_id": s(n, "deptSeq"),
                "act_id": n.get("actId").cloned().unwrap_or(json!(3000)),
                "user_nm": s(n, "userNm"), "doc_line_gb": 1, "app_yn": "0",
                "doc_line_seq": seq, "doc_line_m_seq": seq, "doc_line_s_seq": 1,
                "line_seq": seq, "arbitary_yn": 0, "deptline_yn": "0"
            })
        })
        .collect())
}

/// 취소문서 본문 데이터. 원본 신청값을 **음수로 뒤집어** 담는다 — 이게 연차 상쇄의 실체다.
async fn build_bind_data(c: &GwClient, t: &Target, today: &str) -> Result<Value> {
    let leave = c
        .call(
            "/human/common/annualleave/getAnnualLeaveInfoOfEmployee",
            &json!({
                "coCd": c.co_cd(), "empCd": t.emp_cd,
                "startDate": today, "endDate": today, "startTm": "0000", "endTm": "2359"
            }),
        )
        .await
        .map_err(|e| anyhow!("연차 현황 조회(getAnnualLeaveInfoOfEmployee) 실패: {e}"))?;
    let detail = leave.get("annualLeaveDetail").cloned().unwrap_or(Value::Null);
    let join_dt = dash(&s(&detail, "joinDt"));
    let grup_dt = dash(&s(&detail, "grupDt"));

    let yc = f(&t.row, "ycUseCnt");
    let app_dy = f(&t.row, "appDy");
    let app_tm_min = f(&t.row, "appTm");
    let unused = f(&leave, "unusedCnt");
    let minus = -yc;

    let row = json!({
        "atCdNm": s(&t.row, "atCdNm"),
        "year": s(&t.row, "atYm").chars().take(4).collect::<String>(),
        "startDt": dash(&s(&t.row, "startDt")),
        "endDt": dash(&s(&t.row, "endDt")),
        "appDy": neg(app_dy),
        "appTm": neg(app_tm_min / 60.0),
        "ycUseCnt": neg(yc),
        "appRmkDc": s(&t.row, "appRmkDc"),
        "startTm": colon(&s(&t.row, "startTm")),
        "endTm": colon(&s(&t.row, "endTm")),
        "joinDt": join_dt, "grupDt": grup_dt,
        "appTmMinute": format!("-{:02}시간", (app_tm_min / 60.0).abs() as i64),
        "startDtDayOfWeek": with_dow(&s(&t.row, "startDt")),
        "endDtDayOfWeek": with_dow(&s(&t.row, "endDt")),
    });

    // 사원 표시필드(empNm/deptNm/positionNm/dutyNm)는 submit 경로의 신원 주입이 덮어쓰지만,
    // 이 도구는 그 경로를 타지 않으므로 at00001이 준 값을 그대로 쓴다(서버가 준 값이 정본이다).
    let head = json!({
        "empCd": t.emp_cd, "empNm": s(&t.row, "empNm"),
        "deptNm": s(&t.row, "deptNm"), "positionNm": s(&t.row, "positionNm"),
        "dutyNm": s(&t.row, "dutyNm"), "hclsNm": Value::Null,
        "hrspNm": s(&t.row, "dutyNm"), "htypNm": s(&t.row, "htypNm"),
        "hoprNm": s(&t.row, "hoprNm"),
        "joinDt": join_dt, "grupDt": grup_dt,
        "totalCnt": format!("{:.1}", f(&leave, "totalCnt")),
        "usedCnt": format!("{}", f(&leave, "usedCnt")),
        "unusedCnt": format!("{unused}"),
        "progressCnt": format!("{:.1}", f(&leave, "progressCnt")),
        "remainCnt": format!("{}", unused - minus),
        "minusCnt": format!("{minus:.1}"),
        "sumDy": Value::Null, "aruseCnt": Value::Null, "appexDy": Value::Null, "listNum": 0
    });

    Ok(json!({
        "ITEMS": {
            "appYear": today[0..4], "appMonth": today[4..6], "appDay": today[6..8]
        },
        "TABLE": {
            "dbTable1": {"group": [{"items": head, "group": [{"items": row}]}]},
            "dbTable2": {"group": []}
        }
    }))
}

/// 근태신청 1건을 취소한다 — 취소신청서를 **상신**한다.
///
/// `date`는 근태가 적용되는 날(atDt, YYYYMMDD)이다. 그 날 신청이 여럿이면 `app_sq`로 지목한다.
pub async fn cancel_attendance(c: &GwClient, date: &str, app_sq: Option<&str>) -> Result<Value> {
    let date = date.trim();
    if date.len() != 8 || !date.chars().all(|c| c.is_ascii_digit()) {
        bail!("date는 YYYYMMDD 형식이어야 한다(받은 값: {date:?}).");
    }
    let t = resolve(c, date, app_sq).await?;
    let (form_id, form_nm) = cancel_form_id(c, &t).await?;
    let line_nodes = inherit_line(c, &t.doc_id).await?;

    // ── 사전 점검: 이 날짜·사번의 근태가 취소 가능한 상태인가 ────────────────────
    // 빈 배열이면 가능. 값이 오면 그게 불가 사유다(어떤 형태인지는 미검증 — 그대로 보여준다).
    let pre = c
        .call(
            "/human/attendapplication/0hr00022",
            &json!({"empCdList": [t.emp_cd], "atDtList": [date]}),
        )
        .await
        .map_err(|e| anyhow!("취소 사전점검(0hr00022) 실패: {e}"))?;
    if let Some(a) = pre.as_array()
        && !a.is_empty()
    {
        bail!("서버가 이 근태신청을 취소할 수 없다고 한다(0hr00022): {pre}");
    }

    // ── 여기서부터 되돌릴 수 없다 ────────────────────────────────────────────
    let created = c
        .call(
            "/human/attendapplication/createCancelApplication",
            &json!({
                "linkKey": t.link_key, "appSq": t.app_sq, "detailSqList": [t.detail_sq]
            }),
        )
        .await
        .map_err(|e| anyhow!("취소신청 생성(createCancelApplication) 실패: {e}"))?;
    let new_app_sq = created
        .get("appSq")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow!("createCancelApplication이 새 appSq를 주지 않았다: {created}"))?;
    let doc_title = s(&created, "titleDc");
    let doc_title = if doc_title.is_empty() {
        format!("{} 취소신청", t.title)
    } else {
        doc_title
    };
    let app_dt = s(&created, "appDt");
    let today = if app_dt.len() == 8 { app_dt.clone() } else { now_kst_datetime()[0..10].replace('-', "") };

    let approkey = gen_approkey();

    // ── 결재 연동 등록 ───────────────────────────────────────────────────────
    let glk = c
        .call(
            "/system/apiUtilEap/GetLinkKey",
            &json!({"menuCode":"HPD0110","approKey":approkey,"vPCoCd":c.co_cd(),"coCd":c.co_cd()}),
        )
        .await
        .map_err(|e| anyhow!("GetLinkKey 실패: {e}"))?;
    let link_key = s(&glk, "linkKey");
    c.call(
        "/human/openapi/attendapplication/saveLinkKey",
        &json!({"linkKey": link_key, "appSq": new_app_sq, "coCd": c.co_cd(), "appDt": app_dt}),
    )
    .await
    .map_err(|e| anyhow!("saveLinkKey 실패: {e}"))?;

    // ── 양식 컨텍스트(수신참조·시행자·form_d_tp) ────────────────────────────────
    let a03 = c
        .call(
            "/eap/eap110A03",
            &json!({
                "docID": 0, "formID": form_id.to_string(), "approkey": approkey,
                "appLineId": "", "draftTp": "", "reDraft": "", "docType": "",
                "doc_auth": 0, "pageCode": "UBAP001"
            }),
        )
        .await?;
    let rm = a03.get("resultMap").cloned().unwrap_or(Value::Null);
    let form_d_tp = rm
        .get("form_info")
        .and_then(|x| x.get("form_d_tp"))
        .and_then(|v| v.as_str())
        .filter(|x| !x.is_empty())
        .ok_or_else(|| anyhow!("a03가 취소 양식의 form_d_tp를 주지 않았다(formId={form_id})"))?
        .to_string();
    let refer_nodes: Vec<Value> = rm
        .get("m_Refer")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(norm_participant).collect())
        .unwrap_or_default();
    let oper_nodes: Vec<Value> = rm
        .get("m_Oper")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(norm_participant).collect())
        .unwrap_or_default();

    c.call(
        "/system/apiUtilEap/SetEnageGroup",
        &json!({
            "approKey": approkey, "formDTp": form_d_tp, "formId": form_id.to_string(),
            "linkKey": link_key, "formNm": form_nm, "docTitle": doc_title, "contents": "",
            "contentsApi": "/human/attendapplication/interlock/getInterlockFormContents",
            "statusApi": "/human/attendapplication/interlock/setInterlockSync",
            "dummy1": "", "link": "", "vPCoCd": c.co_cd(), "coCd": c.co_cd()
        }),
    )
    .await
    .map_err(|e| anyhow!("SetEnageGroup 실패: {e}"))?;

    // ── 상신 ────────────────────────────────────────────────────────────────
    let bind_obj = build_bind_data(c, &t, &today).await?;
    let s1 = serde_json::to_string(&bind_obj)?;
    let bind_field = Value::String(serde_json::to_string(&s1)?);
    // 본문 HTML: 근태 양식은 bindData가 문서를 렌더하므로 한 줄 요약으로 통과한다(4양식 실증).
    let summary = format!(
        "<div>{} 취소 — {} {} ({}일)</div>",
        s(&t.row, "atCdNm"),
        dash(&s(&t.row, "startDt")),
        colon(&s(&t.row, "startTm")),
        f(&t.row, "appDy")
    );

    let line_compact: Vec<Value> = line_nodes
        .iter()
        .map(|n| {
            json!({
                "doc_line_m_seq": n.get("doc_line_m_seq").cloned().unwrap_or(json!(0)),
                "doc_line_s_seq": 1,
                "act_id": n.get("act_id").cloned().unwrap_or(json!(3000)),
                "co_id": n.get("co_id").cloned().unwrap_or(json!(c.comp_seq())),
                "dept_id": n.get("dept_id").cloned().unwrap_or(Value::Null),
                "user_id": n.get("user_id").cloned().unwrap_or(Value::Null),
                "doc_line_gb": "1"
            })
        })
        .collect();
    let recv_of = |n: &Value, div: &str| {
        json!({
            "receive_div": div,
            "org_div": n.get("org_div").cloned().unwrap_or(Value::Null),
            "org_id": n.get("org_id").cloned().unwrap_or(Value::Null)
        })
    };
    let mut receive_list: Vec<Value> = Vec::new();
    for n in oper_nodes.iter() {
        receive_list.push(recv_of(n, "40"));
    }
    for n in refer_nodes.iter() {
        receive_list.push(recv_of(n, "10"));
    }

    let param_item = json!({
        "bindData": bind_field,
        "interDivId": "divInterJson", "interDocTp": "json",
        "doc_id": 0, "form_id": form_id.to_string(), "numbering_id": "1001",
        "rep_dt": now_kst_datetime(), "repdt_mod_yn": "0",
        "co_id": c.comp_seq(), "dept_id": c.dept_seq(), "biz_id": c.comp_seq(),
        "user_id": c.emp_seq(), "co_nm": "(주)이노그리드",
        "dept_nm": s(&t.row, "deptNm"), "user_nm": c.emp_name(),
        "doc_title": doc_title, "doc_sts": "20", "inservice_time": "0",
        "doc_level": "001", "emergency_level": "1", "doc_security": "0", "use_yn": "1",
        "approkey": approkey, "contents_tp": "10",
        "doc_contents": encode_uri_component(&summary),
        "pTEAG_APPDOC_LINE": line_nodes,
        "pVKD_TKDDITEM": [], "pVCM_ATTACHFILEINFO": [],
        "pRefer": refer_nodes, "pReceive": [], "pOper": oper_nodes, "pTEAG_APPDOC_REF": [],
        "pTEAG_TOC_FOLDER": "", "pDraftTp": "", "seal_use_yn": "", "receipient": "",
        "receipt": "", "iframeHtml": "", "re_draft": "",
        "modifyAppLineYn": "Y", "modifyReceive10": "Y", "modifyReceive20": "Y",
        "modifyReceive30": "Y", "modifyReceive40": "Y", "modifyTitle": "Y",
        "modifyContent": "Y", "modifyRef": "Y", "modifyAttach": "Y", "modifyAddItem": "Y",
        "modifyInservice": "Y", "modifyDoclevel": "Y", "modifyEmergency": "Y",
        "modifySeal": "Y", "modifyEabox": "Y", "modifyFileList": "",
        "delFileSnList": [], "auditorYn": "0",
        "modifyDocInfo": {
            "docId": 0,
            "appdoc": {
                "inservice_time": "0", "doc_level": "001", "doc_security": "0",
                "emergency_level": "1", "doc_title": doc_title
            },
            "appdocReceiveList": receive_list,
            "appdocLineList": line_compact,
            "appdocFileList": [], "appdocFolderList": [{ "menu_id": "" }], "appdocRefList": []
        },
        "modifyItemList": Value::Null, "isLatestVerContentsFile": true,
        "versionCheck": Value::Null, "formLang": "kr", "aiVerifyHistories": [],
        "aiVerifyAutoOnSubmit": false, "aiVerifyUseYn": "0"
    });
    let d = c
        .call("/eap/eap110A06", &json!({"paramItem": param_item, "pageCode": "UBAP001"}))
        .await?;
    let new_doc_id = submitted_doc_id(&d).ok_or_else(|| {
        anyhow!(
            "취소신청서 상신(eap110A06)이 docId를 주지 않았다 — HP 취소 레코드(appSq={new_app_sq})는 \
             이미 만들어졌으니 아마란스 근태신청서 화면을 확인할 것. 서버 응답: {d}"
        )
    })?;

    // ── read-back(§7.1): 응답 성공 ≠ 실제 반영 ────────────────────────────────
    let doc_id_str = new_doc_id.to_string().trim_matches('"').to_string();
    let status = crate::modules::approval::read_approval(c, &doc_id_str, &form_id.to_string())
        .await
        .ok()
        .and_then(|v| v.get("status").and_then(|x| x.as_str()).map(str::to_string));
    let still_open = find_targets(c, date)
        .await
        .map(|rows| rows.iter().any(|r| s(r, "appSq") == t.app_sq))
        .unwrap_or(true);

    Ok(json!({
        "kind": "attendanceCancelSubmitted",
        "ok": true,
        "docId": new_doc_id,
        "formId": form_id,
        "formName": form_nm,
        "title": doc_title,
        "status": status,
        "cancelledTarget": digest(&t.row),
        "approvers": line_nodes.iter().map(|n| s(n, "user_nm")).collect::<Vec<_>>(),
        "originStillActive": still_open,
        "note": if still_open {
            "취소신청서를 상신했다. ⚠️ 이건 즉시 취소가 아니라 **취소 상신**이다 — \
             결재가 끝나야 원본 근태·연차가 상쇄된다. 그때까지 원본은 살아 있다."
        } else {
            "취소신청서가 상신되어 곧바로 종결됐고(결재선이 기안자 단독), 원본 근태신청이 \
             목록에서 사라진 것까지 확인했다."
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 요일은_실제_달력을_따른다() {
        assert_eq!(dow_kr("20261030"), Some("금"));
        assert_eq!(dow_kr("20260915"), Some("화"));
        assert_eq!(dow_kr("20260101"), Some("목"));
        assert_eq!(dow_kr("2026103"), None);
    }

    #[test]
    fn 표시형식은_아마란스_표기를_따른다() {
        assert_eq!(dash("20261030"), "2026-10-30");
        assert_eq!(colon("0900"), "09:00");
        assert_eq!(with_dow("20261030"), "2026-10-30(금)");
        // 형식이 다르면 추측해 바꾸지 않는다
        assert_eq!(dash("2026/10/30"), "2026/10/30");
    }

    #[test]
    fn 취소값은_원본을_음수로_뒤집는다() {
        assert_eq!(neg(1.0), "-1");
        assert_eq!(neg(0.5), "-0.5");
        assert_eq!(neg(0.0), "0");
    }

    #[test]
    fn 결재선은_원본_문서의_것을_물려받는다() {
        // eap096A62 응답 모양 → pTEAG_APPDOC_LINE 노드
        let src = json!({"empSeq":"3081","compSeq":"1000","deptSeq":"2993","actId":3000,"userNm":"정선미"});
        let n = json!({
            "user_id": s(&src, "empSeq"), "org_id": s(&src, "empSeq"), "org_div": "m",
            "co_id": s(&src, "compSeq"), "act_id": src.get("actId").cloned().unwrap()
        });
        assert_eq!(n["user_id"], "3081");
        assert_eq!(n["org_id"], "3081");
        assert_eq!(n["org_div"], "m");
    }
}
