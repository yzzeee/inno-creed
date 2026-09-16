//! 게시판(UF) 모듈 — `/board/APIHandler/*`. 읽기 전용(목록·상세).
//! 인증은 헤더 서명만으로 완결(companyInfo 불필요).

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::client::GwClient;
use crate::error::InvalidInput;
// json_str은 util 소유. mail 모듈이 `board::json_str`로 참조해 온 경로를 유지하려고 재수출한다.
pub(crate) use crate::util::json_str;
use crate::util::digits_only;

/// 최근 공지/게시글 목록 — `ViewBoardNewAndNoticeArtList`.
/// `use_list_art_content:Y`로 본문 프리뷰까지 내려온다. 결과는 유용 필드만 추려서 반환.
///
/// 필터(빈 문자열이면 미적용):
/// - `search`: 검색어. `field`로 대상 지정 — "title"(제목)/"content"(내용)/"author"(작성자),
///   그 외/빈값은 통합검색(searchTotal).
/// - `start_date`/`end_date`: 등록일 범위(YYYY-MM-DD).
///
/// 게시판별 필터는 이 엔드포인트(전 게시판 공지 집계)에서 지원되지 않는다 — searchBoard를
/// 넣어도 무시됨(실측). 출력의 `boardId`(cat_seq_no)로 클라이언트단 구분만 가능.
#[allow(clippy::too_many_arguments)]
pub async fn list_notices(
    c: &GwClient,
    page: i64,
    page_size: i64,
    search: &str,
    field: &str,
    start_date: &str,
    end_date: &str,
) -> Result<Value> {
    let (total, title, desc, nick) = match field {
        "title" => ("", search, "", ""),
        "content" => ("", "", search, ""),
        "author" => ("", "", "", search),
        _ => (search, "", "", ""),
    };
    // 서버는 날짜를 YYYYMMDD(구분자 없음)만 받는다 — 대시가 있으면 500. 숫자만 남긴다.
    let start_date = digits_only(start_date);
    let end_date = digits_only(end_date);
    let body = json!({
        "adminPage": "N", "searchAuthType": "U",
        "searchTotal": total, "searchTitle": title, "searchNick": nick, "searchDesc": desc,
        "searchBoard": "", "searchRemarkNo": "", "searchEtcValue": "",
        "searchStartDate": start_date, "searchEndDate": end_date, "searchStartTerm": "", "searchEndTerm": "",
        "eventStatus": "", "reserveStatus": "", "counselingOk": "", "searchMailFrom": "",
        "sort": "write_date", "project_id": Value::Null, "page": page, "pageSize": page_size,
        "sortType": "desc", "menuCode": "UFA", "pageCode": "UFA1000", "moduleCode": "UF",
        "noticeYn": "Y", "apiName": "ViewBoardNewAndNoticeArtList", "use_list_art_content": "Y"
    });
    let data = c
        .call("/board/APIHandler/ViewBoardNewAndNoticeArtList", &body)
        .await?;

    let s = |v: &Value, k: &str| json_str(v.get(k));
    let articles: Vec<Value> = data
        .get("articleList")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|a| {
                    let preview = collapse_ws(&s(a, "art_content"));
                    json!({
                        "artSeqNo": s(a, "art_seq_no"),
                        "title": s(a, "art_title"),
                        "board": s(a, "cat_title"),
                        "boardId": s(a, "cat_seq_no"),
                        "writer": s(a, "mbr_nick"),
                        "dept": s(a, "dept_name"),
                        "writeDate": s(a, "write_date"),
                        "readCnt": s(a, "read_cnt"),
                        // ⚠️ 숫자로 낸다 — 문자열이면 "0"이 truthy라 "첨부 있음" 판정이 조용히 틀린다(실측 사고).
                        "fileCnt": file_cnt(a),
                        "attachmentUid": s(a, "uid"),
                        "isNew": s(a, "is_new_yn") == "Y",
                        "read": s(a, "art_read_yn") == "Y",
                        "preview": truncate(&preview, 200)
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(json!({ "totalCnt": data.get("totalCnt").cloned().unwrap_or(Value::Null), "articles": articles }))
}

/// 첨부 개수 — **숫자**로 통일. 서버는 `file_cnt`를 문자열("0"/"3")로도 정수로도 준다.
/// 그대로 흘리면 호출자가 `"0"`을 참으로 읽어 첨부 없는 글을 첨부 있는 글로 취급한다.
fn file_cnt(v: &Value) -> Value {
    json!(json_str(v.get("file_cnt")).trim().parse::<i64>().unwrap_or(0))
}

/// 게시글의 본문 이미지 경로 전부 — 본문 파싱 결과 + 서버가 준 `img_path`(중복이면 안 넣는다).
/// 순서는 본문 등장 순서를 유지하고, 파싱이 놓친 `img_path`만 맨 앞에 붙인다.
fn body_images(art: &Value) -> Vec<String> {
    let mut list = extract_img_srcs(&json_str(art.get("art_content")));
    let server = json_str(art.get("img_path"));
    if !server.is_empty() && !list.contains(&server) {
        list.insert(0, server);
    }
    list
}

/// 게시글 상세 — `ViewPost`. 본문(HTML→텍스트)·댓글 포함.
/// ⚠️ 호출 시 조회수가 증가한다(실제 열람 처리) — 순수 조회는 아님.
pub async fn read_post(c: &GwClient, art_seq_no: &str) -> Result<Value> {
    let body = json!({
        "art_seq_no": art_seq_no, "adminPage": "N", "externalYn": "N",
        "menuCode": "UFA", "pageCode": "UFA1000", "moduleCode": "UF",
        "presentPassword": "", "isPrint": "N", "searchParams": Value::Null
    });
    let data = c.call("/board/APIHandler/ViewPost", &body).await?;
    let art = data
        .get("art")
        .ok_or_else(|| anyhow!("ViewPost 응답에 art 없음 (art_seq_no={art_seq_no})"))?;

    let s = |v: &Value, k: &str| json_str(v.get(k));
    // 댓글 본문 필드명은 미실측(캡처한 글은 댓글 0개) → 유력 키를 방어적으로 탐색.
    let first = |v: &Value, keys: &[&str]| {
        keys.iter()
            .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
            .unwrap_or_default()
            .to_string()
    };
    let comments: Vec<Value> = data
        .get("remarkList")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|r| {
                    json!({
                        "writer": s(r, "mbr_nick"),
                        "writeDate": s(r, "write_date"),
                        "content": collapse_ws(&html_to_text(
                            &first(r, &["remark_desc", "remark_content", "content", "art_content"])
                        ))
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // 상세의 art.cat_title은 null → 게시판명은 resultData.board.cat_title에 있음.
    let board_name = json_str(data.get("board").and_then(|b| b.get("cat_title")));
    Ok(json!({
        "artSeqNo": s(art, "art_seq_no"),
        "title": s(art, "art_title"),
        "board": board_name,
        "writer": s(art, "mbr_nick"),
        "dept": s(art, "dept_name"),
        "writeDate": s(art, "write_date"),
        "readCnt": s(art, "read_cnt"),
        "fileCnt": file_cnt(art),
        "attachmentUid": s(art, "uid"),
        // 본문 삽입 이미지 경로 전부(등장 순서 = `content`의 `[이미지]` 순서).
        // **정식 첨부가 아니므로 `fileCnt`에 안 잡힌다** — 첨부 0건이면서 이미지가 있는 글이
        // 실재한다(실측: 3006은 fileCnt=0, 이미지 1장). `download_body_image`에 그대로 넘긴다.
        // 서버의 `img_path`(첫 장만 주는 필드)도 합쳐 둔다 — 본문 파싱이 놓쳐도 남게.
        "images": body_images(art),
        "content": collapse_ws(&html_to_text(&s(art, "art_content"))),
        "comments": comments
    }))
}

/// 게시글 첨부파일 목록 — `/ecm/ecm001A04`(x-www-form-urlencoded, authKeyMap). `art_seq_no`는
/// 게시글 번호, `uid`는 게시글의 attachmentUid(fileIds). 파일별 fileId/이름/확장자/크기/저장경로 반환.
/// (실측: `moduleGbn=BOARD&authKeyMap={empSeq,cat_seq_no:"U",art_seq_no,fileIds}&fileSn=0&condition=99`)
pub async fn list_attachments(c: &GwClient, art_seq_no: &str, uid: &str) -> Result<Value> {
    if split_uids(uid).is_empty() {
        return Ok(json!({ "files": [] }));
    }
    let auth = json!({
        "empSeq": c.emp_seq(),
        "cat_seq_no": "U",
        "art_seq_no": art_seq_no,
        "survey_no": "",
        "reply_seq_no": "",
        "fileIds": uid
    })
    .to_string();
    let data = c
        .call_form(
            "/ecm/ecm001A04",
            &[
                ("moduleGbn", "BOARD"),
                ("authKeyMap", &auth),
                ("fileSn", "0"),
                ("condition", "99"),
            ],
        )
        .await?;
    let list = data
        .get("list")
        .or_else(|| data.get("storageList"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let files: Vec<Value> = list
        .iter()
        .enumerate()
        .map(|(i, f)| {
            json!({
                // ⚠️ download_attachment 의 file_sn 이 곧 이 값이다. 호출자가 배열을 직접 세지 않게
                // 인덱스를 실어 보낸다(세다가 틀리면 엉뚱한 파일을 받는다).
                "fileSn": i,
                "fileId": json_str(f.get("fileId").or_else(|| f.get("fileUID"))),
                "fileName": json_str(f.get("originalFileName").or_else(|| f.get("fileName"))),
                "fileExt": json_str(f.get("fileExtsn").or_else(|| f.get("fileExt"))),
                "fileSize": json_str(f.get("fileSize")),
                "storagePath": json_str(f.get("linkedFilePath"))
            })
        })
        .collect();
    Ok(json!({ "files": files }))
}

/// 게시글 첨부파일 다운로드 — `/ecm/ecm001A03`(x-www-form-urlencoded, authKeyMap, 서명헤더 필수).
/// `art_seq_no`/`uid`는 목록과 동일, `file_sn`은 목록에서 받은 파일 순번(0-base, 단건 다운로드).
/// `out_path`에 바이트 저장. 서버가 JSON 봉투를 주면(=에러) 실패 처리.
pub async fn download_attachment(
    c: &GwClient,
    art_seq_no: &str,
    uid: &str,
    file_sn: i64,
    out_path: &str,
) -> Result<Value> {
    if split_uids(uid).is_empty() {
        anyhow::bail!("download_attachment: uid 비어있음");
    }
    let auth = json!({
        "empSeq": c.emp_seq(),
        "cat_seq_no": "U",
        "art_seq_no": art_seq_no,
        "survey_no": "",
        "reply_seq_no": "",
        "fileIds": uid
    })
    .to_string();
    let sn = file_sn.to_string();
    let (size, filename) = c
        .download_form(
            "/ecm/ecm001A03",
            &[
                ("moduleGbn", "BOARD"),
                ("authKeyMap", &auth),
                ("fileSn", &sn),
                ("condition", "99"),
            ],
            out_path,
        )
        .await?;
    Ok(json!({
        "ok": true,
        "path": out_path,
        "bytes": size,
        "serverFileName": filename
    }))
}


/// 콤마 구분 uid 문자열 → 개별 uid 벡터(공백/빈값 제거).
fn split_uids(uid: &str) -> Vec<String> {
    uid.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}


/// 본문 삽입 이미지가 실려 오는 엔드포인트. **다운로드 허용 목록**이며 실측된 둘뿐이다
/// (`.claude-workspace/analyze/captures/` 의 `board-viewpost_3006.json`,
/// `mail-inline-img_mail002A01.json` — git 미추적).
const BODY_IMAGE_PREFIXES: [&str; 2] = ["/gw/contentsImgController/download/", "/mail/mail002A30"];

/// 본문 HTML의 `<img src>` 목록(등장 순서 보존). 같은 이미지를 두 번 쓰면 두 번 담는다 —
/// 평문의 `[이미지]` 자리표시자와 **개수가 맞아야** 호출자가 n번째 이미지를 짚을 수 있다.
pub(crate) fn extract_img_srcs(html: &str) -> Vec<String> {
    let lower = html.to_ascii_lowercase(); // ASCII만 바뀌므로 바이트 오프셋은 원문과 같다
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = lower[i..].find("<img") {
        let start = i + rel;
        let Some(rel_end) = html[start..].find('>') else { break };
        let end = start + rel_end;
        if let Some(src) = img_src(&html[start..end]) {
            out.push(src);
        }
        i = end + 1;
    }
    out
}

/// `<img ...>` 태그 문자열에서 `src` 값. 따옴표는 홑/겹 둘 다, 없어도 받는다.
/// `data-src` 같은 다른 속성에 걸리지 않게 **앞이 공백인지**를 본다.
fn img_src(tag: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("src") {
        let p = from + rel;
        let boundary = p == 0 || lower.as_bytes()[p - 1].is_ascii_whitespace();
        let after = &tag[p + 3..];
        // `srcset=`은 여기서 걸러진다 — "src" 뒤가 바로 `=`가 아니다.
        if boundary
            && let Some(v) = after.trim_start().strip_prefix('=')
        {
            let v = v.trim_start();
            let val = match v.chars().next() {
                Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or(""),
                _ => v.split_whitespace().next().unwrap_or(""),
            };
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
        from = p + 3;
    }
    None
}

/// 본문 이미지 src가 **이 서버에서 받아올 수 있는 것**인지. 외부 호스트는 전부 거짓.
pub(crate) fn is_body_image(src: &str) -> bool {
    gw_image_path(src).is_some()
}

/// `..` 세그먼트가 있으면 접두어 검사를 **통과한 뒤 다른 경로로 착지한다** — URL 파서가 요청
/// 직전에 정규화하기 때문이다(실측: `/gw/contentsImgController/download/../../../ecm/ecm001A05`
/// → `/ecm/ecm001A05` = 삭제 API). 파서는 퍼센트 인코딩된 `%2e`도 점으로 해석하므로 같이 막는다.
/// src는 메일 본문에서 오고 **메일은 남이 보낸다** — 이 경로는 신뢰 입력이 아니다.
fn has_dot_segment(path: &str) -> bool {
    path.split('?')
        .next()
        .unwrap_or("")
        .split('/')
        .any(|seg| seg.to_ascii_lowercase().replace("%2e", ".") == "..")
}

/// src → 우리 서버 경로. 절대 URL이면 gw 호스트일 때만, 그리고 허용 목록에 든 경로만 통과.
fn gw_image_path(src: &str) -> Option<&str> {
    let p = match src.strip_prefix("https://gw.innogrid.com") {
        Some(rest) => rest,
        None if src.starts_with('/') => src,
        None => return None,
    };
    if has_dot_segment(p) {
        return None;
    }
    BODY_IMAGE_PREFIXES.iter().any(|q| p.starts_with(q)).then_some(p)
}

/// 본문 삽입 이미지 다운로드 — **게시판·메일 공용**(둘 다 같은 서명 요청 하나로 받아진다).
/// `src`는 `read_notice`의 `images[]` 또는 `read_mail`의 `inlineImages[]` 값을 그대로.
///
/// ⚠️ **경로를 무제한으로 받지 않는다.** `download_form`은 서명 POST라, 임의 경로를 허용하면
/// 다운로드를 가장해 부작용 있는 API를 때릴 수 있다(예: `ecm001A05`는 삭제다). 실측된 이미지
/// 엔드포인트 둘만 통과시킨다.
/// ⚠️ **외부 호스트는 받지 않는다.** 추적 픽셀을 대신 열어주는 꼴이 되고, `read_mail`이 지켜온
/// "외부 리소스를 자동 fetch하지 않는다"는 정책을 도구가 우회로로 뚫는 셈이 된다.
pub async fn download_body_image(c: &GwClient, src: &str, out_path: &str) -> Result<Value> {
    let path = gw_image_path(src).ok_or_else(|| {
        InvalidInput(format!(
            "본문 이미지 경로가 아니다: {src}\n\
             read_notice의 images[] 또는 read_mail의 inlineImages[] 값을 그대로 줄 것. \
             외부 호스트 이미지(서명 로고·추적 픽셀 등)는 의도적으로 받지 않는다."
        ))
    })?;
    let (size, filename) = c.download_form(path, &[], out_path).await?;
    Ok(json!({
        "ok": true,
        "path": out_path,
        "bytes": size,
        "serverFileName": filename,
        "source": path
    }))
}

/// 게시글 HTML 본문을 대략적인 평문으로 변환(외부 크레이트 없이). 블록 경계는 개행으로,
/// 태그 제거, 주요 엔티티 디코드. 완벽한 렌더링이 아니라 에이전트 읽기용 근사.
/// ⛔ **`approval::html_to_text`와 통합 금지 — 동작이 반대다.** 이쪽(게시판·메일)은 블록 태그
/// (br/p/div/tr/li/h1~h3)를 개행으로, td/th를 탭으로 바꿔 **구조를 보존**한다. 결재 쪽은 모든
/// 태그를 공백 하나로 눌러 한 줄로 만든다. 이름이 같다고 합치면 본문 표시가 조용히 바뀐다.
pub(crate) fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' => {
                in_tag = false;
                let t = tag.to_ascii_lowercase();
                let t = t.trim_start_matches('/').trim();
                if t.starts_with("br")
                    || t.starts_with("p")
                    || t.starts_with("div")
                    || t.starts_with("tr")
                    || t.starts_with("li")
                    || t.starts_with("h1")
                    || t.starts_with("h2")
                    || t.starts_with("h3")
                {
                    out.push('\n');
                } else if t.starts_with("td") || t.starts_with("th") {
                    out.push('\t');
                } else if t.starts_with("img") {
                    // 이미지는 태그와 함께 사라지면 **흔적이 0**이라, 본문이 통째로 비어 보인다
                    // (실측: 게시글 3006은 본문의 실질이 이미지 한 장인데 평문화 결과에 아무것도
                    // 남지 않았다). 자리표시자로 존재와 위치를 남긴다.
                    out.push_str("[이미지]");
                }
            }
            _ if in_tag => tag.push(ch),
            _ => out.push(ch),
        }
    }
    decode_entities(&out)
}

/// HTML 엔티티 디코드 — 이름 표기(`&nbsp;`)와 **숫자 표기(`&#160;`/`&#xA0;`)를 함께** 처리한다.
///
/// ⚠️ 숫자 표기를 일반 처리하는 이유: 같은 문자를 어느 표기로 쓸지는 그 HTML을 만든 프로그램이
/// 정하고 우리는 통제하지 못한다(외부 메일·게시글). 예전엔 이름 5종 + `&#39;` 하나만 치환했는데,
/// 그 하나가 박혀 있다는 것 자체가 두더지잡기의 흔적이었다 — 실측에서 Teams 알림 메일이
/// `&#160;`을 123회 흘려보냈고(`captures/mail-*`), 디코드되지 않은 탓에 여백이 되지 못하고
/// `&#160;` 글자 그대로 본문에 남았다. 종류별로 막지 않고 표기 자체를 해석해 그 부류를 닫는다.
///
/// 엔티티가 아닌 `&`(예: "A & B")는 건드리지 않는다 — `;`가 8바이트 안에 없으면 그냥 흘린다.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(j) = tail[1..].find(';').filter(|&j| j <= 8)
            && let Some(c) = entity_char(&tail[1..1 + j])
        {
            out.push(c);
            rest = &tail[1 + j + 1..];
        } else {
            // 엔티티로 해석되지 않으면 `&` 한 글자만 흘리고 그 뒤에서 다시 찾는다.
            out.push('&');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// 엔티티 본문(`&`와 `;` 사이) → 문자. 모르는 이름은 `None`(원문 보존).
/// nbsp는 **평범한 공백으로 낮춘다** — 이름·숫자 어느 표기로 왔든 같게 보이도록, 그리고
/// `collapse_ws`가 접어낼 수 있도록(U+00A0 그대로 두면 유니코드 공백이라 접히긴 하나,
/// `collapse_ws`를 안 거치는 제목·발신자에는 눈에 안 보이는 문자로 남는다).
fn entity_char(body: &str) -> Option<char> {
    let c = match body {
        "nbsp" => '\u{a0}',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "amp" => '&',
        _ => {
            let num = body.strip_prefix('#')?;
            let n = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(n)?
        }
    };
    Some(if c == '\u{a0}' { ' ' } else { c })
}

/// 연속 공백/개행 정리(태그 제거 후 남는 과도한 공백 축약). 빈 줄은 최대 1개까지 유지.
/// ⛔ **`approval::collapse_ws`와 통합 금지** — 이쪽은 **개행을 보존**하고 저쪽은 전부 없앤다.
pub(crate) fn collapse_ws(s: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in s.split('\n') {
        let t = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() {
            if matches!(lines.last(), Some(l) if !l.is_empty()) {
                lines.push(String::new());
            }
        } else {
            lines.push(t);
        }
    }
    lines.join("\n").trim().to_string()
}

/// 문자 기준 잘라내기(멀티바이트 안전, 말줄임 추가).
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let cut: String = s.chars().take(max_chars).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ 이 모듈의 `html_to_text`/`collapse_ws`는 `approval` 의 동명 함수와 **의도적으로 다르다**
    /// (게시판·메일 본문은 줄바꿈을 살리고, 결재 본문은 한 줄로 누른다).
    /// 이름이 같다고 합치면 게시판/메일 본문 표시가 조용히 바뀐다 — 그걸 막는 테스트다.
    #[test]
    fn html_to_text는_블록구조를_개행으로_살린다() {
        assert_eq!(html_to_text("<p>가</p><p>나</p>"), "\n가\n\n나\n");
        assert_eq!(html_to_text("가<br>나"), "가\n나");
        assert_eq!(html_to_text("<td>가</td><td>나</td>"), "\t가\t\t나\t"); // 셀은 탭
        assert_eq!(html_to_text("<span>가</span>나"), "가나");            // 인라인은 그대로
    }

    /// 이미지는 태그와 함께 지워지면 흔적이 0이라, 본문의 실질이 이미지인 글이 빈 글로 보인다.
    /// 실측 원문(`.claude-workspace/analyze/captures/board-viewpost_3006.json`)의 img 태그를 그대로 쓴다.
    #[test]
    fn html_to_text는_이미지_자리를_남긴다() {
        let real = r#"<img src="/gw/contentsImgController/download/gcmsAmaranth31433/editorImg/fc4aa722-ab83-4527-91da-342140303c5c_png" width="1156" height="821" style="width:100%;height:100%;">"#;
        assert_eq!(html_to_text(real), "[이미지]");
        assert_eq!(html_to_text("앞<img src=x>뒤"), "앞[이미지]뒤");
        // 상대경로라 메일의 `count_remote_resources`(src="http)로는 안 잡힌다 → 자리표시자가 유일한 신호.
        assert!(!html_to_text(real).is_empty());
    }

    #[test]
    fn html_to_text는_엔티티를_디코드한다() {
        assert_eq!(html_to_text("a&nbsp;b"), "a b");
        assert_eq!(html_to_text("&lt;tag&gt;"), "<tag>");
        assert_eq!(html_to_text("&quot;q&quot; &#39;s&#39; &amp;"), "\"q\" 's' &");
    }

    /// 숫자 표기는 이름 표기와 **같은 문자**다 — 어느 쪽으로 오든 결과가 같아야 한다.
    /// (실측: Teams 알림 메일이 `&#160;`만 123회 흘렸고 그대로 본문에 글자로 남았다.)
    #[test]
    fn html_to_text는_숫자_엔티티도_디코드한다() {
        assert_eq!(html_to_text("a&#160;b"), "a b"); // &nbsp; 와 동일 결과
        assert_eq!(html_to_text("a&#xA0;b"), "a b"); // 16진도 동일
        assert_eq!(html_to_text("&#8211;&#8203;&#8220;"), "–\u{200b}“");
        assert_eq!(collapse_ws(&html_to_text("가&#160;&#160;&#160;나")), "가 나"); // 접혀서 사라짐
    }

    /// `images[]`의 순서·개수가 평문의 `[이미지]`와 어긋나면 n번째 이미지를 짚을 수 없다.
    #[test]
    fn extract_img_srcs는_등장순서대로_전부_뽑는다() {
        let html = r#"<p>가</p><img src="/a.png"><p>나</p><img src='/b.png'><img src=/c.png >"#;
        assert_eq!(extract_img_srcs(html), ["/a.png", "/b.png", "/c.png"]);
        assert_eq!(html_to_text(html).matches("[이미지]").count(), 3); // 자리표시자와 개수 일치
        assert_eq!(extract_img_srcs("<img src=\"/x.png\"><img src=\"/x.png\">").len(), 2); // 중복도 둘
        assert!(extract_img_srcs("<p>이미지 없음</p>").is_empty());
        assert!(extract_img_srcs("<img data-src=\"/no.png\">").is_empty()); // 다른 속성에 안 걸린다
        assert!(extract_img_srcs("<img srcset=\"/no.png 2x\">").is_empty());
        assert_eq!(extract_img_srcs("<img alt=\"사진\" src=\"/한글.png\">"), ["/한글.png"]); // 멀티바이트
    }

    /// ⛔ 허용 목록이 무너지면 서명 POST로 임의 API를 때릴 수 있게 된다(예: ecm001A05=삭제).
    /// 그리고 외부 호스트를 통과시키면 추적 픽셀을 대신 열어주는 꼴이 된다.
    #[test]
    fn is_body_image는_허용된_두_경로만_통과시킨다() {
        assert!(is_body_image("/gw/contentsImgController/download/gcms/editorImg/x_png"));
        assert!(is_body_image("/mail/mail002A30?domain=innogrid.com&path=abc&index=0"));
        assert!(is_body_image("https://gw.innogrid.com/mail/mail002A30?index=0")); // 절대 URL도
        assert!(!is_body_image("https://www.innogrid.com/api/v1/file/download/x")); // 외부 호스트
        assert!(!is_body_image("https://evil.example.com/gw/contentsImgController/download/x"));
        assert!(!is_body_image("/ecm/ecm001A05")); // 삭제 API
        assert!(!is_body_image("/mail/mail002A05")); // 메일 삭제
        assert!(!is_body_image(""));
    }

    /// ⛔ 접두어만 보면 `..`로 걸어 나갈 수 있다 — URL 파서가 요청 직전에 정규화하므로
    /// 검사를 통과한 경로가 삭제 API에 착지한다(넷 다 실측으로 `/ecm/ecm001A05`가 됐다).
    #[test]
    fn is_body_image는_상위경로_탈출을_막는다() {
        assert!(!is_body_image("/gw/contentsImgController/download/../../../ecm/ecm001A05"));
        assert!(!is_body_image("/mail/mail002A30/../../ecm/ecm001A05?x=1"));
        // 퍼센트 인코딩도 파서가 점으로 해석한다(대소문자 무관).
        assert!(!is_body_image("/gw/contentsImgController/download/%2e%2e/%2e%2e/ecm/ecm001A05"));
        assert!(!is_body_image("/gw/contentsImgController/download/%2E%2E/%2E%2E/ecm/ecm001A05"));
        // 파일명에 점이 붙은 정상 경로까지 막으면 안 된다.
        assert!(is_body_image("/gw/contentsImgController/download/gcms/editorImg/a..b_png"));
    }

    /// 엔티티가 아닌 `&`를 먹어치우면 본문이 조용히 망가진다.
    #[test]
    fn decode_entities는_엔티티_아닌_앰퍼샌드를_보존한다() {
        assert_eq!(html_to_text("A &amp; B"), "A & B");
        assert_eq!(html_to_text("A & B"), "A & B"); // 맨 `&`
        assert_eq!(html_to_text("a & b; c"), "a & b; c"); // `;`가 뒤에 있어도 엔티티 아님
        assert_eq!(html_to_text("&amp;lt;"), "&lt;"); // 이중 디코드 금지
        assert_eq!(html_to_text("&#99999999;"), "&#99999999;"); // 코드포인트 범위 밖
        assert_eq!(html_to_text("&unknown;"), "&unknown;"); // 모르는 이름
        assert_eq!(html_to_text("&"), "&");
    }

    #[test]
    fn collapse_ws는_빈줄을_최대_한개만_남긴다() {
        assert_eq!(collapse_ws("가\n\n\n\n나"), "가\n\n나");
        assert_eq!(collapse_ws("  가   나  "), "가 나");
        assert_eq!(collapse_ws("\n\n가\n\n"), "가"); // 앞뒤는 trim
        assert!(collapse_ws("가\n나").contains('\n'), "게시판/메일은 개행을 보존해야 한다");
    }

    #[test]
    fn truncate는_바이트가_아니라_문자로_자른다() {
        assert_eq!(truncate("가나다라마", 3), "가나다…");
        assert_eq!(truncate("가나다", 3), "가나다");   // 경계값: 자르지 않음
        assert_eq!(truncate("가나다", 10), "가나다");
        assert_eq!(truncate("", 3), "");
        assert_eq!(truncate("가나다라", 0), "…");
        // 멀티바이트를 바이트로 자르면 패닉이 난다 — 안 나는 것 자체가 계약이다.
        assert_eq!(truncate("🙂🙂🙂", 2).chars().count(), 3); // 이모지 2 + 말줄임
    }

    #[test]
    fn split_uids는_공백과_빈값을_거른다() {
        assert_eq!(split_uids("a,b,c"), vec!["a", "b", "c"]);
        assert_eq!(split_uids(" a , b "), vec!["a", "b"]);
        assert_eq!(split_uids("a,,b,"), vec!["a", "b"]);
        assert!(split_uids("").is_empty());
        assert!(split_uids(" , ").is_empty());
    }
}
