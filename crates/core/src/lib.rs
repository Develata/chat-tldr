//! Shared wire types for the CLI, GUI, adapters, and evaluation tools.

macro_rules! wire_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant,)+
            /// A value introduced by a newer compatible protocol version.
            #[serde(other)]
            Unknown,
        }
    };
}

pub mod agent;
pub mod ids;
pub mod insight;
pub mod message;
pub mod protocol;

pub use agent::*;
pub use ids::*;
pub use insight::*;
pub use message::*;
pub use protocol::*;

pub const SCHEMA_VERSION: &str = "1.0";
pub const DB_VERSION: u32 = 1;
