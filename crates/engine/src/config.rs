use std::path::Path;

use chrono::FixedOffset;
use serde::Deserialize;

use crate::{EngineError, Result};

pub const EXAMPLE_CONFIG: &str = include_str!("../../../config.example.toml");

/// Parsed, validated settings. Secrets are looked up by the provider at call time.
#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub timezone: String,
    pub jev: ProviderConfig,
    pub llm: ProviderConfig,
    pub agent: AgentConfig,
    pub segment: SegmentConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    /// Optional local plaintext key. Debug output is always redacted.
    pub api_key: Option<chat_tldr_core::settings::SecretString>,
    pub timeout_secs: u64,
    pub api_format: Option<String>,
    pub temperature: Option<f64>,
    pub json_mode: Option<bool>,
    pub extra_body: Option<toml::Value>,
    pub price_input_per_mtok: Option<f64>,
    pub price_output_per_mtok: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AgentConfig {
    pub max_steps: u32,
    pub budget_usd: f64,
    pub direct_max: u32,
    pub direct_interleave_max: f32,
    pub segment_batch: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SegmentConfig {
    pub weak_gap_secs: u64,
    pub strong_gap_secs: u64,
    pub same_sender_join_secs: u64,
    pub burst_max_messages: u32,
    pub topic_close_secs: u64,
    pub tau_high: f32,
    pub tau_low: f32,
    pub candidate_k: u32,
    pub all_candidates_max: u32,
    pub state_token_budget: u32,
    pub alpha: f32,
    pub beta: f32,
    pub gamma: f32,
    pub sim_threshold: f32,
}

impl Config {
    pub fn load(path: &Path, explicit: bool) -> Result<Self> {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => Some(source),
            Err(e) if !explicit && e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(EngineError::Config(format!(
                    "cannot read {}: {e}",
                    path.display()
                )));
            }
        };
        Self::parse(source.as_deref().unwrap_or(""))
    }

    /// A partial local configuration overrides the checked-in defaults.
    pub fn parse(source: &str) -> Result<Self> {
        let mut value: toml::Value = toml::from_str(EXAMPLE_CONFIG)
            .map_err(|_| EngineError::Config("invalid compiled defaults".into()))?;
        // Do not echo parse errors: their snippets could reveal a pasted secret.
        let overrides: toml::Value = toml::from_str(source)
            .map_err(|_| EngineError::Config("invalid TOML syntax".into()))?;
        merge(&mut value, overrides);
        let config: Self = value
            .try_into()
            .map_err(|_| EngineError::Config("invalid configuration field type".into()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn timezone_offset(&self) -> Result<FixedOffset> {
        let bytes = self.timezone.as_bytes();
        if bytes.len() != 6
            || !matches!(bytes[0], b'+' | b'-')
            || bytes[3] != b':'
            || ![bytes[1], bytes[2], bytes[4], bytes[5]]
                .iter()
                .all(u8::is_ascii_digit)
        {
            return Err(EngineError::Config(
                "timezone must be a fixed offset such as +08:00".into(),
            ));
        }
        let hours = i32::from(bytes[1] - b'0') * 10 + i32::from(bytes[2] - b'0');
        let minutes = i32::from(bytes[4] - b'0') * 10 + i32::from(bytes[5] - b'0');
        if hours > 23 || minutes > 59 {
            return Err(EngineError::Config(
                "timezone offset is out of range".into(),
            ));
        }
        let seconds = (hours * 3600 + minutes * 60) * if bytes[0] == b'-' { -1 } else { 1 };
        FixedOffset::east_opt(seconds).ok_or_else(|| EngineError::Config("invalid timezone".into()))
    }

    fn validate(&self) -> Result<()> {
        self.timezone_offset()?;
        for provider in [&self.jev, &self.llm] {
            if provider.base_url.trim().is_empty()
                || provider.model.trim().is_empty()
                || provider.api_key_env.trim().is_empty()
                || provider.timeout_secs == 0
                || provider.api_key_env.contains(['=', '\0'])
            {
                return Err(EngineError::Config(
                    "provider needs URL, model, environment-variable name and positive timeout"
                        .into(),
                ));
            }
            let url = reqwest::Url::parse(&provider.base_url)
                .map_err(|_| EngineError::Config("invalid provider URL".into()))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(EngineError::Config(
                    "provider URL must be HTTP(S), without credentials, query or fragment".into(),
                ));
            }
            for price in [
                provider.price_input_per_mtok,
                provider.price_output_per_mtok,
            ]
            .into_iter()
            .flatten()
            {
                if !price.is_finite() || price < 0.0 {
                    return Err(EngineError::Config(
                        "provider prices must be finite and nonnegative".into(),
                    ));
                }
            }
        }
        if !matches!(self.llm.api_format.as_deref(), Some("openai" | "anthropic")) {
            return Err(EngineError::Config(
                "llm.api_format must be openai or anthropic".into(),
            ));
        }
        let max_temperature = if self.llm.api_format.as_deref() == Some("anthropic") {
            1.0
        } else {
            2.0
        };
        if self
            .llm
            .temperature
            .is_some_and(|t| !t.is_finite() || !(0.0..=max_temperature).contains(&t))
        {
            return Err(EngineError::Config(
                "LLM temperature is out of range".into(),
            ));
        }
        if self.agent.max_steps == 0
            || self.agent.segment_batch == 0
            || !self.agent.budget_usd.is_finite()
            || self.agent.budget_usd <= 0.0
        {
            return Err(EngineError::Config(
                "agent steps, batch size and finite budget must be positive".into(),
            ));
        }
        let s = &self.segment;
        if s.weak_gap_secs > s.strong_gap_secs
            || s.burst_max_messages == 0
            || s.candidate_k == 0
            || s.all_candidates_max < s.candidate_k
            || s.state_token_budget == 0
            || s.tau_low > s.tau_high
            || [
                s.tau_low,
                s.tau_high,
                s.sim_threshold,
                self.agent.direct_interleave_max,
            ]
            .iter()
            .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
            || [s.alpha, s.beta, s.gamma]
                .iter()
                .any(|x| !x.is_finite() || *x < 0.0)
        {
            return Err(EngineError::Config(
                "invalid segment bounds or thresholds".into(),
            ));
        }
        Ok(())
    }
}

fn merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                // A provider-specific request body must be replaceable with {}.
                if key == "extra_body" {
                    base.insert(key, value);
                    continue;
                }
                match base.get_mut(&key) {
                    Some(current) => merge(current, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_partial_overrides_need_no_keys() {
        let config = Config::parse("timezone = '-03:00'\n[agent]\nmax_steps = 3").unwrap();
        assert_eq!(config.timezone_offset().unwrap().local_minus_utc(), -10800);
        assert_eq!(config.agent.max_steps, 3);
        assert_eq!(config.llm.model, "deepseek-flash");
    }

    #[test]
    fn invalid_settings_fail_without_echoing_source() {
        for value in [
            "timezone='+25:00'",
            "timezone='坏时区'",
            "[agent]\nbudget_usd=nan",
            "[agent]\nmax_steps=0",
            "[segment]\ntau_high=0.1\ntau_low=0.9",
        ] {
            assert_eq!(Config::parse(value).unwrap_err().code(), "E_CONFIG");
        }
        assert!(
            !Config::parse("secret='sensitive")
                .unwrap_err()
                .to_string()
                .contains("sensitive")
        );
    }
}
