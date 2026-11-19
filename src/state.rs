use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct State {
    pub pid: Option<u32>,
    pub mode: Option<String>,
    pub started_at: Option<String>,
    pub recording: bool,
    pub recorder_pid: Option<u32>,
    pub audio_path: Option<String>,
    pub last_event: Option<String>,
    pub last_raw_text: Option<String>,
    pub last_polished_text: Option<String>,
    pub clipboard_copied: Option<bool>,
    pub last_output_path: Option<String>,
    pub error: Option<String>,
}

impl State {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    /// Atomically replace `status.json` via a same-directory temp file so
    /// status bar and tray readers never observe a half-written JSON document.
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let payload = serde_json::to_vec_pretty(self)?;
        let mut temp = tempfile::Builder::new()
            .prefix(".status.")
            .suffix(".tmp")
            .tempfile_in(parent)?;
        temp.write_all(&payload)?;
        temp.as_file().sync_all()?;
        temp.persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("写入状态文件失败: {}", path.display()))?;
        Ok(())
    }
}
