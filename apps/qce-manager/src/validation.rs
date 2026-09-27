use crate::error::{Failure, Result};
use serde::{
    Deserialize, Deserializer,
    de::{IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use std::{fmt, fs::File, io::BufReader, path::Path};

// Count the top-level array without allocating/copying the entire chat export.
struct Messages(u64);
impl<'de> Deserialize<'de> for Messages {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Counter;
        impl<'de> Visitor<'de> for Counter {
            type Value = Messages;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("message array")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Messages, A::Error> {
                let mut count = 0;
                while seq.next_element::<IgnoredAny>()?.is_some() {
                    count += 1;
                }
                Ok(Messages(count))
            }
        }
        deserializer.deserialize_seq(Counter)
    }
}

struct Export {
    messages: Option<u64>,
    chunked: bool,
}
impl<'de> Deserialize<'de> for Export {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Top;
        impl<'de> Visitor<'de> for Top {
            type Value = Export;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("single JSON export")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Export, M::Error> {
                let mut result = Export {
                    messages: None,
                    chunked: false,
                };
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "messages" => {
                            if result.messages.is_some() {
                                return Err(serde::de::Error::duplicate_field("messages"));
                            }
                            result.messages = Some(map.next_value::<Messages>()?.0);
                        }
                        "chunked" => {
                            result.chunked = true;
                            map.next_value::<IgnoredAny>()?;
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(result)
            }
        }
        deserializer.deserialize_map(Top)
    }
}

pub fn count_messages(path: &Path) -> Result<u64> {
    let parse_error = || Failure::new("E_INPUT_PARSE", 3, "下载文件不是完整的单文件 JSON 导出");
    let file = File::open(path).map_err(|_| Failure::io())?;
    let mut deserializer = serde_json::Deserializer::from_reader(BufReader::new(file));
    let export = Export::deserialize(&mut deserializer).map_err(|_| parse_error())?;
    deserializer.end().map_err(|_| parse_error())?;
    if export.chunked {
        return Err(Failure::new(
            "E_QCE_CHUNKED",
            3,
            "不支持分块导出；请选择单文件 JSON",
        ));
    }
    export.messages.ok_or_else(parse_error)
}
