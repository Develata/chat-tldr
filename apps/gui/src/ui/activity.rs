use super::*;
use chat_tldr_core::{AgentAction, DecisionMethod, StatsPayload};

pub(super) fn render(
    ui: &mut egui::Ui,
    state: &mut UiState,
    model: &GuiModel,
    query_enabled: bool,
    demo: bool,
    actions: &mut Vec<Action>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("运行详情").strong().size(18.0));
        ui.weak("最近 200 条 · 费用为本地估算");
        if ui.small_button("收起").clicked() {
            state.logs = false;
        }
    });
    ui.horizontal_wrapped(|ui| {
        for (index, label) in [
            format!("决策日志  {}", model.decisions.len()),
            "运行统计".into(),
            format!("模型记录  {}", model.jev_answers.len()),
            format!("诊断日志  {}", model.logs.len()),
        ]
        .into_iter()
        .enumerate()
        {
            ui.selectable_value(&mut state.activity_tab, index, label);
        }
        ui.separator();
        match state.activity_tab {
            0 => button(
                ui,
                "重放当前运行",
                query_enabled && model.stats.is_some() && model.has_capability("decisions"),
                Action::Decisions,
                actions,
            ),
            1 => button(
                ui,
                "刷新统计",
                query_enabled && model.has_capability("stats"),
                Action::Stats,
                actions,
            ),
            2 => button(
                ui,
                "读取模型记录",
                query_enabled && model.stats.is_some() && model.has_capability("jev-log"),
                Action::JevLog,
                actions,
            ),
            _ => {}
        }
        if demo {
            ui.weak("合成演示数据");
        }
    });
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt(("activity", state.activity_tab))
        .show(ui, |ui| match state.activity_tab {
            0 => decisions(ui, model),
            1 => stats(ui, model),
            2 => model_answers(ui, model),
            _ => diagnostics(ui, model),
        });
}

fn decisions(ui: &mut egui::Ui, model: &GuiModel) {
    if model.decisions.is_empty() {
        empty(
            ui,
            "还没有决策记录",
            "完成一次分析后，可在这里查看每一步的候选、选择方法与理由。",
        );
        return;
    }
    egui::ScrollArea::horizontal()
        .id_salt("decision-table-scroll")
        .show(ui, |ui| {
            egui::Grid::new("decision-table")
                .striped(true)
                .num_columns(6)
                .spacing(egui::vec2(14.0, 8.0))
                .show(ui, |ui| {
                    for title in [
                        "步骤",
                        "方法",
                        "候选动作",
                        "选中动作",
                        "概率 / 置信度",
                        "理由",
                    ] {
                        ui.strong(title);
                    }
                    ui.end_row();
                    for decision in &model.decisions {
                        ui.label(decision.step.to_string());
                        ui.label(method_label(decision.method));
                        ui.add_sized(
                            [190.0, 0.0],
                            egui::Label::new(
                                decision
                                    .allowed
                                    .iter()
                                    .map(action_label)
                                    .collect::<Vec<_>>()
                                    .join("、"),
                            )
                            .wrap(),
                        );
                        ui.add_sized(
                            [145.0, 0.0],
                            egui::Label::new(action_label(&decision.chosen)).wrap(),
                        );
                        ui.add_sized(
                            [150.0, 0.0],
                            egui::Label::new(probability_label(decision)).wrap(),
                        );
                        ui.add_sized([260.0, 0.0], egui::Label::new(&decision.reason).wrap());
                        ui.end_row();
                    }
                });
        });
}

fn stats(ui: &mut egui::Ui, model: &GuiModel) {
    // A stats query returns cumulative data without replacing the last run,
    // which is still needed by decision/model-history actions.
    if model
        .history_stats
        .iter()
        .any(|row| !matches!(row, StatsPayload::Run(_)))
    {
        ui.strong("最近查询结果");
        queried_stats(ui, model);
        ui.separator();
    }
    if let Some(stats) = &model.stats {
        ui.strong("最近一次分析");
        let input_tokens: u64 = stats.usage.iter().map(|row| row.input_tokens).sum();
        let output_tokens: u64 = stats.usage.iter().map(|row| row.output_tokens).sum();
        let cache_hits: u64 = stats.usage.iter().map(|row| row.cache_hits).sum();
        let calls: u64 = stats.usage.iter().map(|row| row.calls).sum();
        // Four columns fit the normal window. At compact widths two columns
        // keep the last KPI card inside the activity panel instead of clipping
        // it at the right edge.
        let metric_columns = if ui.available_width() < 980.0 { 2 } else { 4 };
        let metrics = [
            ("已分析消息", stats.messages_analyzed.to_string()),
            (
                "话题变化",
                format!("+{} / 更新 {}", stats.topics_created, stats.topics_updated),
            ),
            (
                "结论",
                format!(
                    "+{} / 更新 {}",
                    stats.insights.created, stats.insights.updated
                ),
            ),
            (
                "证据状态",
                format!(
                    "{} 已核验 · {} 待核验 · {} 拒绝",
                    stats.insights.verified, stats.insights.unverified, stats.insights.rejected
                ),
            ),
            ("模型用量", format!("{calls} 次 · {cache_hits} 次缓存")),
            (
                "Token",
                format!(
                    "{} 输入 · {} 输出",
                    grouped(input_tokens),
                    grouped(output_tokens)
                ),
            ),
            ("估算费用", format!("${:.4}", stats.cost_usd)),
            (
                "耗时",
                format!("{:.1} 秒", stats.elapsed_ms as f64 / 1000.0),
            ),
        ];
        egui::Grid::new("stats-metrics")
            .num_columns(metric_columns)
            .spacing(egui::vec2(10.0, 10.0))
            .show(ui, |ui| {
                for (index, (label, value)) in metrics.into_iter().enumerate() {
                    metric(ui, label, value);
                    if (index + 1) % metric_columns == 0 {
                        ui.end_row();
                    }
                }
            });
        ui.add_space(8.0);
        ui.strong("按阶段的模型用量");
        egui::ScrollArea::horizontal()
            .id_salt("usage-table-scroll")
            .show(ui, |ui| {
                egui::Grid::new("usage-table")
                    .striped(true)
                    .num_columns(7)
                    .spacing(egui::vec2(18.0, 7.0))
                    .show(ui, |ui| {
                        for title in [
                            "阶段",
                            "服务",
                            "模型",
                            "调用",
                            "缓存",
                            "Token（入 / 出）",
                            "估算费用",
                        ] {
                            ui.strong(title);
                        }
                        ui.end_row();
                        for row in &stats.usage {
                            ui.label(stage_label(&row.stage));
                            ui.label(&row.provider);
                            ui.label(&row.model);
                            ui.label(row.calls.to_string());
                            ui.label(row.cache_hits.to_string());
                            ui.label(format!(
                                "{} / {}",
                                grouped(row.input_tokens),
                                grouped(row.output_tokens)
                            ));
                            ui.label(format!("${:.4}", row.cost_usd));
                            ui.end_row();
                        }
                    });
            });
        ui.add_space(6.0);
        ui.weak(format!(
            "运行 ID：{} · 费用为本地配置估算，不能用于账单对账。",
            stats.run_id
        ));
        return;
    }

    if model.history_stats.is_empty() {
        empty(
            ui,
            "还没有运行统计",
            "分析完成后会显示消息、话题、结论、Token、估算费用和耗时。",
        );
    }
}

fn queried_stats(ui: &mut egui::Ui, model: &GuiModel) {
    for row in &model.history_stats {
        match row {
            StatsPayload::Import(stats) => {
                ui.label(format!(
                    "导入：{} 个文件 · 读取 {} 条 · 新增 {} 条 · 重复 {} 条",
                    stats.files, stats.seen, stats.inserted, stats.duplicate
                ));
            }
            StatsPayload::Global(stats) => {
                ui.label(format!(
                    "全局：{} 类计数 · {} 组模型用量 · 估算费用 ${:.4}",
                    stats.counts.len(),
                    stats.usage.len(),
                    stats.cost_usd
                ));
            }
            StatsPayload::Run(_) => {}
            StatsPayload::Unknown => {
                ui.weak("其他版本的统计记录；当前 GUI 不解析其详情。");
            }
        };
    }
}

fn model_answers(ui: &mut egui::Ui, model: &GuiModel) {
    if model.jev_answers.is_empty() {
        empty(
            ui,
            "还没有模型记录",
            "选择一条已保存的运行后，可读取用于校准的模型回答。",
        );
        return;
    }
    egui::Grid::new("model-answer-table")
        .striped(true)
        .num_columns(4)
        .show(ui, |ui| {
            for title in ["模型", "问题", "对象", "回答"] {
                ui.strong(title);
            }
            ui.end_row();
            for answer in &model.jev_answers {
                ui.label(&answer.model);
                ui.label(&answer.question_id);
                ui.label(format!("{:?} · {}", answer.subject.kind, answer.subject.id));
                ui.label(answer.answer.to_string());
                ui.end_row();
            }
        });
}

fn diagnostics(ui: &mut egui::Ui, model: &GuiModel) {
    if model.logs.is_empty() {
        empty(
            ui,
            "没有诊断日志",
            "CLI 的 stderr、告警和协议提示会显示在这里。",
        );
        return;
    }
    for line in &model.logs {
        ui.monospace(line);
    }
}

fn metric(ui: &mut egui::Ui, label: &str, value: String) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_min_width(130.0);
        ui.weak(label);
        ui.label(RichText::new(value).strong().size(17.0));
    });
}

fn empty(ui: &mut egui::Ui, title: &str, body: &str) {
    ui.add_space(12.0);
    ui.strong(title);
    ui.weak(body);
}

fn method_label(method: DecisionMethod) -> &'static str {
    match method {
        DecisionMethod::Rule => "规则",
        DecisionMethod::Jev => "Jev",
        DecisionMethod::Fallback => "降级",
        DecisionMethod::Unknown => "其他",
    }
}

fn action_label(action: &AgentAction) -> String {
    match action {
        AgentAction::AnalyzeDirect { .. } => "直接分析待处理消息".into(),
        AgentAction::Segment { .. } => "切分待处理消息".into(),
        AgentAction::AnalyzeTopic { topic_id } => format!("分析话题 {topic_id}"),
        AgentAction::Verify { insight_ids } => format!("核验 {} 项结论", insight_ids.len()),
        AgentAction::MergeTopics { from, .. } => format!("合并 {} 个相关话题", from.len()),
        AgentAction::Finish { reason } => format!("结束运行（{reason:?}）"),
        AgentAction::Unknown => "其他动作".into(),
    }
}

fn probability_label(decision: &chat_tldr_core::DecisionPayload) -> String {
    let mut parts = decision
        .probabilities
        .as_ref()
        .map(|rows| {
            rows.iter()
                .map(|(name, value)| format!("{name} {:.0}%", value * 100.0))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(confidence) = decision.confidence {
        parts.push(format!("置信度 {:.0}%", confidence * 100.0));
    }
    if parts.is_empty() && decision.allowed.len() == 1 {
        "单一合法动作".into()
    } else if parts.is_empty() {
        "未提供概率".into()
    } else {
        parts.join(" · ")
    }
}

fn stage_label(stage: &str) -> &str {
    match stage {
        "decide" => "决策",
        "extract" => "提取",
        "verify" => "核验",
        "segment" => "切分",
        "rank" => "排序",
        _ => stage,
    }
}

fn grouped(value: u64) -> String {
    let text = value.to_string();
    let mut out = String::with_capacity(text.len() + text.len() / 3);
    for (index, ch) in text.chars().enumerate() {
        if index > 0 && (text.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_probabilities_do_not_claim_only_one_legal_action() {
        let mut decision = GuiModel::demo().decisions[0].clone();
        decision.method = DecisionMethod::Fallback;
        decision.confidence = None;
        assert!(decision.allowed.len() > 1);
        assert_eq!(probability_label(&decision), "未提供概率");
        decision.allowed.truncate(1);
        assert_eq!(probability_label(&decision), "单一合法动作");
    }
}
