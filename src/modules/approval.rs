//! 전자결재(EAPPROVAL) 모듈 — `/eap/*`. 읽기 전용(함별 목록·문서 상세·미처리 카운트).
//! 인증은 헤더 서명만으로 완결. **이 파일은 읽기만 한다** — 상신·상신취소는 `approval_submit`,
//! 종결된 근태문서 취소는 `attendance_cancel`에 있다. 승인/반려는 아직 미구현이다.
//! 엔드포인트·요청 필드·응답 봉투는 실제 트래픽 캡처로 확정한 값이다.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::client::GwClient;
use crate::util::{days_to_ymd, digits_only, fmt_ymd, json_str};

/// 함(box) → (목록 API, eaBoxId, menuNo, periodPicker, 응답 list 경로).
/// 수신계열(미결/기결/수신참조/시행)은 eap105A04(resultData.map.list), 상신함은 eap107A04(resultData.list.list).
fn box_spec(b: &str) -> Result<(&'static str, &'static str, &'static str, &'static str, &'static str)> {
    Ok(match b {
        "pending" => ("eap105A04", "1000900", "1001000", "ARRIVED_DT", "map"),
        "approved" => ("eap105A04", "1000900", "1001100", "ACTION_TIME", "map"),
        "approved_ongoing" => ("eap105A04", "1000900", "1001110", "ACTION_TIME", "map"),
        "approved_done" => ("eap105A04", "1000900", "1001120", "ACTION_TIME", "map"),
        "reference" => ("eap105A04", "1000900", "1001200", "REP_DT", "map"),
        "enforcement" => ("eap105A04", "1000900", "1001400", "REP_DT", "map"),
        "sent" => ("eap107A04", "1000300", "1000400", "REP_DT", "list"),
        // 임시보관(결재작성 중 저장 + 상신취소로 복귀한 문서). eap107A06, menuNo 1000500.
        // ※ 여기 쌓인 문서가 신규 상신을 막지는 않는다(실측으로 반증됨).
        "draft" => ("eap107A06", "1000300", "1000500", "REP_DT", "list"),
        _ => return Err(anyhow!(
            "알 수 없는 함 '{b}'. 사용 가능: pending(미결)/approved(기결)/approved_ongoing(기결진행)/approved_done(기결종결)/reference(수신참조)/enforcement(시행)/sent(상신)/draft(임시보관)"
        )),
    })
}

/// 함별 문서 목록 — eap105A04(수신계열) / eap107A04(상신함).
/// `from`/`to`는 등록·도착일 범위(YYYY-MM-DD 또는 YYYYMMDD; 빈값이면 서버 기본 최근 3개월).
#[allow(clippy::too_many_arguments)]
pub async fn list_approvals(
    c: &GwClient,
    box_name: &str,
    page: i64,
    page_size: i64,
    from: &str,
    to: &str,
) -> Result<Value> {
    let (api, ea_box_id, menu_no, period, list_path) = box_spec(box_name)?;
    // 빈 날짜면 서버 기본이 좁아 문서를 놓친다 → UI처럼 최근 ~3개월로 채운다.
    let (def_from, def_to) = default_range();
    let sfr = if from.trim().is_empty() { def_from } else { digits_only(from) };
    let sto = if to.trim().is_empty() { def_to } else { digits_only(to) };
    let body = json!({
        "fDocSts": [], "page": page.to_string(), "pageSize": page_size.to_string(),
        "eaBoxId": ea_box_id, "nMenuID": menu_no, "menuNo": menu_no, "upperMenuNo": ea_box_id,
        "sfrDt": sfr, "stoDt": sto,
        "sFormId": ["0"], "periodPicker": period, "sortField": period, "sortType": "DESC",
        "docContentsData": {}, "item": {},
        "useElasticSearch": true, "useElasticSearch_new": true,
        "pageCode": ""
    });
    let data = c.call(&format!("/eap/{api}"), &body).await?;

    // 응답 봉투: 수신계열 resultData.map.{list,totalCount}, 상신함 resultData.list.{list,totalCount}.
    let container = data
        .get(list_path)
        .ok_or_else(|| anyhow!("{api} 응답에 {list_path} 없음"))?;
    let total = container.get("totalCount").cloned().unwrap_or(Value::Null);
    let arr = container.get("list").and_then(|v| v.as_array()).cloned().unwrap_or_default();

    let s = |v: &Value, k: &str| json_str(v.get(k));
    let docs: Vec<Value> = arr
        .iter()
        .map(|d| {
            // 상신/수신함은 FORM_NM, 임시보관(draft)은 DRAFT_FORM_NM에 양식명이 있다.
            let form = {
                let f = s(d, "FORM_NM");
                if f.is_empty() { s(d, "DRAFT_FORM_NM") } else { f }
            };
            json!({
                "docId": s(d, "DOC_ID"),
                "docNo": s(d, "DOC_NO"),
                "title": s(d, "DOC_TITLE"),
                "form": form,
                "formId": s(d, "FORM_ID"),
                "drafter": s(d, "USER_NM"),
                "dept": s(d, "DEPT_NM"),
                "status": s(d, "DOC_STSNM"),
                "currentApprover": s(d, "lineUserName"),
                "readYn": s(d, "READYN"),
                "repDt": s(d, "REP_DT"),
                "arrivedDt": s(d, "ARRIVED_DT"),
                "endDt": s(d, "END_DT"),
                "commentCount": s(d, "COMMENT_COUNT"),
                "fileCount": s(d, "FILE_CNT")
            })
        })
        .collect();
    Ok(json!({ "box": box_name, "totalCount": total, "documents": docs }))
}

/// 문서 상세 — eap111A04. `doc_id`/`form_id`는 목록의 docId/formId.
/// 본문 평문(contentsWord)·헤더·결재선·첨부수 반환. **열람 부작용 없음**(setReadYn:"N").
pub async fn read_approval(c: &GwClient, doc_id: &str, form_id: &str) -> Result<Value> {
    let body = json!({
        "doc_id": doc_id, "form_id": form_id, "bindType": "V", "p_doc_id": 0,
        "doc_auth": "0", "spDocId": "", "setReadYn": "N", "commentReqYn": "N",
        "pageCode": "UBA1100", "docToken": ""
    });
    let d = c.call("/eap/eap111A04", &body).await?;

    let s = |k: &str| json_str(d.get(k));
    // 결재선 처리내역: user_info[] (처리시각/처리여부). 이름 필드는 미노출이라 코드 위주로 요약.
    let line: Vec<Value> = d
        .get("user_info")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|u| {
                    json!({
                        "userId": json_str(u.get("user_id")),
                        "receiveDiv": json_str(u.get("receive_div")),
                        "procYn": json_str(u.get("proc_yn")),
                        "procTime": json_str(u.get("proc_time"))
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let body_text = {
        let w = s("contentsWord");
        if w.trim().is_empty() { html_to_text(&s("docContents")) } else { w }
    };
    Ok(json!({
        "docId": doc_id,
        "docNo": s("docNo"),
        "title": s("docTitle"),
        "form": s("formName"),
        "status": s("docStsName"),
        "drafter": s("empName"),
        "dept": s("deptName"),
        "repDt": s("repDt"),
        "attachCount": s("attachCnt"),
        "currentApprover": s("lineName"),
        "content": collapse_ws(&body_text),
        "approvalLine": line
    }))
}

/// 결재 문서의 첨부 목록. **문서 상태에 따라 나오는 API가 다르다**(07 §11.1):
/// - 상신됨(doc_sts 20↑): `eap111A04`의 `fileList[]`
/// - 임시보관(doc_sts 10): `eap111A04`가 **2385로 거부**("임시저장 된 문서") → `eap110A03`에
///   `docID`를 실어 부르면 `resultMap.fileAttachInfo[]`로 나온다.
///
/// 그래서 a04를 먼저 부르고 2385면 a03로 폴백한다. 호출자가 문서 상태를 미리 알 필요는 없다.
///
/// 두 배열은 **항목 스키마가 완전히 같다**(2026-09-14 실측, 19필드 전부 일치 —
/// `fileId`/`fileKey`/`fileNm`/`dispFileNm`/`fileExtsn`/`fileSize`/`fileSeq`/`fileDiv`/`docId`/
/// `createdBy`/`createdDt`/`filePath`/`oldFileId`/`linkUrl`/`etcSeq`/`modifyBy`/`modifyDt`/`verId`/`fileSizeByte`).
/// 그래서 정규화 하나로 양쪽을 처리한다.
pub async fn list_attachments(c: &GwClient, doc_id: &str, form_id: &str) -> Result<Value> {
    let a04 = c
        .call_raw(
            "/eap/eap111A04",
            &json!({
                "doc_id": doc_id, "form_id": form_id, "bindType": "V", "p_doc_id": 0,
                "doc_auth": "0", "spDocId": "", "setReadYn": "N", "commentReqYn": "N",
                "pageCode": "UBA1100", "docToken": ""
            }),
        )
        .await?;
    let code = a04.pointer("/response/resultCode").and_then(|v| v.as_i64()).unwrap_or(-1);

    let (raw, source) = if code == 0 {
        let list = a04
            .pointer("/response/resultData/fileList")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        (list, "eap111A04.fileList")
    } else if code == 2385 {
        // 임시보관 문서 — 결재작성 화면 경로로 불러온다. docID를 0이 아닌 실제 문서번호로.
        let d = c
            .call(
                "/eap/eap110A03",
                &json!({
                    "docID": doc_id.parse::<i64>().unwrap_or(0),
                    "formID": form_id, "approkey": crate::modules::approval_submit::gen_approkey(),
                    "appLineId": "", "draftTp": "", "reDraft": "", "docType": "",
                    "doc_auth": 0, "pageCode": "UBAP001"
                }),
            )
            .await?;
        let list = d
            .pointer("/resultMap/fileAttachInfo")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        (list, "eap110A03.fileAttachInfo")
    } else {
        let msg = a04
            .pointer("/response/resultMsg")
            .and_then(|v| v.as_str())
            .unwrap_or("(no msg)");
        return Err(anyhow!("eap111A04 실패: resultCode={code} msg={msg}"));
    };

    let files: Vec<Value> = raw.iter().map(normalize_attachment).collect();
    Ok(json!({ "docId": doc_id, "source": source, "count": files.len(), "files": files }))
}

/// 첨부 항목 정규화. `fileId`가 다운로드의 유일한 열쇠라 그것을 앞세운다.
/// ⚠️ 서버는 이름과 확장자를 **따로** 준다(`fileNm:"test 복사본"` + `fileExtsn:"md"`) — 합쳐야 파일명이 된다.
fn normalize_attachment(f: &Value) -> Value {
    let g = |k: &str| json_str(f.get(k));
    let ext = g("fileExtsn");
    // 이름 후보: fileNm(eap110A03·eap111A04 공통) → originalFileName(ecm001A04) → dispFileNm.
    let base = [g("fileNm"), g("originalFileName"), g("dispFileNm")]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or_default();
    let name = if ext.is_empty() || base.to_lowercase().ends_with(&format!(".{}", ext.to_lowercase())) {
        base.clone()
    } else {
        format!("{base}.{ext}")
    };
    json!({
        "fileId": g("fileId"),
        "fileName": name,
        "fileExt": ext,
        "fileSize": f.get("fileSize").cloned().unwrap_or(Value::Null),
        "fileSeq": f.get("fileSeq").cloned().unwrap_or(Value::Null)
    })
}

/// 결재 첨부 1건 다운로드 — `/ecm/ecm001A03` (07 §11.2).
///
/// **결재 전용 `moduleGbn`은 존재하지 않는다.** 12개를 전수 시도해 `BOARD`만 통과했다
/// (`EAP`/`MAIL`은 10197 권한 에러, 나머지는 10522 미등록). ECM이 파일에 `type:"eap"`를
/// 달고 있어 모듈 판별은 서버가 하고, `moduleGbn`은 권한 핸들러 선택자일 뿐이기 때문으로 보인다.
/// ⚠️ 즉 이건 게시판 권한 경로를 빌려 쓰는 것이다 — 서버가 조이면 10197로 막힐 수 있다.
///
/// 게시판·메일이 싣는 `fileSn`·`condition`은 **결재 첨부에선 무시**되므로 보내지 않는다.
pub async fn download_attachment(c: &GwClient, file_id: &str, out_path: &str) -> Result<Value> {
    let id = file_id.trim();
    if id.is_empty() {
        return Err(crate::error::InvalidInput::new("file_id가 비어있습니다 (list_approval_attachments의 files[].fileId)").into());
    }
    // ⚠️ 조용한 함정 방지(07 §11.3): fileIds에 2개 이상을 주면 서버가 에러 대신
    // **downLoad.zip**(묶음)을 내려준다. 단건을 기대한 호출자가 깨진 파일을 얻게 되므로 미리 막는다.
    if id.contains(',') {
        return Err(crate::error::InvalidInput::new(
            "file_id는 1건만 줄 수 있습니다 — 콤마로 여러 개를 주면 서버가 zip으로 묶어 보냅니다. 파일마다 따로 호출하세요.",
        )
        .into());
    }
    let auth = json!({ "fileIds": id }).to_string();
    let (size, srv_name) = c
        .download_form(
            "/ecm/ecm001A03",
            &[("moduleGbn", "BOARD"), ("authKeyMap", &auth)],
            out_path,
        )
        .await?;
    Ok(json!({
        "ok": true,
        "path": out_path,
        "bytes": size,
        "serverFileName": srv_name.unwrap_or_default()
    }))
}

/// 함별 미처리 건수 — `/eap/api/getMenuCountInfo`. companyInfo 필요(ensure_session 선행).
/// menuNo→count 맵을 사람이 읽기 쉬운 라벨로 변환.
pub async fn approval_counts(c: &GwClient) -> Result<Value> {
    let body = json!({
        "deptSeq": c.dept_seq(), "userSe": "USER|AT", "compSeq": c.comp_seq(),
        "bizSeq": c.comp_seq(), "empSeq": c.emp_seq(), "groupSeq": c.group_seq(),
        "menuType": "", "pageCode": "EapSide"
    });
    let d = c.call("/eap/api/getMenuCountInfo", &body).await?;
    let label = |mn: &str| match mn {
        "1001000" => "pending(미결)",
        "1001100" => "approved(기결)",
        "1001110" => "approved_ongoing(기결진행)",
        "1001120" => "approved_done(기결종결)",
        "1001200" => "reference(수신참조)",
        "1001400" => "enforcement(시행)",
        "1000400" => "sent(상신)",
        _ => "",
    };
    let mut counts = serde_json::Map::new();
    if let Some(obj) = d.as_object() {
        for (mn, cnt) in obj {
            let key = label(mn);
            let name = if key.is_empty() { mn.clone() } else { key.to_string() };
            counts.insert(name, cnt.clone());
        }
    }
    Ok(Value::Object(counts))
}

/// 미결함 요약. `approval_counts`가 숫자만 주는 것에 대응 — 실제로 필요한
/// "무엇을 며칠째 붙들고 있는지"를 낸다. 대기일수는 `ARRIVED_DT`(도착일) 기준.
pub async fn pending_digest(c: &GwClient, page_size: i64) -> Result<Value> {
    let listed = list_approvals(c, "pending", 1, page_size, "", "").await?;
    let docs = listed
        .get("documents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let today = {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| ((d.as_secs() as i64) + 9 * 3600) / 86400)
            .unwrap_or(0)
    };
    // YYYYMMDD → epoch days (days_to_ymd의 역함수, Howard Hinnant days_from_civil).
    let to_days = |ymd: &str| -> Option<i64> {
        if ymd.len() < 8 {
            return None;
        }
        let y: i64 = ymd[0..4].parse().ok()?;
        let m: i64 = ymd[4..6].parse().ok()?;
        let d: i64 = ymd[6..8].parse().ok()?;
        let y2 = if m <= 2 { y - 1 } else { y };
        let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
        let yoe = y2 - era * 400;
        let mp = if m > 2 { m - 3 } else { m + 9 };
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        Some(era * 146097 + doe - 719468)
    };

    let mut out: Vec<Value> = docs
        .iter()
        .map(|d| {
            let arrived = d.get("arrivedDt").and_then(|v| v.as_str()).unwrap_or("");
            let digits: String = arrived.chars().filter(|c| c.is_ascii_digit()).collect();
            let waiting = to_days(&digits).map(|a| today - a);
            json!({
                "docId": d.get("docId"),
                "formId": d.get("formId"),
                "title": d.get("title"),
                "form": d.get("form"),
                "drafter": d.get("drafter"),
                "dept": d.get("dept"),
                "arrivedDt": arrived,
                "waitingDays": waiting,
                "unread": d.get("readYn").and_then(|v| v.as_str()) == Some("N")
            })
        })
        .collect();
    // 오래 기다린 것부터
    out.sort_by_key(|d| -d.get("waitingDays").and_then(|v| v.as_i64()).unwrap_or(0));

    Ok(json!({
        "kind": "pendingDigest",
        "totalCount": listed.get("totalCount").cloned().unwrap_or(Value::Null),
        "count": out.len(),
        "oldestWaitingDays": out.first().and_then(|d| d.get("waitingDays").cloned()),
        "documents": out
    }))
}


/// 기본 조회 범위(최근 ~3개월) → (sfrDt, stoDt) YYYYMMDD. chrono 없이 SystemTime으로 계산.
fn default_range() -> (String, String) {
    let day = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() / 86400) as i64)
        .unwrap_or(0);
    (fmt_ymd(days_to_ymd(day - 92)), fmt_ymd(days_to_ymd(day)))
}




/// HTML → 대략 평문(태그 제거·엔티티 디코드). 상세 본문이 contentsWord로 안 올 때 fallback.
/// ⛔ **`board::html_to_text`와 통합 금지 — 동작이 반대다.** 이쪽(결재)은 블록 구분 없이 태그를
/// 공백 하나로 바꿔 본문을 **한 줄로 눌러** 쓴다. 게시판·메일 쪽은 개행·탭으로 구조를 보존한다.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if in_tag => {}
            _ => out.push(ch),
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// 연속 공백/개행 축약. ⛔ **`board::collapse_ws`와 통합 금지** —
/// 이쪽은 **개행까지 전부 없애** 한 줄로 만들고, 저쪽은 빈 줄을 1개까지 보존한다.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 함 이름 → API/menuNo 매핑. 실측으로 확정된 값이라 바뀌면 다른 함을 조회하게 된다.
    /// 상신함(sent)·임시보관(draft)만 **다른 API·다른 응답 경로**를 쓴다는 점이 핵심.
    #[test]
    fn box_spec은_8개_함을_매핑한다() {
        assert_eq!(box_spec("pending").unwrap(), ("eap105A04", "1000900", "1001000", "ARRIVED_DT", "map"));
        assert_eq!(box_spec("sent").unwrap(), ("eap107A04", "1000300", "1000400", "REP_DT", "list"));
        assert_eq!(box_spec("draft").unwrap(), ("eap107A06", "1000300", "1000500", "REP_DT", "list"));
        for b in ["approved", "approved_ongoing", "approved_done", "reference", "enforcement"] {
            let (api, _, menu, _, path) = box_spec(b).unwrap();
            assert_eq!(api, "eap105A04", "{b}는 수신계열 API여야 한다");
            assert_eq!(path, "map", "{b}는 resultData.map.list 경로여야 한다");
            assert!(menu.starts_with("1001"), "{b}의 menuNo가 수신계열 대역이 아니다");
        }
        assert!(box_spec("없는함").is_err());
        assert!(box_spec("").is_err());
    }

    /// ⚠️ 결재의 `html_to_text`/`collapse_ws`는 `board` 의 동명 함수와 **의도적으로 다르다** —
    /// 여기서는 블록 구분 없이 태그를 공백으로 바꾸고 개행을 전부 없앤다(본문을 한 줄로).
    /// 근거: 공통 유틸 추출(2026-08-05) 때 이 둘은 "이름만 같고 계약이 다르다"고 판단해 합치지 않았다.
    #[test]
    fn html_to_text는_블록구분_없이_공백으로만_바꾼다() {
        assert_eq!(html_to_text("<p>가</p><p>나</p>"), " 가  나 ");
        assert!(!html_to_text("가<br>나").contains('\n'), "결재 본문은 개행을 만들지 않는다");
        assert_eq!(html_to_text("&lt;a&gt;&nbsp;b"), "<a> b");
    }

    #[test]
    fn collapse_ws는_개행까지_전부_없앤다() {
        assert_eq!(collapse_ws("가\n나  다"), "가 나 다");
        assert!(!collapse_ws("가\n\n나").contains('\n'), "결재는 한 줄로 눌러야 한다");
    }

    /// 기본 조회 범위는 "오늘로부터 92일 전 ~ 오늘". 오늘에 의존하므로 불변식만 검증한다.
    #[test]
    fn default_range는_92일_구간이다() {
        let (from, to) = default_range();
        assert_eq!(from.len(), 8);
        assert_eq!(to.len(), 8);
        assert!(from < to, "시작이 종료보다 앞서야 한다");
        // 같은 함수의 날짜 계산으로 역산해 정확히 92일 차이인지 확인
        let day = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_secs() / 86400) as i64)
            .unwrap_or(0);
        assert_eq!(to, fmt_ymd(days_to_ymd(day)));
        assert_eq!(from, fmt_ymd(days_to_ymd(day - 92)));
    }
}
