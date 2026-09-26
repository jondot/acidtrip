//! ACiDDraw-style backups before overwriting a file on save.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupMode {
    None,
    /// file.ans -> file.ans.bak
    #[default]
    Bak,
    /// file.ans -> file.ans.001, .002, ... .999
    Numbered,
}

/// If `path` exists, back it up per `mode`. Returns the backup path.
pub fn backup_before_save(path: &Path, mode: BackupMode) -> anyhow::Result<Option<PathBuf>> {
    if mode == BackupMode::None || !path.is_file() {
        return Ok(None);
    }
    let dest = match mode {
        BackupMode::None => unreachable!(),
        BackupMode::Bak => with_suffix(path, "bak"),
        BackupMode::Numbered => (1..=999)
            .map(|n| with_suffix(path, &format!("{n:03}")))
            .find(|p| !p.exists())
            .ok_or_else(|| anyhow::anyhow!("all numbered backups .001-.999 of {} are used", path.display()))?,
    };
    std::fs::copy(path, &dest).with_context(|| format!("backing up {} to {}", path.display(), dest.display()))?;
    Ok(Some(dest))
}

/// `file.ans` + `bak` -> `file.ans.bak`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".");
    s.push(suffix);
    PathBuf::from(s)
}
