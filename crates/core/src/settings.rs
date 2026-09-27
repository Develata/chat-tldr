//! Provider settings exchanged over the CLI pipe. Secrets are write-only.
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Never include the contents in diagnostics, preferences, argv, or an ack.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn edit(&mut self) -> &mut String {
        &mut self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretString([REDACTED])")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialStatus {
    /// `config` when a saved key exists, otherwise `environment`.
    pub source: String,
    pub available: bool,
    pub problem: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderSettings {
    pub base_url: String,
    pub model: String,
    pub api_format: String,
    pub api_key_env: String,
    pub timeout_secs: u64,
    pub temperature: f64,
    pub json_mode: bool,
    pub extra_body: serde_json::Value,
    pub price_input_per_mtok: f64,
    pub price_output_per_mtok: f64,
    pub credential: CredentialStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SettingsSnapshot {
    pub version: u32,
    pub config_file: String,
    /// Optimistic concurrency token for the complete configuration file.
    pub revision: String,
    pub llm: ProviderSettings,
    pub jev: ProviderSettings,
}

/// One provider per atomic configuration update. Omitted fields are retained.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderUpdate {
    pub expected_revision: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_format: Option<String>,
    pub api_key_env: Option<String>,
    pub timeout_secs: Option<u64>,
    pub temperature: Option<f64>,
    pub json_mode: Option<bool>,
    pub extra_body: Option<serde_json::Value>,
    pub price_input_per_mtok: Option<f64>,
    pub price_output_per_mtok: Option<f64>,
    pub key: Option<SecretString>,
    #[serde(default)]
    pub clear_key: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_and_invalid_input_never_echo_key() {
        let update = ProviderUpdate {
            key: Some(SecretString::new("synthetic-private-value".into())),
            ..Default::default()
        };
        assert!(!format!("{update:?}").contains("synthetic-private-value"));
        // Input transport still carries the key, but output snapshots have no key field.
        let encoded = serde_json::to_string(&update).unwrap();
        let decoded: ProviderUpdate = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.key.unwrap().expose(), "synthetic-private-value");
    }
}
