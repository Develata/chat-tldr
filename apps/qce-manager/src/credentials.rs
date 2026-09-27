use std::{fs::File, io::Read, path::PathBuf, time::Duration};

use serde::Deserialize;

use crate::{
    args::Cli,
    docker::{CONFIG_LIMIT, Docker},
    error::{Failure, Result},
    runtime::Budget,
};

// Deliberately no Debug/Serialize: credentials never belong in diagnostics or receipts.
pub struct Secret(pub String);

#[derive(Default)]
pub struct Environment {
    pub qce_token: Option<String>,
    pub napcat_token: Option<String>,
    pub qce_config: Option<PathBuf>,
    pub home: Option<PathBuf>,
}

impl Environment {
    pub fn read() -> Self {
        Self {
            qce_token: std::env::var("CHAT_TLDR_QCE_TOKEN").ok(),
            napcat_token: std::env::var("CHAT_TLDR_NAPCAT_TOKEN").ok(),
            qce_config: std::env::var_os("QCE_CONFIG_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            home: std::env::home_dir(),
        }
    }
}

#[derive(Deserialize)]
struct QceConfig {
    #[serde(rename = "accessToken")]
    access_token: String,
}

#[derive(Deserialize)]
struct NapcatConfig {
    token: String,
}

pub struct Credentials<'a> {
    pub cli: &'a Cli,
    pub environment: Environment,
    pub docker: &'a dyn Docker,
}

impl Credentials<'_> {
    pub fn qce(&self, budget: Budget<'_>) -> Result<Secret> {
        self.resolve(true, budget)
    }

    pub fn napcat(&self, budget: Budget<'_>) -> Result<Secret> {
        self.resolve(false, budget)
    }

    fn resolve(&self, qce: bool, budget: Budget<'_>) -> Result<Secret> {
        budget.check()?;
        let env = if qce {
            &self.environment.qce_token
        } else {
            &self.environment.napcat_token
        };
        if let Some(value) = env.as_ref().filter(|v| !v.trim().is_empty()) {
            return Ok(Secret(value.clone()));
        }
        let mut tried = vec![if qce {
            "CHAT_TLDR_QCE_TOKEN"
        } else {
            "CHAT_TLDR_NAPCAT_TOKEN"
        }];
        if let Some(container) = self.cli.docker.as_deref() {
            tried.push("docker");
            let paths: Vec<&str> = if qce {
                self.cli.security_json_path.as_deref().map_or_else(
                    || {
                        vec![
                            "/app/.qq-chat-exporter/security.json",
                            "/root/.qq-chat-exporter/security.json",
                        ]
                    },
                    |path| vec![path],
                )
            } else {
                vec!["/app/napcat/config/webui.json"]
            };
            for path in paths {
                let result = self.docker.run(
                    &["exec", container, "cat", "--", path],
                    budget.limited(Duration::from_secs(self.cli.timeout_secs), "E_QCE_TIMEOUT"),
                );
                budget.check()?;
                if let Ok(bytes) = result
                    && let Some(token) = parse(&bytes, qce)
                {
                    return Ok(token);
                }
            }
        }
        let mut candidates = Vec::new();
        if qce {
            if let Some(dir) = &self.cli.qce_config_dir {
                candidates.push(("--qce-config-dir", dir.clone()));
            }
            if let Some(dir) = &self.environment.qce_config {
                candidates.push(("QCE_CONFIG_DIR", dir.clone()));
            }
            if let Some(home) = &self.environment.home {
                candidates.push(("home", home.join(".qq-chat-exporter")));
            }
        } else if let Some(dir) = &self.cli.napcat_config_dir {
            candidates.push(("--napcat-config-dir", dir.clone()));
        }
        for (name, dir) in candidates {
            tried.push(name);
            budget.check()?;
            let file = dir.join(if qce { "security.json" } else { "webui.json" });
            if let Ok(file) = File::open(file) {
                let mut bytes = Vec::new();
                if file.take(CONFIG_LIMIT + 1).read_to_end(&mut bytes).is_ok()
                    && let Some(token) = parse(&bytes, qce)
                {
                    return Ok(token);
                }
            }
        }
        Err(Failure::new(
            if qce { "E_QCE_AUTH" } else { "E_NAPCAT_AUTH" },
            4,
            format!("未取得访问令牌；尝试来源：{}", tried.join(", ")),
        ))
    }
}

fn parse(bytes: &[u8], qce: bool) -> Option<Secret> {
    if bytes.len() as u64 > CONFIG_LIMIT {
        return None;
    }
    let token = if qce {
        serde_json::from_slice::<QceConfig>(bytes)
            .ok()?
            .access_token
    } else {
        serde_json::from_slice::<NapcatConfig>(bytes).ok()?.token
    };
    (!token.trim().is_empty()).then_some(Secret(token))
}

#[cfg(test)]
mod tests;
