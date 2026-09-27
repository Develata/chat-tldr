use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::*;
use crate::Config;

pub(crate) struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: String,
}

impl Reply {
    pub(crate) fn json(body: Value) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.to_string(),
        }
    }
    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: "sensitive provider response: synthetic-test-key".into(),
        }
    }
}

pub(crate) struct Captured {
    pub headers: String,
    pub body: Value,
}

pub(crate) fn server(replies: Vec<Reply>) -> (String, JoinHandle<Vec<Captured>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let deadline = Instant::now() + Duration::from_secs(12);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "HTTP mock did not receive the expected request"
                        );
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("Cannot accept HTTP mock request: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            requests.push(read_request(&mut stream));
            write!(stream,"HTTP/1.1 {} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",reply.status,reply.body.len()).unwrap();
            for (name, value) in reply.headers {
                write!(stream, "{name}: {value}\r\n").unwrap();
            }
            write!(stream, "\r\n{}", reply.body).unwrap();
        }
        requests
    });
    (url, worker)
}

fn read_request(stream: &mut TcpStream) -> Captured {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0, "HTTP request ended before its headers");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(offset) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break offset + 4;
        }
        assert!(bytes.len() < 65536, "HTTP mock header limit exceeded");
    };
    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let length: usize = headers
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                .map(|(_, value)| value.trim().parse().unwrap())
        })
        .unwrap();
    while bytes.len() < header_end + length {
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0, "HTTP request body ended prematurely");
        bytes.extend_from_slice(&buffer[..count]);
    }
    Captured {
        headers,
        body: serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap(),
    }
}

pub(crate) fn provider(url: String) -> ProviderConfig {
    let mut config = Config::parse("").unwrap().llm;
    config.base_url = url;
    config.timeout_secs = 5;
    config
}

fn request() -> LlmRequest {
    LlmRequest {
        system: "Extract items from the input as JSON.".into(),
        user: "周五交报告".into(),
        max_tokens: 200,
    }
}

fn openai_response() -> Value {
    json!({"choices":[{"message":{"content":"{\"items\":[]}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":20}})
}

#[test]
fn saved_config_keys_authenticate_both_wire_formats_and_versioned_anthropic_urls() {
    for format in ["openai", "anthropic"] {
        let reply = if format == "openai" {
            openai_response()
        } else {
            json!({"content":[{"type":"text","text":"{\"items\":[]}"}],"stop_reason":"end_turn","usage":{"input_tokens":100,"output_tokens":20}})
        };
        let (url, worker) = server(vec![Reply::json(reply)]);
        let mut config = provider(format!("{url}/v1"));
        config.api_format = Some(format.into());
        config.api_key_env = "NONEXISTENT_SYNTHETIC_ENV".into();
        config.api_key = Some(chat_tldr_core::settings::SecretString::new(
            "stored-wire-key".into(),
        ));
        assert_eq!(
            Client::new(&config)
                .unwrap()
                .complete(&request())
                .unwrap()
                .content,
            "{\"items\":[]}"
        );
        let captured = worker.join().unwrap().remove(0);
        let headers = captured.headers.to_ascii_lowercase();
        if format == "openai" {
            assert!(headers.starts_with("post /v1/chat/completions "));
            assert!(headers.contains("authorization: bearer stored-wire-key"));
        } else {
            assert!(headers.starts_with("post /v1/messages "));
            assert!(headers.contains("x-api-key: stored-wire-key"));
        }
        assert!(!format!("{config:?}").contains("stored-wire-key"));
    }
}

#[test]
fn openai_wire_shape_and_cost_are_checked_without_cloud_access() {
    let (url, worker) = server(vec![Reply::json(openai_response())]);
    let response = OpenAiCompatClient::testing(&provider(url))
        .complete(&request())
        .unwrap();
    assert_eq!(response.content, "{\"items\":[]}");
    assert_eq!(response.usage.input_tokens, 100);
    assert!((response.usage.cost_usd - 0.000054).abs() < 1e-12);
    let requests = worker.join().unwrap();
    let actual = &requests[0];
    assert!(actual.headers.starts_with("POST /chat/completions "));
    assert!(
        actual
            .headers
            .to_ascii_lowercase()
            .contains("authorization: bearer synthetic-test-key")
    );
    assert_eq!(actual.body["model"], "deepseek-flash");
    assert_eq!(actual.body["max_tokens"], 200);
    assert_eq!(actual.body["thinking"]["type"], "disabled");
    assert_eq!(actual.body["response_format"]["type"], "json_object");
    assert_eq!(actual.body["messages"][1]["content"], "周五交报告");
    assert_eq!(actual.body["stream"], false);
}

#[test]
fn anthropic_uses_system_parameter_and_accounts_for_cache_tokens() {
    let (url, worker) = server(vec![Reply::json(
        json!({"content":[{"type":"thinking","thinking":"ignored"},{"type":"text","text":"{\"ok\":"},{"type":"text","text":"true}"}],"stop_reason":"end_turn","usage":{"input_tokens":100,"output_tokens":20,"cache_read_input_tokens":10,"cache_creation_input_tokens":5}}),
    )]);
    let response = AnthropicCompatClient::testing(&provider(url))
        .complete(&request())
        .unwrap();
    assert_eq!(response.content, "{\"ok\":true}");
    assert_eq!(response.usage.input_tokens, 115);
    let requests = worker.join().unwrap();
    assert!(requests[0].headers.starts_with("POST /v1/messages "));
    let headers = requests[0].headers.to_ascii_lowercase();
    assert!(headers.contains("x-api-key: synthetic-test-key"));
    assert!(headers.contains("anthropic-version: 2023-06-01"));
    assert!(!headers.contains("authorization:"));
    assert!(
        requests[0].body["system"]
            .as_str()
            .unwrap()
            .contains("JSON")
    );
    assert_eq!(requests[0].body["messages"][0]["role"], "user");
    assert!(requests[0].body.get("response_format").is_none());
}

#[test]
fn authentication_and_validation_errors_are_not_retried_or_echoed() {
    for status in [400, 401, 403, 422] {
        let (url, worker) = server(vec![Reply::status(status)]);
        let error = OpenAiCompatClient::testing(&provider(url))
            .complete(&request())
            .unwrap_err();
        assert!(!error.retryable());
        assert!(!format!("{error:?}").contains("sensitive"));
        assert!(!format!("{error:?}").contains("synthetic-test-key"));
        assert_eq!(worker.join().unwrap().len(), 1);
    }
}

#[test]
fn temporary_statuses_retry_with_bounded_backoff() {
    let (url, worker) = server(vec![
        Reply::status(429),
        Reply::status(529),
        Reply::json(openai_response()),
    ]);
    let started = Instant::now();
    OpenAiCompatClient::testing(&provider(url))
        .complete(&request())
        .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(1400));
    assert_eq!(worker.join().unwrap().len(), 3);
}

#[test]
fn retry_after_cannot_extend_the_request_deadline() {
    let mut reply = Reply::status(429);
    reply.headers.push(("Retry-After", "86400".into()));
    let (url, worker) = server(vec![reply]);
    let started = Instant::now();
    let error = OpenAiCompatClient::testing(&provider(url))
        .complete(&request())
        .unwrap_err();
    assert_eq!(error.code(), "E_PROVIDER_RATE_LIMIT");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(worker.join().unwrap().len(), 1);
}

#[test]
fn retryable_failures_stop_after_three_retries() {
    let (url, worker) = server((0..4).map(|_| Reply::status(503)).collect());
    let error = OpenAiCompatClient::testing(&provider(url))
        .complete(&request())
        .unwrap_err();
    assert_eq!(error.code(), "E_PROVIDER_OVERLOADED");
    assert_eq!(worker.join().unwrap().len(), 4);
}

#[test]
fn redirects_are_not_followed_and_truncation_preserves_usage() {
    let mut redirect = Reply::status(302);
    redirect
        .headers
        .push(("Location", "http://127.0.0.1:9/secret-destination".into()));
    let (url, worker) = server(vec![redirect]);
    let error = OpenAiCompatClient::testing(&provider(url))
        .complete(&request())
        .unwrap_err();
    assert_eq!(error.code(), "E_PROVIDER_BAD_REQUEST");
    worker.join().unwrap();
    let mut response = openai_response();
    response["choices"][0]["finish_reason"] = json!("length");
    let (url, worker) = server(vec![Reply::json(response)]);
    let error = OpenAiCompatClient::testing(&provider(url))
        .complete(&request())
        .unwrap_err();
    assert_eq!(error.code(), "E_LLM_OUTPUT_INVALID");
    assert_eq!(error.usage().input_tokens, 100);
    worker.join().unwrap();
}

#[test]
fn invalid_success_body_is_not_exposed() {
    let mut reply = Reply::status(200);
    reply.body = "sensitive chat and synthetic-test-key".into();
    let (url, worker) = server(vec![reply]);
    let error = OpenAiCompatClient::testing(&provider(url))
        .complete(&request())
        .unwrap_err();
    assert_eq!(error.code(), "E_LLM_OUTPUT_INVALID");
    assert!(!error.to_string().contains("sensitive"));
    worker.join().unwrap();
}

#[test]
fn missing_key_and_reserved_extra_parameters_fail_before_network() {
    let mut config = provider("https://example.invalid".into());
    config.api_key_env = format!(
        "CHAT_TLDR_TEST_MISSING_{}_{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    );
    let error = match Client::new(&config) {
        Ok(_) => panic!("unexpected environment key"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "E_CONFIG");
    config.extra_body = Some(toml::from_str("max_tokens=99999").unwrap());
    assert_eq!(
        request_body(&config, &request(), false).unwrap_err().code(),
        "E_CONFIG"
    );
}

#[test]
fn mocks_record_requests_and_never_synthesize_success_after_exhaustion() {
    let mock = MockLlm::new(vec![Ok(LlmResponse {
        content: "{}".into(),
        usage: Usage::default(),
    })]);
    mock.complete(&request()).unwrap();
    assert!(mock.complete(&request()).is_err());
    assert_eq!(mock.requests().len(), 2);
}
