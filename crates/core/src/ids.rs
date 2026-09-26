use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

macro_rules! string_id {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl From<String> for $name {
            fn from(value: String) -> Self { Self(value) }
        }
        impl From<&str> for $name {
            fn from(value: &str) -> Self { Self(value.to_owned()) }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str { &self.0 }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.0.fmt(f) }
        }
    )+};
}

string_id!(ChatId, PersonId, MessageId, TopicId, InsightId, RunId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cursor {
    pub sent_at_ms: i64,
    pub ordinal: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderProfile {
    pub version: u16,
}

impl Default for RenderProfile {
    fn default() -> Self {
        Self { version: 1 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid cursor: expected <sent_at_ms>:<nonnegative ordinal>")]
pub struct ParseCursorError;

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.sent_at_ms, self.ordinal)
    }
}

impl FromStr for Cursor {
    type Err = ParseCursorError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (sent_at, ordinal) = value.split_once(':').ok_or(ParseCursorError)?;
        let result = Self {
            sent_at_ms: sent_at.parse().map_err(|_| ParseCursorError)?,
            ordinal: ordinal.parse().map_err(|_| ParseCursorError)?,
        };
        if result.ordinal < 0 || result.to_string() != value {
            return Err(ParseCursorError);
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid render profile: expected r followed by a positive version")]
pub struct ParseRenderProfileError;

impl fmt::Display for RenderProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.version)
    }
}

impl FromStr for RenderProfile {
    type Err = ParseRenderProfileError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let version = value
            .strip_prefix('r')
            .ok_or(ParseRenderProfileError)?
            .parse()
            .map_err(|_| ParseRenderProfileError)?;
        let result = Self { version };
        if version == 0 || result.to_string() != value {
            return Err(ParseRenderProfileError);
        }
        Ok(result)
    }
}

macro_rules! string_serde {
    ($($name:ident),+ $(,)?) => {$(
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer)?.parse().map_err(de::Error::custom)
            }
        }
    )+};
}

string_serde!(Cursor, RenderProfile);
