//! Markdown 입력, 저장 본문 검증, 세션을 넘어 유지하는 본문 검증 기록.
//! 기록에는 본문 원문 대신 계정별 초안 ID와 서버에서 읽은 HTML의 해시만 남긴다.

use anyhow::{Result, anyhow, bail};
use pulldown_cmark::{Event, Options, Parser, html};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::modules::board::html_to_text;
use crate::{client::GwClient, error::InvalidInput};

pub fn render_body(body: &str) -> Result<String> {
    if body.trim().is_empty() {
        return Err(InvalidInput::new("발송하지 않았습니다. body에 비어 있지 않은 일반 텍스트 또는 Markdown 본문을 입력하세요.").into());
    }
    let mut events = Vec::new();
    for event in Parser::new_ext(body, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Html(_) | Event::InlineHtml(_) => {
                return Err(InvalidInput::new("발송하지 않았습니다. body에 HTML 태그를 직접 입력할 수 없습니다. 일반 텍스트 또는 Markdown을 사용하세요. HTML 예시는 코드 블록으로 감싸세요.").into());
            }
            // 일반 텍스트의 줄바꿈도 메일에서 보존한다.
            Event::SoftBreak => events.push(Event::HardBreak),
            other => events.push(other),
        }
    }
    let mut output = String::new();
    html::push_html(&mut output, events.into_iter());
    if visible_text(&output).is_empty() {
        return Err(InvalidInput::new("발송하지 않았습니다. 본문에 표시할 텍스트가 없습니다. 서명만 있는 메일은 보내지 않습니다.").into());
    }
    Ok(output)
}

fn visible_text(html: &str) -> String {
    html_to_text(html)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn verify_saved(expected: &str, actual: &str) -> Result<()> {
    let expected_text = visible_text(expected);
    if expected_text.is_empty() || expected_text != visible_text(actual) {
        bail!("저장된 본문이 작성 본문과 일치하지 않습니다(서명만 남음·일부 본문 누락 포함)");
    }
    Ok(())
}

fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn record_path(c: &GwClient, muid: &str) -> Result<PathBuf> {
    let root = crate::config::dir()
        .ok_or_else(|| anyhow!("본문 검증 기록을 저장할 설정 경로가 없습니다"))?;
    // 서버 입력을 파일 경로에 직접 넣지 않는다. 다른 계정의 같은 muid도 분리한다.
    let key = serde_json::to_string(&(c.email_addr(), c.email_domain(), c.emp_seq(), muid))?;
    Ok(root
        .join("verified-mail-bodies")
        .join(format!("{}.sha256", digest(&key))))
}

pub(super) fn remember(c: &GwClient, muid: &str, html: &str) -> Result<()> {
    let path = record_path(c, muid)?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    // 쓰기가 중단된 기록은 require_verified의 정확한 해시 대조에서 거부된다.
    std::fs::write(path, digest(html))?;
    Ok(())
}

fn verify_record(record: &str, html: &str) -> Result<()> {
    if record != digest(html) {
        bail!(
            "검증 후 초안 본문이 변경되었습니다 — 발송하지 않았습니다. body로 새 초안을 작성하고 확인하세요"
        );
    }
    Ok(())
}

pub(super) fn require_verified(c: &GwClient, muid: &str, html: &str) -> Result<()> {
    let path = record_path(c, muid)?;
    let record = std::fs::read_to_string(path).map_err(|_| anyhow!(
        "draft_muid={muid}의 본문 검증 기록을 읽을 수 없습니다 — 발송하지 않았습니다. 웹·구버전 초안은 웹에서 발송하거나 save_mail_draft(body)로 새로 작성하세요"
    ))?;
    verify_record(&record, html).map_err(|error| anyhow!("draft_muid={muid}: {error}"))
}

pub(super) fn forget(c: &GwClient, muid: &str) -> Result<()> {
    std::fs::remove_file(record_path(c, muid)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_preserves_content_and_line_breaks() {
        let html = render_body("안녕하세요\n둘째 줄 & < 3\n\n- **강조**\n- 목록\n\n| 항목 | 값 |\n| --- | --- |\n| 본문 | 12345 |").unwrap();
        assert!(html.contains("<br />"));
        assert!(html.contains("<strong>강조</strong>"));
        assert!(html.contains("<table>"));
        assert!(html.contains("&amp; &lt; 3"));
        assert!(visible_text(&html).contains("12345"));
    }

    #[test]
    fn reject_empty_and_raw_html_before_signature() {
        for input in [
            "",
            " \n\t",
            "---",
            "<p>본문</p>",
            "본문 <b>태그</b>",
            "<!-- 숨은 내용 -->",
        ] {
            assert!(render_body(input).is_err(), "{input}");
        }
        assert!(
            render_body("`<p>HTML 예시</p>`")
                .unwrap()
                .contains("&lt;p&gt;")
        );
    }

    #[test]
    fn saved_signature_alone_and_partial_body_are_rejected() {
        let expected = "<p>첫째 문단</p><p>둘째 문단</p><div>서명</div>";
        assert!(verify_saved(expected, "<div>서명</div>").is_err());
        assert!(verify_saved(expected, "<p>첫째 문단</p><div>서명</div>").is_err());
        assert!(
            verify_saved(
                expected,
                "<p>첫째 문단</p>\n<p>둘째 문단</p>\n<div>서명</div>"
            )
            .is_ok()
        );
        assert!(verify_saved(expected, "").is_err());
    }

    #[test]
    fn changed_draft_and_truncated_record_cannot_pass() {
        let body = "<p>검증된 본문</p>";
        let record = digest(body);
        assert!(verify_record(&record, body).is_ok());
        assert!(verify_record(&record, "<p>서명만 남음</p>").is_err());
        assert!(verify_record(&record[..10], body).is_err());
    }
}
