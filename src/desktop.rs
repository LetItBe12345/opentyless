use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
use std::thread;
#[cfg(target_os = "linux")]
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
#[cfg(target_os = "linux")]
use ksni::blocking::TrayMethods;

#[cfg(target_os = "linux")]
use crate::config::env_optional;
use crate::config::{
    Config, current_session_type, ensure_installed_env_file, installed_config_dir,
    installed_env_path, split_command_line,
};
#[cfg(target_os = "linux")]
use crate::state::State;

pub fn cmd_install_service(config: &Config, enable: bool) -> Result<()> {
    ensure_systemctl()?;
    ensure_installed_env_file()?;
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

pub fn cmd_uninstall_service() -> Result<()> {
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

pub fn cmd_install_gnome_shortcut(binding: &str) -> Result<()> {
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
    run_command("gsettings", &["set", &base, "binding", binding])?;
    println!("已安装 GNOME 快捷键: {} -> {}", binding, command);
    Ok(())
}

pub fn cmd_install_tray_autostart() -> Result<()> {
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

pub fn cmd_install_tray_service(config: &Config, enable: bool) -> Result<()> {
    ensure_systemctl()?;
    ensure_installed_env_file()?;
    let service_dir = PathBuf::from(std::env::var("HOME")?).join(".config/systemd/user");
    fs::create_dir_all(&service_dir)?;
    let service_path = service_dir.join("opentyless-tray.service");
    fs::write(&service_path, render_tray_service(config))?;
    run_command("systemctl", &["--user", "daemon-reload"])?;
    if enable {
        run_command(
            "systemctl",
            &["--user", "enable", "--now", "opentyless-tray.service"],
        )?;
        println!("已安装并启用: {}", service_path.display());
    } else {
        println!("已安装服务文件: {}", service_path.display());
    }
    Ok(())
}

pub fn cmd_uninstall_tray_service() -> Result<()> {
    ensure_systemctl()?;
    let service_path =
        PathBuf::from(std::env::var("HOME")?).join(".config/systemd/user/opentyless-tray.service");
    run_command_allow_fail(
        "systemctl",
        &["--user", "disable", "--now", "opentyless-tray.service"],
    )?;
    if service_path.exists() {
        fs::remove_file(&service_path)?;
    }
    run_command("systemctl", &["--user", "daemon-reload"])?;
    println!("已卸载托盘服务文件");
    Ok(())
}

pub fn cmd_service_status(unit: &str) -> Result<()> {
    ensure_systemctl()?;
    let output = Command::new("systemctl")
        .args(["--user", "status", unit])
        .output()?;
    print_output(output);
    Ok(())
}

pub fn cmd_systemctl(args: &[&str]) -> Result<()> {
    ensure_systemctl()?;
    run_command("systemctl", &["--user", args[0], args[1]])?;
    println!("已执行: systemctl --user {} {}", args[0], args[1]);
    Ok(())
}

pub fn cmd_logs(unit: &str, lines: usize) -> Result<()> {
    let output = Command::new("journalctl")
        .args(["--user", "-u", unit, "-n", &lines.to_string(), "--no-pager"])
        .output()?;
    print_output(output);
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn cmd_tray(config: Config) -> Result<()> {
    let handle = OpenTylessTray::new(config).spawn()?;
    loop {
        thread::sleep(Duration::from_secs(1));
        let _ = handle.update(|tray| tray.refresh());
    }
}

#[cfg(not(target_os = "linux"))]
pub fn cmd_tray(_config: Config) -> Result<()> {
    bail!("此平台请使用 macOS 应用状态栏")
}

pub fn render_service(_config: &Config) -> String {
    let exe = preferred_binary_path();
    let workdir = installed_config_dir();
    let env_file = installed_env_path();
    format!(
        "[Unit]\n\
         Description=OpenTyless Rust daemon\n\
         After=default.target\n\
         StartLimitIntervalSec=30\n\
         StartLimitBurst=5\n\
         \n\
         [Service]\n\
         Type=simple\n\
         WorkingDirectory={}\n\
         EnvironmentFile=-{}\n\
         ExecStart={} daemon\n\
         Restart=always\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        workdir.display(),
        env_file.display(),
        exe.display(),
    )
}

pub fn render_tray_service(_config: &Config) -> String {
    let exe = preferred_binary_path();
    let workdir = installed_config_dir();
    let env_file = installed_env_path();
    format!(
        "[Unit]\n\
         Description=OpenTyless system tray indicator\n\
         After=graphical-session.target\n\
         PartOf=graphical-session.target\n\
         Wants=opentyless.service\n\
         After=opentyless.service\n\
         \n\
         [Service]\n\
         Type=simple\n\
         WorkingDirectory={}\n\
         EnvironmentFile=-{}\n\
         ExecStart={} tray\n\
         Restart=always\n\
         RestartSec=3\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n",
        workdir.display(),
        env_file.display(),
        exe.display(),
    )
}

pub fn preferred_binary_path() -> PathBuf {
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
        if Command::new("gsettings")
            .args(["get", &schema, "name"])
            .output()
            .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains(name))
        {
            return path.clone();
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

pub fn maybe_copy_to_clipboard(config: &Config, text: &str) -> Result<()> {
    if !config.auto_copy {
        return Ok(());
    }
    copy_to_clipboard(config, text)
}

pub fn copy_to_clipboard(config: &Config, text: &str) -> Result<()> {
    let command = detect_clipboard_command(config).ok_or_else(|| anyhow!("未检测到剪贴板命令"))?;
    #[cfg(target_os = "macos")]
    if Path::new(&command[0])
        .file_name()
        .is_some_and(|name| name == "pbcopy")
    {
        return crate::macos::copy_with_pbcopy(&command[0], &command[1..], text);
    }
    // xclip and wl-copy both keep the clipboard buffer alive; never block the
    // transcription pipeline or tray action on them.
    spawn_command_with_stdin_detached(&command[0], &command[1..], text)
}

pub fn notify_event(config: &Config, summary: &str, body: &str) -> Result<()> {
    if !config.enable_notifications || !command_exists("notify-send") {
        return Ok(());
    }
    run_command("notify-send", &[summary, body])
}

pub fn detect_clipboard_command(config: &Config) -> Option<Vec<String>> {
    if let Some(custom) = &config.clipboard_command
        && let Ok(parts) = split_command_line(custom)
        && !parts.is_empty()
    {
        return Some(parts);
    }
    #[cfg(target_os = "macos")]
    if command_exists("pbcopy") {
        return Some(vec!["pbcopy".into()]);
    }
    clipboard_command_candidates(current_session_type().as_deref())
        .into_iter()
        .find(|candidate| command_exists(&candidate[0]))
}

pub fn clipboard_command_candidates(session_type: Option<&str>) -> Vec<Vec<String>> {
    let wl_copy = vec!["wl-copy".into()];
    let xclip = vec!["xclip".into(), "-selection".into(), "clipboard".into()];
    match session_type {
        Some("x11") => vec![xclip, wl_copy],
        Some("wayland") => vec![wl_copy, xclip],
        _ => vec![wl_copy, xclip],
    }
}

pub fn detect_clipboard_preview(config: &Config) -> Option<String> {
    detect_clipboard_command(config).map(|parts| parts.join(" "))
}

fn spawn_command_with_stdin_detached(program: &str, args: &[String], input: &str) -> Result<()> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_desktop_env(&mut command);
    let mut child = command.spawn()?;
    let write_result = if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes())
    } else {
        Ok(())
    };
    thread::spawn(move || {
        let _ = child.wait();
    });
    write_result.context(format!("写入 {program} 标准输入失败"))?;
    Ok(())
}

#[cfg(test)]
pub fn should_detach_stdin_command(program: &str) -> bool {
    matches!(
        Path::new(program)
            .file_name()
            .and_then(|name| name.to_str()),
        Some("xclip") | Some("wl-copy")
    )
}

pub fn run_command(program: &str, args: &[&str]) -> Result<()> {
    let mut command = Command::new(program);
    command.args(args);
    apply_desktop_env(&mut command);
    let output = command.output()?;
    if !output.status.success() {
        bail!(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

pub fn run_command_allow_fail(program: &str, args: &[&str]) -> Result<()> {
    let mut command = Command::new(program);
    command.args(args);
    apply_desktop_env(&mut command);
    let _ = command.output()?;
    Ok(())
}

fn apply_desktop_env(command: &mut Command) {
    #[cfg(target_os = "linux")]
    for (key, value) in desktop_env_overrides() {
        command.env(key, value);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = command;
}

#[cfg(target_os = "linux")]
fn desktop_env_overrides() -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let manager_env = systemd_user_environment();
    let session_type = current_session_type()
        .or_else(|| manager_env_value(manager_env, "XDG_SESSION_TYPE"))
        .unwrap_or_default();
    let dbus_bus = manager_env_value(manager_env, "DBUS_SESSION_BUS_ADDRESS")
        .or_else(|| env_optional("DBUS_SESSION_BUS_ADDRESS"));
    let runtime_dir = manager_env_value(manager_env, "XDG_RUNTIME_DIR")
        .or_else(|| env_optional("XDG_RUNTIME_DIR"))
        .or_else(|| {
            dbus_bus
                .as_deref()
                .and_then(derive_runtime_dir_from_dbus_bus)
        });
    let xauthority =
        manager_env_value(manager_env, "XAUTHORITY").or_else(|| env_optional("XAUTHORITY"));

    if let Some(value) = dbus_bus {
        pairs.push(("DBUS_SESSION_BUS_ADDRESS".into(), value));
    }

    if let Some(value) = runtime_dir.clone() {
        pairs.push(("XDG_RUNTIME_DIR".into(), value));
    }

    if let Some(value) = manager_env_value(manager_env, "WAYLAND_DISPLAY")
        .or_else(|| env_optional("WAYLAND_DISPLAY"))
        .or_else(|| infer_wayland_display(runtime_dir.as_deref()))
    {
        pairs.push(("WAYLAND_DISPLAY".into(), value));
    }

    if let Some(value) = manager_env_value(manager_env, "DISPLAY")
        .or_else(|| env_optional("DISPLAY"))
        .or_else(infer_x11_display)
    {
        pairs.push(("DISPLAY".into(), value));
    }

    if let Some(value) = xauthority {
        pairs.push(("XAUTHORITY".into(), value));
    }

    if !session_type.is_empty() {
        pairs.push(("XDG_SESSION_TYPE".into(), session_type));
    }

    pairs
}

#[cfg(target_os = "linux")]
fn systemd_user_environment() -> &'static [(String, String)] {
    static ENV_CACHE: OnceLock<Vec<(String, String)>> = OnceLock::new();
    ENV_CACHE
        .get_or_init(|| {
            let output = Command::new("systemctl")
                .args(["--user", "show-environment"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output();
            match output {
                Ok(output) if output.status.success() => {
                    parse_environment_lines(&String::from_utf8_lossy(&output.stdout))
                }
                _ => Vec::new(),
            }
        })
        .as_slice()
}

#[cfg(any(target_os = "linux", test))]
pub fn parse_environment_lines(raw: &str) -> Vec<(String, String)> {
    raw.lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() || value.is_empty() {
                None
            } else {
                Some((key.to_string(), value.to_string()))
            }
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn manager_env_value(env: &[(String, String)], key: &str) -> Option<String> {
    env.iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value.clone())
}

#[cfg(any(target_os = "linux", test))]
pub fn derive_runtime_dir_from_dbus_bus(bus: &str) -> Option<String> {
    let path = bus.strip_prefix("unix:path=")?;
    let bus_path = Path::new(path);
    bus_path.parent().map(|parent| parent.display().to_string())
}

#[cfg(target_os = "linux")]
fn infer_wayland_display(runtime_dir: Option<&str>) -> Option<String> {
    let runtime_dir = runtime_dir?;
    for candidate in ["wayland-0", "wayland-1"] {
        if Path::new(runtime_dir).join(candidate).exists() {
            return Some(candidate.to_string());
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn infer_x11_display() -> Option<String> {
    for candidate in [(":0", "/tmp/.X11-unix/X0"), (":1", "/tmp/.X11-unix/X1")] {
        if Path::new(candidate.1).exists() {
            return Some(candidate.0.to_string());
        }
    }
    None
}

pub fn print_output(output: std::process::Output) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stdout.trim().is_empty() {
        println!("{}", stdout.trim_end());
    }
    if !stderr.trim().is_empty() {
        eprintln!("{}", stderr.trim_end());
    }
}

pub fn ensure_systemctl() -> Result<()> {
    ensure_command("systemctl")
}

pub fn ensure_command(command: &str) -> Result<()> {
    if command_exists(command) {
        Ok(())
    } else {
        bail!("未找到 {}", command)
    }
}

pub fn command_exists(command: &str) -> bool {
    if command.is_empty() {
        return false;
    }
    let candidate = Path::new(command);
    if command.contains('/') {
        return is_executable_file(candidate);
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| is_executable_file(&dir.join(candidate)))
        })
        .unwrap_or(false)
}

fn is_executable_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|meta| meta.is_file() && (meta.permissions().mode() & 0o111 != 0))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct OpenTylessTray {
    config: Config,
    recording: bool,
    transcribing: bool,
    tooltip: String,
}

#[cfg(target_os = "linux")]
impl OpenTylessTray {
    fn new(config: Config) -> Self {
        let state = State::load(&config.state_path());
        Self {
            config,
            recording: state.recording,
            transcribing: state.last_event.as_deref() == Some("transcribing"),
            tooltip: build_tray_tooltip(&state),
        }
    }

    fn refresh(&mut self) {
        let state = State::load(&self.config.state_path());
        self.recording = state.recording;
        self.transcribing = state.last_event.as_deref() == Some("transcribing");
        self.tooltip = build_tray_tooltip(&state);
    }

    fn spawn_action(&self, action: TrayAction) {
        let config = self.config.clone();
        thread::spawn(move || {
            let result: Result<()> = (|| match action {
                TrayAction::Start => crate::cmd_send(&config, "start-record"),
                TrayAction::Stop => crate::cmd_send(&config, "stop-record"),
                TrayAction::Cancel => crate::cmd_send(&config, "cancel-record"),
                TrayAction::CopyLast => crate::cmd_copy_last(&config),
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

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug)]
enum TrayAction {
    Start,
    Stop,
    Cancel,
    CopyLast,
    OpenOutputDir,
    ShowStatus,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for OpenTylessTray {
    fn id(&self) -> String {
        "opentyless-tray".into()
    }

    fn title(&self) -> String {
        if self.recording {
            "OpenTyless ● 录音中".into()
        } else if self.transcribing {
            "OpenTyless … 转写中".into()
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
        let mut items: Vec<ksni::MenuItem<Self>> = Vec::new();
        if self.recording {
            items.push(
                StandardItem {
                    label: "停止录音".into(),
                    activate: Box::new(|tray: &mut OpenTylessTray| {
                        tray.spawn_action(TrayAction::Stop)
                    }),
                    ..Default::default()
                }
                .into(),
            );
            items.push(
                StandardItem {
                    label: "取消录音".into(),
                    activate: Box::new(|tray: &mut OpenTylessTray| {
                        tray.spawn_action(TrayAction::Cancel)
                    }),
                    ..Default::default()
                }
                .into(),
            );
        } else {
            items.push(
                StandardItem {
                    label: if self.transcribing {
                        "正在转写…".into()
                    } else {
                        "开始录音".into()
                    },
                    enabled: !self.transcribing,
                    activate: Box::new(|tray: &mut OpenTylessTray| {
                        tray.spawn_action(TrayAction::Start)
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }

        items.push(
            StandardItem {
                label: "复制最近结果".into(),
                activate: Box::new(|tray: &mut OpenTylessTray| {
                    tray.spawn_action(TrayAction::CopyLast)
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "打开输出目录".into(),
                activate: Box::new(|tray: &mut OpenTylessTray| {
                    tray.spawn_action(TrayAction::OpenOutputDir)
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "显示状态通知".into(),
                activate: Box::new(|tray: &mut OpenTylessTray| {
                    tray.spawn_action(TrayAction::ShowStatus)
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "退出托盘".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

#[cfg(target_os = "linux")]
fn build_tray_tooltip(state: &State) -> String {
    if state.recording {
        return "正在录音；可停止并转写，或直接取消本次录音。".into();
    }
    if state.last_event.as_deref() == Some("transcribing") {
        return "正在转写与整理文本…".into();
    }
    if let Some(text) = &state.last_polished_text {
        return format!("最近结果：{}", truncate_text(text, 60));
    }
    "待命中；可从快捷键或托盘开始录音。".into()
}

#[cfg(target_os = "linux")]
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
