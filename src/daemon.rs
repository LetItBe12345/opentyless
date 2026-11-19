use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::desktop::{maybe_copy_to_clipboard, notify_event};
use crate::pipeline::Pipeline;
use crate::recorder::{Recorder, cleanup_empty_recordings, save_recording};
use crate::state::State;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn request_shutdown(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

pub fn install_shutdown_handlers() -> Result<()> {
    SHUTDOWN.store(false, Ordering::SeqCst);
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = request_shutdown as *const () as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        // Leave SA_RESTART unset so SIGINT/SIGTERM immediately interrupts poll().
        action.sa_flags = 0;
        for signal in [libc::SIGINT, libc::SIGTERM] {
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }
    Ok(())
}

pub fn shutdown_requested() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub action: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    pub message: String,
    pub state: Option<State>,
}

pub struct Daemon {
    pub config: Config,
    pub state: Arc<Mutex<State>>,
    transcribing: Arc<AtomicBool>,
    recorder: Option<Recorder>,
}

impl Daemon {
    pub fn new(config: Config) -> Result<Self> {
        fs::create_dir_all(&config.run_dir)?;
        fs::create_dir_all(&config.state_dir)?;
        let previous = State::load(&config.state_path());
        let state = State {
            pid: Some(std::process::id()),
            mode: Some("daemon".into()),
            started_at: Some(Local::now().to_rfc3339()),
            last_event: Some("daemon_started".into()),
            last_raw_text: previous.last_raw_text,
            last_polished_text: previous.last_polished_text,
            last_output_path: previous.last_output_path,
            ..State::default()
        };
        state.save(&config.state_path())?;
        fs::write(config.pid_path(), std::process::id().to_string())?;
        Ok(Self {
            config,
            state: Arc::new(Mutex::new(state)),
            transcribing: Arc::new(AtomicBool::new(false)),
            recorder: None,
        })
    }

    #[cfg(test)]
    pub fn snapshot_state(&self) -> State {
        self.state.lock().unwrap().clone()
    }

    fn mutate_state<F>(&self, update: F) -> Result<State>
    where
        F: FnOnce(&mut State),
    {
        let mut guard = self.state.lock().unwrap();
        update(&mut guard);
        guard.save(&self.config.state_path())?;
        Ok(guard.clone())
    }

    fn record_error(&self, event: &str, message: &str) {
        let mut guard = self.state.lock().unwrap();
        guard.error = Some(message.to_string());
        guard.last_event = Some(event.to_string());
        guard.save(&self.config.state_path()).ok();
    }

    pub fn run(mut self) -> Result<()> {
        if self.config.socket_path.exists() {
            fs::remove_file(&self.config.socket_path).ok();
        }
        let listener = UnixListener::bind(&self.config.socket_path)
            .with_context(|| format!("创建 socket 失败: {}", self.config.socket_path.display()))?;
        fs::set_permissions(&self.config.socket_path, fs::Permissions::from_mode(0o600)).ok();
        listener.set_nonblocking(true)?;
        install_shutdown_handlers()?;
        println!(
            "daemon 已启动，socket: {}",
            self.config.socket_path.display()
        );

        while !shutdown_requested() {
            let mut pfd = libc::pollfd {
                fd: listener.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, 2_000) };
            if shutdown_requested() {
                break;
            }
            if ready < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                self.record_error("socket_error", &err.to_string());
                continue;
            }
            if ready == 0 {
                continue;
            }

            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        if let Err(error) = self.handle_stream(stream) {
                            self.record_error("request_failed", &error.to_string());
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => break,
                    Err(error) => {
                        self.record_error("socket_error", &error.to_string());
                        break;
                    }
                }
            }
        }

        if let Some(recorder) = self.recorder.take() {
            recorder.discard();
        }
        self.mutate_state(|state| {
            state.recording = false;
            state.recorder_pid = None;
            state.audio_path = None;
            state.pid = None;
            state.last_event = Some("daemon_stopped".into());
        })?;
        fs::remove_file(self.config.pid_path()).ok();
        fs::remove_file(&self.config.socket_path).ok();
        Ok(())
    }

    fn handle_stream(&mut self, mut stream: UnixStream) -> Result<()> {
        let timeout = Some(Duration::from_secs(5));
        stream.set_read_timeout(timeout)?;
        stream.set_write_timeout(timeout)?;
        let mut input = String::new();
        BufReader::new(stream.try_clone()?).read_line(&mut input)?;
        if input.trim().is_empty() {
            return Ok(());
        }
        let request: Request = serde_json::from_str(input.trim())?;
        let response = self.handle_action(&request.action);
        stream.write_all(format!("{}\n", serde_json::to_string(&response)?).as_bytes())?;
        Ok(())
    }

    fn handle_action(&mut self, action: &str) -> Response {
        let result = match action {
            "start-record" => self.start_record(),
            "save-record" => self.stop_record(true),
            "stop-record" => self.stop_record(false),
            "cancel-record" => self.cancel_record(),
            "toggle-record" => {
                if self.recorder.is_some() {
                    self.stop_record(false)
                } else {
                    self.start_record()
                }
            }
            "status" => Ok("状态已返回".into()),
            _ => Err(anyhow!("未知 action: {action}")),
        };

        match result {
            Ok(message) => {
                let snapshot = self.state.lock().unwrap().clone();
                Response {
                    ok: true,
                    message,
                    state: Some(snapshot),
                }
            }
            Err(error) => {
                let msg = error.to_string();
                self.record_error("command_failed", &msg);
                let snapshot = self.state.lock().unwrap().clone();
                Response {
                    ok: false,
                    message: msg,
                    state: Some(snapshot),
                }
            }
        }
    }

    fn start_record(&mut self) -> Result<String> {
        if self.recorder.is_some() {
            bail!("当前已经在录音中");
        }
        if self.transcribing.load(Ordering::Acquire) {
            notify_event(&self.config, "OpenTyless", "正在转写").ok();
            return Ok("正在转写".into());
        }
        let recorder = Recorder::start(&self.config)?;
        let recorder_pid = recorder.child.id();
        let audio_display = recorder.audio_path.display().to_string();
        self.mutate_state(|state| {
            state.recording = true;
            state.recorder_pid = Some(recorder_pid);
            state.audio_path = Some(audio_display);
            state.last_event = Some("recording_started".into());
            state.error = None;
        })?;
        notify_event(&self.config, "OpenTyless", "开始录音").ok();
        self.recorder = Some(recorder);
        Ok("已开始录音".into())
    }

    fn stop_record(&mut self, save_only: bool) -> Result<String> {
        let recorder = self
            .recorder
            .take()
            .ok_or_else(|| anyhow!("当前没有录音任务"))?;
        {
            let mut guard = self.state.lock().unwrap();
            guard.recording = false;
            guard.recorder_pid = None;
        }
        let audio_path = recorder.stop()?;
        if save_only {
            let path = save_recording(&audio_path, &self.config.output_dir)
                .with_context(|| format!("保存失败，原始录音位于 {}", audio_path.display()))?;
            let saved_display = path.display().to_string();
            self.mutate_state(|state| {
                state.audio_path = Some(saved_display);
                state.error = None;
                state.last_event = Some("recording_saved".into());
            })?;
            notify_event(&self.config, "OpenTyless", "录音已保存到输出目录").ok();
            return Ok(format!("录音已保存: {}", path.display()));
        }

        let audio_display = audio_path.display().to_string();
        self.transcribing.store(true, Ordering::Release);
        self.mutate_state(|state| {
            state.last_event = Some("transcribing".into());
            state.audio_path = Some(audio_display);
            state.error = None;
        })?;
        notify_event(&self.config, "OpenTyless", "正在转写").ok();
        spawn_pipeline_job(
            self.config.clone(),
            Arc::clone(&self.state),
            Arc::clone(&self.transcribing),
            audio_path.clone(),
        );

        Ok(format!("已停止录音，正在转写: {}", audio_path.display()))
    }

    fn cancel_record(&mut self) -> Result<String> {
        let recorder = self
            .recorder
            .take()
            .ok_or_else(|| anyhow!("当前没有录音任务"))?;
        let audio_path = recorder.stop()?;
        fs::remove_file(&audio_path).ok();
        self.mutate_state(|state| {
            state.recording = false;
            state.recorder_pid = None;
            state.audio_path = None;
            state.last_event = Some("recording_cancelled".into());
            state.error = None;
        })?;
        notify_event(&self.config, "OpenTyless", "已取消录音").ok();
        Ok("已取消录音".into())
    }
}

struct TranscribingResetGuard(Arc<AtomicBool>);

impl Drop for TranscribingResetGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

fn spawn_pipeline_job(
    config: Config,
    shared_state: Arc<Mutex<State>>,
    transcribing: Arc<AtomicBool>,
    audio_path: PathBuf,
) {
    let state_path = config.state_path();
    thread::spawn(move || {
        let _reset_guard = TranscribingResetGuard(transcribing);
        println!("{} pipeline started", Local::now().to_rfc3339());
        let result: Result<(String, String, PathBuf)> = (|| {
            config.validate_inference()?;
            let pipeline = Pipeline::new(config.clone())?;
            pipeline.run(&audio_path)
        })();

        // Remove the temporary recording from run_dir once processed.
        fs::remove_file(&audio_path).ok();

        match result {
            Ok((raw_text, polished_text, output_path)) => {
                let copied = maybe_copy_to_clipboard(&config, &polished_text);
                let clipboard_ok = config.auto_copy && copied.is_ok();
                let copy_error = copied
                    .as_ref()
                    .err()
                    .map(|error| format!("转写成功，但复制到剪贴板失败：{error}"));

                {
                    let mut state = shared_state.lock().unwrap();
                    state.last_event = Some("pipeline_completed".into());
                    state.audio_path = None;
                    state.last_raw_text = Some(raw_text);
                    state.last_polished_text = Some(polished_text.clone());
                    state.last_output_path = Some(output_path.display().to_string());
                    state.clipboard_copied = Some(clipboard_ok);
                    state.error = copy_error.clone();
                    state.save(&state_path).ok();
                }

                println!(
                    "{} pipeline completed; text_chars={}; clipboard_copied={}; output={}",
                    Local::now().to_rfc3339(),
                    polished_text.chars().count(),
                    clipboard_ok,
                    output_path.display()
                );

                match copy_error {
                    None => {
                        notify_event(&config, "OpenTyless", "转写成功").ok();
                    }
                    Some(message) => {
                        eprintln!("clipboard copy failed: {message}");
                        notify_event(&config, "OpenTyless", &message).ok();
                    }
                }
            }
            Err(error) => {
                let message = config.redact_secrets(&error.to_string());
                eprintln!("{} pipeline failed: {message}", Local::now().to_rfc3339());
                {
                    let mut state = shared_state.lock().unwrap();
                    state.last_event = Some("pipeline_failed".into());
                    state.audio_path = None;
                    state.error = Some(message.clone());
                    state.save(&state_path).ok();
                }
                notify_event(&config, "OpenTyless 错误", &message).ok();
            }
        }
    });
}

pub fn send_request(socket_path: &Path, request: &Request) -> Result<Response> {
    let mut stream = UnixStream::connect(socket_path).with_context(|| {
        format!(
            "无法连接 daemon，请先运行 `opentyless-rs daemon` 或 `install-service --enable`。socket={}",
            socket_path.display()
        )
    })?;
    let timeout = Some(Duration::from_secs(35));
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    stream.write_all(format!("{}\n", serde_json::to_string(request)?).as_bytes())?;
    let mut response = String::new();
    BufReader::new(stream).read_to_string(&mut response)?;
    Ok(serde_json::from_str(response.trim())?)
}

#[cfg(test)]
pub fn process_running(pid_path: &Path) -> bool {
    process_running_with_name(pid_path, None)
}

pub fn process_running_with_name(pid_path: &Path, expected_name: Option<&str>) -> bool {
    fs::read_to_string(pid_path)
        .ok()
        .and_then(|content| content.trim().parse::<u32>().ok())
        .map(|pid| pid_matches_expected_process(pid, expected_name))
        .unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
pub fn pid_matches_expected_process(pid: u32, expected_name: Option<&str>) -> bool {
    let proc_dir = PathBuf::from(format!("/proc/{pid}"));
    if !proc_dir.exists() {
        return false;
    }

    let Some(expected_name) = expected_name else {
        return true;
    };

    let exe_matches = fs::read_link(proc_dir.join("exe"))
        .ok()
        .and_then(|path| path.file_name().map(|name| name.to_os_string()))
        .map(|name| name == expected_name)
        .unwrap_or(false);

    let cmdline_matches = fs::read(proc_dir.join("cmdline"))
        .ok()
        .map(|bytes| {
            bytes
                .split(|byte| *byte == 0)
                .filter_map(|part| std::str::from_utf8(part).ok())
                .any(|part| part.contains(expected_name))
        })
        .unwrap_or(false);

    exe_matches || cmdline_matches
}

#[cfg(target_os = "macos")]
pub fn pid_matches_expected_process(pid: u32, expected_name: Option<&str>) -> bool {
    if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 {
        return false;
    }
    let Some(expected) = expected_name else {
        return true;
    };
    Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            Path::new(String::from_utf8_lossy(&o.stdout).trim())
                .file_name()
                .is_some_and(|n| n == expected)
        })
        .unwrap_or(false)
}

pub fn daemon_running(config: &Config) -> bool {
    let expected_name = env!("CARGO_PKG_NAME");
    if process_running_with_name(&config.pid_path(), Some(expected_name)) {
        return true;
    }

    socket_accepting_connections(&config.socket_path)
}

pub fn ensure_daemon_runtime_ready(config: &Config) -> Result<()> {
    let expected_name = env!("CARGO_PKG_NAME");
    if process_running_with_name(&config.pid_path(), Some(expected_name)) {
        bail!("daemon 已在运行中");
    }

    if config.pid_path().exists() {
        fs::remove_file(config.pid_path()).ok();
    }

    if config.socket_path.exists() {
        if socket_accepting_connections(&config.socket_path) {
            bail!("daemon 已在运行中");
        }
        fs::remove_file(&config.socket_path)
            .with_context(|| format!("清理残留 socket 失败: {}", config.socket_path.display()))?;
    }

    cleanup_empty_recordings(&config.run_dir);
    Ok(())
}

pub fn socket_accepting_connections(socket_path: &Path) -> bool {
    UnixStream::connect(socket_path).is_ok()
}
