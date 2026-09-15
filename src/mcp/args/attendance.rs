//! 근태 도구 인자 스키마.
//!
//! ⚠️ **이 파일의 doc comment는 그대로 LLM에게 전달된다** — MCP 도구 스키마의 `description`이 되어
//! 모델이 인자를 채우는 유일한 근거가 된다. 문구 변경은 주석 수정이 아니라 **동작 변경**이다.

use serde::Deserialize;


#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AttendanceMonthArgs {
    /// 조회 월 YYYYMM (예: "202608"). start/end를 주면 그쪽이 우선.
    #[serde(default)]
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub month: String,
    /// 시작일 YYYYMMDD(선택)
    #[serde(default)]
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub start: String,
    /// 종료일 YYYYMMDD(선택)
    #[serde(default)]
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub end: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AttendanceTodayArgs {
    /// 조회할 날짜 YYYYMMDD(선택, 비우면 오늘 KST).
    #[serde(default)]
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub work_dt: String,
}

/// `cancel_attendance_application` 인자.
///
/// ⚠️ 이 도구는 대상을 **날짜로** 찾는다 — 호출자가 appSq·detailSq·linkKey·formId 같은 내부
/// 식별자를 알 필요가 없다. 같은 날 신청이 여럿일 때만 `app_sq`로 하나를 지목한다(그 값도
/// 에러 메시지가 후보 목록으로 알려준다).
#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct CancelAttendanceArgs {
    /// 취소할 근태가 **적용되는 날**(YYYYMMDD). 상신한 날이 아니라 휴가·출장 당일이다.
    #[serde(deserialize_with = "super::flex_string")]
    #[schemars(schema_with = "super::flex_str_schema")]
    pub date: String,
    /// 같은 날 근태신청이 여럿일 때만 지목용으로 준다. 비우면 1건일 때 그것을 쓰고,
    /// 여러 건이면 후보 목록과 함께 에러로 끝난다(임의로 고르지 않는다).
    #[serde(default)]
    #[serde(deserialize_with = "super::flex_string_opt")]
    #[schemars(schema_with = "super::flex_str_opt_schema")]
    pub app_sq: Option<String>,
}
