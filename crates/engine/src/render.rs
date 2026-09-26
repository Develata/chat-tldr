use chat_tldr_core::{ForwardBundle, UnifiedMessage};

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
        append_forward(&mut text, forward, 1);
    }
    text
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
            append_forward(text, nested, depth + 1);
        }
    }
}
