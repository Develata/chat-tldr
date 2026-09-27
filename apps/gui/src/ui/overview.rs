use super::*;
use crate::model::OverviewView;
use chat_tldr_core::{DeadlineStatus, InsightId, TopicId};

pub(super) fn render(
    ui: &mut egui::Ui,
    state: &mut UiState,
    model: &GuiModel,
    query_enabled: bool,
    actions: &mut Vec<Action>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.heading("群聊分析总览");
        ui.label(
            RichText::new("只读 · 不计为已读")
                .strong()
                .color(crate::appearance::ACCENT),
        );
    });
    if state.overview_hours == 0 {
        state.overview_hours = 24;
    }
    ui.horizontal_wrapped(|ui| {
        for (hours, label) in [(6, "近 6 小时"), (24, "近 24 小时"), (168, "近 7 天")] {
            if ui
                .add_enabled(
                    query_enabled,
                    egui::Button::selectable(state.overview_hours == hours, label),
                )
                .clicked()
            {
                state.overview_hours = hours;
                state.overview_page = 0;
                actions.push(Action::Overview(hours));
            }
        }
        if ui
            .add_enabled(query_enabled, egui::Button::new("刷新总览"))
            .clicked()
        {
            actions.push(Action::Overview(state.overview_hours));
        }
    });
    let Some(view) = &model.overview else {
        ui.label("点击刷新总览，读取这个群的分析结果。");
        return;
    };
    let report = &view.report;
    ui.horizontal_wrapped(|ui| {
        overview_metric(
            ui,
            "统计窗口",
            format!(
                "{} — {}",
                report.since.format("%m-%d %H:%M"),
                report.until.format("%m-%d %H:%M")
            ),
        );
        overview_metric(ui, "窗口消息", report.window_messages.to_string());
        overview_metric(ui, "窗口待分析", report.window_pending.to_string());
        overview_metric(ui, "全群待分析", report.pending_messages.to_string());
        overview_metric(
            ui,
            "数据截至",
            report
                .data_end
                .map(|date| date.format("%m-%d %H:%M").to_string())
                .unwrap_or_else(|| "无记录".into()),
        );
    });
    ui.add_space(6.0);
    let old = state.overview_tab;
    ui.horizontal_wrapped(|ui| {
        for (index, label) in [
            format!("热门话题  {}", report.hot_topics.len()),
            format!("优先话题  {}", report.priority_topics.len()),
            format!("与我有关  {}", report.related.len() + report.mentions.len()),
            format!("截止事项  {}", report.deadlines.len()),
            format!("未读回顾  {}", report.unread_topics.len()),
            format!("资料入口  {}", report.resources.len()),
        ]
        .iter()
        .enumerate()
        {
            ui.selectable_value(&mut state.overview_tab, index, label);
        }
    });
    if old != state.overview_tab {
        state.overview_page = 0;
    }
    let total = match state.overview_tab {
        0 => report.hot_topics.len(),
        1 => report.priority_topics.len(),
        2 => report.related.len() + report.mentions.len(),
        3 => report.deadlines.len(),
        4 => report.unread_topics.len(),
        _ => report.resources.len(),
    };
    let pages = total.div_ceil(20).max(1);
    state.overview_page = state.overview_page.min(pages - 1);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(state.overview_page > 0, egui::Button::new("上一页"))
            .clicked()
        {
            state.overview_page -= 1;
        }
        ui.label(format!(
            "{} 项 · 第 {} / {} 页",
            total,
            state.overview_page + 1,
            pages
        ));
        if ui
            .add_enabled(state.overview_page + 1 < pages, egui::Button::new("下一页"))
            .clicked()
        {
            state.overview_page += 1;
        }
    });
    egui::ScrollArea::vertical()
        .id_salt((
            "overview",
            report.generated_at.timestamp_millis(),
            state.overview_tab,
            state.overview_page,
        ))
        .show(ui, |ui| {
            if total == 0 {
                let reason = if report.window_messages == 0 {
                    "当前时间窗口没有消息；可切换到更长窗口。"
                } else if report.window_pending > 0 {
                    "当前没有可展示结果，但窗口内仍有待分析消息。"
                } else {
                    "当前窗口没有符合这一分类的内容。"
                };
                empty_state(ui, "暂无结果", reason);
            }
            for index in (state.overview_page * 20..total).take(20) {
                ui.push_id(index, |ui| {
                    egui::Frame::group(ui.style()).show(ui, |ui| match state.overview_tab {
                        0 => {
                            let row = &report.hot_topics[index];
                            ui.label(
                                RichText::new(title(view, Some(&row.topic_id)))
                                    .strong()
                                    .size(18.0),
                            );
                            ui.label(format!(
                                "{} 人参与 · {} 条有效讨论 / {} 条已分析消息",
                                row.participants, row.meaningful_messages, row.message_count
                            ));
                            ui.weak(format!(
                                "最近活动 {} · 已限制单人刷屏贡献",
                                row.last_message_at.format("%m-%d %H:%M %:z")
                            ));
                            if let Some(group) = report
                                .priority_topics
                                .iter()
                                .find(|g| g.topic_id.as_ref() == Some(&row.topic_id))
                            {
                                items(ui, view, &group.insight_ids, query_enabled, model, actions);
                            }
                        }
                        1 | 4 => {
                            let groups = if state.overview_tab == 1 {
                                &report.priority_topics
                            } else {
                                &report.unread_topics
                            };
                            let group = &groups[index];
                            ui.label(
                                RichText::new(format!(
                                    "{} · {}",
                                    priority_label(group.priority),
                                    title(view, group.topic_id.as_ref())
                                ))
                                .strong()
                                .size(18.0),
                            );
                            ui.label(group.reasons.join(" · "));
                            items(ui, view, &group.insight_ids, query_enabled, model, actions);
                        }
                        2 => {
                            if index < report.related.len() {
                                let row = &report.related[index];
                                ui.label(row.reasons.join(" · "));
                                items(
                                    ui,
                                    view,
                                    std::slice::from_ref(&row.insight_id),
                                    query_enabled,
                                    model,
                                    actions,
                                );
                            } else {
                                let row = &report.mentions[index - report.related.len()];
                                ui.label(format!(
                                    "{} · {} · {}",
                                    row.sender_display,
                                    row.reasons.join(" · "),
                                    if row.analyzed {
                                        "已分析"
                                    } else {
                                        "待分析"
                                    }
                                ));
                                ui.label(&row.text);
                            }
                        }
                        3 => {
                            let row = &report.deadlines[index];
                            ui.label(
                                RichText::new(match row.status {
                                    DeadlineStatus::Overdue => "已逾期",
                                    DeadlineStatus::Upcoming => "尚未到期",
                                    _ => "时间待确认",
                                })
                                .strong()
                                .color(match row.status {
                                    DeadlineStatus::Overdue => error_color(ui),
                                    _ => crate::appearance::ACCENT,
                                }),
                            );
                            items(
                                ui,
                                view,
                                std::slice::from_ref(&row.insight_id),
                                query_enabled,
                                model,
                                actions,
                            );
                        }
                        _ => {
                            let row = &report.resources[index];
                            ui.label(format!(
                                "{} · {}",
                                row.source.sender_display,
                                row.source.sent_at.format("%m-%d %H:%M %:z")
                            ));
                            ui.label(&row.source.text);
                            for attachment in &row.attachments {
                                ui.label(format!(
                                    "附件：{}",
                                    attachment.name.as_deref().unwrap_or("未命名")
                                ));
                            }
                            for link in &row.links {
                                if link.starts_with("http://") || link.starts_with("https://") {
                                    if ui.link(link).on_hover_text("点击复制链接").clicked() {
                                        ui.ctx().copy_text(link.clone());
                                    }
                                } else {
                                    ui.label(link);
                                }
                            }
                            ui.weak("点击链接可复制；仅展示附件元数据，未读取文件正文。");
                        }
                    });
                });
                ui.add_space(8.0);
            }
        });
}

fn overview_metric(ui: &mut egui::Ui, label: &str, value: String) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.weak(label);
        ui.label(RichText::new(value).strong());
    });
}

fn title<'a>(view: &'a OverviewView, id: Option<&TopicId>) -> &'a str {
    id.and_then(|id| view.titles.get(id))
        .map(String::as_str)
        .unwrap_or("未归属话题")
}

fn items(
    ui: &mut egui::Ui,
    view: &OverviewView,
    ids: &[InsightId],
    write_enabled: bool,
    model: &GuiModel,
    actions: &mut Vec<Action>,
) {
    let page_id = ui.make_persistent_id("overview-insight-page");
    let pages = ids.len().div_ceil(20).max(1);
    let mut page = ui
        .data(|data| data.get_temp::<usize>(page_id))
        .unwrap_or(0)
        .min(pages - 1);
    if pages > 1 {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(page > 0, egui::Button::new("上一组结论"))
                .clicked()
            {
                page -= 1;
            }
            ui.small(format!("{} 项结论 · {} / {}", ids.len(), page + 1, pages));
            if ui
                .add_enabled(page + 1 < pages, egui::Button::new("下一组结论"))
                .clicked()
            {
                page += 1;
            }
        });
    }
    ui.data_mut(|data| data.insert_temp(page_id, page));
    for id in ids.iter().skip(page * 20).take(20) {
        if let Some(index) = view.items.get(id) {
            let row = &view.report.insights[*index];
            ui.push_id(id.as_ref(), |ui| {
                ui.collapsing(
                    format!(
                        "{} · {}",
                        priority_label(row.insight.priority),
                        row.insight.title
                    ),
                    |ui| insight(ui, row, write_enabled, false, model, actions),
                );
            });
        }
    }
}
