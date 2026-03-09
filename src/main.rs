use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use chrono::Local;
use clap::{Args, Parser, Subcommand};
use dotenvy::dotenv;
use ksni::blocking::TrayMethods;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "opentyless-rs")]
#[command(
    about = "面向 Linux 桌面工作流的 Rust 语音转写 CLI：由系统快捷键触发命令，而非监听全局按键"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Daemon,
    StartRecord,
    StopRecord,
    ToggleRecord,
    Status,
    Once(OnceArgs),
    Doctor,
    InstallService(InstallServiceArgs),
    InstallGnomeShortcut(GnomeShortcutArgs),
    InstallTrayAutostart,
    UninstallService,
    ServiceStatus,
    Start,
    Stop,
    Restart,
    Logs(LogsArgs),
    CopyLast,
    Tray,
}

#[derive(Args)]
struct OnceArgs {
    #[arg(long, default_value_t = 5)]
    seconds: u64,
}

#[derive(Args)]
struct InstallServiceArgs {
    #[arg(long)]
    enable: bool,
}

#[derive(Args)]
struct LogsArgs {
    #[arg(short = 'n', long, default_value_t = 50)]
    lines: usize,
}

#[derive(Args)]
struct GnomeShortcutArgs {
    #[arg(long, default_value = "<Super>space")]
    binding: String,
}

#[derive(Debug, Clone)]
struct Config {
    dashscope_base_url: String,
    asr_api_url: String,
    asr_api_key: String,
    asr_model: String,
    flash_api_url: String,
    flash_api_key: String,
    flash_model: String,
    recorder_cmd: Option<String>,
    asr_extra_form: Value,
    flash_extra_body: Value,
    flash_prompt: String,
    auto_copy: bool,
    clipboard_command: Option<String>,
    enable_notifications: bool,
    output_dir: PathBuf,
    run_dir: PathBuf,
    state_dir: PathBuf,
    socket_path: PathBuf,
}

impl Config {
    fn load() -> Result<Self> {
        let _ = dotenv();
        Self::from_env()
    }

    fn from_env() -> Result<Self> {
        let app_state_dir = default_state_home().join("opentyless");
        let app_data_dir = default_data_home().join("opentyless");
        let run_dir = path_env_or("RUN_DIR", app_state_dir.join("run"));
        let state_dir = path_env_or("STATE_DIR", app_state_dir.join("state"));
        let output_dir = path_env_or("OUTPUT_DIR", app_data_dir.join("outputs"));
        Ok(Self {
            dashscope_base_url: env_var(
                "DASHSCOPE_BASE_URL",
                "https://dashscope.aliyuncs.com/compatible-mode/v1",
            ),
            asr_api_url: env_var("ASR_API_URL", ""),
            asr_api_key: env_var_or("ASR_API_KEY", "DASHSCOPE_API_KEY", ""),
            asr_model: env_var("ASR_MODEL", "qwen3-asr-flash"),
            flash_api_url: env_var("FLASH_API_URL", ""),
            flash_api_key: env_var_or("FLASH_API_KEY", "DASHSCOPE_API_KEY", ""),
            flash_model: env_var("FLASH_MODEL", "qwen-flash"),
            recorder_cmd: env_optional("RECORDER_CMD"),
            asr_extra_form: json_env("ASR_EXTRA_FORM_JSON")?,
            flash_extra_body: json_env("FLASH_EXTRA_BODY_JSON")?,
            flash_prompt: env_var(
                "FLASH_PROMPT",
                "请像校对员一样处理下面文本，只改字面，不改意思。保留原句式、原顺序、原语气。若输入是问题，应保留为问题；若输入有明显口吃、口头禅堆叠或无意义重复，可以做轻度合并；但有信息量的重复不要擅自删除。若输入意义不明，也不要解释或猜测其含义。只允许做轻微校对：修正重复标点、明显错别字、少量断句、空格格式和无意义重复。不要回答，不要扩写，不要总结，不要补充任何原文没有的信息。输出时只返回整理后的正文。",
            ),
            auto_copy: env_bool("AUTO_COPY_TO_CLIPBOARD", true),
            clipboard_command: env_optional("CLIPBOARD_COMMAND"),
            enable_notifications: env_bool("ENABLE_NOTIFICATIONS", true),
            output_dir,
            state_dir,
            socket_path: path_env_or("SOCKET_PATH", run_dir.join("opentyless.sock")),
            run_dir,
        })
    }

    fn validate_inference(&self) -> Result<()> {
        let mut missing = Vec::new();
        if self.asr_api_key.is_empty() {
            missing.push("ASR_API_KEY");
        }
        if self.flash_api_key.is_empty() {
            missing.push("FLASH_API_KEY");
        }
        if missing.is_empty() {
            Ok(())
        } else {
            bail!("缺少环境变量: {}", missing.join(", "));
        }
    }

    fn pid_path(&self) -> PathBuf {
        self.run_dir.join("opentyless.pid")
    }

    fn state_path(&self) -> PathBuf {
        self.state_dir.join("status.json")
    }

    fn asr_url(&self) -> String {
        if self.asr_api_url.is_empty() {
            format!(
                "{}/chat/completions",
                self.dashscope_base_url.trim_end_matches('/')
            )
        } else {
            self.asr_api_url.clone()
        }
    }

    fn flash_url(&self) -> String {
        if self.flash_api_url.is_empty() {
            format!(
                "{}/chat/completions",
                self.dashscope_base_url.trim_end_matches('/')
            )
        } else {
            self.flash_api_url.clone()
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct State {
    pid: Option<u32>,
    mode: Option<String>,
    started_at: Option<String>,
    recording: bool,
    recorder_pid: Option<u32>,
    audio_path: Option<String>,
    last_event: Option<String>,
    last_raw_text: Option<String>,
    last_polished_text: Option<String>,
    last_output_path: Option<String>,
    error: Option<String>,
}

impl State {
    fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

#[derive(Debug)]
struct Recorder {
    child: Child,
    audio_path: PathBuf,
}

impl Recorder {
    fn start(config: &Config) -> Result<Self> {
        fs::create_dir_all(&config.run_dir)?;
        let audio_path = config.run_dir.join(format!(
            "recording_{}.wav",
            Local::now().format("%Y%m%d_%H%M%S")
        ));
        let mut command = detect_recorder_command(config)?;
        command.arg(&audio_path);
        let child = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("启动录音进程失败")?;
        Ok(Self { child, audio_path })
    }

    fn stop(mut self) -> Result<PathBuf> {
        self.child.kill().ok();
        self.child.wait().ok();
        Ok(self.audio_path)
    }
}

struct QwenClient {
    client: Client,
    config: Config,
}

impl QwenClient {
    fn new(config: Config) -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()?,
            config,
        })
    }

    fn speech_to_text(&self, audio_path: &Path) -> Result<String> {
        let mut body = build_asr_body(&self.config, audio_path)?;
        merge_json_object(&mut body, &self.config.asr_extra_form)?;
        let payload: Value = self
            .client
            .post(self.config.asr_url())
            .bearer_auth(&self.config.asr_api_key)
            .json(&body)
            .send()?
            .error_for_status()?
            .json()?;
        extract_text(&payload).ok_or_else(|| anyhow!("ASR 返回中未找到文本: {payload}"))
    }

    fn polish_text(&self, raw_text: &str) -> Result<String> {
        let mut body = build_flash_body(&self.config, raw_text);
        merge_json_object(&mut body, &self.config.flash_extra_body)?;
        let payload: Value = self
            .client
            .post(self.config.flash_url())
            .bearer_auth(&self.config.flash_api_key)
            .json(&body)
            .send()?
            .error_for_status()?
            .json()?;
        extract_text(&payload).ok_or_else(|| anyhow!("Flash 返回中未找到文本: {payload}"))
    }
}

struct Pipeline {
    client: QwenClient,
    output_dir: PathBuf,
}

impl Pipeline {
    fn new(config: Config) -> Result<Self> {
        let output_dir = config.output_dir.clone();
        Ok(Self {
            client: QwenClient::new(config)?,
            output_dir,
        })
    }

    fn run(&self, audio_path: &Path) -> Result<(String, String, PathBuf)> {
        fs::create_dir_all(&self.output_dir)?;
        let raw_text = self.client.speech_to_text(audio_path)?;
        let polished_text = self.client.polish_text(&raw_text)?;
        let output_path = self
            .output_dir
            .join(format!("{}.md", Local::now().format("%Y%m%d_%H%M%S")));
        fs::write(
            &output_path,
            format!(
                "# Transcript\n\n## Raw\n\n{}\n\n## Polished\n\n{}\n",
                raw_text, polished_text
            ),
        )?;
        Ok((raw_text, polished_text, output_path))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    action: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    ok: bool,
    message: String,
    state: Option<State>,
}

struct Daemon {
    config: Config,
    state: State,
    recorder: Option<Recorder>,
}

impl Daemon {
    fn new(config: Config) -> Result<Self> {
        fs::create_dir_all(&config.run_dir)?;
        fs::create_dir_all(&config.state_dir)?;
        let state = State {
            pid: Some(std::process::id()),
            mode: Some("daemon".into()),
            started_at: Some(Local::now().to_rfc3339()),
            last_event: Some("daemon_started".into()),
            ..State::default()
        };
        state.save(&config.state_path())?;
        fs::write(config.pid_path(), std::process::id().to_string())?;
        Ok(Self {
            config,
            state,
            recorder: None,
        })
    }

    fn run(mut self) -> Result<()> {
        if self.config.socket_path.exists() {
            fs::remove_file(&self.config.socket_path).ok();
        }
        let listener = UnixListener::bind(&self.config.socket_path)
            .with_context(|| format!("创建 socket 失败: {}", self.config.socket_path.display()))?;
        println!(
            "daemon 已启动，socket: {}",
            self.config.socket_path.display()
        );
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    if let Err(error) = self.handle_stream(stream) {
                        self.state.error = Some(error.to_string());
                        self.state.last_event = Some("request_failed".into());
                        self.state.save(&self.config.state_path()).ok();
                    }
                }
                Err(error) => {
                    self.state.error = Some(error.to_string());
                    self.state.last_event = Some("socket_error".into());
                    self.state.save(&self.config.state_path()).ok();
                }
            }
        }
        Ok(())
    }

    fn handle_stream(&mut self, mut stream: UnixStream) -> Result<()> {
        let timeout = Some(Duration::from_secs(5));
        stream.set_read_timeout(timeout)?;
        stream.set_write_timeout(timeout)?;
        let mut input = String::new();
        BufReader::new(stream.try_clone()?).read_line(&mut input)?;
        let request: Request = serde_json::from_str(input.trim())?;
        let response = self.handle_action(&request.action);
        stream.write_all(format!("{}\n", serde_json::to_string(&response)?).as_bytes())?;
        Ok(())
    }

    fn handle_action(&mut self, action: &str) -> Response {
        let result = match action {
            "start-record" => self.start_record(),
            "stop-record" => self.stop_record(),
            "toggle-record" => {
                if self.recorder.is_some() {
                    self.stop_record()
                } else {
                    self.start_record()
                }
            }
            "status" => Ok("状态已返回".into()),
            _ => Err(anyhow!("未知 action: {action}")),
        };

        match result {
            Ok(message) => Response {
                ok: true,
                message,
                state: Some(self.state.clone()),
            },
            Err(error) => {
                self.state.error = Some(error.to_string());
                self.state.last_event = Some("command_failed".into());
                self.state.save(&self.config.state_path()).ok();
                Response {
                    ok: false,
                    message: error.to_string(),
                    state: Some(self.state.clone()),
                }
            }
        }
    }

    fn start_record(&mut self) -> Result<String> {
        if self.recorder.is_some() {
            bail!("当前已经在录音中");
        }
        let recorder = Recorder::start(&self.config)?;
        self.state.recording = true;
        self.state.recorder_pid = Some(recorder.child.id());
        self.state.audio_path = Some(recorder.audio_path.display().to_string());
        self.state.last_event = Some("recording_started".into());
        self.state.error = None;
        self.state.save(&self.config.state_path())?;
        notify_event(&self.config, "OpenTyless", "开始录音").ok();
        self.recorder = Some(recorder);
        Ok("已开始录音".into())
    }

    fn stop_record(&mut self) -> Result<String> {
        let recorder = self
            .recorder
            .take()
            .ok_or_else(|| anyhow!("当前没有录音任务"))?;
        let audio_path = recorder.stop()?;
        self.state.recording = false;
        self.state.recorder_pid = None;
        self.state.last_event = Some("transcribing".into());
        self.state.audio_path = Some(audio_path.display().to_string());
        self.state.error = None;
        self.state.save(&self.config.state_path())?;
        notify_event(&self.config, "OpenTyless", "已停止录音，正在转写").ok();
        spawn_pipeline_job(self.config.clone(), audio_path.clone());

        Ok(format!("已停止录音，正在转写: {}", audio_path.display()))
    }
}

fn spawn_pipeline_job(config: Config, audio_path: PathBuf) {
    let state_path = config.state_path();
    thread::spawn(move || {
        let result: Result<(String, String, PathBuf)> = (|| {
            config.validate_inference()?;
            let pipeline = Pipeline::new(config.clone())?;
            let (raw_text, polished_text, output_path) = pipeline.run(&audio_path)?;
            maybe_copy_to_clipboard(&config, &polished_text).ok();
            notify_event(
                &config,
                "OpenTyless",
                &format!(
                    "转写完成，已复制到剪贴板\n{}",
                    truncate_text(&polished_text, 80)
                ),
            )
            .ok();
            Ok((raw_text, polished_text, output_path))
        })();

        let mut state = State::load(&state_path);
        match result {
            Ok((raw_text, polished_text, output_path)) => {
                state.last_event = Some("pipeline_completed".into());
                state.last_raw_text = Some(raw_text);
                state.last_polished_text = Some(polished_text);
                state.last_output_path = Some(output_path.display().to_string());
                state.error = None;
            }
            Err(error) => {
                state.last_event = Some("pipeline_failed".into());
                state.error = Some(error.to_string());
                notify_event(&config, "OpenTyless 错误", &error.to_string()).ok();
            }
        }
        state.save(&state_path).ok();
    });
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;
    match cli.command {
        Commands::Daemon => cmd_daemon(config),
        Commands::StartRecord => cmd_send(&config, "start-record"),
        Commands::StopRecord => cmd_send(&config, "stop-record"),
        Commands::ToggleRecord => cmd_send(&config, "toggle-record"),
        Commands::Status => cmd_status(&config),
        Commands::Once(args) => cmd_once(config, args.seconds),
        Commands::Doctor => cmd_doctor(&config),
        Commands::InstallService(args) => cmd_install_service(&config, args.enable),
        Commands::InstallGnomeShortcut(args) => cmd_install_gnome_shortcut(args),
        Commands::InstallTrayAutostart => cmd_install_tray_autostart(),
        Commands::UninstallService => cmd_uninstall_service(),
        Commands::ServiceStatus => cmd_service_status(),
        Commands::Start => cmd_systemctl(&["start", "opentyless.service"]),
        Commands::Stop => cmd_systemctl(&["stop", "opentyless.service"]),
        Commands::Restart => cmd_systemctl(&["restart", "opentyless.service"]),
        Commands::Logs(args) => cmd_logs(args.lines),
        Commands::CopyLast => cmd_copy_last(&config),
        Commands::Tray => cmd_tray(config),
    }
}

fn cmd_daemon(config: Config) -> Result<()> {
    if process_running(&config.pid_path()) {
        bail!("daemon 已在运行中");
    }
    Daemon::new(config)?.run()
}

fn cmd_send(config: &Config, action: &str) -> Result<()> {
    let response = send_request(
        &config.socket_path,
        &Request {
            action: action.into(),
        },
    )?;
    if response.ok {
        println!("{}", response.message);
        Ok(())
    } else {
        bail!(response.message)
    }
}

fn cmd_status(config: &Config) -> Result<()> {
    println!(
        "daemon_running: {}",
        if process_running(&config.pid_path()) {
            "yes"
        } else {
            "no"
        }
    );
    println!("socket_path: {}", config.socket_path.display());
    let state = State::load(&config.state_path());
    println!("{}", serde_json::to_string_pretty(&state)?);
    Ok(())
}

fn cmd_once(config: Config, seconds: u64) -> Result<()> {
    config.validate_inference()?;
    let recorder = Recorder::start(&config)?;
    std::thread::sleep(Duration::from_secs(seconds));
    let audio_path = recorder.stop()?;
    let pipeline = Pipeline::new(config)?;
    let (raw_text, polished_text, output_path) = pipeline.run(&audio_path)?;
    maybe_copy_to_clipboard(&pipeline.client.config, &polished_text).ok();
    println!("--- RAW ---\n{}\n", raw_text);
    println!("--- POLISHED ---\n{}\n", polished_text);
    println!("已保存: {}", output_path.display());
    Ok(())
}

fn cmd_doctor(config: &Config) -> Result<()> {
    let session = current_session_type().unwrap_or_else(|| "unknown".into());
    println!("录音命令: {}", detect_recorder_preview(config)?);
    println!("XDG_SESSION_TYPE: {}", session);
    for name in [
        "DASHSCOPE_BASE_URL",
        "DASHSCOPE_API_KEY",
        "ASR_API_URL",
        "ASR_API_KEY",
        "FLASH_API_URL",
        "FLASH_API_KEY",
    ] {
        let is_set = std::env::var(name)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        println!("{}: {}", name, if is_set { "已设置" } else { "未设置" });
    }
    println!("ASR effective url: {}", config.asr_url());
    println!("Flash effective url: {}", config.flash_url());
    println!("RUN_DIR: {}", config.run_dir.display());
    println!("STATE_DIR: {}", config.state_dir.display());
    println!("SOCKET_PATH: {}", config.socket_path.display());
    println!(
        "AUTO_COPY_TO_CLIPBOARD: {}",
        if config.auto_copy { "true" } else { "false" }
    );
    println!(
        "CLIPBOARD_COMMAND: {}",
        detect_clipboard_preview(config).unwrap_or_else(|| "未检测到".into())
    );
    println!(
        "ENABLE_NOTIFICATIONS: {}",
        if config.enable_notifications {
            "true"
        } else {
            "false"
        }
    );
    println!(
        "提示：请把系统快捷键绑定到 `start-record` / `stop-record` / `toggle-record`；推荐直接绑定 `toggle-record`"
    );
    Ok(())
}

fn cmd_copy_last(config: &Config) -> Result<()> {
    let state = State::load(&config.state_path());
    let text = state
        .last_polished_text
        .ok_or_else(|| anyhow!("最近没有可复制的整理结果"))?;
    maybe_copy_to_clipboard(config, &text)?;
    println!("已复制最近一次整理文本到剪贴板");
    Ok(())
}

fn cmd_install_service(config: &Config, enable: bool) -> Result<()> {
    ensure_systemctl()?;
    let service_dir = PathBuf::from(std::env::var("HOME")?).join(".config/systemd/user");
    fs::create_dir_all(&service_dir)?;
    let service_path = service_dir.join("opentyless.service");
    fs::write(&service_path, render_service(config))?;
    run_command("systemctl", &["--user", "daemon-reload"])?;
    if enable {
        run_command(
            "systemctl",
            &["--user", "enable", "--now", "opentyless.service"],
        )?;
        println!("已安装并启用: {}", service_path.display());
    } else {
        println!("已安装服务文件: {}", service_path.display());
    }
    Ok(())
}

fn cmd_uninstall_service() -> Result<()> {
    ensure_systemctl()?;
    let service_path =
        PathBuf::from(std::env::var("HOME")?).join(".config/systemd/user/opentyless.service");
    run_command_allow_fail(
        "systemctl",
        &["--user", "disable", "--now", "opentyless.service"],
    )?;
    if service_path.exists() {
        fs::remove_file(&service_path)?;
    }
    run_command("systemctl", &["--user", "daemon-reload"])?;
    println!("已卸载服务文件");
    Ok(())
}

fn cmd_install_gnome_shortcut(args: GnomeShortcutArgs) -> Result<()> {
    ensure_command("gsettings")?;
    let command = preferred_shortcut_command();
    let mut keybindings = load_gnome_custom_keybindings()?;
    let path = find_or_create_gnome_binding_path(&mut keybindings, "OpenTyless Toggle");
    run_command(
        "gsettings",
        &[
            "set",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
            &format_gsettings_string_array(&keybindings),
        ],
    )?;
    let base = format!(
        "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:{}",
        path
    );
    run_command("gsettings", &["set", &base, "name", "OpenTyless Toggle"])?;
    run_command("gsettings", &["set", &base, "command", &command])?;
    run_command("gsettings", &["set", &base, "binding", &args.binding])?;
    println!("已安装 GNOME 快捷键: {} -> {}", args.binding, command);
    Ok(())
}

fn cmd_install_tray_autostart() -> Result<()> {
    let autostart_dir = PathBuf::from(std::env::var("HOME")?).join(".config/autostart");
    fs::create_dir_all(&autostart_dir)?;
    let desktop_path = autostart_dir.join("opentyless-tray.desktop");
    let exe = preferred_binary_path();
    fs::write(
        &desktop_path,
        format!(
            "[Desktop Entry]\nType=Application\nName=OpenTyless Tray\nExec={} tray\nX-GNOME-Autostart-enabled=true\nNoDisplay=false\n",
            exe.display()
        ),
    )?;
    println!("已安装托盘自启动: {}", desktop_path.display());
    Ok(())
}

fn cmd_service_status() -> Result<()> {
    ensure_systemctl()?;
    let output = Command::new("systemctl")
        .args(["--user", "status", "opentyless.service"])
        .output()?;
    print_output(output);
    Ok(())
}

fn cmd_systemctl(args: &[&str]) -> Result<()> {
    ensure_systemctl()?;
    run_command("systemctl", &["--user", args[0], args[1]])?;
    println!("已执行: systemctl --user {} {}", args[0], args[1]);
    Ok(())
}

fn cmd_logs(lines: usize) -> Result<()> {
    let output = Command::new("journalctl")
        .args([
            "--user",
            "-u",
            "opentyless.service",
            "-n",
            &lines.to_string(),
            "--no-pager",
        ])
        .output()?;
    print_output(output);
    Ok(())
}

fn cmd_tray(config: Config) -> Result<()> {
    let handle = OpenTylessTray::new(config).spawn()?;
    loop {
        thread::sleep(Duration::from_secs(1));
        let _ = handle.update(|tray| tray.refresh());
    }
}

fn send_request(socket_path: &Path, request: &Request) -> Result<Response> {
    let mut stream = UnixStream::connect(socket_path)
        .with_context(|| format!("无法连接 daemon，请先运行 `opentyless-rs daemon` 或 `install-service --enable`。socket={}", socket_path.display()))?;
    let timeout = Some(Duration::from_secs(5));
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    stream.write_all(format!("{}\n", serde_json::to_string(request)?).as_bytes())?;
    let mut response = String::new();
    BufReader::new(stream).read_to_string(&mut response)?;
    Ok(serde_json::from_str(response.trim())?)
}

fn detect_recorder_command(config: &Config) -> Result<Command> {
    if let Some(custom) = &config.recorder_cmd {
        let mut parts = custom.split_whitespace();
        let executable = parts
            .next()
            .ok_or_else(|| anyhow!("RECORDER_CMD 不能为空"))?;
        let mut command = Command::new(executable);
        command.args(parts);
        return Ok(command);
    }

    if command_exists("ffmpeg") {
        let mut command = Command::new("ffmpeg");
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "pulse",
            "-i",
            "default",
            "-ac",
            "1",
            "-ar",
            "16000",
        ]);
        return Ok(command);
    }

    if command_exists("arecord") {
        let mut command = Command::new("arecord");
        command.args(["-f", "S16_LE", "-r", "16000", "-c", "1"]);
        return Ok(command);
    }

    bail!("未找到录音命令，请安装 ffmpeg 或 arecord，或设置 RECORDER_CMD")
}

fn detect_recorder_preview(config: &Config) -> Result<String> {
    if let Some(custom) = &config.recorder_cmd {
        return Ok(custom.clone());
    }
    if command_exists("ffmpeg") {
        return Ok(
            "ffmpeg -hide_banner -loglevel error -f pulse -i default -ac 1 -ar 16000 <output.wav>"
                .into(),
        );
    }
    if command_exists("arecord") {
        return Ok("arecord -f S16_LE -r 16000 -c 1 <output.wav>".into());
    }
    bail!("未找到录音命令")
}

fn build_asr_body(config: &Config, audio_path: &Path) -> Result<Value> {
    let audio_bytes = fs::read(audio_path)?;
    let audio_base64 = base64::engine::general_purpose::STANDARD.encode(audio_bytes);
    let format = detect_audio_format(audio_path);
    Ok(json!({
        "model": config.asr_model,
        "messages": [{
            "role": "user",
            "content": [{
                "type": "input_audio",
                "input_audio": {
                    "data": format!("data:audio/{};base64,{}", format, audio_base64),
                    "format": format
                }
            }]
        }],
        "stream": false,
        "asr_options": {
            "enable_itn": true
        }
    }))
}

fn build_flash_body(config: &Config, raw_text: &str) -> Value {
    json!({
        "model": config.flash_model,
        "messages": [
            {
                "role": "system",
                "content": config.flash_prompt,
            },
            {
                "role": "user",
                "content": raw_text,
            }
        ],
        "stream": false
    })
}

#[derive(Debug)]
struct OpenTylessTray {
    config: Config,
    recording: bool,
    tooltip: String,
}

impl OpenTylessTray {
    fn new(config: Config) -> Self {
        let state = State::load(&config.state_path());
        Self {
            config,
            recording: state.recording,
            tooltip: build_tray_tooltip(&state),
        }
    }

    fn refresh(&mut self) {
        let state = State::load(&self.config.state_path());
        self.recording = state.recording;
        self.tooltip = build_tray_tooltip(&state);
    }

    fn spawn_action(&self, action: TrayAction) {
        let config = self.config.clone();
        thread::spawn(move || {
            let result: Result<()> = (|| match action {
                TrayAction::Toggle => cmd_send(&config, "toggle-record"),
                TrayAction::Start => cmd_send(&config, "start-record"),
                TrayAction::Stop => cmd_send(&config, "stop-record"),
                TrayAction::CopyLast => cmd_copy_last(&config),
                TrayAction::OpenOutputDir => {
                    ensure_command("gio")?;
                    run_command(
                        "gio",
                        &["open", config.output_dir.to_string_lossy().as_ref()],
                    )
                }
                TrayAction::ShowStatus => {
                    let state = State::load(&config.state_path());
                    notify_event(&config, "OpenTyless 状态", &build_tray_tooltip(&state))
                }
            })();
            if let Err(error) = result {
                let _ = notify_event(&config, "OpenTyless 错误", &error.to_string());
            }
        });
    }
}

#[derive(Clone, Copy, Debug)]
enum TrayAction {
    Toggle,
    Start,
    Stop,
    CopyLast,
    OpenOutputDir,
    ShowStatus,
}

impl ksni::Tray for OpenTylessTray {
    fn id(&self) -> String {
        "opentyless-tray".into()
    }

    fn title(&self) -> String {
        if self.recording {
            "OpenTyless ● 录音中".into()
        } else {
            "OpenTyless".into()
        }
    }

    fn icon_name(&self) -> String {
        if self.recording {
            "audio-input-microphone-symbolic".into()
        } else {
            "audio-input-microphone".into()
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.title(),
            description: self.tooltip.clone(),
            icon_name: self.icon_name(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let toggle_label = if self.recording {
            "停止录音"
        } else {
            "开始录音"
        };
        vec![
            StandardItem {
                label: toggle_label.into(),
                activate: Box::new(|tray: &mut OpenTylessTray| {
                    tray.spawn_action(TrayAction::Toggle)
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "单独开始录音".into(),
                activate: Box::new(|tray: &mut OpenTylessTray| {
                    tray.spawn_action(TrayAction::Start)
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "单独停止并转写".into(),
                activate: Box::new(|tray: &mut OpenTylessTray| tray.spawn_action(TrayAction::Stop)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "复制最近结果".into(),
                activate: Box::new(|tray: &mut OpenTylessTray| {
                    tray.spawn_action(TrayAction::CopyLast)
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "打开输出目录".into(),
                activate: Box::new(|tray: &mut OpenTylessTray| {
                    tray.spawn_action(TrayAction::OpenOutputDir)
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "显示状态通知".into(),
                activate: Box::new(|tray: &mut OpenTylessTray| {
                    tray.spawn_action(TrayAction::ShowStatus)
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "退出托盘".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

fn build_tray_tooltip(state: &State) -> String {
    if state.recording {
        return "正在录音；再次按快捷键或点托盘菜单可停止并转写。".into();
    }
    if let Some(text) = &state.last_polished_text {
        return format!("最近结果：{}", truncate_text(text, 60));
    }
    "待命中；可从快捷键或托盘开始录音。".into()
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut result = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= max_chars {
            result.push('…');
            break;
        }
        result.push(ch);
    }
    result
}

fn detect_audio_format(audio_path: &Path) -> &'static str {
    match audio_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "mpeg",
        "wav" => "wav",
        "ogg" => "ogg",
        "m4a" => "m4a",
        _ => "wav",
    }
}

fn extract_text(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in ["text", "result", "output", "content", "answer"] {
                if let Some(found) = map.get(key).and_then(extract_text) {
                    return Some(found);
                }
            }
            for nested_key in ["data", "message"] {
                if let Some(found) = map.get(nested_key).and_then(extract_text) {
                    return Some(found);
                }
            }
            map.get("choices")
                .and_then(|choices| choices.as_array())
                .and_then(|choices| choices.iter().find_map(extract_text))
        }
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Array(items) => items.iter().find_map(extract_text),
        _ => None,
    }
}

fn json_env(name: &str) -> Result<Value> {
    match env_optional(name) {
        Some(raw) => {
            let value: Value = serde_json::from_str(&raw)
                .with_context(|| format!("环境变量 {name} 不是合法 JSON"))?;
            if !value.is_object() {
                bail!("环境变量 {name} 必须是 JSON 对象");
            }
            Ok(value)
        }
        None => Ok(json!({})),
    }
}

fn merge_json_object(target: &mut Value, extra: &Value) -> Result<()> {
    let target_map = target
        .as_object_mut()
        .ok_or_else(|| anyhow!("target 必须是 JSON 对象"))?;
    let extra_map = extra
        .as_object()
        .ok_or_else(|| anyhow!("附加配置必须是 JSON 对象"))?;
    for (key, value) in extra_map {
        target_map.insert(key.clone(), value.clone());
    }
    Ok(())
}

fn render_service(_config: &Config) -> String {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("cargo run --release --"));
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    format!(
        "[Unit]\nDescription=OpenTyless Rust daemon\nAfter=default.target\n\n[Service]\nType=simple\nWorkingDirectory={}\nEnvironmentFile={}\nExecStart={} daemon\nRestart=always\nRestartSec=2\n\n[Install]\nWantedBy=default.target\n",
        root.display(),
        root.join(".env").display(),
        exe.display(),
    )
}

fn preferred_binary_path() -> PathBuf {
    let current =
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("./target/release/opentyless-rs"));
    if current.to_string_lossy().contains("target/debug") {
        let release = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("target/release/opentyless-rs");
        if release.exists() {
            return release;
        }
    }
    current
}

fn preferred_shortcut_command() -> String {
    if let Ok(home) = std::env::var("HOME") {
        let wrapper = PathBuf::from(home).join(".local/bin/opentyless-hotkey-toggle");
        if wrapper.exists() {
            return wrapper.display().to_string();
        }
    }
    format!("{} toggle-record", preferred_binary_path().display())
}

fn load_gnome_custom_keybindings() -> Result<Vec<String>> {
    let output = Command::new("gsettings")
        .args([
            "get",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
        ])
        .output()?;
    if !output.status.success() {
        bail!(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(parse_gsettings_string_array(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

fn parse_gsettings_string_array(input: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    for ch in input.chars() {
        match ch {
            '\'' if in_string => {
                items.push(current.clone());
                current.clear();
                in_string = false;
            }
            '\'' => in_string = true,
            _ if in_string => current.push(ch),
            _ => {}
        }
    }
    items
}

fn format_gsettings_string_array(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| format!("'{}'", item)).collect();
    format!("[{}]", quoted.join(", "))
}

fn find_or_create_gnome_binding_path(paths: &mut Vec<String>, name: &str) -> String {
    for path in paths.iter() {
        let schema = format!(
            "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:{}",
            path
        );
        if let Ok(output) = Command::new("gsettings")
            .args(["get", &schema, "name"])
            .output()
        {
            if String::from_utf8_lossy(&output.stdout).contains(name) {
                return path.clone();
            }
        }
    }
    let mut index = 0;
    loop {
        let candidate = format!(
            "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom{}/",
            index
        );
        if !paths.iter().any(|path| path == &candidate) {
            paths.push(candidate.clone());
            return candidate;
        }
        index += 1;
    }
}

fn maybe_copy_to_clipboard(config: &Config, text: &str) -> Result<()> {
    if !config.auto_copy {
        return Ok(());
    }
    let command = detect_clipboard_command(config)
        .ok_or_else(|| anyhow!("未检测到剪贴板命令，已跳过自动复制"))?;
    if command
        .first()
        .map(|program| should_detach_stdin_command(program))
        .unwrap_or(false)
    {
        spawn_command_with_stdin_detached(&command[0], &command[1..], text)
    } else {
        run_command_with_stdin(&command[0], &command[1..], text)
    }
}

fn notify_event(config: &Config, summary: &str, body: &str) -> Result<()> {
    if !config.enable_notifications || !command_exists("notify-send") {
        return Ok(());
    }
    run_command("notify-send", &[summary, body])
}

fn detect_clipboard_command(config: &Config) -> Option<Vec<String>> {
    if let Some(custom) = &config.clipboard_command {
        let parts: Vec<String> = custom.split_whitespace().map(|s| s.to_string()).collect();
        if !parts.is_empty() {
            return Some(parts);
        }
    }
    for candidate in clipboard_command_candidates(current_session_type().as_deref()) {
        if command_exists(&candidate[0]) {
            return Some(candidate);
        }
    }
    None
}

fn clipboard_command_candidates(session_type: Option<&str>) -> Vec<Vec<String>> {
    let wl_copy = vec!["wl-copy".into()];
    let xclip = vec!["xclip".into(), "-selection".into(), "clipboard".into()];
    match session_type {
        Some("x11") => vec![xclip, wl_copy],
        Some("wayland") => vec![wl_copy, xclip],
        _ => vec![wl_copy, xclip],
    }
}

fn detect_clipboard_preview(config: &Config) -> Option<String> {
    detect_clipboard_command(config).map(|parts| parts.join(" "))
}

fn run_command_with_stdin(program: &str, args: &[String], input: &str) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(input.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

fn spawn_command_with_stdin_detached(program: &str, args: &[String], input: &str) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes())?;
    }
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn should_detach_stdin_command(program: &str) -> bool {
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        == Some("xclip")
}

fn run_command(program: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(program).args(args).output()?;
    if !output.status.success() {
        bail!(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

fn run_command_allow_fail(program: &str, args: &[&str]) -> Result<()> {
    let _ = Command::new(program).args(args).output()?;
    Ok(())
}

fn print_output(output: std::process::Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.trim().is_empty() {
        println!("{}", stdout.trim_end());
    }
    if !stderr.trim().is_empty() {
        eprintln!("{}", stderr.trim_end());
    }
}

fn ensure_systemctl() -> Result<()> {
    ensure_command("systemctl")
}

fn ensure_command(command: &str) -> Result<()> {
    if command_exists(command) {
        Ok(())
    } else {
        bail!("未找到 {}", command)
    }
}

fn command_exists(command: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {} >/dev/null 2>&1", command)])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn process_running(pid_path: &Path) -> bool {
    fs::read_to_string(pid_path)
        .ok()
        .and_then(|content| content.trim().parse::<u32>().ok())
        .map(|pid| PathBuf::from(format!("/proc/{pid}")).exists())
        .unwrap_or(false)
}

fn env_var(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn path_env_or(name: &str, default: PathBuf) -> PathBuf {
    env_optional(name).map(PathBuf::from).unwrap_or(default)
}

fn env_optional(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(|v| {
        let trimmed = v.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn current_session_type() -> Option<String> {
    env_optional("XDG_SESSION_TYPE").map(|value| value.trim().to_ascii_lowercase())
}

fn env_var_or(primary: &str, fallback: &str, default: &str) -> String {
    env_optional(primary)
        .or_else(|| env_optional(fallback))
        .unwrap_or_else(|| default.to_string())
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn default_state_home() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn default_data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn sample_config() -> Config {
        Config {
            dashscope_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            asr_api_url: "".into(),
            asr_api_key: "k".into(),
            asr_model: "qwen3-asr-flash".into(),
            flash_api_url: "".into(),
            flash_api_key: "k".into(),
            flash_model: "qwen-flash".into(),
            recorder_cmd: None,
            asr_extra_form: json!({}),
            flash_extra_body: json!({}),
            flash_prompt: "整理一下".into(),
            auto_copy: false,
            clipboard_command: None,
            enable_notifications: false,
            output_dir: PathBuf::from("outputs"),
            run_dir: PathBuf::from("run"),
            state_dir: PathBuf::from("state"),
            socket_path: PathBuf::from("run/opentyless.sock"),
        }
    }

    #[test]
    fn extract_text_from_openai_style_content_array() {
        let payload = json!({
            "choices": [{
                "message": {
                    "content": [{"type": "text", "text": "你好世界"}]
                }
            }]
        });
        assert_eq!(extract_text(&payload).as_deref(), Some("你好世界"));
    }

    #[test]
    fn build_asr_body_contains_data_url() {
        let temp = tempdir().unwrap();
        let wav = temp.path().join("sample.wav");
        fs::write(&wav, b"RIFF....WAVE").unwrap();
        let body = build_asr_body(&sample_config(), &wav).unwrap();
        let data = body["messages"][0]["content"][0]["input_audio"]["data"]
            .as_str()
            .unwrap();
        assert!(data.starts_with("data:audio/wav;base64,"));
    }

    #[test]
    fn flash_url_defaults_to_dashscope() {
        let config = sample_config();
        assert_eq!(
            config.flash_url(),
            "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
        );
        assert_eq!(
            config.asr_url(),
            "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
        );
    }

    #[test]
    fn path_env_defaults_use_xdg_absolute_dirs() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempdir().unwrap();
        let home = temp.path().join("home");
        let state_home = temp.path().join("state-home");
        let data_home = temp.path().join("data-home");
        fs::create_dir_all(&home).unwrap();
        let old_home = std::env::var_os("HOME");
        let old_state_home = std::env::var_os("XDG_STATE_HOME");
        let old_data_home = std::env::var_os("XDG_DATA_HOME");
        let old_run_dir = std::env::var_os("RUN_DIR");
        let old_state_dir = std::env::var_os("STATE_DIR");
        let old_output_dir = std::env::var_os("OUTPUT_DIR");
        let old_socket_path = std::env::var_os("SOCKET_PATH");
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("XDG_STATE_HOME", &state_home);
            std::env::set_var("XDG_DATA_HOME", &data_home);
            std::env::remove_var("RUN_DIR");
            std::env::remove_var("STATE_DIR");
            std::env::remove_var("OUTPUT_DIR");
            std::env::remove_var("SOCKET_PATH");
        }

        let config = Config::from_env().unwrap();
        assert_eq!(config.run_dir, state_home.join("opentyless/run"));
        assert_eq!(config.state_dir, state_home.join("opentyless/state"));
        assert_eq!(config.output_dir, data_home.join("opentyless/outputs"));
        assert_eq!(
            config.socket_path,
            state_home.join("opentyless/run/opentyless.sock")
        );

        unsafe {
            match old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match old_state_home {
                Some(value) => std::env::set_var("XDG_STATE_HOME", value),
                None => std::env::remove_var("XDG_STATE_HOME"),
            }
            match old_data_home {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
            match old_run_dir {
                Some(value) => std::env::set_var("RUN_DIR", value),
                None => std::env::remove_var("RUN_DIR"),
            }
            match old_state_dir {
                Some(value) => std::env::set_var("STATE_DIR", value),
                None => std::env::remove_var("STATE_DIR"),
            }
            match old_output_dir {
                Some(value) => std::env::set_var("OUTPUT_DIR", value),
                None => std::env::remove_var("OUTPUT_DIR"),
            }
            match old_socket_path {
                Some(value) => std::env::set_var("SOCKET_PATH", value),
                None => std::env::remove_var("SOCKET_PATH"),
            }
        }
    }

    #[test]
    fn absolute_xclip_path_uses_detached_clipboard_flow() {
        assert!(should_detach_stdin_command("xclip"));
        assert!(should_detach_stdin_command("/home/jin/.local/bin/xclip"));
        assert!(!should_detach_stdin_command("wl-copy"));
    }

    #[test]
    fn clipboard_candidates_prefer_xclip_on_x11() {
        let candidates = clipboard_command_candidates(Some("x11"));
        assert_eq!(
            candidates.first(),
            Some(&vec![
                "xclip".to_string(),
                "-selection".to_string(),
                "clipboard".to_string()
            ])
        );
    }

    #[test]
    fn clipboard_candidates_prefer_wl_copy_on_wayland() {
        let candidates = clipboard_command_candidates(Some("wayland"));
        assert_eq!(candidates.first(), Some(&vec!["wl-copy".to_string()]));
    }

    #[test]
    fn current_session_type_normalizes_case() {
        let _guard = env_lock().lock().unwrap();
        let old = std::env::var_os("XDG_SESSION_TYPE");
        unsafe {
            std::env::set_var("XDG_SESSION_TYPE", "WayLand");
        }
        assert_eq!(current_session_type().as_deref(), Some("wayland"));
        unsafe {
            match old {
                Some(value) => std::env::set_var("XDG_SESSION_TYPE", value),
                None => std::env::remove_var("XDG_SESSION_TYPE"),
            }
        }
    }
}
