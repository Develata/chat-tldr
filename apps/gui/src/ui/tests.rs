use super::*;
use crate::model::Capabilities;

/// Runs the production renderer without an app/bridge, so emitted actions cannot
/// start a CLI process. All clicks use the text actually painted by egui.
struct Harness {
    ctx: egui::Context,
    state: UiState,
    model: GuiModel,
    size: egui::Vec2,
    demo: bool,
    tick: u32,
}

impl Harness {
    fn new(width: f32, demo: bool) -> Self {
        let ctx = egui::Context::default();
        crate::appearance::theme(&ctx, false);
        let mut model = GuiModel::demo();
        model.capabilities = Some(capabilities());
        Self {
            ctx,
            state: UiState::new(Preferences {
                cli: "chat-tldr.exe".into(),
                data_dir: "synthetic-test-data".into(),
                ..Default::default()
            }),
            model,
            size: egui::vec2(width, 1000.0),
            demo,
            tick: 0,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> (egui::FullOutput, Vec<Action>) {
        self.tick += 1;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            time: Some(f64::from(self.tick) / 60.0),
            events,
            ..Default::default()
        };
        let mut result = (Vec::new(), None);
        let mut output = self.ctx.run_ui(input, |ui| {
            result = render(ui, &mut self.state, &self.model, false, false, self.demo);
        });
        // This headless harness inspects shapes and has no GPU texture backend.
        output.textures_delta.clear();
        if let Some(request_id) = result.1 {
            self.model.mark_inbox_displayed(request_id);
        }
        (output, result.0)
    }

    fn settled(&mut self) -> egui::FullOutput {
        // Windows and panels need a prior layout for hit testing. No actions are
        // synthesized here: only pointer events below can change test UI state.
        self.frame(Vec::new());
        self.frame(Vec::new()).0
    }

    fn click(&mut self, label: &str) -> Vec<Action> {
        let output = self.settled();
        let pos = text_position(&output, label)
            .unwrap_or_else(|| panic!("painted control not found: {label}"));
        let mut actions = self.frame(vec![egui::Event::PointerMoved(pos)]).1;
        for pressed in [true, false] {
            actions.extend(
                self.frame(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }])
                .1,
            );
        }
        actions
    }
}

fn capabilities() -> Capabilities {
    Capabilities {
        commands: [
            "chats",
            "inbox",
            "import",
            "analyze",
            "mark-read",
            "feedback",
            "resolve",
            "stats",
            "overview",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        strategies: vec!["ours".into()],
        deciders: vec!["jev".into()],
    }
}

fn text_position(output: &egui::FullOutput, label: &str) -> Option<egui::Pos2> {
    fn collect(shape: &egui::Shape, clip: egui::Rect, label: &str, found: &mut Vec<egui::Pos2>) {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                let visible = text.visual_bounding_rect().intersect(clip);
                if visible.is_positive() {
                    found.push(visible.center());
                }
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, clip, label, found);
                }
            }
            _ => {}
        }
    }
    let mut positions = Vec::new();
    for shape in &output.shapes {
        collect(&shape.shape, shape.clip_rect, label, &mut positions);
    }
    // Toolbar labels can also occur as window/panel headings. Prefer the top
    // painted occurrence, which is the toolbar's clickable control.
    positions.into_iter().min_by(|a, b| a.y.total_cmp(&b.y))
}

#[test]
fn overview_entry_switches_windows_and_never_enables_mark_read() {
    let mut gui = Harness::new(1100.0, false);
    gui.settled();
    assert_eq!(gui.click("分析总览"), vec![Action::Overview(24)]);
    assert!(gui.state.overview);
    assert!(gui.click("标为已读").is_empty());
    assert_eq!(gui.click("近 6 小时"), vec![Action::Overview(6)]);
    assert_eq!(gui.state.overview_hours, 6);
    assert_eq!(gui.click("刷新总览"), vec![Action::Overview(6)]);
    assert!(gui.click("收件箱").is_empty());
    assert!(!gui.state.overview);
    assert_eq!(gui.click("分析总览"), vec![Action::Overview(6)]);
}

#[test]
fn pointer_clicks_open_settings_change_theme_and_toggle_logs() {
    let mut gui = Harness::new(1400.0, false);
    assert!(!gui.state.logs);
    assert!(gui.click("运行详情").is_empty());
    assert!(gui.state.logs);
    assert!(text_position(&gui.settled(), "运行统计").is_some());
    assert!(gui.click("运行详情").is_empty());
    assert!(!gui.state.logs);

    assert!(!gui.state.settings);
    assert!(gui.click("设置").is_empty());
    assert!(gui.state.settings);
    assert!(!gui.state.draft.dark);
    assert!(gui.click("深色外观").is_empty());
    assert!(gui.state.draft.dark);
    assert!(gui.ctx.global_style().visuals.dark_mode);
    assert_eq!(gui.click("保存并重新连接"), vec![Action::SaveSettings]);
}

#[test]
fn narrow_layout_tabs_respond_to_pointer_clicks() {
    let mut gui = Harness::new(900.0, false);
    assert_eq!(gui.state.lane, 0);
    assert!(gui.click("P1 值得关注  1").is_empty());
    assert_eq!(gui.state.lane, 1);
    assert!(text_position(&gui.settled(), "P1 值得关注 · 1").is_some());
    assert!(gui.click("P2 / P3 参考  2").is_empty());
    assert_eq!(gui.state.lane, 2);
    assert!(gui.click("P0 必须处理  1").is_empty());
    assert_eq!(gui.state.lane, 0);
    assert!(text_position(&gui.settled(), "有用").is_some());
}

#[test]
fn demo_disables_mutations_even_with_capabilities_and_displayed_cursor() {
    let mut gui = Harness::new(1400.0, true);
    gui.settled();
    assert!(gui.model.handshake_ok());
    assert!(gui.model.mark_read_cursor().is_some());
    for label in [
        "导入 QCE 文件",
        "分析新消息",
        "标为已读",
        "有用",
        "不重要",
        "完成",
        "忽略",
        "包含已读",
        "包含已处理",
    ] {
        assert!(gui.click(label).is_empty(), "demo enabled {label}");
    }
    assert!(!gui.state.all);
    assert!(!gui.state.resolved);
    assert!(gui.click("设置").is_empty());
    assert!(gui.state.settings);
    assert!(gui.click("保存并重新连接").is_empty());
}

#[test]
fn filters_require_handshake_and_capabilities_then_emit_refresh() {
    let mut gui = Harness::new(1400.0, false);
    gui.model.capabilities = None;
    for label in ["包含已读", "包含已处理"] {
        assert!(gui.click(label).is_empty());
    }
    assert!(!gui.state.all);
    assert!(!gui.state.resolved);

    let mut missing_inbox = capabilities();
    missing_inbox.commands.retain(|command| command != "inbox");
    gui.model.capabilities = Some(missing_inbox);
    assert!(gui.click("包含已读").is_empty());
    assert!(!gui.state.all);

    gui.model.capabilities = Some(capabilities());
    assert_eq!(gui.click("包含已读"), vec![Action::Refresh]);
    assert!(gui.state.all);
    assert_eq!(gui.click("包含已处理"), vec![Action::Refresh]);
    assert!(gui.state.resolved);
    assert_eq!(gui.click("包含已读"), vec![Action::Refresh]);
    assert!(!gui.state.all);
}

#[test]
fn unopened_narrow_lanes_do_not_authorize_the_snapshot_cursor() {
    let mut gui = Harness::new(900.0, false);
    let inbox = gui.model.inbox.as_mut().unwrap();
    let mut second = inbox.insights[0].clone();
    second.insight.id = "insight-unseen-p1".into();
    second.insight.priority = Priority::P1;
    inbox.insights.push(second);
    gui.settled();
    assert!(gui.model.mark_read_cursor().is_none());
    assert!(gui.click("标为已读").is_empty());
    gui.click("P1 值得关注  2");
    gui.settled();
    assert!(gui.model.mark_read_cursor().is_none());
    gui.click("P2 / P3 参考  2");
    gui.settled();
    assert!(gui.model.mark_read_cursor().is_some());
}

#[test]
fn unvisited_pages_do_not_authorize_the_snapshot_cursor() {
    let mut gui = Harness::new(900.0, false);
    // Every row of page one fits; page two is still never rendered.
    gui.size.y = 8000.0;
    let inbox = gui.model.inbox.as_mut().unwrap();
    let mut row = inbox.insights[0].clone();
    row.evidence_view.clear();
    inbox.insights = (0..21)
        .map(|index| {
            let mut row = row.clone();
            row.insight.id = format!("insight-page-{index}").into();
            row
        })
        .collect();
    gui.settled();
    assert!(gui.model.mark_read_cursor().is_none());
    assert_eq!(gui.state.review.remaining(), 1);
    assert!(gui.click("下一页").is_empty());
    gui.settled();
    assert!(gui.model.mark_read_cursor().is_some());
}

#[test]
fn cloud_confirmation_names_and_binds_the_displayed_chat() {
    let mut gui = Harness::new(1400.0, false);
    let target = gui.model.selected_chat.clone().unwrap();
    gui.state.cloud_notice = true;
    gui.state.cloud_target = Some(target.clone());
    assert_eq!(
        gui.click("我已了解，开始分析"),
        vec![Action::AcceptCloud(target)]
    );
    gui.model.selected_chat = Some("another-chat".into());
    assert!(gui.click("我已了解，开始分析").is_empty());
}
