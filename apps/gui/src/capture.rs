//! Explicit, opt-in smoke evidence. Captures never access the database.
use std::path::{Path, PathBuf};

use eframe::egui;
use serde_json::{Value, json};

use crate::model::GuiModel;

pub fn report(model: &GuiModel, demo: bool) -> Value {
    json!({
        "format_version": 1,
        "gui_version": env!("CARGO_PKG_VERSION"),
        "demo": demo,
        "ready": model.last_error.is_none() && model.inbox_is_current()
            && (demo || model.handshake_ok()),
        "error": model.last_error,
        "cli_version": model.cli_version,
        "selected_chat": model.selected_chat,
        "chats": model.chats.len(),
        "inbox": model.inbox.as_ref().map(|view| json!({
            "chat_id": view.meta.chat_id,
            "counts": view.meta.counts,
            "topics": view.topics.len(),
            "insights": view.insights.len(),
        })),
    })
}

pub fn save(
    path: &Path,
    image: &egui::ColorImage,
    report: Option<(PathBuf, Value)>,
) -> Result<(), String> {
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_array())
        .collect();
    image::save_buffer(
        path,
        &bytes,
        image.width() as u32,
        image.height() as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|e| format!("截图保存失败：{e}"))?;
    // A receipt is published only after the native renderer's PNG was saved.
    if let Some((path, mut report)) = report {
        report["image_width"] = image.width().into();
        report["image_height"] = image.height().into();
        let parent = path.parent().ok_or("验收回执缺少父目录")?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut file, &report).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_failed_and_unhandshaken_views_cannot_pass_smoke() {
        let mut model = GuiModel::demo();
        assert_eq!(report(&model, true)["ready"], true);
        assert_eq!(report(&model, false)["ready"], false);
        model.last_error = Some("synthetic failure".into());
        assert_eq!(report(&model, true)["ready"], false);
        assert_eq!(report(&GuiModel::default(), false)["ready"], false);
    }

    #[test]
    fn failed_capture_never_publishes_a_success_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let receipt = dir.path().join("receipt.json");
        let pixels = egui::ColorImage::filled([2, 2], egui::Color32::WHITE);
        let result = save(
            &dir.path().join("missing/image.png"),
            &pixels,
            Some((receipt.clone(), report(&GuiModel::demo(), true))),
        );
        assert!(result.is_err());
        assert!(!receipt.exists());
    }
}
