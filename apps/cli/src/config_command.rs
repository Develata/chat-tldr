//! Configuration publication and credentials are coordinated here, never in the GUI.
use std::{
    fs,
    io::{self, IsTerminal, Read, Write},
    path::Path,
};

use chat_tldr_core::{
    AckPayload, EventBody,
    settings::{ProviderSettings, ProviderUpdate, SecretString, SettingsSnapshot},
};
use chat_tldr_engine::{Config, config::ProviderConfig};
use fs2::FileExt;
use toml_edit::{DocumentMut, Item, Table, value};
use zeroize::Zeroizing;

use crate::{Failure, args::ConfigSetArgs, credentials, output::Output, paths::Paths};

const MAX_INPUT: u64 = 64 * 1024;

pub fn show<W: Write>(
    paths: &Paths,
    explicit: bool,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    let source = read_source(&paths.config_file, explicit)?;
    let config = Config::parse(&source)?;
    emit(
        output,
        "config show",
        false,
        &snapshot(paths, &source, &config),
    )
}

pub fn set<W: Write>(
    args: ConfigSetArgs,
    paths: &Paths,
    output: &mut Output<W>,
) -> Result<(), Failure> {
    let provider = args.provider.clone();
    let update = read_update(args)?;
    let result = apply(paths, &provider, update, publish)?;
    emit(
        output,
        "config set",
        result.changed,
        &snapshot(paths, &result.source, &result.config),
    )
}

fn emit<W: Write>(
    output: &mut Output<W>,
    command: &str,
    changed: bool,
    snapshot: &SettingsSnapshot,
) -> Result<(), Failure> {
    output.emit(EventBody::Ack(AckPayload {
        command: command.into(),
        target: None,
        changed,
        detail: serde_json::to_value(snapshot)
            .map_err(|_| Failure::new("E_INTERNAL", 1, "Cannot encode configuration status"))?,
    }))?;
    Ok(())
}

fn revision(source: &str) -> String {
    blake3::hash(source.as_bytes()).to_hex().to_string()
}

fn snapshot(paths: &Paths, source: &str, config: &Config) -> SettingsSnapshot {
    SettingsSnapshot {
        version: 1,
        config_file: paths.config_file.to_string_lossy().into_owned(),
        revision: revision(source),
        llm: provider_settings("llm", &config.llm),
        jev: provider_settings("jev", &config.jev),
    }
}

fn provider_settings(name: &str, provider: &ProviderConfig) -> ProviderSettings {
    ProviderSettings {
        base_url: provider.base_url.clone(),
        model: provider.model.clone(),
        api_format: provider.api_format.clone().unwrap_or_else(|| "jev".into()),
        api_key_env: provider.api_key_env.clone(),
        timeout_secs: provider.timeout_secs,
        temperature: provider.temperature.unwrap_or(0.0),
        json_mode: provider.json_mode.unwrap_or(false),
        extra_body: provider
            .extra_body
            .as_ref()
            .and_then(|body| serde_json::to_value(body).ok())
            .unwrap_or_else(|| serde_json::json!({})),
        price_input_per_mtok: provider.price_input_per_mtok.unwrap_or(if name == "jev" {
            0.042
        } else {
            0.0
        }),
        price_output_per_mtok: provider.price_output_per_mtok.unwrap_or(0.0),
        credential: credentials::status(&credentials::resolve(provider), provider),
    }
}

fn read_bounded(input: impl Read) -> Result<Zeroizing<Vec<u8>>, Failure> {
    let mut bytes = Zeroizing::new(Vec::new());
    input
        .take(MAX_INPUT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::new("E_CONFIG", 4, "Cannot read configuration input"))?;
    if bytes.len() as u64 > MAX_INPUT {
        return Err(Failure::new(
            "E_CONFIG",
            4,
            "Configuration input exceeds 64 KiB",
        ));
    }
    Ok(bytes)
}

fn read_update(args: ConfigSetArgs) -> Result<ProviderUpdate, Failure> {
    if args.request_stdin {
        let bytes = read_bounded(io::stdin().lock())?;
        return serde_json::from_slice(&bytes).map_err(|_| {
            Failure::new(
                "E_CONFIG",
                4,
                "Invalid provider update JSON; input values are not logged",
            )
        });
    }
    let key = if args.key_prompt {
        if !io::stdin().is_terminal() {
            return Err(Failure::new(
                "E_CONFIG",
                4,
                "A terminal is required for --key-prompt; use --key-stdin for a pipe",
            ));
        }
        eprint!("API key (hidden): ");
        io::stderr().flush()?;
        Some(SecretString::new(rpassword::read_password().map_err(
            |_| Failure::new("E_CONFIG", 4, "Cannot read API key from terminal"),
        )?))
    } else if args.key_stdin {
        let bytes = read_bounded(io::stdin().lock())?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| Failure::new("E_CONFIG", 4, "API key must be UTF-8"))?;
        let text = text.strip_suffix('\n').unwrap_or(text);
        Some(SecretString::new(
            text.strip_suffix('\r').unwrap_or(text).into(),
        ))
    } else if let Some(name) = &args.key_from_env {
        Some(SecretString::new(std::env::var(name).map_err(|_| {
            Failure::new(
                "E_CONFIG",
                4,
                "The requested key environment variable is unavailable",
            )
        })?))
    } else {
        None
    };
    Ok(ProviderUpdate {
        base_url: args.base_url,
        model: args.model,
        api_format: args.api_format,
        api_key_env: args.api_key_env,
        timeout_secs: args.timeout_secs,
        temperature: args.temperature,
        json_mode: args.json_mode,
        extra_body: args
            .extra_body
            .map(|text| {
                serde_json::from_str(&text)
                    .map_err(|_| Failure::new("E_CONFIG", 4, "extra-body must be a JSON object"))
            })
            .transpose()?,
        price_input_per_mtok: args.price_input_per_mtok,
        price_output_per_mtok: args.price_output_per_mtok,
        key,
        clear_key: args.clear_key,
        ..Default::default()
    })
}

fn read_source(path: &Path, explicit: bool) -> Result<String, Failure> {
    match fs::read_to_string(path) {
        Ok(source) => Ok(source),
        Err(error) if !explicit && error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(_) => Err(Failure::new(
            "E_CONFIG",
            4,
            "Cannot read configuration file",
        )),
    }
}

struct UpdateResult {
    source: String,
    config: Config,
    changed: bool,
}

fn provider<'a>(config: &'a Config, name: &str) -> &'a ProviderConfig {
    if name == "llm" {
        &config.llm
    } else {
        &config.jev
    }
}

fn apply(
    paths: &Paths,
    name: &str,
    update: ProviderUpdate,
    publish: impl FnOnce(&Path, &str) -> Result<(), Failure>,
) -> Result<UpdateResult, Failure> {
    if !matches!(name, "llm" | "jev") || (update.clear_key && update.key.is_some()) {
        return Err(Failure::new(
            "E_CONFIG",
            4,
            "Invalid provider or conflicting key operations",
        ));
    }
    if let Some(key) = &update.key {
        credentials::validate(key)?;
    }
    let parent = paths
        .config_file
        .parent()
        .ok_or_else(|| Failure::new("E_CONFIG", 4, "Configuration needs a parent directory"))?;
    fs::create_dir_all(parent)?;
    // A separate stable lock survives atomic file replacement across processes.
    let lock_path = paths.config_file.with_extension("toml.lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    lock.try_lock_exclusive().map_err(|_| {
        Failure::new(
            "E_CONFIG",
            4,
            "Configuration is being updated by another process; retry",
        )
    })?;
    let original = read_source(&paths.config_file, false)?;
    if update
        .expected_revision
        .as_ref()
        .is_some_and(|expected| *expected != revision(&original))
    {
        return Err(Failure::new(
            "E_CONFIG",
            4,
            "Configuration changed since it was loaded. Reload settings before saving",
        ));
    }
    let old = Config::parse(&original)?;
    let mut doc: DocumentMut = original
        .parse()
        .map_err(|_| Failure::new("E_CONFIG", 4, "Invalid TOML syntax"))?;
    doc.entry(name).or_insert(Item::Table(Table::new()));
    patch(&mut doc, name, &update)?;
    // Provider-specific defaults (e.g. DeepSeek's thinking switch) must not
    // travel to a different API unless the caller explicitly supplies them.
    let previous = provider(&old, name);
    if name == "llm"
        && update.extra_body.is_none()
        && (update.base_url.as_ref().is_some_and(|url| {
            url.trim_end_matches('/') != previous.base_url.trim_end_matches('/')
        }) || update
            .api_format
            .as_ref()
            .is_some_and(|format| Some(format.as_str()) != previous.api_format.as_deref()))
    {
        doc[name]["extra_body"] = Item::Value(toml_edit::InlineTable::new().into());
    }
    let candidate = Config::parse(&doc.to_string())?;
    let before = provider(&old, name);
    let after = provider(&candidate, name);
    if before.api_key.is_some()
        && update.key.is_none()
        && !update.clear_key
        && (before.base_url.trim_end_matches('/') != after.base_url.trim_end_matches('/')
            || before.api_format != after.api_format)
    {
        return Err(Failure::new(
            "E_CONFIG",
            4,
            "Changing provider address or API format requires a new key or --clear-key",
        ));
    }
    if let Some(key) = &update.key {
        doc[name]["api_key"] = value(key.expose());
    } else if update.clear_key {
        doc[name]
            .as_table_like_mut()
            .expect("validated provider table")
            .remove("api_key");
    }
    let source = doc.to_string();
    let config = Config::parse(&source)?;
    let changed = original != source;
    if changed {
        publish(&paths.config_file, &source)?;
    }
    Ok(UpdateResult {
        source,
        config,
        changed,
    })
}

fn patch(doc: &mut DocumentMut, name: &str, update: &ProviderUpdate) -> Result<(), Failure> {
    if name == "jev"
        && update
            .api_format
            .as_deref()
            .is_some_and(|format| format != "jev")
    {
        return Err(Failure::new(
            "E_CONFIG",
            4,
            "Jev uses the SystemOne protocol; openai/anthropic apply only to llm",
        ));
    }
    for (key, text) in [
        ("base_url", &update.base_url),
        ("model", &update.model),
        ("api_key_env", &update.api_key_env),
    ] {
        if let Some(text) = text {
            doc[name][key] = value(text.trim());
        }
    }
    if name == "llm"
        && let Some(format) = &update.api_format
    {
        doc[name]["api_format"] = value(format);
    }
    if let Some(timeout) = update.timeout_secs {
        doc[name]["timeout_secs"] = value(
            i64::try_from(timeout)
                .map_err(|_| Failure::new("E_CONFIG", 4, "Timeout is too large"))?,
        );
    }
    if let Some(temperature) = update.temperature {
        doc[name]["temperature"] = value(temperature);
    }
    if let Some(json_mode) = update.json_mode {
        doc[name]["json_mode"] = value(json_mode);
    }
    for (key, price) in [
        ("price_input_per_mtok", update.price_input_per_mtok),
        ("price_output_per_mtok", update.price_output_per_mtok),
    ] {
        if let Some(price) = price {
            doc[name][key] = value(price);
        }
    }
    if let Some(extra) = &update.extra_body {
        if !extra.is_object() {
            return Err(Failure::new(
                "E_CONFIG",
                4,
                "extra_body must be a JSON object",
            ));
        }
        doc[name]["extra_body"] = Item::Value(json_to_toml(extra)?);
    }
    Ok(())
}

fn json_to_toml(json: &serde_json::Value) -> Result<toml_edit::Value, Failure> {
    use serde_json::Value as J;
    Ok(match json {
        J::String(s) => s.as_str().into(),
        J::Bool(b) => (*b).into(),
        J::Number(n) if n.is_i64() => n.as_i64().unwrap().into(),
        J::Number(n) if n.is_u64() => {
            return Err(Failure::new(
                "E_CONFIG",
                4,
                "Request-body integer exceeds TOML's signed 64-bit range",
            ));
        }
        J::Number(n) => n
            .as_f64()
            .ok_or_else(|| Failure::new("E_CONFIG", 4, "Invalid request-body number"))?
            .into(),
        J::Array(values) => values
            .iter()
            .map(json_to_toml)
            .collect::<Result<toml_edit::Array, _>>()?
            .into(),
        J::Object(values) => values
            .iter()
            .map(|(k, v)| Ok((k.clone(), json_to_toml(v)?)))
            .collect::<Result<toml_edit::InlineTable, Failure>>()?
            .into(),
        J::Null => {
            return Err(Failure::new(
                "E_CONFIG",
                4,
                "TOML request-body extensions do not support null",
            ));
        }
    })
}

fn publish(path: &Path, source: &str) -> Result<(), Failure> {
    let mut file = tempfile::NamedTempFile::new_in(path.parent().expect("resolved parent"))?;
    file.write_all(source.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|_| {
        Failure::new(
            "E_OUTPUT_WRITE",
            8,
            "Cannot publish configuration; original settings were retained",
        )
    })?;
    Ok(())
}

#[cfg(test)]
#[path = "config_command_tests.rs"]
mod tests;
