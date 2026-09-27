mod app;
mod appearance;
mod bridge;
mod capture;
mod model;
mod prefs;
mod ui;

use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum DemoView {
    #[default]
    Inbox,
    Evidence,
    Decisions,
    Stats,
    Overview,
}

#[derive(Parser)]
#[command(version, about = "群聊省流桌面端；通过本地 CLI 读写聊天数据")]
struct Args {
    /// Path to chat-tldr; defaults to the executable beside this GUI.
    #[arg(long)]
    cli: Option<PathBuf>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    config: Option<PathBuf>,
    /// Show synthetic fixtures; never starts a CLI or writes preferences.
    #[arg(long)]
    demo: bool,
    /// Select a reproducible synthetic view for screenshots.
    #[arg(long, value_enum, hide = true, requires = "demo")]
    demo_view: Option<DemoView>,
    #[arg(long, hide = true)]
    screenshot: Option<PathBuf>,
    /// Write opt-in capture metadata for isolated native smoke tests.
    #[arg(long, hide = true, requires = "screenshot")]
    smoke_report: Option<PathBuf>,
    #[arg(long, hide = true, requires = "screenshot")]
    quit_after_capture: bool,
    #[arg(long, hide = true, default_value_t = 1280.0)]
    width: f32,
    #[arg(long, hide = true, default_value_t = 820.0)]
    height: f32,
    #[arg(long)]
    dark: bool,
}

fn main() -> eframe::Result {
    let args = Args::parse();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([args.width, args.height])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    let startup = app::Startup::load(args);
    eframe::run_native(
        "群聊省流",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, startup)))),
    )
}
