use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "opentyless-rs")]
#[command(about = "Rust 语音转写 CLI，支持 Linux 桌面和 macOS 状态栏")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    Daemon,
    StartRecord,
    SaveRecord,
    StopRecord,
    CancelRecord,
    ToggleRecord,
    Status,
    Once(OnceArgs),
    /// Transcribe an existing recording without changing the clipboard or daemon state.
    TranscribeFile {
        audio: PathBuf,
        #[arg(long)]
        raw_only: bool,
    },
    Doctor,
    /// Validate ASR and cleanup credentials without recording or making network requests.
    CheckConfig,
    InstallService(InstallServiceArgs),
    InstallGnomeShortcut(GnomeShortcutArgs),
    InstallTrayAutostart,
    InstallTrayService(InstallServiceArgs),
    UninstallService,
    UninstallTrayService,
    ServiceStatus,
    TrayServiceStatus,
    Start,
    Stop,
    Restart,
    StartTray,
    StopTray,
    RestartTray,
    Logs(LogsArgs),
    TrayLogs(LogsArgs),
    CopyLast,
    Tray,
}

#[derive(Args)]
pub(crate) struct OnceArgs {
    #[arg(long, default_value_t = 5)]
    pub(crate) seconds: u64,
}

#[derive(Args)]
pub(crate) struct InstallServiceArgs {
    #[arg(long)]
    pub(crate) enable: bool,
}

#[derive(Args)]
pub(crate) struct LogsArgs {
    #[arg(short = 'n', long, default_value_t = 50)]
    pub(crate) lines: usize,
}

#[derive(Args)]
pub(crate) struct GnomeShortcutArgs {
    #[arg(long, default_value = "<Super>space")]
    pub(crate) binding: String,
}
