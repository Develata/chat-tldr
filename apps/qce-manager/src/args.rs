use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(version, about = "本机 QCE 登录、完整 JSON 导出与受控清理")]
pub struct Cli {
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
    #[arg(long, global = true, default_value = "http://127.0.0.1:40653")]
    pub base_url: String,
    #[arg(long, global = true, default_value = "http://127.0.0.1:6099")]
    pub napcat_url: String,
    #[arg(long, global = true, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=3600))]
    pub timeout_secs: u64,
    #[arg(long, global = true)]
    pub docker: Option<String>,
    #[arg(long, global = true, requires = "docker")]
    pub security_json_path: Option<String>,
    #[arg(long, global = true)]
    pub qce_config_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    pub napcat_config_dir: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    Version,
    Status,
    /// 扫码仅在交互终端显示；非交互调用仅检查已有登录。
    Login {
        #[arg(long, default_value_t = 180, value_parser = clap::value_parser!(u64).range(1..=86400))]
        max_wait_secs: u64,
    },
    Chats,
    Export(ExportArgs),
    Clean {
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ChatType {
    Group,
    Private,
}

impl ChatType {
    pub fn name(self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::Private => "private",
        }
    }

    pub fn number(self) -> u8 {
        match self {
            Self::Group => 2,
            Self::Private => 1,
        }
    }
}

#[derive(Args)]
pub struct ExportArgs {
    #[arg(long = "type", value_enum)]
    pub chat_type: ChatType,
    #[arg(long)]
    pub peer: String,
    /// 包含下界，必须带时区且精确到毫秒或更低精度。
    #[arg(long)]
    pub since: String,
    /// 包含上界（QCE 毫秒闭区间）。
    #[arg(long)]
    pub until: String,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..=3600))]
    pub poll_secs: u64,
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=86400))]
    pub max_wait_secs: u64,
}
