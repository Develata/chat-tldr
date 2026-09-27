use std::io::Read;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, RETRY_AFTER};
use serde_json::Value;

use super::ProviderError;
use crate::config::ProviderConfig;

const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const DELAYS: [Duration; 3] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];

pub(crate) enum AuthStyle {
    Bearer,
    Anthropic,
}

pub(crate) struct HttpTransport {
    client: Client,
    endpoint: reqwest::Url,
    timeout: Duration,
}

impl HttpTransport {
    pub(crate) fn new(
        config: &ProviderConfig,
        suffix: &str,
        auth: AuthStyle,
    ) -> Result<Self, ProviderError> {
        let key = match &config.api_key {
            Some(key) => key.clone(),
            None => chat_tldr_core::settings::SecretString::new(std::env::var(&config.api_key_env).map_err(|_| {
                ProviderError::config("API key is unavailable; configure it in GUI settings, config set, or the provider environment variable")
            })?),
        };
        Self::with_key(config, suffix, auth, key.expose())
    }

    pub(crate) fn with_key(
        config: &ProviderConfig,
        suffix: &str,
        auth: AuthStyle,
        key: &str,
    ) -> Result<Self, ProviderError> {
        if key.trim().is_empty() || config.timeout_secs == 0 {
            return Err(ProviderError::config(
                "Provider credentials and positive timeout are required",
            ));
        }
        let base = config.base_url.trim_end_matches('/');
        let suffix = if base.ends_with("/v1") {
            suffix.strip_prefix("v1/").unwrap_or(suffix)
        } else {
            suffix
        };
        let endpoint = reqwest::Url::parse(&format!("{base}/{suffix}"))
            .map_err(|_| ProviderError::config("Invalid provider URL"))?;
        if !matches!(endpoint.scheme(), "https" | "http")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(ProviderError::config(
                "Provider URL must be HTTP(S) without credentials, query, or fragment",
            ));
        }
        let mut headers = HeaderMap::new();
        let (name, value) = match auth {
            AuthStyle::Bearer => (AUTHORIZATION, format!("Bearer {key}")),
            AuthStyle::Anthropic => {
                headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
                (
                    reqwest::header::HeaderName::from_static("x-api-key"),
                    key.to_owned(),
                )
            }
        };
        let mut value = HeaderValue::from_str(&value)
            .map_err(|_| ProviderError::config("Invalid provider credential format"))?;
        value.set_sensitive(true);
        headers.insert(name, value);
        let timeout = Duration::from_secs(config.timeout_secs);
        let builder = Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .timeout(timeout);
        #[cfg(test)]
        let builder = builder.no_proxy();
        let client = builder
            .build()
            .map_err(|_| ProviderError::config("Cannot initialize the provider HTTP client"))?;
        Ok(Self {
            client,
            endpoint,
            timeout,
        })
    }

    #[cfg(test)]
    pub(crate) fn testing(config: &ProviderConfig, suffix: &str, auth: AuthStyle) -> Self {
        Self::with_key(config, suffix, auth, "synthetic-test-key").unwrap()
    }

    pub(crate) fn post(&self, body: &Value) -> Result<Value, ProviderError> {
        let started = Instant::now();
        let mut last_error = timeout_error();
        for next_delay in DELAYS.into_iter().map(Some).chain(std::iter::once(None)) {
            let remaining = self.timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(last_error);
            }
            let response = self
                .client
                .post(self.endpoint.clone())
                .timeout(remaining)
                .json(body)
                .send();
            let mut retry_after = None;
            match response {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        if response
                            .content_length()
                            .is_some_and(|length| length > MAX_RESPONSE_BYTES)
                        {
                            return Err(ProviderError::invalid_output());
                        }
                        let mut bytes = Vec::new();
                        if response
                            .take(MAX_RESPONSE_BYTES + 1)
                            .read_to_end(&mut bytes)
                            .is_ok()
                        {
                            if bytes.len() as u64 > MAX_RESPONSE_BYTES {
                                return Err(ProviderError::invalid_output());
                            }
                            return serde_json::from_slice(&bytes)
                                .map_err(|_| ProviderError::invalid_output());
                        }
                        last_error = timeout_error();
                    } else {
                        retry_after = response
                            .headers()
                            .get(RETRY_AFTER)
                            .and_then(|value| value.to_str().ok())
                            .and_then(parse_retry_after);
                        last_error = status_error(status.as_u16());
                    }
                }
                Err(error) => {
                    last_error = if error.is_builder() {
                        ProviderError::new(
                            "E_PROVIDER_BAD_REQUEST",
                            false,
                            "Cannot encode provider request",
                        )
                    } else {
                        timeout_error()
                    };
                }
            }
            if !last_error.retryable() {
                return Err(last_error);
            }
            let Some(base_delay) = next_delay else {
                return Err(last_error);
            };
            let delay = retry_after.unwrap_or(base_delay).max(base_delay);
            let remaining = self.timeout.saturating_sub(started.elapsed());
            if delay >= remaining {
                return Err(last_error);
            }
            std::thread::sleep(delay);
        }
        Err(last_error)
    }
}

fn timeout_error() -> ProviderError {
    ProviderError::new(
        "E_PROVIDER_TIMEOUT",
        true,
        "Provider request timed out or could not be completed",
    )
}

fn status_error(status: u16) -> ProviderError {
    match status {
        401 | 403 => {
            ProviderError::new("E_PROVIDER_AUTH", false, "Provider rejected authentication")
        }
        408 => timeout_error(),
        429 => ProviderError::new("E_PROVIDER_RATE_LIMIT", true, "Provider rate limit reached"),
        500..=599 => ProviderError::new(
            "E_PROVIDER_OVERLOADED",
            true,
            "Provider is temporarily unavailable",
        ),
        _ => ProviderError::new(
            "E_PROVIDER_BAD_REQUEST",
            false,
            "Provider rejected the request",
        ),
    }
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let duration = date.signed_duration_since(chrono::Utc::now());
    Some(duration.to_std().unwrap_or(Duration::ZERO))
}
