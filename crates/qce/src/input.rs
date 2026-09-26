//! Private input shapes derived from the documented QCE JSON field contract.
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub(crate) struct Export {
    #[serde(rename = "chatInfo")]
    pub chat_info: ChatInfo,
    pub messages: Vec<RawMessage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatInfo {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    pub peer_uid: Option<String>,
    pub peer_uin: Option<String>,
    pub self_uid: Option<String>,
    pub self_uin: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Sender {
    pub uid: Option<String>,
    #[serde(default)]
    pub name: String,
    pub nickname: Option<String>,
    pub group_card: Option<String>,
    pub remark: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct RawMessage {
    #[serde(default, deserialize_with = "string_or_integer")]
    pub id: Option<String>,
    #[serde(default, deserialize_with = "string_or_integer")]
    pub seq: Option<String>,
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub sender: Sender,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub content: RawContent,
    #[serde(default)]
    pub recalled: bool,
    #[serde(default)]
    pub system: bool,
}

#[derive(Default, Deserialize)]
pub(crate) struct RawContent {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub elements: Vec<Element>,
}

#[derive(Deserialize)]
pub(crate) struct Element {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub data: Value,
}

#[derive(Deserialize)]
pub(crate) struct Forwarded {
    #[serde(default)]
    pub sender: Sender,
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub content: RawContent,
    #[serde(default)]
    pub recalled: bool,
}

pub(crate) fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|v| !v.trim().is_empty())
}

fn string_or_integer<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Option::<Value>::deserialize(d)?;
    match value {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => Ok(Some(n.to_string())),
        _ => Err(serde::de::Error::custom(
            "expected a string or an integer identifier",
        )),
    }
}
