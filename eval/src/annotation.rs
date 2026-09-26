use std::collections::BTreeSet;

use chat_tldr_core::MessagePayload;
use chrono::{DateTime, NaiveDate};
use serde::{Deserialize, Serialize};

pub const HEADERS: [&str; 10] = [
    "sheet_version",
    "message_id",
    "sent_at",
    "sender",
    "sender_display",
    "display_text",
    "thread",
    "todo",
    "announcement",
    "items_json",
];

#[derive(Serialize, Deserialize)]
pub struct SheetRow {
    pub sheet_version: String,
    pub message_id: String,
    pub sent_at: String,
    pub sender: String,
    pub sender_display: String,
    pub display_text: String,
    pub thread: String,
    pub todo: String,
    pub announcement: String,
    pub items_json: String,
}

impl SheetRow {
    pub fn unlabelled(message: MessagePayload) -> Self {
        Self {
            sheet_version: "1".into(),
            message_id: message.message_id.to_string(),
            sent_at: encode(&message.sent_at.to_rfc3339()),
            sender: encode(message.sender.as_ref()),
            sender_display: encode(&message.sender_display),
            display_text: encode(&message.display_text),
            thread: String::new(),
            todo: String::new(),
            announcement: String::new(),
            items_json: String::new(),
        }
    }

    pub fn labelled(&self) -> Result<(GoldMessage, Vec<GoldItem>), String> {
        if self.sheet_version != "1" {
            return Err("unsupported sheet_version".into());
        }
        message_id(&self.message_id)?;
        DateTime::parse_from_rfc3339(decode(&self.sent_at)?)
            .map_err(|_| "sent_at must retain its RFC3339 value")?;
        let sender = decode(&self.sender)?;
        if !sender.starts_with("qq:") || sender.len() == 3 {
            return Err("invalid sender ID".into());
        }
        decode(&self.sender_display)?;
        decode(&self.display_text)?;
        if self.thread.trim().is_empty() {
            return Err("thread is required (any nonempty stable label)".into());
        }
        let todo = boolean(&self.todo, "todo")?;
        let announcement = boolean(&self.announcement, "announcement")?;
        if self.items_json.trim().is_empty() {
            return Err("items_json is required; use [] explicitly for no items".into());
        }
        let items: Vec<GoldItem> = serde_json::from_str(&self.items_json).map_err(
            |_| "items_json must be an array of complete item objects with valid field types",
        )?;
        for item in &items {
            item.validate()?;
        }
        Ok((
            GoldMessage {
                message_id: self.message_id.clone(),
                thread: self.thread.clone(),
                todo,
                announcement,
            },
            items,
        ))
    }
}

// Prefix every source cell, including values already starting with text:. This
// avoids formula/automatic date conversion without lossy apostrophe heuristics.
fn encode(value: &str) -> String {
    format!("text:{value}")
}
fn decode(value: &str) -> Result<&str, String> {
    value
        .strip_prefix("text:")
        .ok_or_else(|| "source cells must retain their text: prefix".into())
}

pub fn message_id(value: &str) -> Result<(), String> {
    if value.len() == 18
        && value.starts_with("m_")
        && value.as_bytes()[2..].iter().all(u8::is_ascii_hexdigit)
    {
        Ok(())
    } else {
        Err("message_id must be m_ followed by 16 hexadecimal digits".into())
    }
}

fn boolean(value: &str, field: &str) -> Result<bool, String> {
    match value.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{field} must explicitly be true or false")),
    }
}

#[derive(Serialize)]
pub struct GoldMessage {
    pub message_id: String,
    pub thread: String,
    pub todo: bool,
    pub announcement: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldItem {
    pub item_id: String,
    pub kind: String,
    pub assignee: String,
    pub anchors: Vec<String>,
    // Absence is an annotation omission; explicit null means no deadline.
    #[serde(deserialize_with = "required_deadline")]
    pub deadline: Option<GoldDeadline>,
    pub importance: String,
}

fn required_deadline<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<GoldDeadline>, D::Error> {
    Option::<GoldDeadline>::deserialize(deserializer)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldDeadline {
    pub raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bound_date: Option<String>,
}

impl GoldItem {
    fn validate(&self) -> Result<(), String> {
        if self.item_id.trim().is_empty()
            || self.item_id.trim() != self.item_id
            || self.item_id.chars().any(char::is_control)
        {
            return Err("item_id must be a nonempty stable ID without surrounding whitespace or control characters".into());
        }
        if !matches!(self.kind.as_str(), "todo" | "announcement" | "decision") {
            return Err("invalid item kind".into());
        }
        if !matches!(self.assignee.as_str(), "me" | "all" | "other" | "unknown") {
            return Err("invalid item assignee".into());
        }
        if !matches!(self.importance.as_str(), "P0" | "P1" | "P2" | "P3") {
            return Err("invalid item importance".into());
        }
        if self.anchors.is_empty() {
            return Err("item anchors cannot be empty".into());
        }
        let mut seen = BTreeSet::new();
        for anchor in &self.anchors {
            message_id(anchor)?;
            if !seen.insert(anchor) {
                return Err("duplicate anchor within an item".into());
            }
        }
        if let Some(deadline) = &self.deadline {
            if deadline.raw.trim().is_empty() {
                return Err("deadline.raw cannot be empty".into());
            }
            match (&deadline.relation, &deadline.bound_date) {
                (None, None) => {}
                (Some(relation), Some(date))
                    if matches!(relation.as_str(), "before" | "at" | "after") =>
                {
                    if date.len() != 10 || NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
                        return Err("deadline.bound_date must be a valid YYYY-MM-DD date".into());
                    }
                }
                _ => {
                    return Err(
                        "deadline relation and bound_date must both be valid or both absent".into(),
                    );
                }
            }
        }
        Ok(())
    }
}
