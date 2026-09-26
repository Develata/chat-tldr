use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chat_tldr_core::{
    AckPayload, ChatId, DB_VERSION, EventBody, ProgressPayload, SCHEMA_VERSION, StatsPayload,
    WarningPayload,
};
use chat_tldr_engine::store::ImportOptions;
use chat_tldr_engine::{Config, store};
use chrono::{DateTime, FixedOffset};
use serde_json::json;

use crate::Failure;
use crate::args::{Cli, Command, ConfigCommand, ImportArgs, MessagesArgs};
use crate::output::Output;
use crate::paths::{Paths, absolute};

const CONFIG_TEMPLATE: &str = include_str!("../../../config.example.toml");
const IMPLEMENTED_COMMANDS: &[&str] = &[
    "version",
    "config init",
    "doctor",
    "import",
    "chats",
    "messages",
];

pub fn run<W: Write>(cli: Cli, output: &mut Output<W>) -> Result<(), Failure> {
    if matches!(cli.command, Command::Version) {
        return version(output);
    }

    let paths = Paths::resolve(cli.data_dir.as_deref(), cli.config.as_deref())?;
    if cli.verbose > 0 {
        eprintln!("Using data directory: {}", paths.data_dir.display());
    }
    let config = Config::load(&paths.config_file, cli.config.is_some())?;
    match cli.command {
        Command::Version => unreachable!("version was handled without reading configuration"),
        Command::Config {
            command: ConfigCommand::Init { out },
        } => config_init(out, &paths, output),
        Command::Doctor => doctor(&paths, &config, output),
        Command::Import(args) => import(args, &paths, &config, output),
        Command::Chats => {
            for chat in store::list_chats(&paths.database)? {
                output.emit(EventBody::Chat(chat))?;
            }
            Ok(())
        }
        Command::Messages(args) => messages(args, &paths, output),
    }
}

fn ack<W: Write>(
    output: &mut Output<W>,
    command: &str,
    changed: bool,
    detail: serde_json::Value,
) -> Result<(), Failure> {
    output.emit(EventBody::Ack(AckPayload {
        command: command.to_owned(),
        target: None,
        changed,
        detail,
    }))?;
    Ok(())
}

fn version<W: Write>(output: &mut Output<W>) -> Result<(), Failure> {
    ack(
        output,
        "version",
        false,
        json!({
            "cli_version": env!("CARGO_PKG_VERSION"),
            "schema_version": SCHEMA_VERSION,
            "db_version": DB_VERSION,
            "capabilities": {
                "commands": IMPLEMENTED_COMMANDS,
                "strategies": [],
                "deciders": [],
            },
        }),
    )
}

fn config_init<W: Write>(
    out: Option<PathBuf>,
    paths: &Paths,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    let destination = match out {
        Some(path) => absolute(&path)?,
        None => paths.data_dir.join("config.toml"),
    };
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            Failure::new(
                "E_OUTPUT_WRITE",
                8,
                format!("Cannot create configuration directory: {error}"),
            )
        })?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                Failure::new(
                    "E_CONFIG",
                    4,
                    "Configuration file already exists; it was not overwritten",
                )
            } else {
                Failure::new(
                    "E_OUTPUT_WRITE",
                    8,
                    format!("Cannot create configuration file: {error}"),
                )
            }
        })?;
    file.write_all(CONFIG_TEMPLATE.as_bytes())?;
    file.sync_all()?;
    ack(
        output,
        "config init",
        true,
        json!({"config_file": destination}),
    )
}

fn doctor<W: Write>(paths: &Paths, config: &Config, output: &mut Output<W>) -> Result<(), Failure> {
    let database_version = store::inspect(&paths.database)?;
    if paths.data_dir.exists() && !paths.data_dir.is_dir() {
        return Err(Failure::new(
            "E_CONFIG",
            4,
            "The data directory path is not a directory",
        ));
    }
    let jev_key_present = env_present(&config.jev.api_key_env);
    let llm_key_present = env_present(&config.llm.api_key_env);
    ack(
        output,
        "doctor",
        false,
        json!({
            "paths": paths.as_json(),
            "database_version": database_version,
            "config_file_exists": paths.config_file.is_file(),
            "readiness": {"read": true, "import": true, "analyze": false},
            "analyze_implemented": false,
            "providers": {
                "jev": {"api_key_env": config.jev.api_key_env, "key_present": jev_key_present},
                "llm": {"api_key_env": config.llm.api_key_env, "key_present": llm_key_present},
            },
            "remote_checked": false,
        }),
    )?;
    if !llm_key_present {
        return Err(Failure::new(
            "E_CONFIG",
            4,
            format!(
                "LLM environment variable {} is not set; offline read and import remain available",
                config.llm.api_key_env
            ),
        ));
    }
    if !jev_key_present {
        output.emit(EventBody::Warning(WarningPayload {
            stage: "cli".to_owned(),
            code: "W_DECIDER_FALLBACK".to_owned(),
            message: "Jev credentials are absent; model analysis will require LLM fallback when implemented".to_owned(),
        }))?;
    }
    Ok(())
}

fn env_present(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

fn import<W: Write>(
    args: ImportArgs,
    paths: &Paths,
    config: &Config,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    for value in [&args.self_uid, &args.self_uin].into_iter().flatten() {
        if value.trim().is_empty() {
            return Err(Failure::new(
                "E_USAGE",
                2,
                "User identity overrides cannot be empty",
            ));
        }
    }
    let mut batches = Vec::with_capacity(args.paths.len());
    let mut file_paths = Vec::with_capacity(args.paths.len());
    let total = args.paths.len() as u64;
    for (index, path) in args.paths.iter().enumerate() {
        let path = absolute(path)?;
        batches.push(parse_export(&path, config)?);
        file_paths.push(path.to_string_lossy().into_owned());
        output.emit(EventBody::Progress(ProgressPayload {
            stage: "import".to_owned(),
            current: index as u64 + 1,
            total: Some(total),
            message: "Validated input file".to_owned(),
        }))?;
    }

    let report = store::import_batches(
        &paths.database,
        &batches,
        &ImportOptions {
            self_uid: args.self_uid,
            self_uin: args.self_uin,
            run_id: Some(output.run_id()),
            file_paths,
        },
    )?;
    for warning in report.warnings {
        let (code, message) = match warning.split_once(':') {
            Some((code, message))
                if code.starts_with("W_")
                    && code
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte == b'_') =>
            {
                (code.to_owned(), message.trim().to_owned())
            }
            _ => ("W_INPUT_NORMALIZED".to_owned(), warning),
        };
        output.emit(EventBody::Warning(WarningPayload {
            stage: "import".to_owned(),
            code,
            message,
        }))?;
    }
    ack(
        output,
        "import",
        report.changed,
        json!({"chat_ids": report.chat_ids}),
    )?;
    let stats: StatsPayload = serde_json::from_value(report.stats).map_err(|_| {
        Failure::new(
            "E_INTERNAL",
            1,
            "The import report did not match the shared protocol",
        )
    })?;
    output.emit(EventBody::Stats(stats))?;
    Ok(())
}

fn parse_export(path: &Path, config: &Config) -> Result<chat_tldr_core::ImportBatch, Failure> {
    let bytes = fs::read(path).map_err(|error| {
        let code = if error.kind() == std::io::ErrorKind::NotFound {
            "E_INPUT_NOT_FOUND"
        } else {
            "E_INPUT_PARSE"
        };
        let mut failure = Failure::new(
            code,
            3,
            format!("Cannot read input file {}: {error}", path.display()),
        );
        failure.stage = "import";
        failure
    })?;
    chat_tldr_qce::parse_qce_json(
        &bytes,
        &chat_tldr_qce::QceOptions {
            timezone: config.timezone_offset()?,
            ..Default::default()
        },
    )
    .map_err(|error| {
        let code = if matches!(error, chat_tldr_qce::QceError::ChatType) {
            "E_INPUT_UNSUPPORTED"
        } else {
            "E_INPUT_PARSE"
        };
        let mut failure = Failure::new(code, 3, error.to_string());
        failure.stage = "import";
        failure
    })
}

fn messages<W: Write>(
    args: MessagesArgs,
    paths: &Paths,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    let since = parse_time(args.since.as_deref(), "--since")?;
    let until = parse_time(args.until.as_deref(), "--until")?;
    if since.zip(until).is_some_and(|(since, until)| since > until) {
        return Err(Failure::new(
            "E_USAGE",
            2,
            "--since must not be later than --until",
        ));
    }
    for message in store::list_messages(&paths.database, &ChatId(args.chat), since, until)? {
        output.emit(EventBody::Message(message))?;
    }
    Ok(())
}

fn parse_time(
    value: Option<&str>,
    parameter: &str,
) -> Result<Option<DateTime<FixedOffset>>, Failure> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(value).map_err(|_| {
                Failure::new(
                    "E_USAGE",
                    2,
                    format!("{parameter} must be an RFC 3339 timestamp with a timezone offset"),
                )
            })
        })
        .transpose()
}
