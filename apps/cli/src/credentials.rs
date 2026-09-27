//! Explicit configuration credentials take precedence over inherited environment.
use chat_tldr_core::settings::{CredentialStatus, SecretString};
use chat_tldr_engine::config::ProviderConfig;

use crate::Failure;

pub fn validate(key: &SecretString) -> Result<(), Failure> {
    if key.is_empty()
        || key.expose().len() > 4096
        || !key.expose().bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(Failure::new(
            "E_CONFIG",
            4,
            "API key must contain 1–4096 printable ASCII characters without whitespace",
        ));
    }
    Ok(())
}

pub fn resolve(config: &ProviderConfig) -> Result<SecretString, Failure> {
    resolve_with(config, |name| std::env::var(name).ok())
}

fn resolve_with(
    config: &ProviderConfig,
    env: impl FnOnce(&str) -> Option<String>,
) -> Result<SecretString, Failure> {
    let key = if let Some(key) = &config.api_key {
        key.clone()
    } else {
        SecretString::new(env(&config.api_key_env).ok_or_else(|| Failure::new("E_CONFIG", 4,
            "API key is unavailable. Set it in GUI settings / config set --key-prompt, or supply the configured environment variable"))?)
    };
    validate(&key)?;
    Ok(key)
}

pub fn status(result: &Result<SecretString, Failure>, config: &ProviderConfig) -> CredentialStatus {
    CredentialStatus {
        source: if config.api_key.is_some() {
            "config"
        } else {
            "environment"
        }
        .into(),
        available: result.is_ok(),
        problem: result.as_ref().err().map(|error| error.message.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_key_precedes_environment_and_invalid_saved_key_does_not_fall_back() {
        let mut config = chat_tldr_engine::Config::parse("").unwrap().llm;
        config.api_key = Some(SecretString::new("stored-secret".into()));
        assert_eq!(
            resolve_with(&config, |_| Some("env-secret".into()))
                .unwrap()
                .expose(),
            "stored-secret"
        );
        config.api_key = Some(SecretString::new(" \n".into()));
        assert!(resolve_with(&config, |_| Some("env-secret".into())).is_err());
        config.api_key = None;
        assert_eq!(
            resolve_with(&config, |_| Some("env-secret".into()))
                .unwrap()
                .expose(),
            "env-secret"
        );
        assert!(resolve_with(&config, |_| None).is_err());
        assert!(!format!("{config:?}").contains("stored-secret"));
    }
}
