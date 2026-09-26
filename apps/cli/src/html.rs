use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;

use chat_tldr_engine::store::InboxSnapshot;

use crate::Failure;

pub fn write(snapshot: &InboxSnapshot, path: &Path) -> Result<(), Failure> {
    let parent = path
        .parent()
        .ok_or_else(|| Failure::new("E_OUTPUT_WRITE", 8, "HTML output needs a parent directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(output_error)?;
    temporary
        .write_all(render(snapshot).as_bytes())
        .map_err(output_error)?;
    temporary.as_file().sync_all().map_err(output_error)?;
    temporary
        .persist(path)
        .map_err(|error| output_error(error.error))?;
    Ok(())
}

fn output_error(error: std::io::Error) -> Failure {
    Failure::new(
        "E_OUTPUT_WRITE",
        8,
        format!("Cannot write HTML output: {error}"),
    )
}

fn render(snapshot: &InboxSnapshot) -> String {
    let mut page = String::from(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Chat TL;DR</title><style>body{font:16px/1.6 system-ui,sans-serif;max-width:900px;margin:40px auto;padding:0 20px;color:#20242c;background:#f6f7f9}article{background:white;padding:20px 24px;margin:16px 0;border:1px solid #dce0e7;border-radius:12px}h1,h2{line-height:1.3}small{color:#536070}blockquote{margin:12px 0;padding:8px 16px;border-left:3px solid #8194b0;white-space:pre-wrap}p{white-space:pre-wrap}mark{background:#ffed99}</style></head><body><h1>群聊收件箱</h1>",
    );
    let _ = write!(
        page,
        "<p>{}</p><small>生成时间：{}</small>",
        escape(snapshot.meta.chat_id.as_ref()),
        escape(&snapshot.meta.generated_at.to_rfc3339())
    );
    if snapshot.insights.is_empty() {
        page.push_str("<p>当前没有符合筛选条件的结论。</p>");
    }
    for payload in &snapshot.insights {
        let insight = &payload.insight;
        let priority = serde_json::to_value(insight.priority).expect("enum serializes");
        let state = serde_json::to_value(insight.lifecycle).expect("enum serializes");
        let _ = write!(
            page,
            "<article><small>{} · {}</small><h2>{}</h2><p>{}</p>",
            escape(priority.as_str().unwrap_or("?")),
            escape(state.as_str().unwrap_or("?")),
            escape(&insight.title),
            escape(&insight.summary)
        );
        if let Some(deadline) = &insight.deadline {
            let _ = write!(page, "<p>截止时间：{}</p>", escape(&deadline.raw));
        }
        for evidence in &payload.evidence_view {
            let _ = write!(
                page,
                "<blockquote>{}</blockquote><small>{} · {}</small>",
                highlight(&evidence.display_text, evidence.highlight),
                escape(&evidence.sender_display),
                escape(&evidence.sent_at.to_rfc3339())
            );
        }
        page.push_str("</article>");
    }
    page.push_str("</body></html>");
    page
}

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn highlight(text: &str, range: Option<[usize; 2]>) -> String {
    let Some([start, end]) =
        range.filter(|[start, end]| start < end && *end <= text.chars().count())
    else {
        return escape(text);
    };
    let before: String = text.chars().take(start).collect();
    let selected: String = text.chars().skip(start).take(end - start).collect();
    let after: String = text.chars().skip(end).collect();
    format!(
        "{}<mark>{}</mark>{}",
        escape(&before),
        escape(&selected),
        escape(&after)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn html_escapes_untrusted_text_before_adding_scalar_highlights() {
        assert_eq!(
            highlight("甲<script>乙", Some([1, 9])),
            "甲<mark>&lt;script&gt;</mark>乙"
        );
        assert_eq!(highlight("&🙂", Some([9, 10])), "&amp;🙂");
        assert_eq!(escape("\"'"), "&quot;&#39;");
    }
}
