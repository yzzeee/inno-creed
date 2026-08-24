//! 메일 도구 인자 스키마.
//!
//! ⚠️ **이 파일의 doc comment는 그대로 LLM에게 전달된다** — MCP 도구 스키마의 `description`이 되어
//! 모델이 인자를 채우는 유일한 근거가 된다. 문구 변경은 주석 수정이 아니라 **동작 변경**이다.

use serde::Deserialize;


#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct SendMailArgs {
    /// 받는사람 (표시형 "이름 <email>" 또는 email). **여러 명이면 콤마로 잇는다** — 예:
    /// `"홍길동 <hong@innogrid.com>,kim@innogrid.com"`. 미지정 시 본인에게 발송.
    #[serde(default)]
    pub to: Option<String>,
    /// 참조(cc, 선택). 형식은 to와 같다(콤마 구분). 비우면 참조 없음.
    #[serde(default)]
    pub cc: Option<String>,
    /// 숨은참조(bcc, 선택). 형식은 to와 같다(콤마 구분). 비우면 숨은참조 없음.
    /// ⚠️ 숨은참조는 다른 수신자에게 보이지 않지만 **발송 자체는 되돌릴 수 없다.**
    #[serde(default)]
    pub bcc: Option<String>,
    /// 제목
    pub subject: String,
    /// 본문(필수). 일반 텍스트 또는 Markdown. HTML 태그는 사용하지 않는다.
    /// HTML 변환·서명 삽입·저장 본문 검증은 도구가 처리한다.
    #[serde(deserialize_with = "deserialize_body")]
    pub body: String,
    /// 첨부할 로컬 파일 경로 목록(선택, 절대경로). 비우면 첨부 없음.
    #[serde(default)]
    pub attachments: Vec<String>,
    /// 서명 자동 첨부 여부(선택, **기본 true**). 아마란스에 등록해 둔 서명을 본문 끝에 붙여
    /// **웹에서 보낼 때와 같은 형상**으로 만든다 — 사람이 보내는 메일이면 켜 두는 것이 맞다.
    /// 서명이 등록돼 있지 않으면 아무것도 붙지 않는다(에러 아님).
    /// `false`는 시스템 알림·자동화처럼 서명이 없어야 하는 발송에만 쓴다.
    #[serde(default = "super::yes")]
    pub signature: bool,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct SaveMailDraftArgs {
    /// 받는사람 (표시형 "이름 <email>" 또는 email). **여러 명이면 콤마로 잇는다** — 예:
    /// `"홍길동 <hong@innogrid.com>,kim@innogrid.com"`. 미지정 시 본인.
    #[serde(default)]
    pub to: Option<String>,
    /// 참조(cc, 선택). 형식은 to와 같다(콤마 구분). 비우면 참조 없음.
    /// 여기 넣어두면 send_mail_from_draft가 **그대로 승계해** 발송한다.
    #[serde(default)]
    pub cc: Option<String>,
    /// 숨은참조(bcc, 선택). 형식은 to와 같다(콤마 구분). 비우면 숨은참조 없음.
    /// 여기 넣어두면 send_mail_from_draft가 **그대로 승계해** 발송한다.
    #[serde(default)]
    pub bcc: Option<String>,
    /// 제목. 비워두면 "(제목없음)"으로 저장된다.
    pub subject: String,
    /// 본문(필수). 일반 텍스트 또는 Markdown. HTML 태그는 사용하지 않는다.
    /// HTML 변환·서명 삽입·저장 본문 검증은 도구가 처리한다.
    #[serde(deserialize_with = "deserialize_body")]
    pub body: String,
    /// 첨부할 로컬 파일 경로 목록(선택, 절대경로). 비우면 첨부 없음.
    #[serde(default)]
    pub attachments: Vec<String>,
    /// 서명 자동 첨부 여부(선택, **기본 true**). 아마란스에 등록해 둔 서명을 본문 끝에 붙여
    /// **웹에서 보낼 때와 같은 형상**으로 저장한다 — 초안은 사람이 확인하고 보내는 것이므로
    /// 켜 두는 것이 맞다. 여기 붙은 서명은 send_mail_from_draft가 **본문째로 승계**한다
    /// (그 도구는 서명을 다시 붙이지 않는다 — 두 번 붙는 일이 없다).
    /// 서명이 등록돼 있지 않으면 아무것도 붙지 않는다(에러 아님).
    #[serde(default = "super::yes")]
    pub signature: bool,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub struct SendMailFromDraftArgs {
    /// 발송할 임시보관 메일의 muid. save_mail_draft의 `draft_muid` 또는 list_mail_drafts 결과의 muid.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub draft_muid: String,
    /// 받는사람(선택, 표시형 "이름 <email>" 또는 email). 미지정 시 **초안에 저장된 수신자**로 보낸다.
    /// 초안에 수신자가 없으면 에러가 나므로 그때 지정할 것.
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DeleteMailArgs {
    /// 삭제할 메일 muid 목록(콤마 구분). list_mail_inbox의 muid 사용.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub uids: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MoveMailArgs {
    /// 옮길 메일 muid 목록(콤마 구분). list_mail_inbox의 muid 사용.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub uids: String,
    /// 목적지 메일함(폴더) 이름. list_mailboxes의 name 사용(예: "자료", "SECLOUDIT", "INBOX").
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub to_box: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct CreateMailboxArgs {
    /// 만들 메일함(폴더) 이름. 최상위에 생성된다.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub name: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct RenameMailboxArgs {
    /// 현재 메일함(폴더) 이름. list_mailboxes의 name.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub name: String,
    /// 새 이름.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub new_name: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DeleteMailboxArgs {
    /// 삭제할 메일함(폴더) 이름. list_mailboxes의 name. ⚠️ 폴더가 실제로 사라진다.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub name: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SetMailRuleArgs {
    /// 조건에 맞는 메일을 옮길 폴더 이름. list_mailboxes의 name(예: "뉴스레터", "Jira").
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub to_box: String,
    /// 조건 필드: "mailfrom"(발신자 주소) / "mailfromdomain"(발신 도메인) / "subject"(제목). sender/domain/제목 등 별칭도 가능.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub field: String,
    /// 매칭할 값(예: "info@kcloud.or.kr", "kcloud.or.kr", "입사인사").
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub match_value: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DeleteMailRuleArgs {
    /// 삭제할 규칙 ID. list_mail_rules 결과의 autoDivSeq.
    pub auto_div_seq: i64,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ReadMailArgs {
    /// 메일 muid. list_mail_inbox 결과의 muid 사용.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub muid: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MarkMailUnreadArgs {
    /// 읽지 않음으로 되돌릴 메일 muid. list_mail_inbox 결과의 muid 사용.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub muid: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DownloadMailAttachmentArgs {
    /// 메일 muid. read_mail/list_mail_inbox의 muid.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub muid: String,
    /// read_mail 응답 `attachments[].fileSn` 의 토큰 문자열을 **그대로** 붙여넣는다.
    /// ⚠️ 순번(0,1,2)이 아니다 — 숫자를 주면 서버가 422로 거절한다.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub file_sn: String,
    /// 저장 경로(절대경로 권장). 예: /tmp/attach.png
    pub out_path: String,
}

// 파싱 단계에서 거부하므로 세션 확보·첨부 업로드·발송 모두 실행되지 않는다.
fn deserialize_body<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let body = String::deserialize(d)?;
    crate::modules::mail::render_body(&body).map_err(serde::de::Error::custom)?;
    Ok(body)
}
