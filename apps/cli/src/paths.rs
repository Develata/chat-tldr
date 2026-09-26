use std::path::{Path, PathBuf};

use crate::Failure;

#[derive(Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    pub database: PathBuf,
    pub outputs_dir: PathBuf,
    pub qce_exports_dir: PathBuf,
}

impl Paths {
    pub fn resolve(data_dir: Option<&Path>, config: Option<&Path>) -> Result<Self, Failure> {
        let data_dir = absolute(&match data_dir {
            Some(path) => path.to_path_buf(),
            None => default_data_dir()?,
        })?;
        let config_file = match config {
            Some(path) => absolute(path)?,
            None => data_dir.join("config.toml"),
        };
        Ok(Self {
            config_file,
            database: data_dir.join("chat-tldr.db"),
            outputs_dir: data_dir.join("outputs"),
            qce_exports_dir: data_dir.join("sources/qce/exports"),
            data_dir,
        })
    }

    pub fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "data_dir": self.data_dir,
            "config_file": self.config_file,
            "database": self.database,
            "outputs_dir": self.outputs_dir,
            "qce_exports_dir": self.qce_exports_dir,
        })
    }
}

pub fn absolute(path: &Path) -> Result<PathBuf, Failure> {
    std::path::absolute(path).map_err(|error| {
        Failure::new(
            "E_CONFIG",
            4,
            format!("Cannot resolve the requested path: {error}"),
        )
    })
}

fn required_env(name: &str) -> Result<PathBuf, Failure> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            Failure::new(
                "E_CONFIG",
                4,
                format!("{name} is not set; pass --data-dir explicitly"),
            )
        })
}

#[cfg(target_os = "windows")]
fn default_data_dir() -> Result<PathBuf, Failure> {
    Ok(required_env("APPDATA")?.join("chat-tldr"))
}

#[cfg(target_os = "macos")]
fn default_data_dir() -> Result<PathBuf, Failure> {
    Ok(required_env("HOME")?.join("Library/Application Support/chat-tldr"))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn default_data_dir() -> Result<PathBuf, Failure> {
    let base = match std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(Failure::new(
                    "E_CONFIG",
                    4,
                    "XDG_DATA_HOME must be absolute",
                ));
            }
            path
        }
        None => required_env("HOME")?.join(".local/share"),
    };
    Ok(base.join("chat-tldr"))
}
