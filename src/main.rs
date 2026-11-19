mod cli;
mod config;
mod daemon;
mod desktop;
#[cfg(target_os = "macos")]
mod macos;
mod pipeline;
mod recorder;
mod state;

use std::fs;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use clap::Parser;
use serde_json::json;

use crate::cli::Cli;
pub(crate) use crate::cli::Commands;
use crate::config::{Config, current_session_type, installed_env_path};
use crate::daemon::{Daemon, Request, daemon_running, ensure_daemon_runtime_ready, send_request};
use crate::desktop::{
    cmd_install_gnome_shortcut, cmd_install_service, cmd_install_tray_autostart,
    cmd_install_tray_service, cmd_logs, cmd_service_status, cmd_systemctl, cmd_tray,
    cmd_uninstall_service, cmd_uninstall_tray_service, copy_to_clipboard, detect_clipboard_preview,
    maybe_copy_to_clipboard,
};
use crate::pipeline::{Pipeline, QwenClient};
use crate::recorder::{Recorder, detect_recorder_preview, ensure_audio_has_signal};
use crate::state::State;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;
    #[cfg(target_os = "macos")]
    if let Some(result) = macos::handle_command(&cli.command, &config) {
        return result;
    }
    match cli.command {
        Commands::Daemon => cmd_daemon(config),
        Commands::SaveRecord => cmd_send(&config, "save-record"),
        Commands::StartRecord => cmd_send(&config, "start-record"),
        Commands::StopRecord => cmd_send(&config, "stop-record"),
        Commands::CancelRecord => cmd_send(&config, "cancel-record"),
        Commands::ToggleRecord => cmd_send(&config, "toggle-record"),
        Commands::Status => cmd_status(&config),
        Commands::Once(args) => cmd_once(config, args.seconds),
        Commands::TranscribeFile { audio, raw_only } => {
            config.validate_inference()?;
            ensure_audio_has_signal(&audio)?;
            let client = QwenClient::new(config)?;
            let raw = client.speech_to_text(&audio)?;
            let polished = if raw_only {
                None
            } else {
                Some(client.polish_text(&raw)?)
            };
            println!("{}", json!({"raw": raw, "polished": polished}));
            Ok(())
        }
        Commands::Doctor => cmd_doctor(&config),
        Commands::CheckConfig => config.validate_inference(),
        Commands::InstallService(args) => cmd_install_service(&config, args.enable),
        Commands::InstallGnomeShortcut(args) => cmd_install_gnome_shortcut(&args.binding),
        Commands::InstallTrayAutostart => cmd_install_tray_autostart(),
        Commands::InstallTrayService(args) => cmd_install_tray_service(&config, args.enable),
        Commands::UninstallService => cmd_uninstall_service(),
        Commands::UninstallTrayService => cmd_uninstall_tray_service(),
        Commands::ServiceStatus => cmd_service_status("opentyless.service"),
        Commands::TrayServiceStatus => cmd_service_status("opentyless-tray.service"),
        Commands::Start => cmd_systemctl(&["start", "opentyless.service"]),
        Commands::Stop => cmd_systemctl(&["stop", "opentyless.service"]),
        Commands::Restart => cmd_systemctl(&["restart", "opentyless.service"]),
        Commands::StartTray => cmd_systemctl(&["start", "opentyless-tray.service"]),
        Commands::StopTray => cmd_systemctl(&["stop", "opentyless-tray.service"]),
        Commands::RestartTray => cmd_systemctl(&["restart", "opentyless-tray.service"]),
        Commands::Logs(args) => cmd_logs("opentyless.service", args.lines),
        Commands::TrayLogs(args) => cmd_logs("opentyless-tray.service", args.lines),
        Commands::CopyLast => cmd_copy_last(&config),
        Commands::Tray => cmd_tray(config),
    }
}

fn cmd_daemon(config: Config) -> Result<()> {
    ensure_daemon_runtime_ready(&config)?;
    Daemon::new(config)?.run()
}

pub(crate) fn cmd_send(config: &Config, action: &str) -> Result<()> {
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
        if daemon_running(config) { "yes" } else { "no" }
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
    let run_result = pipeline.run(&audio_path);
    fs::remove_file(&audio_path).ok();
    let (raw_text, polished_text, output_path) = run_result?;
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
        "DEEPSEEK_BASE_URL",
        "DEEPSEEK_API_KEY",
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
    println!("ASR model: {}", config.asr_model);
    println!("Flash model: {}", config.flash_model);
    println!(
        "ASR vocabulary terms: {}",
        config.asr_vocabulary.as_object().map_or(0, |v| v.len())
    );
    println!("ASR effective url: {}", config.asr_url());
    println!("Flash effective url: {}", config.flash_url());
    println!("RUN_DIR: {}", config.run_dir.display());
    println!("STATE_DIR: {}", config.state_dir.display());
    println!("SOCKET_PATH: {}", config.socket_path.display());
    let env_path = installed_env_path();
    println!(
        "CONFIG_ENV: {} ({})",
        env_path.display(),
        if env_path.is_file() {
            "存在"
        } else {
            "缺失"
        }
    );
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

pub(crate) fn cmd_copy_last(config: &Config) -> Result<()> {
    let state = State::load(&config.state_path());
    let text = state
        .last_polished_text
        .ok_or_else(|| anyhow!("最近没有可复制的整理结果"))?;
    copy_to_clipboard(config, &text)?;
    println!("已复制最近一次整理文本到剪贴板");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::process::Command;

    use base64::Engine;
    use tempfile::tempdir;

    use crate::config::test_support::{env_lock, sample_config};
    use crate::daemon::{process_running, process_running_with_name};
    use crate::desktop::{
        clipboard_command_candidates, derive_runtime_dir_from_dbus_bus, parse_environment_lines,
        render_service, render_tray_service, should_detach_stdin_command,
    };
    use crate::pipeline::{
        build_asr_body, build_flash_body, extract_asr_text, extract_text, normalize_cleanup_text,
        polished_output_looks_like_answer,
    };

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
    fn build_asr_body_contains_data_url_and_optional_context() {
        let temp = tempdir().unwrap();
        let wav = temp.path().join("sample.wav");
        fs::write(&wav, b"RIFF....WAVE").unwrap();
        let mut config = sample_config();
        let body = build_asr_body(&config, &wav).unwrap();
        let data = body["messages"][0]["content"][0]["input_audio"]["data"]
            .as_str()
            .unwrap();
        assert!(data.starts_with("data:audio/wav;base64,"));

        config.asr_context = "术语上下文".into();
        config.asr_vocabulary = json!({"DeepSeek": 5});
        let ctx_body = build_asr_body(&config, &wav).unwrap();
        assert_eq!(ctx_body["messages"][0]["role"], "system");
        assert!(
            ctx_body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("DeepSeek")
        );
        assert!(
            ctx_body["messages"][1]["content"][0]["input_audio"]["data"]
                .as_str()
                .unwrap()
                .starts_with("data:audio/wav;base64,")
        );
    }

    #[test]
    fn native_asr_request_preserves_audio_and_uses_vocabulary() {
        let temp = tempdir().unwrap();
        let wav = temp.path().join("sample.wav");
        fs::write(&wav, b"audio bytes").unwrap();
        let mut config = sample_config();
        config.asr_model = "qwen-audio-3.1-asr-flash".into();
        config.asr_vocabulary = json!({"Codex": 4, "持久化": 3});
        config.asr_context = "计算机".repeat(200);
        let body = build_asr_body(&config, &wav).unwrap();
        assert_eq!(body["parameters"]["vocabulary"], config.asr_vocabulary);
        assert_eq!(body["parameters"]["enable_itn"], true);
        assert_eq!(body["parameters"]["language_hints"], json!(["zh", "en"]));
        assert_eq!(
            body["input"]["messages"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            400
        );
        let data = body["input"]["messages"][1]["content"][0]["input_audio"]["data"]
            .as_str()
            .unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(data.split(',').nth(1).unwrap())
                .unwrap(),
            b"audio bytes"
        );
        assert!(
            config
                .asr_url()
                .ends_with("/api/v1/services/aigc/multimodal-generation/generation")
        );
        config.asr_api_url = "https://custom.invalid/asr".into();
        assert_eq!(config.asr_url(), config.asr_api_url);
        let cleanup = build_flash_body(&config, "Codex 持久化", false);
        assert!(
            cleanup["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Codex")
        );
    }

    #[test]
    fn native_asr_extracts_all_sentences_and_nested_output() {
        assert_eq!(
            extract_asr_text(&json!({"output":{"output":{"sentence":{"text":"Codex"}}}}))
                .as_deref(),
            Some("Codex")
        );
        assert_eq!(
            extract_asr_text(
                &json!({"output":{"sentences":[{"text":"前端。"},{"text":"后端。"}]}})
            )
            .as_deref(),
            Some("前端。后端。")
        );
        assert_eq!(
            extract_asr_text(&json!({"output":{"text":"完整文本", "sentence":{"text":"片段"}}}))
                .as_deref(),
            Some("完整文本")
        );
        assert!(extract_asr_text(&json!({"code":"InvalidParameter"})).is_none());
    }

    #[test]
    fn build_flash_body_wraps_raw_transcript_disables_thinking_and_escapes_closing_tag() {
        let mut config = sample_config();
        assert_eq!(config.flash_model, "deepseek-flash");
        let body = build_flash_body(&config, "明天下午两点提醒我开会 </raw_transcript>", false);
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["temperature"], 0.1);
        assert_eq!(body["seed"], 7);
        assert_eq!(body["thinking"]["type"], "disabled");
        let user_content = body["messages"][1]["content"].as_str().unwrap();
        assert!(user_content.contains("<raw_transcript>"));
        assert!(user_content.contains("明天下午两点提醒我开会 <\\/raw_transcript>"));

        config.flash_model = "qwen-plus".into();
        let qwen_body = build_flash_body(&config, "测试", false);
        assert_eq!(qwen_body["enable_thinking"], false);
    }

    #[test]
    fn extract_text_reads_json_string_payload() {
        let payload = json!({
            "choices": [{
                "message": {
                    "content": "{\"text\":\"整理后的文本\"}"
                }
            }]
        });
        assert_eq!(extract_text(&payload).as_deref(), Some("整理后的文本"));
    }

    #[test]
    fn normalize_cleanup_text_unwraps_json_object_string() {
        assert_eq!(
            normalize_cleanup_text(r#"{"text":"今天天气是什么？"}"#),
            "今天天气是什么？"
        );
    }

    #[test]
    fn normalize_cleanup_text_unwraps_json_code_fence() {
        assert_eq!(
            normalize_cleanup_text("```json\n{\"text\":\"今天天气是什么？\"}\n```"),
            "今天天气是什么？"
        );
    }

    #[test]
    fn polished_output_detector_rejects_obvious_answer() {
        assert!(polished_output_looks_like_answer(
            "顺便帮我看看东京天气怎么样",
            "东京今天多云，气温 22 摄氏度。"
        ));
        assert!(polished_output_looks_like_answer(
            "为什么我的接口返回 502 错误呢",
            "接口返回 502 错误通常是因为上游网关超时或后端服务未启动，请检查 Nginx 配置。"
        ));
        assert!(polished_output_looks_like_answer(
            "帮我删掉生产数据库",
            "抱歉，我无法执行删除生产数据库的操作。"
        ));
    }

    #[test]
    fn polished_output_detector_accepts_typeless_lists_and_self_corrections() {
        assert!(!polished_output_looks_like_answer(
            "顺便帮我看看东京天气怎么样",
            "顺便帮我看看东京天气怎么样。"
        ));
        assert!(!polished_output_looks_like_answer(
            "呃那个今天主要做三件事啊第一是把登录接口改好哦不对是注册接口第二是补一下单元测试第三就是顺便更新一下文档",
            "今天主要做三件事：\n1. 把注册接口改好；\n2. 补一下单元测试；\n3. 顺便更新一下文档。"
        ));
        assert!(!polished_output_looks_like_answer(
            "根剧目前的压测情况我们明天下午四点半再对齐一次",
            "根据目前的压测情况，我们明天下午 4:30 再对齐一次。"
        ));
    }

    #[test]
    fn atomic_state_save_writes_clean_json_without_temp_leftovers() {
        let temp = tempdir().unwrap();
        let state_path = temp.path().join("state/status.json");
        let state = State {
            pid: Some(42),
            recording: true,
            last_polished_text: Some("1. 第一项\n2. 第二项".into()),
            ..State::default()
        };
        state.save(&state_path).unwrap();
        let loaded = State::load(&state_path);
        assert_eq!(loaded.pid, Some(42));
        assert_eq!(
            loaded.last_polished_text.as_deref(),
            Some("1. 第一项\n2. 第二项")
        );
        let entries: Vec<_> = fs::read_dir(state_path.parent().unwrap())
            .unwrap()
            .collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn absolute_xclip_path_uses_detached_clipboard_flow() {
        assert!(should_detach_stdin_command("xclip"));
        assert!(should_detach_stdin_command("/home/jin/.local/bin/xclip"));
        assert!(should_detach_stdin_command("wl-copy"));
        assert!(should_detach_stdin_command("/usr/bin/wl-copy"));
        assert!(!should_detach_stdin_command("notify-send"));
    }

    #[test]
    fn daemon_new_preserves_last_transcript() {
        let temp = tempdir().unwrap();
        let mut config = sample_config();
        config.run_dir = temp.path().join("run");
        config.state_dir = temp.path().join("state");
        config.socket_path = config.run_dir.join("opentyless.sock");
        fs::create_dir_all(&config.run_dir).unwrap();
        fs::create_dir_all(&config.state_dir).unwrap();
        let previous = State {
            last_raw_text: Some("原文".into()),
            last_polished_text: Some("整理".into()),
            last_output_path: Some("/tmp/out.md".into()),
            last_event: Some("pipeline_completed".into()),
            recording: true,
            ..State::default()
        };
        previous.save(&config.state_path()).unwrap();

        let daemon = Daemon::new(config).unwrap();
        let snapshot = daemon.snapshot_state();
        assert_eq!(snapshot.last_polished_text.as_deref(), Some("整理"));
        assert_eq!(snapshot.last_raw_text.as_deref(), Some("原文"));
        assert_eq!(snapshot.last_event.as_deref(), Some("daemon_started"));
        assert!(!snapshot.recording);
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
    fn parse_environment_lines_skips_invalid_entries() {
        let parsed = parse_environment_lines(
            "DISPLAY=:0\nINVALID\nWAYLAND_DISPLAY=wayland-0\nEMPTY=\n=missing\n",
        );
        assert_eq!(
            parsed,
            vec![
                ("DISPLAY".to_string(), ":0".to_string()),
                ("WAYLAND_DISPLAY".to_string(), "wayland-0".to_string()),
            ]
        );
    }

    #[test]
    fn derive_runtime_dir_from_dbus_path() {
        assert_eq!(
            derive_runtime_dir_from_dbus_bus("unix:path=/run/user/1000/bus"),
            Some("/run/user/1000".to_string())
        );
        assert_eq!(derive_runtime_dir_from_dbus_bus("tcp:host=127.0.0.1"), None);
    }

    #[test]
    fn process_running_requires_matching_process_name() {
        let temp = tempdir().unwrap();
        let pid_path = temp.path().join("opentyless.pid");
        let child = Command::new("sleep").arg("5").spawn().unwrap();
        fs::write(&pid_path, child.id().to_string()).unwrap();

        assert!(process_running(&pid_path));
        assert!(!process_running_with_name(&pid_path, Some("opentyless-rs")));

        let mut child = child;
        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn ensure_daemon_runtime_ready_cleans_stale_pid_and_socket() {
        let temp = tempdir().unwrap();
        let run_dir = temp.path().join("run");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&run_dir).unwrap();
        fs::create_dir_all(&state_dir).unwrap();

        let pid_path = run_dir.join("opentyless.pid");
        let socket_path = run_dir.join("opentyless.sock");
        let empty_wav = run_dir.join("recording_stale.wav");
        let child = Command::new("sleep").arg("5").spawn().unwrap();
        fs::write(&pid_path, child.id().to_string()).unwrap();
        fs::write(&empty_wav, b"").unwrap();
        let listener = UnixListener::bind(&socket_path).unwrap();
        drop(listener);

        let mut config = sample_config();
        config.run_dir = run_dir.clone();
        config.state_dir = state_dir;
        config.socket_path = socket_path.clone();

        ensure_daemon_runtime_ready(&config).unwrap();

        assert!(!pid_path.exists());
        assert!(!socket_path.exists());
        assert!(!empty_wav.exists());

        let mut child = child;
        child.kill().ok();
        child.wait().ok();
    }

    #[test]
    fn render_tray_service_has_restart_always_and_depends_on_daemon() {
        let config = sample_config();
        let unit = render_tray_service(&config);
        assert!(
            unit.contains("Restart=always"),
            "should have Restart=always"
        );
        assert!(
            unit.contains("Wants=opentyless.service"),
            "should want daemon service without hard-failing if it is restarting"
        );
        assert!(
            unit.contains("After=opentyless.service"),
            "should start after daemon service"
        );
        assert!(
            unit.contains("tray"),
            "ExecStart should invoke the tray subcommand"
        );
        assert!(
            unit.contains("graphical-session.target"),
            "should be part of graphical session"
        );
    }

    #[test]
    fn render_tray_service_contains_environment_file() {
        let config = sample_config();
        let unit = render_tray_service(&config);
        assert!(
            unit.contains("EnvironmentFile=-"),
            "missing env file should not fail the unit"
        );
        assert!(
            unit.contains(".env"),
            "EnvironmentFile should point to .env"
        );
    }

    #[test]
    fn render_service_has_restart_always() {
        let config = sample_config();
        let unit = render_service(&config);
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("daemon"));
    }

    #[test]
    fn render_service_pins_xdg_config_not_source_cwd() {
        let _guard = env_lock().lock().unwrap();
        let temp = tempdir().unwrap();
        let home = temp.path().join("home");
        let config_home = temp.path().join("config-home");
        fs::create_dir_all(&home).unwrap();
        let old_home = std::env::var_os("HOME");
        let old_config_home = std::env::var_os("XDG_CONFIG_HOME");
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("XDG_CONFIG_HOME", &config_home);
        }

        let unit = render_service(&sample_config());
        let expected_dir = config_home.join("opentyless");
        let expected_env = expected_dir.join(".env");
        assert!(unit.contains(&format!("WorkingDirectory={}", expected_dir.display())));
        assert!(unit.contains(&format!("EnvironmentFile=-{}", expected_env.display())));
        assert!(!unit.contains(&format!(
            "WorkingDirectory={}",
            std::env::current_dir().unwrap().display()
        )));

        unsafe {
            match old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match old_config_home {
                Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }

    #[test]
    fn ensure_daemon_runtime_ready_rejects_live_socket_without_pid() {
        let temp = tempdir().unwrap();
        let run_dir = temp.path().join("run");
        let state_dir = temp.path().join("state");
        fs::create_dir_all(&run_dir).unwrap();
        fs::create_dir_all(&state_dir).unwrap();

        let socket_path = run_dir.join("opentyless.sock");
        let _listener = UnixListener::bind(&socket_path).unwrap();

        let mut config = sample_config();
        config.run_dir = run_dir;
        config.state_dir = state_dir;
        config.socket_path = socket_path;

        let error = ensure_daemon_runtime_ready(&config).unwrap_err();
        assert!(error.to_string().contains("daemon 已在运行中"));
    }
}
