use super::super::*;

struct Harness {
    ctx: egui::Context,
    wizard: Wizard,
    tick: u32,
    size: egui::Vec2,
}
impl Harness {
    fn new() -> Self {
        let mut wizard = Wizard {
            open: true,
            ..Default::default()
        };
        wizard.model.compatible = true;
        wizard.model.status = Some(model::Status {
            qce_reachable: true,
            authenticated: true,
            qq_logged_in: Some(true),
            qce_ready: true,
        });
        wizard.model.contacts.push(model::Contact {
            chat_type: "group".into(),
            peer_uid: "synthetic".into(),
            display_name: "合成群".into(),
            search_name: "合成群".into(),
        });
        Self {
            ctx: egui::Context::default(),
            wizard,
            tick: 0,
            size: egui::vec2(1000.0, 1000.0),
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> (egui::FullOutput, Option<Action>) {
        self.tick += 1;
        let mut action = None;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            time: Some(f64::from(self.tick) / 60.0),
            events,
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| {
            action = self.wizard.ui(ui.ctx(), true);
        });
        output.textures_delta.clear();
        (output, action)
    }
    fn click(&mut self, label: &str) -> Option<Action> {
        self.frame(Vec::new());
        let output = self.frame(Vec::new()).0;
        fn position(shape: &egui::Shape, label: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.visual_bounding_rect().center())
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| position(shape, label)),
                _ => None,
            }
        }
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| {
                position(&shape.shape, label).filter(|pos| shape.clip_rect.contains(*pos))
            })
            .unwrap_or_else(|| panic!("control not visible: {label}"));
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        let mut action = None;
        for pressed in [true, false] {
            action = self
                .frame(vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }])
                .1
                .or(action);
        }
        action
    }
}

#[test]
fn selecting_a_contact_is_separate_from_export_and_filtering_clears_selection() {
    let mut h = Harness::new();
    assert_eq!(h.click("导出并导入"), None);
    assert_eq!(h.click("合成群  ·  synthetic"), None);
    assert_eq!(h.wizard.selected, Some(0));
    assert_eq!(h.click("导出并导入"), Some(Action::Export));
    h.wizard.search = "no-match".into();
    assert_eq!(h.click("导出并导入"), None);
    assert!(h.wizard.selected.is_none());
}

#[test]
fn changed_connection_requires_check_and_qr_fits_minimum_window() {
    let mut h = Harness::new();
    h.wizard.model.status = None;
    assert_eq!(h.click("扫码登录"), Some(Action::Login));
    h.wizard.settings.base_url = "http://localhost:12345".into();
    assert_eq!(h.click("扫码登录"), None);
    assert_eq!(h.click("检查连接"), Some(Action::Connect));
    h.size = egui::vec2(760.0, 520.0);
    h.wizard.importing = true; // Reserve the same busy/status row used during login.
    h.wizard.model.qr =
        Some(qrcode::QrCode::new(b"https://example.invalid/synthetic-qq-login/2").unwrap());
    for _ in 0..4 {
        h.frame(Vec::new());
    }
    assert!(
        h.wizard.qr_fully_visible,
        "QR must fit the minimum supported viewport"
    );
}

#[test]
fn completion_and_retry_controls_are_visible_without_scrolling() {
    let mut h = Harness::new();
    h.wizard.model.export = Some(serde_json::from_value(super::export_detail()).unwrap());
    h.wizard.model.error = Some("合成导入失败".into());
    assert_eq!(h.click("重试导入已导出文件"), Some(Action::RetryImport));
    h.wizard.imported = true;
    h.wizard.model.error = None;
    assert_eq!(h.click("返回收件箱"), Some(Action::Close));
}
