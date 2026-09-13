use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::{TempDir, tempdir};

struct Harness {
    _temp: TempDir,
    run_dir: PathBuf,
    state_dir: PathBuf,
    output_dir: PathBuf,
    socket_path: PathBuf,
    recorder: PathBuf,
    fixture: PathBuf,
    exe: PathBuf,
}

#[derive(Clone, Copy)]
enum MockBehavior {
    Success,
    ServerError,
    SlowSuccess,
}

fn debug_exe() -> PathBuf {
    option_env!("CARGO_BIN_EXE_opentyless_rs")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/opentyless-rs")
        })
}

fn handle_connection(mut stream: TcpStream, behavior: MockBehavior) {
    match behavior {
        MockBehavior::SlowSuccess => thread::sleep(Duration::from_millis(800)),
        MockBehavior::ServerError => {}
        MockBehavior::Success => {}
    }

    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
        headers.push_str(&line);
    }
    let content_length = headers
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("content-length:") {
                line.split(':')
                    .nth(1)
                    .and_then(|v| v.trim().parse::<usize>().ok())
            } else {
                None
            }
        })
        .unwrap_or(0);
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).unwrap();
    }
    let body_str = String::from_utf8_lossy(&body);

    if matches!(behavior, MockBehavior::ServerError) {
        let response_body = r#"{"error":{"message":"mock asr failed"}}"#;
        let response = format!(
            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).unwrap();
        return;
    }

    let request_body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let response_body = if body_str.contains("qwen3-asr-flash") {
        r#"{"choices":[{"message":{"content":[{"type":"text","text":"测试语音原文"}]}}]}"#
    } else {
        assert_eq!(request_body["response_format"]["type"], "json_object");
        assert_eq!(request_body["temperature"], 0.1);
        assert_eq!(request_body["seed"], 7);
        let user_content = request_body["messages"][1]["content"].as_str().unwrap();
        assert!(user_content.contains("<raw_transcript>"));
        assert!(user_content.contains("测试语音原文"));
        r#"{"choices":[{"message":{"content":"{\"text\":\"测试语音整理后\"}"}}]}"#
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    stream.write_all(response.as_bytes()).unwrap();
}

fn spawn_mock_server(behavior: MockBehavior, connections: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server = thread::spawn(move || {
        for _ in 0..connections {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream, behavior);
        }
    });
    (addr, server)
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("path not ready: {}", path.display());
}

fn wait_for_state_audio(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(state) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(audio_path) = state.get("audio_path").and_then(|v| v.as_str()) {
                    if Path::new(audio_path).exists() {
                        return;
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("audio path not ready: {}", path.display());
}

fn wait_for_event(path: &Path, event: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = fs::read_to_string(path).unwrap_or_default();
        if status.contains(&format!("\"last_event\": \"{event}\""))
            || status.contains(&format!("\"last_event\":\"{event}\""))
        {
            return status;
        }
        if Instant::now() >= deadline {
            panic!("event {event} not reached. last status: {status}");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

impl Harness {
    fn new() -> Self {
        let temp = tempdir().unwrap();
        let run_dir = temp.path().join("run");
        let state_dir = temp.path().join("state");
        let output_dir = temp.path().join("outputs");
        fs::create_dir_all(&run_dir).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        let fixture = run_dir.join("fixture.wav");
        fs::write(&fixture, b"RIFF....WAVE").unwrap();
        let recorder = run_dir.join("fake-recorder.sh");
        let socket_path = run_dir.join("opentyless.sock");
        Self {
            _temp: temp,
            run_dir,
            state_dir,
            output_dir,
            socket_path,
            recorder,
            fixture,
            exe: debug_exe(),
        }
    }

    fn write_recorder(&self, script: &str) {
        fs::write(&self.recorder, script).unwrap();
        Command::new("chmod")
            .args(["+x", self.recorder.to_str().unwrap()])
            .status()
            .unwrap();
    }

    fn looping_recorder(&self) {
        self.write_recorder(&format!(
            "#!/usr/bin/env bash\ntrap 'exit 0' INT\ncp '{}' \"$1\"\nwhile true; do sleep 0.1; done\n",
            self.fixture.display()
        ));
    }

    fn empty_recorder(&self) {
        self.write_recorder("#!/usr/bin/env bash\ntrap 'exit 0' INT\nwhile true; do sleep 0.1; done\n");
    }

    fn sigint_recorder(&self, flag_path: &Path) {
        self.write_recorder(&format!(
            "#!/usr/bin/env bash\nout=\"$1\"\ntrap 'cp \"{}\" \"$out\"; echo interrupted > \"{}\"; exit 0' INT\nwhile true; do sleep 0.1; done\n",
            self.fixture.display(),
            flag_path.display()
        ));
    }

    fn once_recorder(&self) {
        self.write_recorder(&format!(
            "#!/usr/bin/env bash\ncp '{}' \"$1\"\nexec sleep 30\n",
            self.fixture.display()
        ));
    }

    fn base_command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.exe);
        command
            .args(args)
            .env("RUN_DIR", &self.run_dir)
            .env("STATE_DIR", &self.state_dir)
            .env("OUTPUT_DIR", &self.output_dir)
            .env("SOCKET_PATH", &self.socket_path)
            .env("RECORDER_CMD", self.recorder.to_str().unwrap())
            .env("AUTO_COPY_TO_CLIPBOARD", "false")
            .env("ENABLE_NOTIFICATIONS", "false");
        command
    }

    fn spawn_daemon(&self, api: &str) -> Child {
        self.base_command(&["daemon"])
            .env("ASR_API_URL", format!("http://{api}/chat/completions"))
            .env("FLASH_API_URL", format!("http://{api}/chat/completions"))
            .env("ASR_API_KEY", "test")
            .env("FLASH_API_KEY", "test")
            .stdout(Stdio::null())
            .stderr(Stdio::from(fs::File::create(self.run_dir.join("daemon.err")).unwrap()))
            .spawn()
            .unwrap()
    }

    fn run(&self, args: &[&str]) -> Output {
        self.base_command(args).output().unwrap()
    }

    fn run_with_keys(&self, args: &[&str]) -> Output {
        self.base_command(args)
            .env("ASR_API_KEY", "test")
            .env("FLASH_API_KEY", "test")
            .output()
            .unwrap()
    }

    fn status_path(&self) -> PathBuf {
        self.state_dir.join("status.json")
    }

    fn send_socket(&self, action: &str) -> String {
        let mut stream = UnixStream::connect(&self.socket_path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(format!("{{\"action\":\"{action}\"}}\n").as_bytes())
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn kill_daemon(mut daemon: Child) {
    daemon.kill().ok();
    daemon.wait().ok();
}

#[test]
fn help_lists_core_commands() {
    let output = Command::new(debug_exe()).arg("--help").output().unwrap();
    assert_success(&output);
    let text = stdout_text(&output);
    for command in [
        "daemon",
        "start-record",
        "stop-record",
        "toggle-record",
        "once",
        "doctor",
        "status",
        "copy-last",
    ] {
        assert!(text.contains(command), "missing {command} in help:\n{text}");
    }
}

#[test]
fn doctor_reports_configured_runtime_paths() {
    let harness = Harness::new();
    harness.looping_recorder();
    let output = harness
        .base_command(&["doctor"])
        .env("ASR_API_KEY", "test")
        .env("FLASH_API_KEY", "test")
        .env("CLIPBOARD_COMMAND", "wl-copy")
        .output()
        .unwrap();
    assert_success(&output);
    let text = stdout_text(&output);
    assert!(text.contains(harness.recorder.to_str().unwrap()));
    assert!(text.contains(&harness.run_dir.display().to_string()));
    assert!(text.contains(&harness.socket_path.display().to_string()));
    assert!(text.contains("AUTO_COPY_TO_CLIPBOARD: false"));
    assert!(text.contains("ENABLE_NOTIFICATIONS: false"));
}

#[test]
fn start_record_without_daemon_fails() {
    let harness = Harness::new();
    let output = harness.run(&["start-record"]);
    assert!(!output.status.success());
    let combined = format!("{}{}", stdout_text(&output), stderr_text(&output));
    assert!(combined.contains("无法连接 daemon"), "{combined}");
}

#[test]
fn daemon_cli_flow_works_with_mock_server() {
    let harness = Harness::new();
    harness.looping_recorder();
    let (addr, server) = spawn_mock_server(MockBehavior::Success, 2);
    let daemon = harness.spawn_daemon(&addr);
    wait_for(&harness.socket_path);

    let start = harness.run(&["start-record"]);
    assert_success(&start);
    wait_for_state_audio(&harness.status_path());

    let stop = harness.run_with_keys(&["stop-record"]);
    assert_success(&stop);
    assert!(stdout_text(&stop).contains("已停止录音，正在转写"));
    let immediate_status = fs::read_to_string(harness.status_path()).unwrap();
    assert!(immediate_status.contains("\"last_event\": \"transcribing\""));

    let status = wait_for_event(&harness.status_path(), "pipeline_completed");
    assert!(status.contains("测试语音原文"));
    assert!(status.contains("测试语音整理后"));

    let outputs: Vec<_> = fs::read_dir(&harness.output_dir).unwrap().collect();
    assert_eq!(outputs.len(), 1);
    let md = fs::read_to_string(outputs[0].as_ref().unwrap().path()).unwrap();
    assert!(md.contains("测试语音整理后"));

    let cli_status = harness.run(&["status"]);
    assert_success(&cli_status);
    let status_text = stdout_text(&cli_status);
    assert!(status_text.contains("daemon_running: yes"));
    assert!(status_text.contains("pipeline_completed"));

    kill_daemon(daemon);
    server.join().unwrap();
}

#[test]
fn toggle_record_flow_works_with_mock_server() {
    let harness = Harness::new();
    harness.looping_recorder();
    let (addr, server) = spawn_mock_server(MockBehavior::SlowSuccess, 2);
    let daemon = harness.spawn_daemon(&addr);
    wait_for(&harness.socket_path);

    let start = harness.run(&["toggle-record"]);
    assert_success(&start);
    assert!(stdout_text(&start).contains("已开始录音"));
    wait_for_state_audio(&harness.status_path());

    let stop = harness.run_with_keys(&["toggle-record"]);
    assert_success(&stop);
    assert!(stdout_text(&stop).contains("已停止录音，正在转写"));

    let while_transcribing = harness.run(&["toggle-record"]);
    assert_success(&while_transcribing);
    assert!(stdout_text(&while_transcribing).contains("正在转写"));

    let status = wait_for_event(&harness.status_path(), "pipeline_completed");
    assert!(status.contains("测试语音原文"));
    assert!(status.contains("测试语音整理后"));

    kill_daemon(daemon);
    server.join().unwrap();
}

#[test]
fn once_cli_flow_works_with_mock_server() {
    let harness = Harness::new();
    harness.once_recorder();
    let (addr, server) = spawn_mock_server(MockBehavior::Success, 2);
    let output = harness
        .base_command(&["once", "--seconds", "1"])
        .env("ASR_API_URL", format!("http://{addr}/chat/completions"))
        .env("FLASH_API_URL", format!("http://{addr}/chat/completions"))
        .env("ASR_API_KEY", "test")
        .env("FLASH_API_KEY", "test")
        .output()
        .unwrap();
    assert_success(&output);
    let text = stdout_text(&output);
    assert!(text.contains("测试语音原文"), "{text}");
    assert!(text.contains("测试语音整理后"), "{text}");
    assert!(text.contains("已保存:"));
    let outputs: Vec<_> = fs::read_dir(&harness.output_dir).unwrap().collect();
    assert_eq!(outputs.len(), 1);
    server.join().unwrap();
}

#[test]
fn copy_last_uses_custom_clipboard_command() {
    let harness = Harness::new();
    let clip_out = harness.run_dir.join("clipboard.txt");
    let clip_cmd = harness.run_dir.join("fake-clip.sh");
    fs::write(
        &clip_cmd,
        format!("#!/usr/bin/env bash\ncat > '{}'\n", clip_out.display()),
    )
    .unwrap();
    Command::new("chmod")
        .args(["+x", clip_cmd.to_str().unwrap()])
        .status()
        .unwrap();
    fs::create_dir_all(&harness.state_dir).unwrap();
    fs::write(
        harness.status_path(),
        r#"{"recording":false,"last_polished_text":"剪贴板内容"}"#,
    )
    .unwrap();

    let output = harness
        .base_command(&["copy-last"])
        .env("AUTO_COPY_TO_CLIPBOARD", "true")
        .env("CLIPBOARD_COMMAND", clip_cmd.to_str().unwrap())
        .output()
        .unwrap();
    assert_success(&output);
    assert!(stdout_text(&output).contains("已复制最近一次整理文本"));
    wait_for(&clip_out);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let content = fs::read_to_string(&clip_out).unwrap_or_default();
        if content == "剪贴板内容" {
            break;
        }
        if Instant::now() >= deadline {
            panic!("clipboard content mismatch: {content:?}");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn hanging_clipboard_does_not_block_pipeline_completion() {
    let harness = Harness::new();
    harness.looping_recorder();
    let hang = harness.run_dir.join("hang-clip.sh");
    let hang_pid = harness.run_dir.join("hang-clip.pid");
    fs::write(
        &hang,
        format!(
            "#!/usr/bin/env bash\necho $$ > '{}'\ncat >/dev/null\nexec sleep 60\n",
            hang_pid.display()
        ),
    )
    .unwrap();
    Command::new("chmod")
        .args(["+x", hang.to_str().unwrap()])
        .status()
        .unwrap();

    let (addr, server) = spawn_mock_server(MockBehavior::Success, 2);
    let daemon = harness
        .base_command(&["daemon"])
        .env("ASR_API_URL", format!("http://{addr}/chat/completions"))
        .env("FLASH_API_URL", format!("http://{addr}/chat/completions"))
        .env("ASR_API_KEY", "test")
        .env("FLASH_API_KEY", "test")
        .env("AUTO_COPY_TO_CLIPBOARD", "true")
        .env("CLIPBOARD_COMMAND", hang.to_str().unwrap())
        .env("ENABLE_NOTIFICATIONS", "false")
        .stdout(Stdio::null())
        .stderr(Stdio::from(
            fs::File::create(harness.run_dir.join("daemon.err")).unwrap(),
        ))
        .spawn()
        .unwrap();
    wait_for(&harness.socket_path);

    let start = harness.run(&["start-record"]);
    assert_success(&start);
    wait_for_state_audio(&harness.status_path());
    let stop = harness.run_with_keys(&["stop-record"]);
    assert_success(&stop);

    let started = Instant::now();
    let status = wait_for_event(&harness.status_path(), "pipeline_completed");
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "pipeline should finish without waiting for clipboard"
    );
    assert!(status.contains("测试语音整理后"));

    kill_daemon(daemon);
    if let Ok(pid) = fs::read_to_string(&hang_pid) {
        let _ = Command::new("kill").args(["-9", pid.trim()]).status();
    }
    server.join().unwrap();
}

#[test]
fn cancel_record_discards_audio_without_pipeline() {
    let harness = Harness::new();
    harness.looping_recorder();
    let (addr, server) = spawn_mock_server(MockBehavior::Success, 0);
    let daemon = harness.spawn_daemon(&addr);
    wait_for(&harness.socket_path);

    let start = harness.run(&["start-record"]);
    assert_success(&start);
    wait_for_state_audio(&harness.status_path());
    let audio_path = {
        let state: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(harness.status_path()).unwrap()).unwrap();
        PathBuf::from(state["audio_path"].as_str().unwrap())
    };
    assert!(audio_path.exists());

    let response = harness.send_socket("cancel-record");
    assert!(response.contains("已取消录音"), "{response}");
    let status = wait_for_event(&harness.status_path(), "recording_cancelled");
    assert!(!audio_path.exists());
    assert!(!status.contains("pipeline_completed"));
    assert!(!harness.output_dir.exists() || fs::read_dir(&harness.output_dir).unwrap().next().is_none());

    kill_daemon(daemon);
    drop(server);
}

#[test]
fn stop_without_recording_fails() {
    let harness = Harness::new();
    harness.looping_recorder();
    let (addr, server) = spawn_mock_server(MockBehavior::Success, 0);
    let daemon = harness.spawn_daemon(&addr);
    wait_for(&harness.socket_path);

    let stop = harness.run(&["stop-record"]);
    assert!(!stop.status.success());
    let combined = format!("{}{}", stdout_text(&stop), stderr_text(&stop));
    assert!(combined.contains("当前没有录音任务"), "{combined}");

    kill_daemon(daemon);
    drop(server);
}

#[test]
fn empty_recording_fails_stop() {
    let harness = Harness::new();
    harness.empty_recorder();
    let (addr, server) = spawn_mock_server(MockBehavior::Success, 0);
    let daemon = harness.spawn_daemon(&addr);
    wait_for(&harness.socket_path);

    let start = harness.run(&["start-record"]);
    assert_success(&start);
    wait_for_event(&harness.status_path(), "recording_started");

    let stop = harness.run_with_keys(&["stop-record"]);
    assert!(!stop.status.success());
    let combined = format!("{}{}", stdout_text(&stop), stderr_text(&stop));
    assert!(combined.contains("录音文件为空"), "{combined}");

    kill_daemon(daemon);
    drop(server);
}

#[test]
fn recorder_stop_sends_sigint_so_wav_can_be_finalized() {
    let harness = Harness::new();
    let flag = harness.run_dir.join("sigint.flag");
    harness.sigint_recorder(&flag);
    let (addr, server) = spawn_mock_server(MockBehavior::Success, 2);
    let daemon = harness.spawn_daemon(&addr);
    wait_for(&harness.socket_path);

    let start = harness.run(&["start-record"]);
    assert_success(&start);
    wait_for_event(&harness.status_path(), "recording_started");
    thread::sleep(Duration::from_millis(150));

    let stop = harness.run_with_keys(&["stop-record"]);
    assert_success(&stop);
    wait_for(&flag);
    assert_eq!(fs::read_to_string(&flag).unwrap().trim(), "interrupted");
    wait_for_event(&harness.status_path(), "pipeline_completed");

    kill_daemon(daemon);
    server.join().unwrap();
}

#[test]
fn pipeline_failure_sets_error_state() {
    let harness = Harness::new();
    harness.looping_recorder();
    let (addr, server) = spawn_mock_server(MockBehavior::ServerError, 1);
    let daemon = harness.spawn_daemon(&addr);
    wait_for(&harness.socket_path);

    let start = harness.run(&["start-record"]);
    assert_success(&start);
    wait_for_state_audio(&harness.status_path());
    let stop = harness.run_with_keys(&["stop-record"]);
    assert_success(&stop);

    let status = wait_for_event(&harness.status_path(), "pipeline_failed");
    assert!(status.contains("error"), "{status}");

    kill_daemon(daemon);
    server.join().unwrap();
}

#[test]
fn install_tray_autostart_writes_desktop_file() {
    let harness = Harness::new();
    let fake_home = harness.run_dir.join("home");
    fs::create_dir_all(&fake_home).unwrap();
    let output = Command::new(debug_exe())
        .arg("install-tray-autostart")
        .env("HOME", &fake_home)
        .output()
        .unwrap();
    assert_success(&output);
    let desktop = fake_home.join(".config/autostart/opentyless-tray.desktop");
    assert!(desktop.exists());
    let content = fs::read_to_string(desktop).unwrap();
    assert!(content.contains("OpenTyless Tray"));
    assert!(content.contains("tray"));
}

#[test]
fn install_tray_service_writes_unit_file() {
    let temp = tempdir().unwrap();
    let fake_home = temp.path().join("home");
    fs::create_dir_all(&fake_home).unwrap();

    let output = Command::new(debug_exe())
        .arg("install-tray-service")
        .env("HOME", &fake_home)
        .env("ASR_API_KEY", "test")
        .env("FLASH_API_KEY", "test")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() && stderr.contains("systemctl") {
        eprintln!("skipping: systemctl not usable in test env");
        return;
    }

    let unit_path = fake_home.join(".config/systemd/user/opentyless-tray.service");
    assert!(unit_path.exists(), "unit file should be created");
    let content = fs::read_to_string(&unit_path).unwrap();
    assert!(content.contains("Restart=always"));
    assert!(content.contains("opentyless.service"));
    assert!(content.contains("tray"));
    assert!(content.contains("graphical-session.target"));
}

#[test]
fn uninstall_tray_service_removes_unit_file() {
    let temp = tempdir().unwrap();
    let fake_home = temp.path().join("home");
    let unit_dir = fake_home.join(".config/systemd/user");
    fs::create_dir_all(&unit_dir).unwrap();
    let unit_path = unit_dir.join("opentyless-tray.service");
    fs::write(&unit_path, "[Unit]\nDescription=test\n").unwrap();

    let output = Command::new(debug_exe())
        .arg("uninstall-tray-service")
        .env("HOME", &fake_home)
        .env("ASR_API_KEY", "test")
        .env("FLASH_API_KEY", "test")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() && stderr.contains("systemctl") {
        eprintln!("skipping: systemctl not usable in test env");
        return;
    }

    assert!(
        !unit_path.exists(),
        "unit file should be removed after uninstall"
    );
}

#[test]
fn hyprland_shortcut_install_is_idempotent_and_uninstallable() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/hyprland_shortcut_e2e.sh");
    let output = Command::new("bash").arg(&script).output().unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
