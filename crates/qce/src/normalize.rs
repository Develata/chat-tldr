use chat_tldr_core::{
    Attachment, AttachmentKind, ForwardBundle, ForwardedMessage, Mention, MentionTarget, ReplyRef,
};
use chrono::DateTime;
use serde_json::Value;

use crate::{
    QceError, QceOptions,
    input::{Forwarded, RawContent, nonempty},
};

#[derive(Default)]
pub(crate) struct Content {
    pub text: String,
    pub mentions: Vec<Mention>,
    pub reply_to: Option<ReplyRef>,
    pub attachments: Vec<Attachment>,
    pub forward: Option<ForwardBundle>,
}

pub(crate) fn content(
    raw: &RawContent,
    depth: u8,
    opts: &QceOptions,
    warnings: &mut Vec<String>,
    path: &str,
) -> Result<Content, QceError> {
    let mut out = Content::default();
    if raw.elements.is_empty() && !raw.text.is_empty() {
        out.text.clone_from(&raw.text);
        warnings.push(format!(
            "{path}: no content.elements; using content.text without structural metadata"
        ));
    }
    for (index, element) in raw.elements.iter().enumerate() {
        let at = format!("{path}.content.elements[{index}]");
        let data = &element.data;
        match element.kind.as_str() {
            "text" => {
                let text = data
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| QceError::Field(format!("{at}.data.text")))?;
                out.text.push_str(text);
            }
            "at" => {
                let uid = field(data, "uid");
                let display = if uid == Some("all") {
                    "全体成员"
                } else {
                    field(data, "name").unwrap_or("未知")
                }
                .to_owned();
                let target = if uid == Some("all") {
                    MentionTarget::All
                } else {
                    let usable_uid = uid.filter(|v| !matches!(*v, "unknown" | "未知" | "0"));
                    let (uid, inferred_uin) = match usable_uid {
                        Some(v) if v.bytes().all(|b| b.is_ascii_digit()) => {
                            (None, Some(v.to_owned()))
                        }
                        Some(v) => (Some(v.to_owned()), None),
                        None => (None, None),
                    };
                    let uin = field(data, "uin")
                        .filter(|v| *v != "0" && v.bytes().all(|b| b.is_ascii_digit()))
                        .map(str::to_owned)
                        .or(inferred_uin);
                    MentionTarget::User { uid, uin }
                };
                out.text.push('@');
                out.text.push_str(&display);
                out.mentions.push(Mention { target, display });
            }
            "face" | "market_face" => {
                out.text.push_str(&match field(data, "name") {
                    Some(name) => format!("[表情:{name}]"),
                    None => "[表情]".into(),
                });
            }
            "image" | "video" | "audio" | "file" => {
                let name = field(data, "filename").map(str::to_owned);
                let kind = match element.kind.as_str() {
                    "image" => {
                        out.text.push_str("[图片]");
                        AttachmentKind::Image
                    }
                    "video" => {
                        out.text.push_str("[视频]");
                        AttachmentKind::Video
                    }
                    "audio" => {
                        out.text.push_str("[语音]");
                        AttachmentKind::Audio
                    }
                    _ => {
                        out.text
                            .push_str(&format!("[文件:{}]", name.as_deref().unwrap_or("未知")));
                        AttachmentKind::File
                    }
                };
                out.attachments.push(Attachment {
                    kind,
                    name,
                    size: data.get("size").and_then(Value::as_u64),
                });
            }
            "reply" => {
                let id = field(data, "referencedMessageId").or_else(|| field(data, "messageId"));
                if let Some(id) = id {
                    if out.reply_to.is_none() {
                        out.reply_to = Some(ReplyRef {
                            source_message_id: id.into(),
                            resolved: None,
                        });
                    } else {
                        warnings.push(format!(
                            "{at}: multiple reply elements; keeping the first reference"
                        ));
                    }
                } else {
                    warnings.push(format!(
                        "{at}: reply has no referencedMessageId or messageId"
                    ));
                }
            }
            "forward" => {
                let bundle = forward(data, depth, opts, warnings, &at)?;
                out.text.push_str(&format!("[合并转发:{}]", bundle.title));
                if let Some(previous) = &mut out.forward {
                    previous.title.push_str(" / ");
                    previous.title.push_str(&bundle.title);
                    previous.messages.extend(bundle.messages);
                    warnings.push(format!(
                        "{at}: combined multiple forward bundles in element order"
                    ));
                } else {
                    out.forward = Some(bundle);
                }
            }
            other => {
                out.text.push_str(&format!("[{other}]"));
                warnings.push(format!(
                    "W_UNKNOWN_ELEMENT: {at}: unsupported element type {other}"
                ));
            }
        }
    }
    Ok(out)
}

fn forward(
    data: &Value,
    depth: u8,
    opts: &QceOptions,
    warnings: &mut Vec<String>,
    path: &str,
) -> Result<ForwardBundle, QceError> {
    let title = field(data, "title").unwrap_or("聊天记录").to_owned();
    let mut bundle = ForwardBundle {
        title,
        messages: Vec::new(),
    };
    if depth >= opts.max_forward_depth {
        warnings.push(format!(
            "{path}: forward depth limit reached; keeping title only"
        ));
        return Ok(bundle);
    }
    let entries = match data.get("messages") {
        None | Some(Value::Null) => {
            warnings.push(format!(
                "{path}: forward has no messages; keeping title only"
            ));
            return Ok(bundle);
        }
        Some(Value::Array(entries)) => entries,
        _ => return Err(QceError::Field(format!("{path}.data.messages"))),
    };
    for (index, entry) in entries.iter().enumerate() {
        let raw: Forwarded = serde_json::from_value(entry.clone())?;
        let at = format!("{path}.data.messages[{index}]");
        let sent_at = raw
            .timestamp
            .map(|ms| {
                DateTime::from_timestamp_millis(ms)
                    .map(|t| t.with_timezone(&opts.timezone))
                    .ok_or_else(|| QceError::Field(format!("{at}.timestamp")))
            })
            .transpose()?;
        let inner = if raw.recalled {
            Content::default()
        } else {
            content(&raw.content, depth + 1, opts, warnings, &at)?
        };
        bundle.messages.push(ForwardedMessage {
            sender_display: raw.sender.name,
            sent_at,
            text: inner.text,
            forward: inner.forward.map(Box::new),
        });
    }
    Ok(bundle)
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    nonempty(value.get(name).and_then(Value::as_str))
}
