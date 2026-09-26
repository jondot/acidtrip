//! Crash recovery: autosave dirty docs to `recovery/<doc-id>.acid` with a
//! small sidecar describing the original file path.

use std::path::{Path, PathBuf};

use acidtrip_core::Document;
use acidtrip_core::replay::EditLog;
use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::library::write_atomic;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecoveryEntry {
    pub doc_id: uuid::Uuid,
    pub file: Option<PathBuf>,
    /// RFC3339.
    pub saved_at: String,
    pub path: PathBuf,
}

/// Keep `doc` and its edit log (so replay survives a crash).
pub fn write(recovery_dir: &Path, doc: &Document, file: Option<&Path>, log: Option<&EditLog>) -> anyhow::Result<()> {
    let id = doc.meta.id;
    let path = recovery_dir.join(format!("{id}.acid"));
    write_atomic(&path, &crate::format::native::save(doc, log)?)?;
    let entry = RecoveryEntry {
        doc_id: id,
        file: file.map(Path::to_path_buf),
        saved_at: chrono::Utc::now().to_rfc3339(),
        path,
    };
    write_atomic(&recovery_dir.join(format!("{id}.json")), &serde_json::to_vec_pretty(&entry)?)
}

/// All recoverable docs, newest first. Entries whose data file is gone are skipped.
pub fn list(recovery_dir: &Path) -> Vec<RecoveryEntry> {
    let Ok(rd) = std::fs::read_dir(recovery_dir) else {
        return Vec::new();
    };
    let mut out: Vec<RecoveryEntry> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| serde_json::from_slice::<RecoveryEntry>(&std::fs::read(p).ok()?).ok())
        .filter(|e| e.path.is_file())
        .collect();
    out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    out
}

pub fn load(entry: &RecoveryEntry) -> anyhow::Result<Document> {
    Ok(load_with_log(entry)?.0)
}

/// The document and the edit log kept with it (none in older recovery files).
pub fn load_with_log(entry: &RecoveryEntry) -> anyhow::Result<(Document, Option<EditLog>)> {
    let bytes = std::fs::read(&entry.path).with_context(|| format!("reading {}", entry.path.display()))?;
    crate::format::native::load_with_log(&bytes)
}

/// Remove after a clean save/close.
pub fn clear(recovery_dir: &Path, doc_id: uuid::Uuid) {
    for ext in ["acid", "json"] {
        let _ = std::fs::remove_file(recovery_dir.join(format!("{doc_id}.{ext}")));
    }
}
