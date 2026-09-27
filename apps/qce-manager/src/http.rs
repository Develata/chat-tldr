use std::{
    io::Read,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use reqwest::{
    Method, Url,
    blocking::{Client, Response},
    header::{AUTHORIZATION, HeaderValue},
};
use serde_json::Value;

use crate::{
    credentials::Secret,
    error::{Failure, Result},
    runtime::Budget,
};

pub struct Http {
    client: Client,
    pub base: Url,
    timeout: Duration,
    napcat: bool,
}

impl Http {
    pub fn new(base: &str, timeout_secs: u64, napcat: bool) -> Result<Self> {
        let base = local_url(base)?;
        let mut builder = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none());
        // Pin localhost instead of trusting DNS/hosts to preserve the loopback boundary.
        if base.host_str() == Some("localhost") {
            builder = builder.resolve(
                "localhost",
                SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                    base.port_or_known_default().unwrap_or(80),
                ),
            );
        }
        let client = builder
            .build()
            .map_err(|_| Failure::new("E_INTERNAL", 1, "无法初始化 HTTP 客户端"))?;
        Ok(Self {
            client,
            base,
            timeout: Duration::from_secs(timeout_secs),
            napcat,
        })
    }

    pub fn auth_error(&self) -> Failure {
        Failure::new(
            if self.napcat {
                "E_NAPCAT_AUTH"
            } else {
                "E_QCE_AUTH"
            },
            4,
            "服务拒绝认证；Token 可能已重新生成或过期",
        )
    }

    pub fn request(
        &self,
        method: Method,
        url: Url,
        token: Option<&Secret>,
        body: Option<&Value>,
        budget: Budget<'_>,
    ) -> Result<Response> {
        if url.origin() != self.base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(Failure::new(
                "E_QCE_DOWNLOAD",
                3,
                "拒绝非同源或携带凭据的下载地址",
            ));
        }
        let mut request = self
            .client
            .request(method, url)
            .timeout(budget.timeout(self.timeout)?);
        if let Some(token) = token {
            let mut value = HeaderValue::from_str(&format!("Bearer {}", token.0))
                .map_err(|_| self.auth_error())?;
            value.set_sensitive(true);
            request = request.header(AUTHORIZATION, value);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send();
        budget.check()?;
        let response = response.map_err(|error| {
            Failure::new(
                if error.is_timeout() {
                    "E_QCE_TIMEOUT"
                } else {
                    "E_QCE_UNREACHABLE"
                },
                5,
                "本机服务不可达或请求超时",
            )
        })?;
        if matches!(response.status().as_u16(), 401 | 403) {
            return Err(self.auth_error());
        }
        if !response.status().is_success() {
            return Err(Failure::new(
                if response.status().is_server_error() {
                    "E_QCE_UNREACHABLE"
                } else {
                    "E_QCE_RESPONSE"
                },
                5,
                format!(
                    "服务返回 HTTP {}；未回显响应内容",
                    response.status().as_u16()
                ),
            ));
        }
        Ok(response)
    }

    pub fn json(
        &self,
        method: Method,
        route: &str,
        token: Option<&Secret>,
        body: Option<&Value>,
        budget: Budget<'_>,
    ) -> Result<Value> {
        let url = self.base.join(route).map_err(|_| Failure::protocol())?;
        let response = self.request(method, url, token, body, budget)?;
        let limit = 8 * 1024 * 1024;
        let mut bytes = Vec::new();
        let read = response.take(limit + 1).read_to_end(&mut bytes);
        budget.check()?;
        read.map_err(|error| match error.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                Failure::new("E_QCE_TIMEOUT", 5, "读取服务响应超时")
            }
            _ => Failure::protocol(),
        })?;
        if bytes.len() as u64 > limit {
            return Err(Failure::protocol());
        }
        let mut value: Value = serde_json::from_slice(&bytes).map_err(|_| Failure::protocol())?;
        if value
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|v| v.eq_ignore_ascii_case("unauthorized"))
        {
            return Err(self.auth_error());
        }
        if value.get("success").and_then(Value::as_bool) == Some(false)
            || value
                .get("code")
                .and_then(Value::as_i64)
                .is_some_and(|code| code != 0 && code != 200)
        {
            return Err(Failure::protocol());
        }
        if let Some(data) = value.get_mut("data") {
            Ok(data.take())
        } else {
            Ok(value)
        }
    }

    pub fn qce_online(&self, token: &Secret, budget: Budget<'_>) -> Result<bool> {
        self.json(Method::GET, "/api/system/status", Some(token), None, budget)?
            .get("online")
            .and_then(Value::as_bool)
            .ok_or_else(Failure::protocol)
    }
}

fn local_url(raw: &str) -> Result<Url> {
    let bad = || Failure::usage("服务地址必须是无凭据、查询参数和路径的 HTTP(S) loopback 地址");
    let url = Url::parse(raw).map_err(|_| bad())?;
    let host = url.host_str().unwrap_or("").trim_matches(['[', ']']);
    let local = host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if !local
        || !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(bad());
    }
    Ok(url)
}
