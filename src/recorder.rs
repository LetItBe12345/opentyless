use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Local;

use crate::config::{Config, split_command_line};
#[cfg(not(target_os = "macos"))]
use crate::desktop::command_exists;

const STALE_TEMP_RECORDING_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug)]
pub struct Recorder {
    pub child: Child,
    pub audio_path: PathBuf,
}

impl Recorder {
    pub fn start(config: &Config) -> Result<Self> {
        fs::create_dir_all(&config.run_dir)?;
        let audio_path = unique_recording_path(&config.run_dir);
        let mut command = detect_recorder_command(config)?;
        command.arg(&audio_path);
        // A CLI started by a shell/service may inherit SIGINT as ignored.
        // Restore it in the recorder child so stop() can finalize its WAV.
        #[cfg(unix)]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                libc::signal(libc::SIGINT, libc::SIG_DFL);
                libc::signal(libc::SIGTERM, libc::SIG_DFL);
                Ok(())
            });
        }
        let child = command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("启动录音进程失败")?;
        #[cfg(target_os = "macos")]
        let mut child = child;
        #[cfg(target_os = "macos")]
        if config.recorder_cmd.is_none() {
            let ready = sidecar_path(&audio_path, "ready");
            let error = sidecar_path(&audio_path, "error");
            let deadline = Instant::now() + Duration::from_secs(30);
            while !ready.exists() {
                if let Some(status) = child.try_wait()? {
                    let reason = fs::read_to_string(&error)
                        .unwrap_or_else(|_| format!("录音进程退出: {status}"));
                    fs::remove_file(&error).ok();
                    bail!("{reason}");
                }
                if Instant::now() >= deadline {
                    terminate_child_bounded(&mut child);
                    bail!("麦克风启动超时，请先在 OpenTyless 状态栏菜单中授权麦克风");
                }
                thread::sleep(Duration::from_millis(50));
            }
            fs::remove_file(ready).ok();
        }
        Ok(Self { child, audio_path })
    }

    pub fn stop(mut self) -> Result<PathBuf> {
        terminate_child_bounded(&mut self.child);
        cleanup_sidecars(&self.audio_path);
        if !self.audio_path.is_file() || fs::metadata(&self.audio_path)?.len() == 0 {
            fs::remove_file(&self.audio_path).ok();
            bail!("录音文件为空，请确认麦克风和录音设备可用");
        }
        Ok(self.audio_path)
    }

    pub fn discard(mut self) {
        terminate_child_bounded(&mut self.child);
        cleanup_sidecars(&self.audio_path);
        fs::remove_file(&self.audio_path).ok();
    }
}

fn sidecar_path(audio_path: &Path, ext: &str) -> PathBuf {
    PathBuf::from(format!("{}.{ext}", audio_path.display()))
}

fn cleanup_sidecars(audio_path: &Path) {
    for ext in ["ready", "error"] {
        fs::remove_file(sidecar_path(audio_path, ext)).ok();
    }
}

/// Gracefully interrupt the recorder so WAV headers are flushed, escalating to
/// SIGTERM and SIGKILL if the child hangs so the daemon never deadlocks.
fn terminate_child_bounded(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as libc::pid_t;
        unsafe {
            libc::kill(pid, libc::SIGINT);
        }
        if wait_child_timeout(child, Duration::from_secs(3)) {
            return;
        }
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        if wait_child_timeout(child, Duration::from_secs(1)) {
            return;
        }
    }
    child.kill().ok();
    child.wait().ok();
}

fn wait_child_timeout(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {
                if Instant::now() >= deadline {
                    return false;
                }
                thread::sleep(Duration::from_millis(15));
            }
            Err(_) => return true,
        }
    }
}

fn unique_recording_path(run_dir: &Path) -> PathBuf {
    let stem = Local::now().format("recording_%Y%m%d_%H%M%S_%3f");
    let candidate = run_dir.join(format!("{stem}.wav"));
    if !candidate.exists() {
        return candidate;
    }
    for suffix in 1..1000u32 {
        let fallback = run_dir.join(format!("{stem}_{suffix}.wav"));
        if !fallback.exists() {
            return fallback;
        }
    }
    run_dir.join(format!("{stem}_{}.wav", std::process::id()))
}

pub fn save_recording(source: &Path, directory: &Path) -> Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let mut input = fs::File::open(source)?;
    let mut output = tempfile::Builder::new()
        .prefix(&Local::now().format("recording_%Y%m%d_%H%M%S_").to_string())
        .suffix(".wav")
        .tempfile_in(directory)?;
    std::io::copy(&mut input, output.as_file_mut())?;
    output.as_file().sync_all()?;
    let (_, path) = output.keep()?;
    fs::remove_file(source).ok();
    Ok(path)
}

pub fn detect_recorder_command(config: &Config) -> Result<Command> {
    if let Some(custom) = &config.recorder_cmd {
        let parts = split_command_line(custom)?;
        let (executable, args) = parts
            .split_first()
            .ok_or_else(|| anyhow!("RECORDER_CMD 不能为空"))?;
        let mut command = Command::new(executable);
        command.args(args);
        return Ok(command);
    }

    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new(crate::macos::helper_path()?);
        command.arg("--record");
        Ok(command)
    }

    #[cfg(not(target_os = "macos"))]
    {
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
}

pub fn detect_recorder_preview(config: &Config) -> Result<String> {
    if let Some(custom) = &config.recorder_cmd {
        return Ok(custom.clone());
    }
    #[cfg(target_os = "macos")]
    return Ok(format!(
        "{} --record <output.wav>",
        crate::macos::helper_path()?.display()
    ));
    #[cfg(not(target_os = "macos"))]
    {
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
}

pub fn detect_audio_format(audio_path: &Path) -> &'static str {
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

const MAX_AUDIO_BYTES: u64 = 25 * 1024 * 1024;

pub fn ensure_audio_has_signal(audio_path: &Path) -> Result<()> {
    let len = fs::metadata(audio_path)
        .with_context(|| format!("无法读取音频文件: {}", audio_path.display()))?
        .len();
    if len == 0 {
        bail!("音频文件为空: {}", audio_path.display());
    }
    if len > MAX_AUDIO_BYTES {
        bail!(
            "音频文件过大（{:.1} MB，上限 {:.1} MB），请分段录制或压缩音频后再转写",
            len as f64 / (1024.0 * 1024.0),
            MAX_AUDIO_BYTES as f64 / (1024.0 * 1024.0)
        );
    }
    match wav_peak_amplitude(audio_path)? {
        Some(peak) if peak <= 32 => bail!(
            "录音近乎静音，已跳过转写，避免模型在空白音频上幻听。请检查麦克风输入或 RECORDER_CMD: {}",
            audio_path.display()
        ),
        _ => Ok(()),
    }
}

pub fn wav_peak_amplitude(audio_path: &Path) -> Result<Option<i32>> {
    if audio_path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| !ext.eq_ignore_ascii_case("wav"))
        .unwrap_or(true)
    {
        return Ok(None);
    }

    let bytes = fs::read(audio_path)?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Ok(None);
    }

    let mut cursor = 12usize;
    let mut audio_format = None;
    let mut bits_per_sample = None;
    let mut data_chunk = None;

    while cursor + 8 <= bytes.len() {
        let chunk_id = &bytes[cursor..cursor + 4];
        let chunk_size = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into()?) as usize;
        cursor += 8;
        if cursor + chunk_size > bytes.len() {
            break;
        }

        let chunk = &bytes[cursor..cursor + chunk_size];
        match chunk_id {
            b"fmt " if chunk.len() >= 16 => {
                audio_format = Some(u16::from_le_bytes(chunk[0..2].try_into()?));
                bits_per_sample = Some(u16::from_le_bytes(chunk[14..16].try_into()?));
            }
            b"data" => data_chunk = Some(chunk),
            _ => {}
        }

        cursor += chunk_size;
        if chunk_size % 2 == 1 {
            cursor += 1;
        }
    }

    if audio_format != Some(1) || bits_per_sample != Some(16) {
        return Ok(None);
    }

    let Some(data) = data_chunk else {
        return Ok(None);
    };

    let peak = data
        .chunks(2)
        .filter(|sample| sample.len() == 2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as i32)
        .map(i32::abs)
        .max()
        .unwrap_or(0);
    Ok(Some(peak))
}

/// Clean up 0-byte WAVs immediately and garbage-collect stale temp recordings
/// (and sidecar markers) left behind in `run_dir` for longer than 24 hours.
pub fn cleanup_empty_recordings(run_dir: &Path) {
    let Ok(entries) = fs::read_dir(run_dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !name.starts_with("recording_") {
            continue;
        }
        if name.ends_with(".ready") || name.ends_with(".error") {
            fs::remove_file(&path).ok();
            continue;
        }
        if !name.ends_with(".wav") {
            continue;
        }
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        let is_empty = meta.len() == 0;
        let is_stale = meta
            .modified()
            .ok()
            .and_then(|mtime| now.duration_since(mtime).ok())
            .is_some_and(|age| age > STALE_TEMP_RECORDING_AGE);
        if is_empty || is_stale {
            fs::remove_file(&path).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn write_pcm16_wav(path: &Path, samples: &[i16]) {
        let data_len = (samples.len() * 2) as u32;
        let riff_len = 36 + data_len;
        let mut bytes = Vec::with_capacity(44 + data_len as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&riff_len.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&(16_000u32 * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn ensure_audio_has_signal_rejects_near_silent_wav() {
        let temp = tempdir().unwrap();
        let wav = temp.path().join("silent.wav");
        write_pcm16_wav(&wav, &[0; 1600]);

        let error = ensure_audio_has_signal(&wav).unwrap_err();
        assert!(error.to_string().contains("录音近乎静音"));
    }

    #[test]
    fn ensure_audio_has_signal_accepts_non_silent_wav() {
        let temp = tempdir().unwrap();
        let wav = temp.path().join("speech.wav");
        let samples: Vec<i16> = (0..1600)
            .map(|index| if index % 2 == 0 { 1200 } else { -1200 })
            .collect();
        write_pcm16_wav(&wav, &samples);

        ensure_audio_has_signal(&wav).unwrap();
    }

    #[test]
    fn cleanup_empty_recordings_removes_only_empty_wavs_and_sidecars() {
        let temp = tempdir().unwrap();
        let empty = temp.path().join("recording_empty.wav");
        let nonempty = temp.path().join("recording_ok.wav");
        let sidecar = temp.path().join("recording_ok.wav.ready");
        let other = temp.path().join("notes.txt");
        fs::write(&empty, b"").unwrap();
        fs::write(&nonempty, b"RIFF").unwrap();
        fs::write(&sidecar, b"ready").unwrap();
        fs::write(&other, b"").unwrap();

        cleanup_empty_recordings(temp.path());

        assert!(!empty.exists());
        assert!(nonempty.exists());
        assert!(!sidecar.exists());
        assert!(other.exists());
    }

    #[test]
    fn stop_escalates_when_recorder_ignores_sigint() {
        let temp = tempdir().unwrap();
        let script = temp.path().join("stubborn-recorder.sh");
        fs::write(
            &script,
            "#!/bin/sh\ntrap '' INT\ntrap 'exit 0' TERM\necho RIFF > \"$1\"\nwhile :; do sleep 0.1; done\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let mut config = crate::config::test_support::sample_config();
        config.run_dir = temp.path().join("run");
        config.recorder_cmd = Some(format!("\"{}\"", script.display()));

        let recorder = Recorder::start(&config).unwrap();
        let deadline = Instant::now() + Duration::from_secs(6);
        while !recorder.audio_path.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        let stopped = recorder.stop().unwrap();
        assert!(stopped.is_file());
        assert!(Instant::now() < deadline);
    }
}
