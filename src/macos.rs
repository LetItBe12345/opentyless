//! macOS service integration. The Swift app owns AppKit, audio and hotkeys;
//! transcription, state and the socket protocol remain in Rust.
use super::*;

const DAEMON: &str = "local.opentyless.daemon";
const TRAY: &str = "local.opentyless.tray";

pub(super) fn helper_path() -> Result<PathBuf> {
    let current = fs::canonicalize(std::env::current_exe()?)?;
    let sibling = current.parent().unwrap().join("OpenTyless");
    if sibling.is_file() {
        return Ok(sibling);
    }
    let app = PathBuf::from("/Applications/OpenTyless.app/Contents/MacOS/OpenTyless");
    if app.is_file() {
        return Ok(app);
    }
    bail!("未找到 macOS 组件，请运行 ./scripts/install-macos.sh")
}

fn label(tray: bool) -> &'static str {
    if tray { TRAY } else { DAEMON }
}
fn agent_path(tray: bool) -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var("HOME")?)
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", label(tray))))
}
fn domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}
fn target(tray: bool) -> String {
    format!("{}/{}", domain(), label(tray))
}

fn render_agent(executable: &Path, config_dir: &Path, logs: &Path, tray: bool) -> plist::Value {
    let mut d = plist::Dictionary::new();
    d.insert("Label".into(), label(tray).into());
    let mut args = vec![plist::Value::String(executable.display().to_string())];
    if !tray {
        args.push("daemon".into());
    }
    d.insert("ProgramArguments".into(), plist::Value::Array(args));
    d.insert(
        "WorkingDirectory".into(),
        config_dir.display().to_string().into(),
    );
    d.insert("RunAtLoad".into(), true.into());
    // Quitting the menu app must not immediately relaunch it.
    d.insert("KeepAlive".into(), (!tray).into());
    d.insert("ThrottleInterval".into(), 10i64.into());
    let stem = if tray { "tray" } else { "daemon" };
    d.insert(
        "StandardOutPath".into(),
        logs.join(format!("{stem}.log"))
            .display()
            .to_string()
            .into(),
    );
    d.insert(
        "StandardErrorPath".into(),
        logs.join(format!("{stem}-error.log"))
            .display()
            .to_string()
            .into(),
    );
    let mut env = plist::Dictionary::new();
    if let Some(home) = std::env::var_os("HOME") {
        env.insert("HOME".into(), home.to_string_lossy().into_owned().into());
    }
    for name in ["PATH", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME"] {
        if let Ok(value) = std::env::var(name) {
            env.insert(name.into(), value.into());
        }
    }
    d.insert("EnvironmentVariables".into(), plist::Value::Dictionary(env));
    plist::Value::Dictionary(d)
}

fn install(tray: bool, enable: bool) -> Result<()> {
    ensure_installed_env_file()?;
    let dir = installed_config_dir();
    let logs = PathBuf::from(std::env::var("HOME")?).join("Library/Logs/opentyless");
    fs::create_dir_all(&logs)?;
    let path = agent_path(tray)?;
    fs::create_dir_all(path.parent().unwrap())?;
    let exe = if tray {
        helper_path()?
    } else {
        std::env::current_exe()?
    };
    // Validate/render before stopping the current service.
    let value = render_agent(&exe, &dir, &logs, tray);
    value.to_file_xml(&path)?;
    if enable {
        run_command_allow_fail("launchctl", &["bootout", &target(tray)])?;
        run_command(
            "launchctl",
            &[
                "bootstrap",
                &domain(),
                path.to_str().context("服务路径不是 UTF-8")?,
            ],
        )?;
    }
    println!(
        "已安装登录自启动: {}{}",
        path.display(),
        if enable {
            "（已启动）"
        } else {
            "（下次登录启动）"
        }
    );
    Ok(())
}
fn uninstall(tray: bool) -> Result<()> {
    run_command_allow_fail("launchctl", &["bootout", &target(tray)])?;
    let path = agent_path(tray)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}
fn status(tray: bool) -> Result<()> {
    let output = Command::new("launchctl")
        .args(["print", &target(tray)])
        .output()?;
    let success = output.status.success();
    print_output(output);
    if !success {
        bail!("服务未加载");
    }
    Ok(())
}
fn control(tray: bool, action: &str) -> Result<()> {
    let loaded = Command::new("launchctl")
        .args(["print", &target(tray)])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success();
    match action {
        "restart" if loaded => run_command("launchctl", &["kickstart", "-k", &target(tray)]),
        "restart" => {
            control(tray, "stop")?;
            control(tray, "start")
        }
        "start" if loaded => run_command("launchctl", &["kickstart", &target(tray)]),
        "start" if agent_path(tray)?.is_file() => run_command(
            "launchctl",
            &[
                "bootstrap",
                &domain(),
                agent_path(tray)?.to_str().context("服务路径不是 UTF-8")?,
            ],
        ),
        "start" if tray => {
            let helper = helper_path()?;
            Command::new(helper)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            Ok(())
        }
        "start" => {
            let config = Config::load()?;
            if daemon_running(&config) {
                return Ok(());
            }
            let logs = PathBuf::from(std::env::var("HOME")?).join("Library/Logs/opentyless");
            fs::create_dir_all(&logs)?;
            Command::new(std::env::current_exe()?)
                .arg("daemon")
                .current_dir(installed_config_dir())
                .stdin(Stdio::null())
                .stdout(
                    fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(logs.join("daemon.log"))?,
                )
                .stderr(
                    fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(logs.join("daemon-error.log"))?,
                )
                .spawn()?;
            Ok(())
        }
        "stop" if loaded => run_command("launchctl", &["bootout", &target(tray)]),
        "stop" if !tray => {
            let config = Config::load()?;
            if !daemon_running(&config) {
                return Ok(());
            }
            let state = State::load(&config.state_path());
            if state.recording {
                send_request(
                    &config.socket_path,
                    &Request {
                        action: "cancel-record".into(),
                    },
                )?;
            }
            let pid = state.pid.context("后台服务 PID 缺失")?;
            if !pid_matches_expected_process(pid, Some(env!("CARGO_PKG_NAME"))) {
                bail!("PID 不属于 OpenTyless，拒绝停止其他进程");
            }
            if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(())
        }
        "stop" => bail!("状态栏未通过 launchd 启动，请从菜单退出"),
        _ => bail!("未知服务操作"),
    }
}

fn logs(tray: bool, count: usize) -> Result<()> {
    let stem = if tray { "tray" } else { "daemon" };
    let dir = PathBuf::from(std::env::var("HOME")?).join("Library/Logs/opentyless");
    for name in [format!("{stem}.log"), format!("{stem}-error.log")] {
        let text = fs::read_to_string(dir.join(&name)).unwrap_or_default();
        let lines: Vec<_> = text.lines().collect();
        println!(
            "{name}:\n{}",
            lines[lines.len().saturating_sub(count)..].join("\n")
        );
    }
    Ok(())
}

pub(super) fn handle_command(command: &Commands, _config: &Config) -> Option<Result<()>> {
    Some(match command {
        Commands::InstallService(a) => install(false, a.enable),
        Commands::InstallTrayService(a) => install(true, a.enable),
        Commands::InstallTrayAutostart => install(true, true),
        Commands::UninstallService => uninstall(false),
        Commands::UninstallTrayService => uninstall(true),
        Commands::ServiceStatus => status(false),
        Commands::TrayServiceStatus => status(true),
        Commands::Start => control(false, "start"),
        Commands::Stop => control(false, "stop"),
        Commands::Restart => control(false, "restart"),
        Commands::StartTray => control(true, "start"),
        Commands::StopTray => control(true, "stop"),
        Commands::RestartTray => control(true, "restart"),
        Commands::Logs(a) => logs(false, a.lines),
        Commands::TrayLogs(a) => logs(true, a.lines),
        Commands::Tray => helper_path().and_then(|p| {
            run_command(
                "open",
                &[
                    "-a",
                    p.parent()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .parent()
                        .unwrap()
                        .to_str()
                        .unwrap(),
                ],
            )
        }),
        Commands::InstallGnomeShortcut(_) => {
            Err(anyhow!("macOS 请在状态栏的「快捷键」菜单中选择全局快捷键"))
        }
        _ => return None,
    })
}

static SHUTDOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
extern "C" fn request_shutdown(_: libc::c_int) {
    SHUTDOWN.store(true, std::sync::atomic::Ordering::Relaxed);
}
pub(super) fn install_shutdown_handlers() -> Result<()> {
    SHUTDOWN.store(false, std::sync::atomic::Ordering::Relaxed);
    for signal in [libc::SIGINT, libc::SIGTERM] {
        if unsafe { libc::signal(signal, request_shutdown as *const () as libc::sighandler_t) }
            == libc::SIG_ERR
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}
pub(super) fn shutdown_requested() -> bool {
    SHUTDOWN.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_agents_preserve_paths_and_quit_semantics() {
        let exe = Path::new("/Users/a b/OpenTyless.app/Contents/MacOS/opentyless-rs");
        for tray in [false, true] {
            let value = render_agent(
                exe,
                Path::new("/Users/a b/.config/opentyless"),
                Path::new("/tmp/logs"),
                tray,
            );
            let mut xml = Vec::new();
            value.to_writer_xml(&mut xml).unwrap();
            let decoded = plist::Value::from_reader(std::io::Cursor::new(xml)).unwrap();
            let d = decoded.as_dictionary().unwrap();
            assert_eq!(
                d["ProgramArguments"].as_array().unwrap()[0].as_string(),
                exe.to_str()
            );
            assert_eq!(d["KeepAlive"].as_boolean(), Some(!tray));
            assert_eq!(d["RunAtLoad"].as_boolean(), Some(true));
        }
    }
}
