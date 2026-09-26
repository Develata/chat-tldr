//! Rendering returns actions; it never starts processes or reads files.
use chat_tldr_core::{
    Assignee, ChatId, EvidenceView, InsightPayload, Lifecycle, Priority, VerificationStatus,
};
use eframe::egui::{self, Color32, RichText};

use crate::{model::GuiModel, prefs::Preferences};

mod overview;
mod review;
use review::ReviewCoverage;

#[cfg(test)]
#[path = "ui/tests.rs"]
mod interaction_tests;

#[derive(Default)]
pub struct UiState {
    pub overview: bool,
    pub overview_hours: u32,
    overview_tab: usize,
    overview_page: usize,
    pub all: bool,
    pub resolved: bool,
    pub lane: usize,
    pub settings: bool,
    pub logs: bool,
    pub cloud_notice: bool,
    pub cloud_target: Option<ChatId>,
    pub draft: Preferences,
    inbox_request: Option<u64>,
    lanes: [Vec<usize>; 3],
    pages: [usize; 3],
    review: ReviewCoverage,
}

impl UiState {
    pub fn new(draft: Preferences) -> Self {
        Self {
            draft,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Select(ChatId),
    Refresh,
    PickImport,
    Analyze,
    MarkRead,
    Cancel,
    Feedback(String, bool),
    Resolve(String, &'static str),
    SaveSettings,
    AcceptCloud(ChatId),
    Stats,
    Decisions,
    JevLog,
    Overview(u32),
}

pub fn render(
    ui: &mut egui::Ui,
    state: &mut UiState,
    model: &GuiModel,
    busy: bool,
    demo: bool,
) -> (Vec<Action>, Option<u64>) {
    let mut actions = Vec::new();
    let enabled = !busy && !demo && model.handshake_ok();
    let analysis_supported = model.capabilities.as_ref().is_some_and(|caps| {
        caps.strategies.iter().any(|value| value == "ours")
            && caps.deciders.iter().any(|value| value == "jev")
    });
    if let Some(inbox) = &model.inbox
        && state.inbox_request != Some(inbox.request_id)
    {
        for lane in &mut state.lanes {
            lane.clear();
        }
        for (index, row) in inbox.insights.iter().enumerate() {
            state.lanes[lane_of(row.insight.priority)].push(index);
        }
        state.inbox_request = Some(inbox.request_id);
        state.pages = [0; 3];
        state.review.reset(inbox.insights.len());
    }
    egui::Panel::top("toolbar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("群聊省流").strong().size(21.0));
            ui.add_space(18.0);
            if ui.selectable_label(state.overview, "分析总览").clicked()
                && enabled
                && model.has_capability("overview")
                && model.selected_chat.is_some()
            {
                state.overview = true;
                state.overview_page = 0;
                actions.push(Action::Overview(if state.overview_hours == 0 {
                    24
                } else {
                    state.overview_hours
                }));
            }
            if ui.selectable_label(!state.overview, "收件箱").clicked() {
                state.overview = false;
            }
            button(
                ui,
                "导入 JSON",
                enabled && model.has_capability("import"),
                Action::PickImport,
                &mut actions,
            );
            button(
                ui,
                "分析新消息",
                enabled
                    && model.selected_chat.is_some()
                    && model.has_capability("analyze")
                    && analysis_supported,
                Action::Analyze,
                &mut actions,
            );
            button(
                ui,
                "标为已读",
                enabled
                    && !state.overview
                    && model.mark_read_cursor().is_some()
                    && model.has_capability("mark-read"),
                Action::MarkRead,
                &mut actions,
            );
            button(
                ui,
                "刷新",
                enabled && model.has_capability("chats"),
                Action::Refresh,
                &mut actions,
            );
            if busy {
                ui.spinner();
                button(ui, "停止", true, Action::Cancel, &mut actions);
            }
            if ui.button("设置").clicked() {
                state.settings = true;
            }
            ui.toggle_value(&mut state.logs, "运行记录");
        });
        if demo {
            ui.small("合成演示 · 不读取聊天数据库，不调用云服务，所有写操作已禁用");
        }
        if let Some(progress) = &model.progress {
            ui.horizontal(|ui| {
                ui.small(&progress.message);
                if let Some(total) = progress.total.filter(|total| *total > 0) {
                    ui.add(
                        egui::ProgressBar::new((progress.current as f32 / total as f32).min(1.0))
                            .desired_width(160.0),
                    );
                }
            });
        }
        if let Some(error) = &model.last_error {
            ui.colored_label(error_color(ui), error);
        }
    });
    egui::Panel::left("chats")
        .default_size(212.0)
        .min_size(160.0)
        .resizable(true)
        .show(ui, |ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("群聊").strong());
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .id_salt("chat-list")
                .show(ui, |ui| {
                    for chat in &model.chats {
                        let selected = model.selected_chat.as_ref() == Some(&chat.chat_id);
                        let label = format!(
                            "{}\n{} 条未读 · {} 项 P0",
                            chat.display_name, chat.unreviewed_messages, chat.open_p0
                        );
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(label)
                                    .selected(selected)
                                    .min_size(egui::vec2(ui.available_width(), 64.0)),
                            )
                            .clicked()
                        {
                            actions.push(Action::Select(chat.chat_id.clone()));
                        }
                    }
                    if model.chats.is_empty() {
                        ui.weak("导入 QCE 导出的 JSON，\n即可在这里选择群聊。");
                    }
                });
        });
    if state.logs {
        egui::Panel::bottom("run-log")
            .default_size(210.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("运行记录");
                    ui.weak("每类保留最近 200 条");
                    button(
                        ui,
                        "统计",
                        enabled && model.has_capability("stats"),
                        Action::Stats,
                        &mut actions,
                    );
                    button(
                        ui,
                        "决策记录",
                        enabled && model.stats.is_some() && model.has_capability("decisions"),
                        Action::Decisions,
                        &mut actions,
                    );
                    button(
                        ui,
                        "模型回答",
                        enabled && model.stats.is_some() && model.has_capability("jev-log"),
                        Action::JevLog,
                        &mut actions,
                    );
                });
                egui::ScrollArea::vertical()
                    .id_salt("log-scroll")
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if let Some(stats) = &model.stats {
                            ui.small(format!(
                                "本轮 {} 条消息 · 新增 {} 项 · 报告用量费用 ${:.4} · {:.1} 秒",
                                stats.messages_analyzed,
                                stats.insights.created,
                                stats.cost_usd,
                                stats.elapsed_ms as f64 / 1000.0
                            ));
                        }
                        for decision in &model.decisions {
                            ui.small(format!(
                                "步骤 {} · {:?} · {}",
                                decision.step, decision.chosen, decision.reason
                            ));
                        }
                        for stats in &model.history_stats {
                            ui.small(format!("{stats:?}"));
                        }
                        for answer in &model.jev_answers {
                            ui.small(format!(
                                "{} · {} · {}",
                                answer.model, answer.question_id, answer.answer
                            ));
                        }
                        for line in &model.logs {
                            ui.small(line);
                        }
                    });
            });
    }
    let mut displayed = None;
    egui::CentralPanel::default().show(ui, |ui| {
        if state.overview {
            overview::render(ui, state, model, enabled, busy, &mut actions);
            return;
        }
        ui.add_space(12.0);
        let title = model
            .chats
            .iter()
            .find(|chat| Some(&chat.chat_id) == model.selected_chat.as_ref())
            .map(|c| c.display_name.as_str())
            .unwrap_or("收件箱");
        ui.heading(title);
        ui.horizontal_wrapped(|ui| {
            let filter_enabled =
                enabled && model.has_capability("chats") && model.has_capability("inbox");
            let changed = ui
                .add_enabled(
                    filter_enabled,
                    egui::Checkbox::new(&mut state.all, "包含已读"),
                )
                .changed()
                | ui.add_enabled(
                    filter_enabled,
                    egui::Checkbox::new(&mut state.resolved, "包含已处理"),
                )
                .changed();
            if changed {
                actions.push(Action::Refresh);
            }
            if let Some(inbox) = &model.inbox {
                ui.weak(format!(
                    "更新于 {}",
                    inbox.meta.generated_at.format("%m-%d %H:%M")
                ));
            }
        });
        ui.add_space(12.0);
        let Some(inbox) = &model.inbox else {
            if busy {
                ui.label("正在读取，请稍候…");
            } else if !model.handshake_ok() && !demo {
                ui.label("尚未连接 CLI。请在设置中检查程序路径，然后重新连接。");
            } else {
                ui.label("选择群聊，查看结论和原文证据。首次使用请先导入 JSON。");
            }
            return;
        };
        ui.weak(format!(
            "{} 个话题 · {} 项结论",
            inbox.topics.len(),
            inbox.insights.len()
        ));
        if !state.review.complete() {
            ui.weak(format!(
                "还有 {} 项尚未完整显示；查看各栏、翻页和滚动后才能标为已读。",
                state.review.remaining()
            ));
        }
        // Partition only when a new snapshot arrives; lay out at most 20 rows per lane.
        if ui.available_width() >= 1000.0 {
            ui.columns(3, |columns| {
                for (index, column) in columns.iter_mut().enumerate() {
                    lane(column, index, state, enabled, model, &mut actions);
                }
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                for (index, label) in ["P0 必须处理", "P1 值得关注", "P2 / P3 参考"]
                    .iter()
                    .enumerate()
                {
                    ui.selectable_value(
                        &mut state.lane,
                        index,
                        format!("{label}  {}", state.lanes[index].len()),
                    );
                }
            });
            lane(ui, state.lane, state, enabled, model, &mut actions);
        }
        if state.review.complete() {
            displayed = Some(inbox.request_id);
        }
    });
    dialogs(ui.ctx(), state, model, busy, demo, &mut actions);
    (actions, displayed)
}

fn lane(
    ui: &mut egui::Ui,
    index: usize,
    state: &mut UiState,
    enabled: bool,
    model: &GuiModel,
    actions: &mut Vec<Action>,
) {
    let rows = &state.lanes[index];
    let page = &mut state.pages[index];
    let titles = ["P0  必须处理", "P1  值得关注", "P2 / P3  参考"];
    ui.label(
        RichText::new(format!("{} · {}", titles[index], rows.len()))
            .strong()
            .size(17.0),
    );
    ui.weak(
        [
            "含所有带截止日期的结论",
            "与你有关的消息与群公告",
            "话题摘要与其他信息",
        ][index],
    );
    ui.separator();
    let pages = rows.len().div_ceil(20).max(1);
    *page = (*page).min(pages - 1);
    if pages > 1 {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(*page > 0, egui::Button::new("上一页"))
                .clicked()
            {
                *page -= 1;
            }
            ui.small(format!("{} / {pages}", *page + 1));
            if ui
                .add_enabled(*page + 1 < pages, egui::Button::new("下一页"))
                .clicked()
            {
                *page += 1;
            }
        });
    }
    egui::ScrollArea::vertical()
        .id_salt(("lane", index))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if rows.is_empty() {
                ui.add_space(18.0);
                ui.weak("暂无事项");
            }
            for &row_index in rows.iter().skip(*page * 20).take(20) {
                let row = &model.inbox.as_ref().expect("lane needs inbox").insights[row_index];
                let rendered = ui.push_id(&row.insight.id.0, |ui| {
                    insight(ui, row, enabled, model, actions);
                });
                state
                    .review
                    .observe(row_index, rendered.response.rect, ui.clip_rect());
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(12.0);
            }
        });
}

fn insight(
    ui: &mut egui::Ui,
    row: &InsightPayload,
    enabled: bool,
    model: &GuiModel,
    actions: &mut Vec<Action>,
) {
    let item = &row.insight;
    ui.label(RichText::new(&item.title).size(19.0).strong());
    ui.label(&item.summary);
    let assignee = match item.assignee {
        Assignee::Me => "我",
        Assignee::All => "全体",
        Assignee::Other => "其他人",
        Assignee::Unknown => "待确认",
    };
    ui.horizontal_wrapped(|ui| {
        ui.small(format!("负责人：{assignee}"));
        let verification = match item.verification_status {
            VerificationStatus::Verified => "证据已核验",
            VerificationStatus::Unverified => "证据待核验",
            _ => "证据未通过",
        };
        ui.small(verification);
        if item.lifecycle != Lifecycle::Open {
            ui.small(match item.lifecycle {
                Lifecycle::Done => "已完成",
                Lifecycle::Dismissed => "已忽略",
                _ => "未知状态",
            });
        }
    });
    if let Some(deadline) = &item.deadline {
        ui.label(RichText::new(format!("时间：{}", deadline.raw)).color(error_color(ui)));
        if let Some(date) = deadline.bound_date {
            ui.small(format!(
                "{} {} · 时区 {}",
                date,
                deadline
                    .bound_time
                    .map(|t| t.format("%H:%M").to_string())
                    .unwrap_or_default(),
                deadline.anchor.offset()
            ));
        }
    }
    egui::CollapsingHeader::new(format!("原文证据 · {} 条", row.evidence_view.len()))
        .default_open(true)
        .show(ui, |ui| {
            for evidence in &row.evidence_view {
                ui.small(format!(
                    "{} · {}",
                    evidence.sender_display,
                    evidence.sent_at.format("%m-%d %H:%M")
                ));
                ui.label(evidence_layout(
                    evidence,
                    ui.visuals().text_color(),
                    ui.visuals().dark_mode,
                ));
                if !evidence.ok {
                    ui.colored_label(error_color(ui), "引用未通过核验，请直接核对原文。");
                }
            }
        });
    ui.horizontal_wrapped(|ui| {
        let vote = enabled && model.has_capability("feedback");
        button(
            ui,
            "有用",
            vote,
            Action::Feedback(item.id.0.clone(), true),
            actions,
        );
        button(
            ui,
            "不重要",
            vote,
            Action::Feedback(item.id.0.clone(), false),
            actions,
        );
        let resolve = enabled && model.has_capability("resolve");
        if item.lifecycle == Lifecycle::Open {
            button(
                ui,
                "完成",
                resolve,
                Action::Resolve(item.id.0.clone(), "--done"),
                actions,
            );
            button(
                ui,
                "忽略",
                resolve,
                Action::Resolve(item.id.0.clone(), "--dismiss"),
                actions,
            );
        } else if matches!(item.lifecycle, Lifecycle::Done | Lifecycle::Dismissed) {
            button(
                ui,
                "恢复",
                resolve,
                Action::Resolve(item.id.0.clone(), "--reopen"),
                actions,
            );
        }
    });
}

fn lane_of(priority: Priority) -> usize {
    match priority {
        Priority::P0 => 0,
        Priority::P1 => 1,
        _ => 2,
    }
}
fn error_color(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(255, 169, 148)
    } else {
        Color32::from_rgb(155, 54, 38)
    }
}

fn button(
    ui: &mut egui::Ui,
    title: &str,
    enabled: bool,
    action: Action,
    actions: &mut Vec<Action>,
) {
    if ui.add_enabled(enabled, egui::Button::new(title)).clicked() {
        actions.push(action);
    }
}

/// Protocol ranges count Unicode scalars, not UTF-8 bytes or grapheme clusters.
fn highlight_bytes(text: &str, range: Option<[usize; 2]>) -> Option<std::ops::Range<usize>> {
    let [start, end] = range?;
    if start >= end {
        return None;
    }
    let mut begin = None;
    for (scalar, byte) in text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .enumerate()
    {
        if scalar == start {
            begin = Some(byte);
        }
        if scalar == end {
            return begin.map(|begin| begin..byte);
        }
    }
    None
}

fn evidence_layout(evidence: &EvidenceView, color: Color32, dark: bool) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let normal = egui::TextFormat {
        font_id: egui::FontId::proportional(15.0),
        color,
        ..Default::default()
    };
    let text = &evidence.display_text;
    if let Some(range) = highlight_bytes(text, evidence.highlight.filter(|_| evidence.ok)) {
        job.append(&text[..range.start], 0.0, normal.clone());
        let mut highlighted = normal.clone();
        highlighted.background = if dark {
            Color32::from_rgb(65, 69, 38)
        } else {
            Color32::from_rgb(247, 236, 190)
        };
        job.append(&text[range.clone()], 0.0, highlighted);
        job.append(&text[range.end..], 0.0, normal);
    } else {
        job.append(text, 0.0, normal);
    }
    job
}

fn dialogs(
    ctx: &egui::Context,
    state: &mut UiState,
    model: &GuiModel,
    busy: bool,
    demo: bool,
    actions: &mut Vec<Action>,
) {
    egui::Window::new("设置").open(&mut state.settings).default_width(530.0).resizable(true).show(ctx, |ui| {
        ui.label("CLI 程序路径"); ui.text_edit_singleline(&mut state.draft.cli);
        ui.label("数据目录"); ui.text_edit_singleline(&mut state.draft.data_dir);
        ui.label("配置文件（留空使用数据目录内 config.toml）"); ui.text_edit_singleline(&mut state.draft.config);
        if ui.checkbox(&mut state.draft.dark, "深色外观").changed() { crate::appearance::theme(ctx, state.draft.dark); }
        ui.small("API 密钥只从启动时的环境变量读取，不存入 GUI 设置。配置与环境变量就绪情况可用 CLI doctor 离线检查。自身 QQ 身份请通过 CLI import --self-uin 设置。");
        button(ui, "保存并重新连接", !busy && !demo && !state.draft.cli.trim().is_empty() && !state.draft.data_dir.trim().is_empty(), Action::SaveSettings, actions);
    });
    egui::Window::new("开始云端分析").open(&mut state.cloud_notice).collapsible(false).resizable(false).default_width(470.0).show(ctx, |ui| {
        ui.label("程序与数据库在本机，但分析会把聊天原文发送到配置的云服务（默认 Jev 与 DeepSeek）。聊天不做脱敏，图片不上传。");
        if let Some(chat) = &state.cloud_target {
            let name = model.chats.iter().find(|row| &row.chat_id == chat).map(|row| row.display_name.as_str()).unwrap_or(&chat.0);
            ui.strong(format!("待分析群聊：{name}"));
            ui.small(&chat.0);
            ui.label("本次分析上方显示的群聊。当前连接会记住此云端提示，之后分析其他群聊不再重复提示；更改连接设置后会重新提示。");
            button(ui, "我已了解，开始分析", !busy && !demo && model.selected_chat.as_ref() == Some(chat), Action::AcceptCloud(chat.clone()), actions);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_highlight_handles_emoji_combining_scalars_and_invalid_offsets() {
        let text = "甲🙂e\u{301}乙";
        let range = highlight_bytes(text, Some([1, 4])).unwrap();
        assert_eq!(&text[range], "🙂e\u{301}");
        assert_eq!(&text[highlight_bytes(text, Some([0, 5])).unwrap()], text);
        for range in [[4, 9], [2, 1], [0, 0], [usize::MAX, usize::MAX]] {
            assert!(highlight_bytes(text, Some(range)).is_none());
        }
    }
}
