use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::tempdir;

fn handle_connection(mut stream: TcpStream) {
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
    reader.read_exact(&mut body).unwrap();
    let body_str = String::from_utf8_lossy(&body);
    let response_body = if body_str.contains("qwen3-asr-flash") {
        r#"{"choices":[{"message":{"content":[{"type":"text","text":"测试语音原文"}]}}]}"#
    } else {
        r#"{"choices":[{"message":{"content":"测试语音整理后"}}]}"#
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    stream.write_all(response.as_bytes()).unwrap();
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
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
        thread::sleep(Duration::from_millis(100));
    }
    panic!("audio path not ready: {}", path.display());
}

#[test]
fn daemon_cli_flow_works_with_mock_server() {
    let temp = tempdir().unwrap();
    let run_dir = temp.path().join("run");
    let state_dir = temp.path().join("state");
    let output_dir = temp.path().join("outputs");
    fs::create_dir_all(&run_dir).unwrap();

    let fixture = run_dir.join("fixture.wav");
    fs::write(&fixture, b"RIFF....WAVE").unwrap();
    let recorder = run_dir.join("fake-recorder.sh");
    fs::write(
        &recorder,
        format!(
            "#!/usr/bin/env bash\ncp '{}' \"$1\"\nwhile true; do sleep 1; done\n",
            fixture.display()
        ),
    )
    .unwrap();
    Command::new("chmod")
        .args(["+x", recorder.to_str().unwrap()])
        .status()
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream);
        }
    });

    let exe = std::env::current_dir()
        .unwrap()
        .join("target/debug/opentyless-rs");
    let socket_path = run_dir.join("opentyless.sock");
    let mut daemon = Command::new(&exe)
        .arg("daemon")
        .env("RUN_DIR", &run_dir)
        .env("STATE_DIR", &state_dir)
        .env("OUTPUT_DIR", &output_dir)
        .env("SOCKET_PATH", &socket_path)
        .env("RECORDER_CMD", recorder.to_str().unwrap())
        .env("ASR_API_URL", format!("http://{}/chat/completions", addr))
        .env("FLASH_API_URL", format!("http://{}/chat/completions", addr))
        .env("ASR_API_KEY", "test")
        .env("FLASH_API_KEY", "test")
        .env("AUTO_COPY_TO_CLIPBOARD", "false")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    wait_for(&socket_path);

    let start = Command::new(&exe)
        .arg("start-record")
        .env("RUN_DIR", &run_dir)
        .env("STATE_DIR", &state_dir)
        .env("OUTPUT_DIR", &output_dir)
        .env("SOCKET_PATH", &socket_path)
        .output()
        .unwrap();
    assert!(start.status.success());
    wait_for_state_audio(&state_dir.join("status.json"));

    let stop = Command::new(&exe)
        .arg("stop-record")
        .env("RUN_DIR", &run_dir)
        .env("STATE_DIR", &state_dir)
        .env("OUTPUT_DIR", &output_dir)
        .env("SOCKET_PATH", &socket_path)
        .env("ASR_API_KEY", "test")
        .env("FLASH_API_KEY", "test")
        .output()
        .unwrap();
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    assert!(String::from_utf8_lossy(&stop.stdout).contains("已停止录音，正在转写"));

    let immediate_status = fs::read_to_string(state_dir.join("status.json")).unwrap();
    assert!(immediate_status.contains("\"last_event\": \"transcribing\""));

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        let status = fs::read_to_string(state_dir.join("status.json")).unwrap_or_default();
        if status.contains("pipeline_completed") {
            break status;
        }
        if Instant::now() >= deadline {
            panic!("daemon flow did not complete in time");
        }
        thread::sleep(Duration::from_millis(100));
    };
    assert!(status.contains("测试语音原文"));
    assert!(status.contains("测试语音整理后"));

    let outputs: Vec<_> = fs::read_dir(&output_dir).unwrap().collect();
    assert_eq!(outputs.len(), 1);
    let md = fs::read_to_string(outputs[0].as_ref().unwrap().path()).unwrap();
    assert!(md.contains("测试语音整理后"));

    daemon.kill().ok();
    daemon.wait().ok();
    server.join().unwrap();
}

#[test]
fn toggle_record_flow_works_with_mock_server() {
    let temp = tempdir().unwrap();
    let run_dir = temp.path().join("run");
    let state_dir = temp.path().join("state");
    let output_dir = temp.path().join("outputs");
    fs::create_dir_all(&run_dir).unwrap();

    let fixture = run_dir.join("fixture.wav");
    fs::write(&fixture, b"RIFF....WAVE").unwrap();
    let recorder = run_dir.join("fake-recorder.sh");
    fs::write(
        &recorder,
        format!(
            "#!/usr/bin/env bash\ncp '{}' \"$1\"\nwhile true; do sleep 1; done\n",
            fixture.display()
        ),
    )
    .unwrap();
    Command::new("chmod")
        .args(["+x", recorder.to_str().unwrap()])
        .status()
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            handle_connection(stream);
        }
    });

    let exe = std::env::current_dir()
        .unwrap()
        .join("target/debug/opentyless-rs");
    let socket_path = run_dir.join("opentyless.sock");
    let mut daemon = Command::new(&exe)
        .arg("daemon")
        .env("RUN_DIR", &run_dir)
        .env("STATE_DIR", &state_dir)
        .env("OUTPUT_DIR", &output_dir)
        .env("SOCKET_PATH", &socket_path)
        .env("RECORDER_CMD", recorder.to_str().unwrap())
        .env("ASR_API_URL", format!("http://{}/chat/completions", addr))
        .env("FLASH_API_URL", format!("http://{}/chat/completions", addr))
        .env("ASR_API_KEY", "test")
        .env("FLASH_API_KEY", "test")
        .env("AUTO_COPY_TO_CLIPBOARD", "false")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    wait_for(&socket_path);

    let start = Command::new(&exe)
        .arg("toggle-record")
        .env("RUN_DIR", &run_dir)
        .env("STATE_DIR", &state_dir)
        .env("OUTPUT_DIR", &output_dir)
        .env("SOCKET_PATH", &socket_path)
        .output()
        .unwrap();
    assert!(start.status.success());
    assert!(String::from_utf8_lossy(&start.stdout).contains("已开始录音"));
    wait_for_state_audio(&state_dir.join("status.json"));

    let stop = Command::new(&exe)
        .arg("toggle-record")
        .env("RUN_DIR", &run_dir)
        .env("STATE_DIR", &state_dir)
        .env("OUTPUT_DIR", &output_dir)
        .env("SOCKET_PATH", &socket_path)
        .env("ASR_API_KEY", "test")
        .env("FLASH_API_KEY", "test")
        .output()
        .unwrap();
    assert!(stop.status.success());
    assert!(String::from_utf8_lossy(&stop.stdout).contains("已停止录音，正在转写"));

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = fs::read_to_string(state_dir.join("status.json")).unwrap_or_default();
        if status.contains("pipeline_completed") {
            assert!(status.contains("测试语音原文"));
            assert!(status.contains("测试语音整理后"));
            break;
        }
        if Instant::now() >= deadline {
            panic!("toggle flow did not complete in time");
        }
        thread::sleep(Duration::from_millis(100));
    }

    daemon.kill().ok();
    daemon.wait().ok();
    server.join().unwrap();
}
