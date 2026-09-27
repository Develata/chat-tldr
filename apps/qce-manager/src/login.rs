use crate::{
    credentials::{Credentials, Secret},
    error::{Failure, Result},
    export::hex,
    http::Http,
    output::Output,
    runtime::Budget,
};
use reqwest::Method;
use serde_json::{Value, json};
use std::{
    io::{IsTerminal, Write},
    time::Duration,
};

pub trait QrDisplay {
    fn interactive(&self) -> bool;
    fn show(&mut self, content: &str) -> Result<()>;
}

pub struct Terminal;
impl QrDisplay for Terminal {
    fn interactive(&self) -> bool {
        std::io::stderr().is_terminal()
    }
    fn show(&mut self, content: &str) -> Result<()> {
        if !self.interactive() {
            return Err(interaction_required());
        }
        let text = render(content)?;
        let mut stderr = std::io::stderr().lock();
        writeln!(stderr, "请用手机 QQ 扫码：\n{text}")
            .and_then(|_| stderr.flush())
            .map_err(|_| Failure::io())
    }
}

fn render(content: &str) -> Result<String> {
    let code = qrcode::QrCode::new(content.as_bytes()).map_err(|_| Failure::protocol())?;
    Ok(code
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .build())
}

fn interaction_required() -> Failure {
    Failure::new(
        "E_QCE_LOGIN_REQUIRED",
        4,
        "QQ 未登录；请在本机交互终端运行提示的 login 命令扫码。非交互调用不会请求或输出二维码",
    )
}

pub struct Napcat<'a> {
    http: &'a Http,
    credentials: &'a Credentials<'a>,
    credential: Option<Secret>,
}

impl<'a> Napcat<'a> {
    pub fn new(http: &'a Http, credentials: &'a Credentials<'a>) -> Self {
        Self {
            http,
            credentials,
            credential: None,
        }
    }

    fn authenticate(&mut self, budget: Budget<'_>) -> Result<()> {
        let token = self.credentials.napcat(budget)?;
        let hash = hex(ring::digest::digest(
            &ring::digest::SHA256,
            format!("{}.napcat", token.0).as_bytes(),
        )
        .as_ref());
        let response = self.http.json(
            Method::POST,
            "/api/auth/login",
            None,
            Some(&json!({"hash":hash})),
            budget,
        )?;
        let value = response
            .get("Credential")
            .or_else(|| response.get("credential"))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| self.http.auth_error())?;
        self.credential = Some(Secret(value.into()));
        Ok(())
    }

    fn request(&mut self, path: &str, budget: Budget<'_>) -> Result<Value> {
        for attempt in 0..2 {
            let result = (|| {
                if self.credential.is_none() {
                    self.authenticate(budget)?;
                }
                self.http.json(
                    Method::POST,
                    path,
                    self.credential.as_ref(),
                    Some(&json!({})),
                    budget,
                )
            })();
            match result {
                Err(error)
                    if attempt == 0
                        && matches!(
                            error.code,
                            "E_NAPCAT_AUTH" | "E_QCE_UNREACHABLE" | "E_QCE_TIMEOUT"
                        ) =>
                {
                    self.credential = None;
                    budget.sleep(Duration::from_millis(200))?;
                }
                result => return result,
            }
        }
        Err(self.http.auth_error())
    }

    pub fn status(&mut self, budget: Budget<'_>) -> Result<Value> {
        self.request("/api/QQLogin/CheckLoginStatus", budget)
    }
}

pub fn online(status: &Value) -> Result<bool> {
    status
        .get("isLogin")
        .or_else(|| status.get("online"))
        .and_then(Value::as_bool)
        .ok_or_else(Failure::protocol)
}

pub struct Options {
    pub max_wait_secs: u64,
    pub qr_events: bool,
}

pub fn run<W: Write>(
    options: Options,
    credentials: &Credentials<'_>,
    qce: &Http,
    napcat: &Http,
    display: &mut dyn QrDisplay,
    output: &mut Output<W>,
    budget: Budget<'_>,
) -> Result<()> {
    let budget = budget.limited(
        Duration::from_secs(options.max_wait_secs),
        "E_QCE_LOGIN_TIMEOUT",
    );
    let mut client = Napcat::new(napcat, credentials);
    let mut last_qr = String::new();
    loop {
        let status = client.status(budget)?;
        if online(&status)? {
            break;
        }
        if !options.qr_events && !display.interactive() {
            return Err(interaction_required());
        }
        let response = client.request("/api/QQLogin/GetQQLoginQrcode", budget);
        let qr = match &response {
            Ok(value) => qr_field(value).or_else(|| qr_field(&status)),
            Err(error)
                if matches!(
                    error.code,
                    "E_NAPCAT_AUTH" | "E_CANCELLED" | "E_QCE_LOGIN_TIMEOUT"
                ) =>
            {
                return Err(Failure::new(error.code, error.exit, error.message.clone()));
            }
            Err(_) => qr_field(&status),
        };
        if let Some(qr) = qr
            && qr != last_qr
        {
            if options.qr_events {
                // Only the explicitly opted-in pipe receives QR material. No files or logs.
                if qr.len() > 4096 || qrcode::QrCode::new(qr.as_bytes()).is_err() {
                    return Err(Failure::protocol());
                }
                output.emit(chat_tldr_core::EventBody::Unknown {
                    event: "qce_login_qr".into(),
                    payload: json!({"version":1,"content":qr}),
                })?;
            } else {
                display.show(qr)?;
            }
            last_qr = qr.to_owned();
            output.progress("waiting_scan", "等待手机 QQ 扫码")?;
        }
        budget.sleep(Duration::from_millis(500))?;
    }
    output.progress("logged_in", "QQ 已登录，等待 QCE 就绪")?;
    loop {
        budget.check()?;
        // Re-read files on every startup probe: QCE may not have created its token yet.
        match credentials.qce(budget) {
            Ok(token) => match qce.qce_online(&token, budget) {
                Ok(true) => {
                    return output.ack(
                        "login",
                        false,
                        json!({"qq_logged_in":true,"qce_ready":true}),
                    );
                }
                Ok(false) => {}
                Err(error) if error.code == "E_QCE_AUTH" => return Err(error),
                Err(error) if !matches!(error.code, "E_QCE_UNREACHABLE" | "E_QCE_TIMEOUT") => {
                    return Err(error);
                }
                Err(_) => {}
            },
            Err(error) if error.code != "E_QCE_AUTH" => return Err(error),
            Err(_) => {}
        }
        // Losing an existing login never transitions into the QR branch, even on a terminal.
        if !online(&client.status(budget)?)? {
            return Err(interaction_required());
        }
        budget.sleep(Duration::from_millis(500))?;
    }
}

fn qr_field(value: &Value) -> Option<&str> {
    ["qrcode", "qrcodeurl", "qrcodeUrl", "url"]
        .iter()
        .find_map(|key| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        })
}

#[cfg(test)]
mod tests;
