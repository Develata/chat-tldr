//! Synchronous model clients. Provider diagnostics never contain response bodies or credentials.
//! Wire references: https://api-docs.deepseek.com/api/create-chat-completion/
//! and https://platform.claude.com/docs/en/api/messages/create (checked 2026-09-26).
mod transport;

use std::cell::RefCell;
use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::ProviderConfig;
pub(crate) use transport::{AuthStyle, HttpTransport};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

impl Usage {
    pub fn add(&mut self, other: Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cost_usd += other.cost_usd;
    }
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct ProviderError {
    code: &'static str,
    retryable: bool,
    message: String,
    usage: Usage,
}

impl ProviderError {
    pub fn new(code: &'static str, retryable: bool, message: impl Into<String>) -> Self {
        Self {
            code,
            retryable,
            message: message.into(),
            usage: Usage::default(),
        }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
    pub fn retryable(&self) -> bool {
        self.retryable
    }
    pub fn exit_code(&self) -> u8 {
        match self.code {
            "E_INTERNAL" => 1,
            "E_CONFIG" | "E_PROVIDER_AUTH" => 4,
            _ => 5,
        }
    }
    pub fn usage(&self) -> Usage {
        self.usage
    }
    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = usage;
        self
    }
    pub fn invalid_output() -> Self {
        Self::new(
            "E_LLM_OUTPUT_INVALID",
            true,
            "Model output did not match the expected response schema",
        )
    }
    pub fn config(message: &'static str) -> Self {
        Self::new("E_CONFIG", false, message)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmRequest {
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub usage: Usage,
}

pub trait LlmClient {
    fn complete(&self, req: &LlmRequest) -> Result<LlmResponse, ProviderError>;
}

pub enum Client {
    OpenAi(OpenAiCompatClient),
    Anthropic(AnthropicCompatClient),
}

impl Client {
    pub fn with_key(config: &ProviderConfig, key: &str) -> Result<Self, ProviderError> {
        match config.api_format.as_deref() {
            Some("openai") => Ok(Self::OpenAi(OpenAiCompatClient::with_key(config, key)?)),
            Some("anthropic") => Ok(Self::Anthropic(AnthropicCompatClient::with_key(
                config, key,
            )?)),
            _ => Err(ProviderError::config("Unsupported LLM API format")),
        }
    }
    pub fn new(config: &ProviderConfig) -> Result<Self, ProviderError> {
        match config.api_format.as_deref() {
            Some("openai") => Ok(Self::OpenAi(OpenAiCompatClient::new(config)?)),
            Some("anthropic") => Ok(Self::Anthropic(AnthropicCompatClient::new(config)?)),
            _ => Err(ProviderError::config("Unsupported LLM API format")),
        }
    }
}

impl LlmClient for Client {
    fn complete(&self, req: &LlmRequest) -> Result<LlmResponse, ProviderError> {
        match self {
            Self::OpenAi(client) => client.complete(req),
            Self::Anthropic(client) => client.complete(req),
        }
    }
}

pub struct OpenAiCompatClient {
    transport: HttpTransport,
    config: ProviderConfig,
}
pub struct AnthropicCompatClient {
    transport: HttpTransport,
    config: ProviderConfig,
}

impl OpenAiCompatClient {
    pub fn with_key(config: &ProviderConfig, key: &str) -> Result<Self, ProviderError> {
        validate_config(config, 2.0)?;
        Ok(Self {
            transport: HttpTransport::with_key(config, "chat/completions", AuthStyle::Bearer, key)?,
            config: config.clone(),
        })
    }
    pub fn new(config: &ProviderConfig) -> Result<Self, ProviderError> {
        validate_config(config, 2.0)?;
        Ok(Self {
            transport: HttpTransport::new(config, "chat/completions", AuthStyle::Bearer)?,
            config: config.clone(),
        })
    }
    #[cfg(test)]
    pub(crate) fn testing(config: &ProviderConfig) -> Self {
        Self {
            transport: HttpTransport::testing(config, "chat/completions", AuthStyle::Bearer),
            config: config.clone(),
        }
    }
}

impl AnthropicCompatClient {
    pub fn with_key(config: &ProviderConfig, key: &str) -> Result<Self, ProviderError> {
        validate_config(config, 1.0)?;
        Ok(Self {
            transport: HttpTransport::with_key(config, "v1/messages", AuthStyle::Anthropic, key)?,
            config: config.clone(),
        })
    }
    pub fn new(config: &ProviderConfig) -> Result<Self, ProviderError> {
        validate_config(config, 1.0)?;
        Ok(Self {
            transport: HttpTransport::new(config, "v1/messages", AuthStyle::Anthropic)?,
            config: config.clone(),
        })
    }
    #[cfg(test)]
    pub(crate) fn testing(config: &ProviderConfig) -> Self {
        Self {
            transport: HttpTransport::testing(config, "v1/messages", AuthStyle::Anthropic),
            config: config.clone(),
        }
    }
}

fn validate_config(config: &ProviderConfig, maximum_temperature: f64) -> Result<(), ProviderError> {
    if config.model.trim().is_empty() || config.timeout_secs == 0 {
        return Err(ProviderError::config(
            "Model and positive timeout are required",
        ));
    }
    if config
        .temperature
        .is_some_and(|value| !value.is_finite() || !(0.0..=maximum_temperature).contains(&value))
    {
        return Err(ProviderError::config("Model temperature is out of range"));
    }
    for price in [config.price_input_per_mtok, config.price_output_per_mtok]
        .into_iter()
        .flatten()
    {
        if !price.is_finite() || price < 0.0 {
            return Err(ProviderError::config(
                "Model prices must be finite and nonnegative",
            ));
        }
    }
    Ok(())
}

fn request_body(
    config: &ProviderConfig,
    req: &LlmRequest,
    anthropic: bool,
) -> Result<Value, ProviderError> {
    if req.max_tokens == 0 {
        return Err(ProviderError::config(
            "Model output token limit must be positive",
        ));
    }
    let mut system = req.system.clone();
    if config.json_mode.unwrap_or(false) {
        system.push_str("\nReturn a JSON object only.");
    }
    let mut body = if anthropic {
        json!({"model":config.model,"system":system,"messages":[{"role":"user","content":req.user}],"max_tokens":req.max_tokens,"stream":false})
    } else {
        json!({"model":config.model,"messages":[{"role":"system","content":system},{"role":"user","content":req.user}],"max_tokens":req.max_tokens,"stream":false})
    };
    body["temperature"] = json!(config.temperature.unwrap_or(0.0));
    if !anthropic && config.json_mode.unwrap_or(false) {
        body["response_format"] = json!({"type":"json_object"});
    }
    if let Some(extra) = &config.extra_body {
        let extra = serde_json::to_value(extra)
            .map_err(|_| ProviderError::config("Invalid extra model parameters"))?;
        let extra = extra
            .as_object()
            .ok_or_else(|| ProviderError::config("Extra model parameters must be an object"))?;
        for (key, value) in extra {
            if [
                "model",
                "messages",
                "system",
                "max_tokens",
                "stream",
                "temperature",
                "response_format",
            ]
            .contains(&key.as_str())
            {
                return Err(ProviderError::config(
                    "Extra model parameters cannot override core request fields",
                ));
            }
            body[key] = value.clone();
        }
    }
    Ok(body)
}

pub(crate) fn usage(
    value: &Value,
    config: &ProviderConfig,
    input_default: f64,
    output_default: f64,
) -> Result<Usage, ProviderError> {
    let input_tokens = value
        .get("input_tokens")
        .or_else(|| value.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .ok_or_else(ProviderError::invalid_output)?;
    let output_tokens = value
        .get("output_tokens")
        .or_else(|| value.get("completion_tokens"))
        .and_then(Value::as_u64)
        .ok_or_else(ProviderError::invalid_output)?;
    let input_tokens = input_tokens
        .checked_add(
            value
                .get("cache_read_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        )
        .and_then(|tokens| {
            tokens.checked_add(
                value
                    .get("cache_creation_input_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            )
        })
        .ok_or_else(ProviderError::invalid_output)?;
    let cost_usd = (input_tokens as f64 * config.price_input_per_mtok.unwrap_or(input_default)
        + output_tokens as f64 * config.price_output_per_mtok.unwrap_or(output_default))
        / 1_000_000.0;
    if !cost_usd.is_finite() {
        return Err(ProviderError::invalid_output());
    }
    Ok(Usage {
        input_tokens,
        output_tokens,
        cost_usd,
    })
}

impl LlmClient for OpenAiCompatClient {
    fn complete(&self, req: &LlmRequest) -> Result<LlmResponse, ProviderError> {
        let response = self
            .transport
            .post(&request_body(&self.config, req, false)?)?;
        let usage = usage(&response["usage"], &self.config, 0.0, 0.0)?;
        let choices = response["choices"]
            .as_array()
            .filter(|choices| choices.len() == 1)
            .ok_or_else(|| ProviderError::invalid_output().with_usage(usage))?;
        let choice = &choices[0];
        if choice["finish_reason"] != "stop" {
            return Err(ProviderError::invalid_output().with_usage(usage));
        }
        let content = choice["message"]["content"]
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| ProviderError::invalid_output().with_usage(usage))?;
        Ok(LlmResponse {
            content: content.to_owned(),
            usage,
        })
    }
}

impl LlmClient for AnthropicCompatClient {
    fn complete(&self, req: &LlmRequest) -> Result<LlmResponse, ProviderError> {
        let response = self
            .transport
            .post(&request_body(&self.config, req, true)?)?;
        let usage = usage(&response["usage"], &self.config, 0.0, 0.0)?;
        if !matches!(
            response["stop_reason"].as_str(),
            Some("end_turn" | "stop_sequence")
        ) {
            return Err(ProviderError::invalid_output().with_usage(usage));
        }
        let blocks = response["content"]
            .as_array()
            .ok_or_else(|| ProviderError::invalid_output().with_usage(usage))?;
        let mut content = String::new();
        for block in blocks {
            if block["type"] == "text" {
                content.push_str(
                    block["text"]
                        .as_str()
                        .ok_or_else(|| ProviderError::invalid_output().with_usage(usage))?,
                );
            }
        }
        if content.trim().is_empty() {
            return Err(ProviderError::invalid_output().with_usage(usage));
        }
        Ok(LlmResponse { content, usage })
    }
}

pub struct MockLlm {
    responses: RefCell<VecDeque<Result<LlmResponse, ProviderError>>>,
    requests: RefCell<Vec<LlmRequest>>,
}

impl MockLlm {
    pub fn new(responses: Vec<Result<LlmResponse, ProviderError>>) -> Self {
        Self {
            responses: RefCell::new(responses.into()),
            requests: RefCell::new(Vec::new()),
        }
    }
    pub fn requests(&self) -> Vec<LlmRequest> {
        self.requests.borrow().clone()
    }
}

impl LlmClient for MockLlm {
    fn complete(&self, req: &LlmRequest) -> Result<LlmResponse, ProviderError> {
        self.requests.borrow_mut().push(req.clone());
        self.responses.borrow_mut().pop_front().unwrap_or_else(|| {
            Err(ProviderError::new(
                "E_INTERNAL",
                false,
                "Mock model responses exhausted",
            ))
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
