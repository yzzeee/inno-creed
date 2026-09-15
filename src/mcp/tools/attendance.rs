//! 근태 도구.
//!
//! 라우터는 `attendance_router`로 생성돼 `super::Amaranth::all_tools()`에서 합성된다.
//! 담당 도메인 로직은 `modules::attendance`에 있고, 여기 핸들러는 **`ensure_session` → 모듈 호출 → 감싸기**만 한다.

use rmcp::{handler::server::wrapper::Parameters, model::{CallToolResult, ContentBlock}, tool, tool_router, ErrorData};

use crate::mcp::{map_domain_err, map_domain_err_ctx, Amaranth};
use crate::mcp::args::attendance::*;
use crate::modules;

#[tool_router(router = attendance_router, vis = "pub(crate)")]
impl Amaranth {
    #[tool(description = "근태신청(휴가·출장·외근·휴일근무) 1건을 **취소한다**. ⚠️ 실제로 나가는 조작이다 — 명시적 지시가 있을 때만. ⛔ **이건 즉시 취소가 아니라 「취소신청서」를 새로 상신하는 것이다**(아마란스 웹의 '결재취소' 버튼과 같은 동작). 원본 문서를 지우는 게 아니라 같은 내용을 음수(-1일)로 담은 별도 문서를 올려 상쇄한다. 그래서 **cancel_approval이 거부하는 종결(doc_sts 90) 근태 문서도 이 경로로는 되돌릴 수 있다** — 반대로 비근태 문서에는 쓸 수 없다(그쪽은 cancel_approval). ⭐ 필요한 인자는 **날짜 하나**다(`date`=근태가 적용되는 날 YYYYMMDD, 상신한 날이 아님). appSq·detailSq·linkKey·취소 양식 formId·본문 데이터는 전부 도구가 서버에서 찾아 채운다. 같은 날 신청이 여럿이면 후보 목록과 함께 에러로 끝나므로 그때만 `app_sq`로 지목하면 된다(임의로 고르지 않는다). 결재선은 **원본 문서의 결재선을 그대로 물려받는다** — 원본을 승인한 사람이 취소도 승인한다. 따라서 원본이 정상 결재선이면 취소는 **결재가 끝나야** 반영되고 그 전까지 원본 근태는 살아 있다. 응답의 `originStillActive`가 그걸 알려준다(false면 이미 상쇄 확인). 남의 신청은 거부한다. 실증 범위: 연차(form 36→취소 44) 1건 e2e. 출장·외근·휴일의 취소 양식은 미검증이나 양식 id를 서버에서 얻으므로 같은 경로로 동작할 것으로 본다.")]
    async fn cancel_attendance_application(
        &self,
        Parameters(a): Parameters<CancelAttendanceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.ensure_session().await?;
        let data = modules::attendance_cancel::cancel_attendance(
            &self.client,
            &a.date,
            a.app_sq.as_deref(),
        )
        .await
        .map_err(map_domain_err_ctx("근태신청 취소 실패"))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(data.to_string())]))
    }

    #[tool(
        description = "[아마란스] 기간(월) 근태 현황을 조회한다. 일자별 출퇴근·근무시간·지각/연차 등 + 기간 합계. month=\"202608\" 또는 start/end(YYYYMMDD)."
    )]
    async fn attendance_month(
        &self,
        Parameters(a): Parameters<AttendanceMonthArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.ensure_session().await?;
        let (start, end) = if !a.start.trim().is_empty() && !a.end.trim().is_empty() {
            (a.start.clone(), a.end.clone())
        } else {
            modules::attendance::month_range(a.month.trim())
                .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?
        };
        let data = modules::attendance::work_time_status(&self.client, &start, &end)
            .await
            .map_err(map_domain_err)?;
        Ok(CallToolResult::success(vec![ContentBlock::text(data.to_string())]))
    }

    #[tool(
        description = "오늘(또는 지정일)의 출퇴근 현황을 조회한다(읽기, 부작용 없음). comeTm(출근)/leaveTm(퇴근) YYYYMMDDHHmm, 빈값=미등록."
    )]
    async fn get_attendance_today(
        &self,
        Parameters(a): Parameters<AttendanceTodayArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.ensure_session().await?;
        let wd = if a.work_dt.trim().is_empty() {
            modules::attendance::today_kst()
        } else {
            a.work_dt.trim().to_string()
        };
        let data = modules::attendance::today(&self.client, &wd)
            .await
            .map_err(map_domain_err)?;
        Ok(CallToolResult::success(vec![ContentBlock::text(data.to_string())]))
    }

    #[tool(
        description = "출근(clock in)을 기록한다(attendFg 1). ⚠️ 실제 근태 punch — 실제 출근 시점에 사용자가 명시 지시할 때만. 이미 출근 기록(comeTm)이 있으면 재기록 안 함(덮어쓰기 방지). punch 후 read-back(comeTm)으로 확인."
    )]
    async fn attendance_clock_in(&self) -> Result<CallToolResult, ErrorData> {
        self.ensure_session().await?;
        let data = modules::attendance::punch_and_verify(&self.client, "1")
            .await
            .map_err(map_domain_err_ctx("출근 기록 실패"))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(data.to_string())]))
    }

    #[tool(
        description = "퇴근(clock out)을 기록한다(attendFg 4). ⚠️ 실제 근태 punch — 실제 퇴근 시점에 사용자가 명시 지시할 때만. 이미 퇴근 기록(leaveTm)이 있으면 재기록 안 함. punch 후 read-back(leaveTm)으로 확인."
    )]
    async fn attendance_clock_out(&self) -> Result<CallToolResult, ErrorData> {
        self.ensure_session().await?;
        let data = modules::attendance::punch_and_verify(&self.client, "4")
            .await
            .map_err(map_domain_err_ctx("퇴근 기록 실패"))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(data.to_string())]))
    }
}
