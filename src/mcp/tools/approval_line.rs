//! 전자결재 — 결재선 도구.
//!
//! 라우터는 `approval_line_router`로 생성돼 `super::Amaranth::all_tools()`에서 합성된다.
//! 담당 도메인 로직은 `modules::approval_line / approval_line_suggest`에 있고, 여기 핸들러는 **`ensure_session` → 모듈 호출 → 감싸기**만 한다.

use rmcp::{handler::server::wrapper::Parameters, model::{CallToolResult, ContentBlock}, tool, tool_router, ErrorData};

use crate::mcp::{map_domain_err, map_domain_err_ctx, Amaranth};
use crate::mcp::args::approval::*;
use crate::modules;

#[tool_router(router = approval_line_router, vis = "pub(crate)")]
impl Amaranth {
    #[tool(
        description = "이 양식을 **내가 기안할 때의 결재선 후보**를 한 번에 제안한다(스키마 + 조직도 해석). 하는 일: 본인 직책(duty)으로 grade 구간 판정 → 해당 branch 선택(출장은 trip 국내/해외) → 각 직책을 실제 사람 후보로 해석(L_* 상대직책은 기안자 부서에서 상위로, 고정직책은 지정 부서에서). ⛔ **결과는 확정 결재선이 아니라 후보다** — 응답의 `verificationRequired:true`·`warnings`·단계별 `status`(후보1/후보다수/미해결)를 그대로 사용자에게 보여주고 **이름을 확인받은 뒤에** save_approval_line으로 등록할 것. 공석·겸직·대행·직책 라벨 차이(규칙 '사업부장' ↔ 조직 '센터장')·위임전결 개정 때문에 해석이 틀릴 수 있다. 등록 시에는 결재(3000) 노드만 담는다(양식필수 합의자·수신참조·시행자는 상신 때 서버가 자동 병합). 스키마 원본만 보려면 get_approval_line_schema."
    )]
    async fn suggest_approval_line(
        &self,
        Parameters(a): Parameters<SuggestApprovalLineArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.ensure_session().await?;
        let data = modules::approval_line_suggest::suggest_line(&self.client, &a.doc_type, &a.trip)
            .await
            .map_err(map_domain_err)?;
        Ok(CallToolResult::success(vec![ContentBlock::text(data.to_string())]))
    }

    #[tool(
        description = "저장된 개인결재라인 목록을 조회한다(eap102A02). 각 항목의 lineId는 read/save에, `_row`는 delete에 사용. 상신 아님(재사용 config)."
    )]
    async fn list_approval_lines(&self) -> Result<CallToolResult, ErrorData> {
        let data = modules::approval_line::list_lines(&self.client)
            .await
            .map_err(map_domain_err)?;
        Ok(CallToolResult::success(vec![ContentBlock::text(data.to_string())]))
    }

    #[tool(
        description = "개인결재라인 1건의 결재자 구성을 조회한다(eap102A05). `count`=결재자 수, `members[]`=결재자 원본 객체(순서=결재 순서, `user_id`가 empSeq, `user_nm`이 이름). 상신 전에 **누가 결재선에 있는지 사용자에게 확인시키는 용도**다. 라인을 새로 만들 땐 이 객체를 베낄 필요가 없다 — `save_approval_line`이 empSeq 목록만 받아 나머지를 채운다."
    )]
    async fn read_approval_line(
        &self,
        Parameters(a): Parameters<ReadApprovalLineArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let data = modules::approval_line::read_line(&self.client, &a.line_id)
            .await
            .map_err(map_domain_err)?;
        Ok(CallToolResult::success(vec![ContentBlock::text(data.to_string())]))
    }

    #[tool(
        description = "개인결재라인을 생성/수정한다(eap102A10). `approvers`에 **결재자 empSeq 목록**만 주면 된다(배열 순서 = 결재 순서) — 서버 payload 필드(co_id·act_id·org_id·org_div·순서)는 도구가 채운다. line_id=0 신규, 기존 id면 수정. ⚠️ 이건 재사용 config 저장이지 상신이 아님. ⛔ **결재자 0명은 거부한다**(서버는 빈 라인도 '저장됨'으로 응답하므로 도구가 저장 후 재조회해 판정한다 — 어긋나면 신규 라인은 되돌린다). ⛔ **기안자 단독 결재선도 거부한다** — 상신 즉시 종결(doc_sts 90)되어 cancel_approval로 취소할 수 없다. 결재선은 규칙(위임전결)이 있으니 `suggest_approval_line`으로 후보를 받아 **사용자에게 이름을 확인받은 뒤** 등록할 것. 합의자·수신참조·시행자는 담지 않는다(상신 때 서버가 양식필수로 병합). 응답의 `approverCount`·`approvers`(이름 포함)로 실제 저장 결과를 확인한다."
    )]
    async fn save_approval_line(
        &self,
        Parameters(a): Parameters<SaveApprovalLineArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let data = modules::approval_line::save_line(
            &self.client,
            a.line_id,
            &a.line_nm,
            a.form_id,
            &a.proc_id,
            &a.approvers,
        )
        .await
        .map_err(map_domain_err_ctx("결재라인 저장 실패"))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(data.to_string())]))
    }

    #[tool(
        description = "개인결재라인을 삭제한다(eap102A09). `line_id`는 list_approval_lines 결과의 lineId — 서버가 요구하는 행 객체는 도구가 조회해 채운다. 삭제 후 목록 재조회로 부재를 확인해 `verified_by_readback`으로 보고한다(`ok:false`면 아직 남아 있다는 뜻). 없는 lineId면 실행 없이 에러."
    )]
    async fn delete_approval_line(
        &self,
        Parameters(a): Parameters<DeleteApprovalLineArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let data = modules::approval_line::delete_line(&self.client, &a.line_id)
            .await
            .map_err(map_domain_err_ctx("결재라인 삭제 실패"))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(data.to_string())]))
    }
}
