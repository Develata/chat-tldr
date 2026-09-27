//! Rendering returns actions; it never starts processes or reads files.
use chat_tldr_core::{
    Assignee, ChatId, EvidenceView, InsightKind, InsightPayload, Lifecycle, Priority,
    TemporalConstraint, VerificationStatus,
};
use eframe::egui::{self, Color32, RichText};

use crate::{model::GuiModel, prefs::Preferences};

mod activity;
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
    activity_tab: usize,
    evidence_open: bool,
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

    pub(crate) fn configure_demo(&mut self, view: crate::DemoView) {
        match view {
            crate::DemoView::Inbox => {}
            crate::DemoView::Evidence => self.evidence_open = true,
            crate::DemoView::Decisions => {
                self.logs = true;
                self.activity_tab = 0;
            }
            crate::DemoView::Stats => {
                self.logs = true;
                self.activity_tab = 1;
            }
            crate::DemoView::Overview => self.overview = true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Select(ChatId),
    Refresh,
    PickImport,
    OpenQce,
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
    DismissError,
}

pub fn render(
    ui: &mut egui::Ui,
    state: &mut UiState,
    model: &GuiModel,
    cli_busy: bool,
    picking_file: bool,
    demo: bool,
) -> (Vec<Action>, Option<u64>) {
    let mut actions = Vec::new();
    let busy = cli_busy || picking_file;
    let connected = demo || model.handshake_ok();
    let local_enabled = !busy && connected;
    let query_enabled = !busy && !demo && model.handshake_ok();
    let write_enabled = query_enabled;
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
    let compact = ui.available_width() < 920.0;
    egui::Panel::top("toolbar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("群聊省流").strong().size(22.0));
            ui.add_space(12.0);
            if ui
                .add_enabled(
                    local_enabled,
                    egui::Button::selectable(!state.overview, "收件箱"),
                )
                .clicked()
            {
                state.overview = false;
            }
            let overview_enabled = local_enabled
                && model.selected_chat.is_some()
                && (demo || model.has_capability("overview"));
            if ui
                .add_enabled(
                    overview_enabled,
                    egui::Button::selectable(state.overview, "分析总览"),
                )
                .clicked()
            {
                state.overview = true;
                state.overview_page = 0;
                if !demo {
                    actions.push(Action::Overview(if state.overview_hours == 0 {
                        24
                    } else {
                        state.overview_hours
                    }));
                }
            }
            ui.separator();
            ui.toggle_value(&mut state.logs, "运行详情");
            if ui.button("设置").clicked() {
                state.settings = true;
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            button(
                ui,
                "导入 QCE 文件",
                write_enabled && model.has_capability("import"),
                Action::PickImport,
                &mut actions,
            );
            button(
                ui,
                "从 QQ 获取聊天",
                write_enabled && model.has_capability("import"),
                Action::OpenQce,
                &mut actions,
            );
            if ui
                .add_enabled(
                    write_enabled
                        && model.selected_chat.is_some()
                        && model.has_capability("analyze")
                        && analysis_supported,
                    egui::Button::new(RichText::new("分析新消息").strong())
                        .fill(crate::appearance::ACCENT),
                )
                .clicked()
            {
                actions.push(Action::Analyze);
            }
            let mark_read_enabled = write_enabled
                && !state.overview
                && model.mark_read_cursor().is_some()
                && model.has_capability("mark-read");
            let mark_read_reason = mark_read_reason(state, model, demo, busy);
            if ui
                .add_enabled(mark_read_enabled, egui::Button::new("标为已读"))
                .on_disabled_hover_text(mark_read_reason)
                .clicked()
            {
                actions.push(Action::MarkRead);
            }
            button(
                ui,
                "刷新",
                query_enabled && model.has_capability("chats"),
                Action::Refresh,
                &mut actions,
            );
            if cli_busy {
                ui.spinner();
                ui.weak("正在执行 CLI…");
                button(ui, "停止", true, Action::Cancel, &mut actions);
            } else if picking_file {
                ui.spinner();
                ui.weak("等待选择文件…");
            }
        });
        if demo {
            ui.colored_label(
                crate::appearance::ACCENT,
                "合成演示 · 固定虚构数据 · 不读取数据库、不调用云服务，写操作已禁用",
            );
        }
        if let Some(progress) = &model.progress {
            ui.horizontal(|ui| {
                ui.small(format!(
                    "{} · {}",
                    progress_stage(&progress.stage),
                    progress.message
                ));
                if let Some(total) = progress.total.filter(|total| *total > 0) {
                    ui.add(
                        egui::ProgressBar::new((progress.current as f32 / total as f32).min(1.0))
                            .desired_width(180.0)
                            .text(format!("{} / {}", progress.current, total)),
                    );
                }
            });
        }
        if let Some(error) = &model.last_error {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(error_color(ui), RichText::new("操作未完成").strong());
                    ui.label(error);
                    if ui.small_button("检查设置").clicked() {
                        state.settings = true;
                    }
                    if ui.small_button("关闭").clicked() {
                        actions.push(Action::DismissError);
                    }
                });
            });
        }
    });
    if !compact {
        egui::Panel::left("chats")
            .default_size(212.0)
            .min_size(176.0)
            .max_size(280.0)
            .resizable(true)
            .show(ui, |ui| chat_list(ui, model, busy, &mut actions));
    }
    if state.logs {
        // The detail panel contains real tables rather than a one-line log.
        // Give it enough first-render height to show the table header and at
        // least one row; users can still resize it with the splitter.
        egui::Panel::bottom("run-log")
            .default_size(340.0)
            .resizable(true)
            .show(ui, |ui| {
                activity::render(ui, state, model, query_enabled, demo, &mut actions);
            });
    }
    let mut displayed = None;
    egui::CentralPanel::default().show(ui, |ui| {
        if compact {
            compact_chat_picker(ui, model, busy, &mut actions);
            ui.add_space(6.0);
        }
        if state.overview {
            overview::render(
                ui,
                state,
                model,
                query_enabled && model.has_capability("overview"),
                &mut actions,
            );
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
                query_enabled && model.has_capability("chats") && model.has_capability("inbox");
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
                empty_state(
                    ui,
                    "尚未连接 CLI",
                    "请在设置中检查 CLI 程序与数据目录，然后重新连接。",
                );
                if ui.button("打开设置").clicked() {
                    state.settings = true;
                }
            } else if model.chats.is_empty() {
                empty_state(
                    ui,
                    "还没有群聊",
                    "导入 QCE 已完成并关闭的单文件 JSON 后，即可查看收件箱。",
                );
                button(
                    ui,
                    "导入 QCE 文件",
                    write_enabled && model.has_capability("import"),
                    Action::PickImport,
                    &mut actions,
                );
            } else if model.selected_chat.is_none() {
                empty_state(ui, "请选择群聊", "从群聊列表选择一个群，查看结论与原文证据。");
            } else {
                empty_state(
                    ui,
                    "收件箱尚未载入",
                    "点击刷新重试；尚未分析的群聊会显示为空收件箱。",
                );
            }
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.weak(format!(
                "{} 个话题 · {} 项结论",
                inbox.topics.len(),
                inbox.insights.len()
            ));
            if inbox.meta.rejected_insights > 0 {
                ui.colored_label(
                    error_color(ui),
                    format!("{} 项证据未通过，未放入收件箱", inbox.meta.rejected_insights),
                );
            }
        });
        if !state.review.complete() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(format!(
                    "阅读进度：还有 {} 项未完整查看。请打开各栏、翻页并滚动到底；新快照会重新计算。",
                    state.review.remaining()
                ));
            });
        } else if inbox.meta.view_cursor.is_none() {
            ui.weak("已查看当前全部结论，但 CLI 没有返回安全已读位置，因此不能标为已读。");
        } else {
            ui.colored_label(
                crate::appearance::ACCENT,
                "当前快照已完整查看，可以标为已读。",
            );
        }
        // Partition only when a new snapshot arrives; lay out at most 20 rows per lane.
        if ui.available_width() >= 1000.0 {
            ui.columns(3, |columns| {
                for (index, column) in columns.iter_mut().enumerate() {
                    lane(column, index, state, write_enabled, model, &mut actions);
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
            lane(ui, state.lane, state, write_enabled, model, &mut actions);
        }
        if state.review.complete() {
            displayed = Some(inbox.request_id);
        }
    });
    dialogs(ui.ctx(), state, model, busy, demo, &mut actions);
    (actions, displayed)
}

fn chat_list(ui: &mut egui::Ui, model: &GuiModel, busy: bool, actions: &mut Vec<Action>) {
    ui.add_space(12.0);
    ui.label(RichText::new("群聊").strong().size(17.0));
    ui.weak(format!("{} 个已导入群聊", model.chats.len()));
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
                ui.weak("导入 QCE 单文件 JSON 后，群聊会显示在这里。");
            }
        });
}

fn compact_chat_picker(ui: &mut egui::Ui, model: &GuiModel, busy: bool, actions: &mut Vec<Action>) {
    let selected = model
        .chats
        .iter()
        .find(|chat| Some(&chat.chat_id) == model.selected_chat.as_ref())
        .map(|chat| chat.display_name.as_str())
        .unwrap_or("选择群聊");
    ui.horizontal(|ui| {
        ui.strong("当前群聊");
        ui.add_enabled_ui(!busy, |ui| {
            egui::ComboBox::from_id_salt("compact-chat-picker")
                .selected_text(selected)
                .width(ui.available_width().min(420.0))
                .show_ui(ui, |ui| {
                    for chat in &model.chats {
                        let current = model.selected_chat.as_ref() == Some(&chat.chat_id);
                        if ui
                            .selectable_label(
                                current,
                                format!(
                                    "{} · {} 条未读 · {} 项 P0",
                                    chat.display_name, chat.unreviewed_messages, chat.open_p0
                                ),
                            )
                            .clicked()
                            && !current
                        {
                            actions.push(Action::Select(chat.chat_id.clone()));
                        }
                    }
                });
        });
    });
}

fn empty_state(ui: &mut egui::Ui, title: &str, body: &str) {
    ui.add_space(36.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).strong().size(20.0));
        ui.add_space(6.0);
        ui.weak(body);
    });
}

fn mark_read_reason(state: &UiState, model: &GuiModel, demo: bool, busy: bool) -> String {
    if demo {
        return "合成演示不会修改业务状态。".into();
    }
    if state.overview {
        return "分析总览是只读视图，不会推进已读位置。".into();
    }
    if busy {
        return "请等待当前操作完成。".into();
    }
    let Some(inbox) = &model.inbox else {
        return "请先选择群聊并载入收件箱。".into();
    };
    if inbox.meta.view_cursor.is_none() {
        return "CLI 没有返回安全已读位置。".into();
    }
    if !state.review.complete() {
        return format!("还有 {} 项尚未完整查看。", state.review.remaining());
    }
    "当前快照刷新中，完成后可标为已读。".into()
}

fn progress_stage(stage: &str) -> &str {
    match stage {
        "import" => "导入",
        "segment" => "切分话题",
        "decide" => "选择步骤",
        "extract" => "提取结论",
        "verify" => "核验证据",
        "rank" => "整理优先级",
        "store" => "保存结果",
        "render" => "生成视图",
        _ => "处理中",
    }
}

fn lane(
    ui: &mut egui::Ui,
    index: usize,
    state: &mut UiState,
    write_enabled: bool,
    model: &GuiModel,
    actions: &mut Vec<Action>,
) {
    let rows = &state.lanes[index];
    let page = &mut state.pages[index];
    let titles = ["P0 必须处理", "P1 值得关注", "P2 / P3 参考"];
    let viewed = rows.iter().filter(|&&row| state.review.seen(row)).count();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("{} · {}", titles[index], rows.len()))
                .strong()
                .size(17.0),
        );
        ui.weak(format!("已查看 {viewed}/{}", rows.len()));
    });
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
            let first = *page * 20 + 1;
            let last = ((*page + 1) * 20).min(rows.len());
            ui.small(format!("第 {first}–{last} 项 · {} / {pages} 页", *page + 1));
            if ui
                .add_enabled(*page + 1 < pages, egui::Button::new("下一页"))
                .clicked()
            {
                *page += 1;
            }
        });
    }
    egui::ScrollArea::vertical()
        .id_salt((
            "lane",
            state.inbox_request.unwrap_or_default(),
            index,
            *page,
        ))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if rows.is_empty() {
                ui.add_space(18.0);
                ui.weak(
                    [
                        "当前没有必须处理的截止事项。",
                        "当前没有与你直接相关的关注项。",
                        "当前没有其他参考信息。",
                    ][index],
                );
            }
            for &row_index in rows.iter().skip(*page * 20).take(20) {
                let row = &model.inbox.as_ref().expect("lane needs inbox").insights[row_index];
                let rendered = ui.push_id(&row.insight.id.0, |ui| {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        insight(ui, row, write_enabled, state.evidence_open, model, actions);
                    })
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
    write_enabled: bool,
    evidence_default_open: bool,
    model: &GuiModel,
    actions: &mut Vec<Action>,
) {
    let item = &row.insight;
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(priority_label(item.priority))
                .strong()
                .color(Color32::WHITE)
                .background_color(priority_color(item.priority)),
        );
        ui.weak(kind_label(item.kind));
        if item.lifecycle != Lifecycle::Open {
            ui.label(
                RichText::new(match item.lifecycle {
                    Lifecycle::Done => "已完成",
                    Lifecycle::Dismissed => "已忽略",
                    Lifecycle::Unknown => "其他状态",
                    Lifecycle::Open => "进行中",
                })
                .strong(),
            );
        }
    });
    ui.label(RichText::new(&item.title).size(18.0).strong());
    ui.label(&item.summary);
    let assignee = match item.assignee {
        Assignee::Me => "我",
        Assignee::All => "全体",
        Assignee::Other => "其他人",
        Assignee::Unknown => "待确认",
    };
    ui.horizontal_wrapped(|ui| {
        ui.small(format!("负责人 · {assignee}"));
        let verification = match item.verification_status {
            VerificationStatus::Verified => "证据已核验",
            VerificationStatus::Unverified => "证据待核验",
            VerificationStatus::Rejected => "证据未通过",
            VerificationStatus::Unknown => "其他验证状态",
        };
        ui.small(format!("· {verification}"));
    });
    if let Some(deadline) = &item.deadline {
        let overdue = deadline_overdue(deadline);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(if overdue { "已逾期" } else { "截止时间" })
                    .strong()
                    .color(if overdue {
                        error_color(ui)
                    } else {
                        crate::appearance::ACCENT
                    }),
            );
            ui.label(&deadline.raw);
        });
        if let Some(date) = deadline.bound_date {
            let time = deadline
                .bound_time
                .map(|time| time.format("%H:%M").to_string())
                .unwrap_or_else(|| "当天".into());
            ui.small(format!(
                "系统识别 · {} {time} · {}",
                date,
                deadline.anchor.offset()
            ));
        } else {
            ui.small("时间待确认 · 保留原文，不把模型猜测当作确定日期");
        }
    }
    egui::CollapsingHeader::new(format!("原文证据 · {} 条", row.evidence_view.len()))
        .default_open(evidence_default_open)
        .show(ui, |ui| {
            for evidence in &row.evidence_view {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.small(format!(
                            "{} · {}",
                            evidence.sender_display,
                            evidence.sent_at.format("%m-%d %H:%M")
                        ));
                        ui.small(if evidence.ok {
                            "已核验"
                        } else {
                            "待人工核对"
                        });
                    });
                    ui.label(evidence_layout(
                        evidence,
                        ui.visuals().text_color(),
                        ui.visuals().dark_mode,
                    ));
                    if evidence.highlight.is_none() {
                        ui.weak("未定位到逐字引用，以上为 CLI 返回的原文。");
                    }
                    if !evidence.ok {
                        ui.colored_label(error_color(ui), "引用未通过核验，请直接核对原文。");
                    }
                });
            }
        });
    ui.horizontal_wrapped(|ui| {
        let vote = write_enabled && model.has_capability("feedback");
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
        let resolve = write_enabled && model.has_capability("resolve");
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

fn priority_label(priority: Priority) -> &'static str {
    match priority {
        Priority::P0 => "P0",
        Priority::P1 => "P1",
        Priority::P2 => "P2",
        Priority::P3 => "P3",
        Priority::Unknown => "其他",
    }
}

fn priority_color(priority: Priority) -> Color32 {
    match priority {
        Priority::P0 => Color32::from_rgb(177, 55, 45),
        Priority::P1 => Color32::from_rgb(184, 112, 24),
        Priority::P2 => crate::appearance::ACCENT,
        Priority::P3 | Priority::Unknown => Color32::from_rgb(103, 112, 126),
    }
}

fn kind_label(kind: InsightKind) -> &'static str {
    match kind {
        InsightKind::MentionMe => "与我有关",
        InsightKind::Todo => "待办",
        InsightKind::Announcement => "公告",
        InsightKind::Decision => "决定",
        InsightKind::TopicSummary => "话题摘要",
        InsightKind::Unknown => "其他类型",
    }
}

fn deadline_overdue(deadline: &TemporalConstraint) -> bool {
    let Some(date) = deadline.bound_date else {
        return false;
    };
    let time = deadline.bound_time.unwrap_or_else(|| {
        chrono::NaiveTime::from_hms_opt(23, 59, 59).expect("valid end-of-day time")
    });
    let now = chrono::Utc::now()
        .with_timezone(deadline.anchor.offset())
        .naive_local();
    date.and_time(time) < now
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
