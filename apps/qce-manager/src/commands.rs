use crate::{
    args::{Cli, Command},
    credentials::{Credentials, Environment},
    docker::{Docker, validate_name},
    error::{Failure, Result},
    export,
    files::Files,
    http::Http,
    login::{self, Napcat, QrDisplay},
    output::Output,
    runtime::Budget,
};
use chat_tldr_core::EventBody;
use reqwest::Method;
use serde_json::{Value, json};
use std::{io::Write, time::Duration};

pub fn run<W: Write>(
    cli: &Cli,
    docker: &dyn Docker,
    display: &mut dyn QrDisplay,
    output: &mut Output<W>,
    budget: Budget<'_>,
) -> Result<()> {
    if let Command::Version = cli.command {
        return output.ack("version", false, json!({"name":env!("CARGO_PKG_NAME"),"version":env!("CARGO_PKG_VERSION"),"schema_version":chat_tldr_core::SCHEMA_VERSION,"capabilities":["version","status","login","login.qr-events","chats","export","clean"]}));
    }
    if let Command::Clean { dry_run } = cli.command {
        return output.ack(
            "clean",
            !dry_run,
            Files::new(cli.data_dir.as_deref())?.clean(dry_run)?,
        );
    }
    if let Some(name) = &cli.docker {
        validate_name(name)?;
    }
    if cli
        .security_json_path
        .as_deref()
        .is_some_and(|p| !p.starts_with('/') || p.chars().any(char::is_control))
    {
        return Err(Failure::usage("--security-json-path 必须为容器内绝对路径"));
    }
    let qce = Http::new(&cli.base_url, cli.timeout_secs, false)?;
    let napcat = Http::new(&cli.napcat_url, cli.timeout_secs, true)?;
    let credentials = Credentials {
        cli,
        environment: Environment::read(),
        docker,
    };
    match &cli.command {
        Command::Status => status(&credentials, &qce, &napcat, output, budget),
        Command::Login {
            max_wait_secs,
            qr_events,
        } => login::run(
            login::Options {
                max_wait_secs: *max_wait_secs,
                qr_events: *qr_events,
            },
            &credentials,
            &qce,
            &napcat,
            display,
            output,
            budget,
        ),
        Command::Chats => {
            let token = credentials.qce(budget)?;
            let result = qce.json(
                Method::GET,
                "/api/recent-contacts?includeAll=true&limit=2000",
                Some(&token),
                None,
                budget,
            )?;
            let contacts = result
                .get("contacts")
                .and_then(Value::as_array)
                .ok_or_else(Failure::protocol)?;
            for contact in contacts {
                let kind = match contact.get("chatType").and_then(Value::as_u64) {
                    Some(1) => "private",
                    Some(2) => "group",
                    _ => continue,
                };
                let peer = contact
                    .get("peerUid")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(Failure::protocol)?;
                output.emit(EventBody::Unknown { event:"qce_chat".into(), payload:json!({"chat_type":kind,"peer_uid":peer,"display_name":contact.get("name").and_then(Value::as_str).unwrap_or("")}) })?;
            }
            Ok(())
        }
        Command::Export(args) => export::run(
            args,
            &credentials,
            &qce,
            &Files::new(cli.data_dir.as_deref())?,
            output,
            budget,
        ),
        Command::Version | Command::Clean { .. } => unreachable!(),
    }
}

fn status<W: Write>(
    credentials: &Credentials<'_>,
    qce: &Http,
    napcat: &Http,
    output: &mut Output<W>,
    budget: Budget<'_>,
) -> Result<()> {
    let cli = credentials.cli;
    let docker_status = if let Some(container) = &cli.docker {
        let format =
            "{\"status\":{{json .State.Status}},\"ports\":{{json .NetworkSettings.Ports}}}";
        match credentials.docker.run(
            &["inspect", "--format", format, container],
            budget.limited(Duration::from_secs(cli.timeout_secs), "E_QCE_TIMEOUT"),
        ) {
            Ok(bytes) => {
                serde_json::from_slice::<Value>(&bytes).unwrap_or(json!({"available":false}))
            }
            Err(error) if error.code == "E_CANCELLED" => return Err(error),
            Err(_) => json!({"available":false}),
        }
    } else {
        Value::Null
    };
    let token = credentials.qce(budget);
    let token_available = token.is_ok();
    let state = token.and_then(|token| qce.qce_online(&token, budget));
    budget.check()?;
    let authenticated = state.is_ok();
    let reachable = authenticated
        || token_available
            && state
                .as_ref()
                .is_err_and(|e| matches!(e.code, "E_QCE_AUTH" | "E_QCE_RESPONSE"));
    let logged_in = match &state {
        Ok(online) => Some(*online),
        Err(_) => Napcat::new(napcat, credentials)
            .status(budget)
            .and_then(|v| login::online(&v))
            .ok(),
    };
    budget.check()?;
    let container_arg = cli
        .docker
        .as_deref()
        .map(|v| format!(" --docker {v}"))
        .unwrap_or_default();
    let hint = format!(
        "chat-tldr-qce-manager --base-url {} --napcat-url {}{} login",
        qce.base, napcat.base, container_arg
    );
    output.ack("status", false, json!({"qce_reachable":reachable,"authenticated":authenticated,"qq_logged_in":logged_in,"qce_ready":state.as_ref().is_ok_and(|v| *v),"docker":docker_status,"login_hint":hint}))?;
    match state {
        Ok(true) => Ok(()),
        Ok(false) => Err(Failure::new(
            "E_QCE_LOGIN_REQUIRED",
            4,
            "QQ 未登录；请按 status 的 login_hint 在本机终端扫码",
        )),
        Err(error) => Err(error),
    }
}
