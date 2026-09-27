use super::Wizard;
use eframe::egui::{self, Color32, RichText};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Connect,
    Login,
    Export,
    RetryImport,
    Cancel,
    Close,
}

impl Wizard {
    pub fn ui(&mut self, ctx: &egui::Context, cli_ready: bool) -> Option<Action> {
        self.qr_fully_visible = false;
        if !self.open {
            return None;
        }
        let mut open = true;
        let mut action = None;
        let busy = self.busy();
        egui::Window::new("从 QQ 获取聊天")
            .id(egui::Id::new("qce-acquisition"))
            .open(&mut open)
            .default_size(egui::vec2(640.0, (ctx.content_rect().height() - 60.0).min(820.0)))
            .min_size(egui::vec2(460.0, 380.0))
            .max_size(ctx.content_rect().size() - egui::vec2(32.0, 32.0))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false).resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height() - 130.0).max(250.0)).show(ui, |ui| {
                    ui.label("从本机 QCE 获取最近会话，导出后导入当前数据目录。");
                    ui.small("获取和导入不调用模型。完成后可在收件箱点击“分析新消息”。");
                    ui.add_space(8.0);
                    if let Some(status) = &self.model.status {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(if status.qce_reachable { "服务可达" } else { "服务未就绪" });
                            ui.separator();
                            ui.label(if status.authenticated { "认证通过" } else { "未通过认证" });
                            ui.separator();
                            ui.label(match status.qq_logged_in { Some(true) => "QQ 已登录", Some(false) => "QQ 未登录", None => "QQ 登录状态未知" });
                        });
                    }
                    if let Some(error) = &self.model.error { ui.colored_label(ui.visuals().error_fg_color, error); }
                    if busy {
                        ui.horizontal_wrapped(|ui| {
                            ui.spinner();
                            ui.label(if self.importing { "正在导入聊天…" } else { &self.model.progress });
                            if ui.button("停止等待").clicked() { action = Some(Action::Cancel); }
                        });
                    }
                    if let Some(qr) = &self.model.qr {
                        ui.add_space(8.0);
                        ui.label(RichText::new("用手机 QQ 扫码登录").strong());
                        self.qr_fully_visible = paint_qr(ui, qr);
                        ui.small("二维码会随服务端更新；失效后可停止等待并重新登录。");
                    }
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!busy, egui::Button::new("检查连接")).clicked() { action = Some(Action::Connect); }
                        if ui.add_enabled(!busy && !self.settings_changed() && self.model.compatible && !self.model.status.as_ref().is_some_and(|s| s.qce_ready), egui::Button::new("扫码登录")).clicked() { action = Some(Action::Login); }
                    });
                    if self.model.qr.is_none() { egui::CollapsingHeader::new("高级连接设置").show(ui, |ui| {
                        ui.add_enabled_ui(!busy, |ui| {
                            for (label, value) in [("QCE 地址", &mut self.settings.base_url), ("NapCat 地址", &mut self.settings.napcat_url), ("Docker 容器名（可选）", &mut self.settings.docker), ("管理程序路径（留空使用随附程序）", &mut self.settings.executable), ("QCE 配置目录（可选）", &mut self.settings.qce_config_dir), ("NapCat 配置目录（可选）", &mut self.settings.napcat_config_dir)] {
                                ui.label(label);
                                ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
                            }
                            ui.small("仅连接本机服务；Docker 部署请填写容器名。修改后点击“检查连接”。");
                        });
                    }); }
                    if let Some(export) = &self.model.export {
                        ui.separator();
                        ui.label(format!("已导出 {} 条消息", export.message_count));
                        ui.label(export.path.to_string_lossy());
                        if self.imported {
                            ui.label("导入完成。返回收件箱后可分析新消息。");
                            if ui.button("返回收件箱").clicked() { action = Some(Action::Close); }
                        } else if ui.add_enabled(!busy && cli_ready, egui::Button::new("重试导入已导出文件")).clicked() { action = Some(Action::RetryImport); }
                        return;
                    }
                    if !self.model.status.as_ref().is_some_and(|s| s.qce_ready) { return; }
                    ui.separator();
                    ui.label(RichText::new("选择会话与时间").strong());
                    ui.add_enabled_ui(!busy && !self.settings_changed() && self.model.status.as_ref().is_some_and(|s| s.qce_ready), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui.checkbox(&mut self.groups_only, "只看群聊").changed() { self.selected = None; }
                            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("搜索名称或会话标识"));
                        });
                        ui.small("列表来自 QCE 的最近联系人，不代表全部历史会话。");
                        let query = self.search.trim().to_lowercase();
                        let visible: Vec<_> = self.model.contacts.iter().enumerate().filter(|(_, row)| (!self.groups_only || row.chat_type == "group") && (row.search_name.contains(&query) || row.peer_uid.contains(&query))).collect();
                        // Selection cannot invisibly point at a contact excluded by the filter.
                        if self.selected.is_some_and(|index| !visible.iter().any(|(i, _)| *i == index)) { self.selected = None; }
                        if visible.is_empty() { ui.label("暂无符合条件的会话。登录后检查连接以刷新列表。"); }
                        egui::ScrollArea::vertical().id_salt("qce-contact-list").max_height(150.0).show_rows(ui, 28.0, visible.len(), |ui, range| {
                            for (index, row) in &visible[range] {
                                let label = format!("{}  ·  {}", if row.display_name.is_empty() { "未命名会话" } else { &row.display_name }, row.peer_uid);
                                if ui.selectable_label(self.selected == Some(*index), label).clicked() { self.selected = Some(*index); }
                            }
                        });
                        ui.label("开始时间（包含）");
                        ui.add(egui::TextEdit::singleline(&mut self.since).desired_width(f32::INFINITY));
                        ui.label("结束时间（包含）");
                        ui.add(egui::TextEdit::singleline(&mut self.until).desired_width(f32::INFINITY));
                        ui.small("带时区的时间，默认最近 24 小时。QCE 导出包含起止两个端点。");
                        if ui.add_enabled(self.selected.is_some() && cli_ready, egui::Button::new("导出并导入")).clicked() { action = Some(Action::Export); }
                    });
                });
            });
        if !open {
            action = Some(Action::Close);
        }
        action
    }
}

fn paint_qr(ui: &mut egui::Ui, code: &qrcode::QrCode) -> bool {
    let width = code.width();
    let modules = width + 8; // Four-module quiet zone on every edge.
    let pixels = ui.ctx().pixels_per_point();
    let available = ui
        .available_width()
        .min((ui.ctx().content_rect().height() - 380.0).clamp(112.0, 264.0));
    let scale = (available * pixels / modules as f32).floor().max(1.0) / pixels;
    let side = scale * modules as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, Color32::WHITE);
    for y in 0..width {
        for x in 0..width {
            if code[(x, y)] == qrcode::Color::Dark {
                let pos = rect.min + egui::vec2((x + 4) as f32 * scale, (y + 4) as f32 * scale);
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(pos, egui::vec2(scale, scale)),
                    0.0,
                    Color32::BLACK,
                );
            }
        }
    }
    ui.clip_rect().contains_rect(rect)
}
