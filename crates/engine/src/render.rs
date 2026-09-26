use chat_tldr_core::{ForwardBundle, RenderProfile, UnifiedMessage};

/// The r1 view shared by model input, evidence verification and message browsing.
pub fn render(message: &UnifiedMessage) -> String {
    if message.recalled {
        return String::new();
    }
    let mut text = String::new();
    if message.system {
        text.push_str("[系统]");
    }
    text.push_str(&message.text);
    if let Some(forward) = &message.forward {
        append_missing_title(&mut text, &message.text, forward);
        append_forward(&mut text, forward, 1);
    }
    text
}

/// Unsupported profiles must not silently fall back to different evidence text.
pub fn render_with_profile(message: &UnifiedMessage, profile: RenderProfile) -> Option<String> {
    (profile.version == 1).then(|| render(message))
}

fn append_missing_title(text: &mut String, base: &str, bundle: &ForwardBundle) {
    let title = format!("[合并转发:{}]", bundle.title);
    // The QCE adapter normally already includes this placeholder in base text.
    if !base.contains(&title) {
        text.push_str(&title);
    }
}

fn append_forward(text: &mut String, bundle: &ForwardBundle, depth: u8) {
    if depth > 2 {
        return;
    }
    for message in &bundle.messages {
        text.push('\n');
        text.push_str(&"  ".repeat(usize::from(depth)));
        text.push_str("> ");
        text.push_str(&message.sender_display);
        text.push_str(": ");
        text.push_str(&message.text);
        if let Some(nested) = &message.forward {
            // Even beyond the expansion limit, preserve the deeper title.
            append_missing_title(text, &message.text, nested);
            append_forward(text, nested, depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use chat_tldr_core::{ForwardedMessage, SourceMeta};
    use chrono::DateTime;

    use super::*;

    fn message() -> UnifiedMessage {
        UnifiedMessage {
            id: "m_synthetic".into(),
            chat_id: "qq:group:synthetic".into(),
            sender: "qq:synthetic".into(),
            sender_display: "合成用户".into(),
            sent_at: DateTime::parse_from_rfc3339("2026-09-26T10:00:00+08:00").unwrap(),
            text: "原文🙂[图片]".into(),
            mentions: vec![],
            reply_to: None,
            attachments: vec![],
            forward: None,
            recalled: false,
            system: false,
            source: SourceMeta {
                format: "synthetic".into(),
                identity: "synthetic".into(),
                qce_id: None,
                qce_seq: None,
                qce_type: None,
                file_hash: "synthetic".into(),
            },
        }
    }

    fn forwarded(title: &str, text: &str, nested: Option<ForwardBundle>) -> ForwardBundle {
        ForwardBundle {
            title: title.into(),
            messages: vec![ForwardedMessage {
                sender_display: "合成转发人".into(),
                sent_at: None,
                text: text.into(),
                forward: nested.map(Box::new),
            }],
        }
    }

    #[test]
    fn r1_preserves_original_text_system_markers_and_recalled_semantics() {
        let mut message = message();
        assert_eq!(render(&message), "原文🙂[图片]");
        message.system = true;
        assert_eq!(render(&message), "[系统]原文🙂[图片]");
        assert_eq!(
            render_with_profile(&message, RenderProfile::default()),
            Some(render(&message))
        );
        assert_eq!(
            render_with_profile(&message, RenderProfile { version: 2 }),
            None
        );
        message.recalled = true;
        message.forward = Some(forwarded("旧内容", "不应出现", None));
        assert_eq!(render(&message), "");
    }

    #[test]
    fn forward_titles_are_kept_once_and_only_two_levels_are_expanded() {
        let mut message = message();
        let third = forwarded("第三层标题", "第三层正文不应展开", None);
        let second = forwarded("第二层标题", "第二层正文", Some(third));
        message.forward = Some(forwarded(
            "第一层标题",
            "第一层正文[合并转发:第二层标题]",
            Some(second),
        ));
        message.text = "请看[合并转发:第一层标题]".into();
        assert_eq!(
            render(&message),
            concat!(
                "请看[合并转发:第一层标题]",
                "\n  > 合成转发人: 第一层正文[合并转发:第二层标题]",
                "\n    > 合成转发人: 第二层正文[合并转发:第三层标题]"
            )
        );
        message.text = "请看".into();
        assert!(render(&message).starts_with("请看[合并转发:第一层标题]\n"));
    }
}
