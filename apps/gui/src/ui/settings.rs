//! Provider editing is local UI state; the CLI owns validation and persistence.
use chat_tldr_core::settings::{ProviderSettings, ProviderUpdate, SecretString};
use eframe::egui::{self, RichText};

use super::{Action, button};
use crate::{model::GuiModel, prefs::Preferences};

#[derive(Default)]
pub struct ProviderPanel {
    pub key_visible: bool,
    pub save_visible: bool,
    pub attempted_load: bool,
    pub saving: Option<&'static str>,
    pub message: Option<String>,
    loaded_request: Option<u64>,
    revision: String,
    config_file: String,
    llm: Option<ProviderDraft>,
    jev: Option<ProviderDraft>,
    selected_jev: bool,
    connection: (String, String, String),
}

pub struct ProviderDraft {
    settings: ProviderSettings,
    key: SecretString,
    clear_key: bool,
    extra_body: String,
}

#[cfg(test)]
pub(crate) fn synthetic_snapshot() -> chat_tldr_core::settings::SettingsSnapshot {
    use chat_tldr_core::settings::{CredentialStatus, SettingsSnapshot};
    let llm = ProviderSettings {
        base_url: "https://synthetic.example/v1".into(),
        model: "synthetic-model".into(),
        api_format: "openai".into(),
        api_key_env: "SYNTHETIC_KEY".into(),
        timeout_secs: 30,
        temperature: 0.0,
        json_mode: true,
        extra_body: serde_json::json!({}),
        price_input_per_mtok: 0.0,
        price_output_per_mtok: 0.0,
        credential: CredentialStatus {
            source: "config".into(),
            available: true,
            problem: None,
        },
    };
    let jev = ProviderSettings {
        api_format: "jev".into(),
        ..llm.clone()
    };
    SettingsSnapshot {
        version: 1,
        revision: "synthetic-revision".into(),
        config_file: "synthetic/config.toml".into(),
        llm,
        jev,
    }
}

impl From<&ProviderSettings> for ProviderDraft {
    fn from(settings: &ProviderSettings) -> Self {
        Self {
            settings: settings.clone(),
            key: SecretString::default(),
            clear_key: false,
            extra_body: serde_json::to_string_pretty(&settings.extra_body)
                .unwrap_or_else(|_| "{}".into()),
        }
    }
}

impl ProviderPanel {
    pub fn new(prefs: &Preferences) -> Self {
        Self {
            connection: (
                prefs.cli.clone(),
                prefs.data_dir.clone(),
                prefs.config.clone(),
            ),
            ..Default::default()
        }
    }

    pub fn connection_matches(&self, prefs: &Preferences) -> bool {
        self.connection.0 == prefs.cli
            && self.connection.1 == prefs.data_dir
            && self.connection.2 == prefs.config
    }

    pub fn clear_inputs(&mut self) {
        for draft in [&mut self.llm, &mut self.jev].into_iter().flatten() {
            draft.key = SecretString::default();
        }
    }

    pub fn sync(&mut self, model: &GuiModel) {
        let Some((request, snapshot)) = &model.settings else {
            return;
        };
        if self.loaded_request == Some(*request) {
            return;
        }
        self.loaded_request = Some(*request);
        self.revision = snapshot.revision.clone();
        self.config_file = snapshot.config_file.clone();
        let saved = self.saving.take();
        if saved != Some("jev") {
            self.llm = Some((&snapshot.llm).into());
        }
        if saved != Some("llm") {
            self.jev = Some((&snapshot.jev).into());
        }
        self.message = saved.map(|name| {
            format!(
                "{} 配置已保存，下次分析立即使用。",
                if name == "llm" { "LLM" } else { "Jev" }
            )
        });
    }

    pub fn update(&self, provider: &str) -> Result<ProviderUpdate, String> {
        let draft = if provider == "llm" {
            &self.llm
        } else {
            &self.jev
        };
        let draft = draft.as_ref().ok_or("请先读取模型配置")?;
        let extra: serde_json::Value = serde_json::from_str(&draft.extra_body)
            .map_err(|_| "额外请求参数必须是 JSON 对象".to_owned())?;
        if !extra.is_object() {
            return Err("额外请求参数必须是 JSON 对象".into());
        }
        let s = &draft.settings;
        Ok(ProviderUpdate {
            expected_revision: Some(self.revision.clone()),
            base_url: Some(s.base_url.clone()),
            model: Some(s.model.clone()),
            api_format: Some(s.api_format.clone()),
            api_key_env: Some(s.api_key_env.clone()),
            timeout_secs: Some(s.timeout_secs),
            temperature: Some(s.temperature),
            json_mode: Some(s.json_mode),
            extra_body: Some(extra),
            price_input_per_mtok: Some(s.price_input_per_mtok),
            price_output_per_mtok: Some(s.price_output_per_mtok),
            key: (!draft.key.is_empty() && !draft.clear_key).then(|| draft.key.clone()),
            clear_key: draft.clear_key,
        })
    }
}

pub fn render(
    ui: &mut egui::Ui,
    panel: &mut ProviderPanel,
    model: &GuiModel,
    enabled: bool,
    actions: &mut Vec<Action>,
) {
    panel.key_visible = false;
    panel.sync(model);
    ui.heading("模型与 API");
    ui.small(
        "API key 明文保存在本地 config.toml；输入框遮住内容，日志不会回显。留空会保留已有密钥。",
    );
    let supported = model.has_capability("config show") && model.has_capability("config set");
    if !supported {
        ui.label("连接支持模型设置的 CLI 后，可在这里填写服务地址、模型和 API key。");
        return;
    }
    if let Some(message) = &panel.message {
        ui.label(RichText::new(message).strong());
    }
    ui.horizontal(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            ui.selectable_value(&mut panel.selected_jev, false, "LLM · 内容分析");
            ui.selectable_value(&mut panel.selected_jev, true, "Jev · 决策");
        });
        button(ui, "重新读取配置", enabled, Action::LoadProviders, actions);
    });
    if !panel.config_file.is_empty() {
        ui.small(&panel.config_file);
    }
    let name = if panel.selected_jev { "jev" } else { "llm" };
    let draft = if panel.selected_jev {
        &mut panel.jev
    } else {
        &mut panel.llm
    };
    let Some(draft) = draft else {
        ui.label(if enabled {
            "尚未读取配置；点击重新读取配置。"
        } else {
            "正在读取配置…"
        });
        return;
    };
    ui.add_enabled_ui(enabled, |ui| {
        if name == "llm" {
            ui.horizontal(|ui| {
                ui.label("接口格式");
                let old = draft.settings.api_format.clone();
                egui::ComboBox::from_id_salt("llm-api-format")
                    .selected_text(if old == "anthropic" { "Anthropic Messages" } else { "OpenAI Chat Completions" })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut draft.settings.api_format, "openai".into(), "OpenAI Chat Completions");
                        ui.selectable_value(&mut draft.settings.api_format, "anthropic".into(), "Anthropic Messages");
                    });
                if old != draft.settings.api_format {
                    draft.extra_body = "{}".into();
                    if draft.settings.api_format == "anthropic" { draft.settings.temperature = draft.settings.temperature.min(1.0); }
                }
            });
        } else {
            ui.label("接口格式：Jev SystemOne");
            ui.small("未配置 Jev 时，内容分析仍可使用 LLM 作为决策回退。");
        }
        ui.label("服务地址（Base URL）");
        if ui.add(egui::TextEdit::singleline(&mut draft.settings.base_url).desired_width(f32::INFINITY)).changed() {
            draft.extra_body = "{}".into();
        }
        ui.small(match draft.settings.api_format.as_str() {
            "anthropic" => "例如 https://api.anthropic.com；程序调用 /v1/messages。",
            "jev" => "默认 https://api.typesafe.ai；程序调用 /v1/systemone。",
            _ => "例如 https://api.openai.com/v1 或 https://api.deepseek.com；程序调用 /chat/completions。",
        });
        ui.label("模型名称");
        ui.add(egui::TextEdit::singleline(&mut draft.settings.model).desired_width(f32::INFINITY));
        ui.horizontal_wrapped(|ui| {
            ui.label("API key");
            let status = &draft.settings.credential;
            let label = if !status.available { "尚未配置有效密钥" }
                else if status.source == "config" { "已保存在 config（明文）" }
                else { "当前使用环境变量" };
            ui.small(label);
        });
        let key_input = ui.add_enabled(!draft.clear_key, egui::TextEdit::singleline(draft.key.edit())
            .password(true).char_limit(4096).hint_text("填写新密钥；留空保留原值").desired_width(f32::INFINITY));
        panel.key_visible = ui.clip_rect().contains_rect(key_input.rect);
        ui.checkbox(&mut draft.clear_key, "移除 config 中的密钥，改用环境变量");
        if draft.settings.credential.source == "config" {
            ui.small("更改服务地址或接口格式时，请重新填写该服务的密钥。");
        }
        egui::CollapsingHeader::new("高级设置").id_salt(name).show(ui, |ui| {
            ui.label("环境变量名（config 没有密钥时使用）");
            ui.text_edit_singleline(&mut draft.settings.api_key_env);
            ui.horizontal(|ui| {
                ui.label("请求超时（秒）");
                ui.add(egui::DragValue::new(&mut draft.settings.timeout_secs).range(1..=3600));
                if name == "llm" {
                    ui.label("Temperature");
                    let max = if draft.settings.api_format == "anthropic" { 1.0 } else { 2.0 };
                    ui.add(egui::DragValue::new(&mut draft.settings.temperature).speed(0.05).range(0.0..=max));
                }
            });
            if name == "llm" {
                ui.add_enabled(draft.settings.api_format == "openai", egui::Checkbox::new(&mut draft.settings.json_mode, "请求 JSON 输出"));
                ui.label("额外请求参数（JSON 对象）");
                ui.small("更改地址或格式会清空这里的服务专属参数，需要时可重新填写。");
                ui.add(egui::TextEdit::multiline(&mut draft.extra_body).desired_rows(3).desired_width(f32::INFINITY).code_editor());
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("费用估算 · 美元 / 百万 token");
                ui.label("输入"); ui.add(egui::DragValue::new(&mut draft.settings.price_input_per_mtok).range(0.0..=10000.0).speed(0.01));
                ui.label("输出"); ui.add(egui::DragValue::new(&mut draft.settings.price_output_per_mtok).range(0.0..=10000.0).speed(0.01));
            });
        });
    });
}

pub fn footer(
    ui: &mut egui::Ui,
    panel: &mut ProviderPanel,
    enabled: bool,
    actions: &mut Vec<Action>,
) {
    let (name, draft) = if panel.selected_jev {
        ("jev", &panel.jev)
    } else {
        ("llm", &panel.llm)
    };
    let valid = draft.as_ref().is_some_and(|draft| {
        !draft.settings.base_url.trim().is_empty() && !draft.settings.model.trim().is_empty()
    });
    let response = ui.add_enabled(
        enabled && valid,
        egui::Button::new(if name == "llm" {
            "保存 LLM 配置"
        } else {
            "保存 Jev 配置"
        }),
    );
    panel.save_visible = ui.clip_rect().contains_rect(response.rect);
    if response.clicked() {
        actions.push(Action::SaveProvider(name));
    }
}
