//! Content-addressed version snapshots per document id:
//! `versions/<doc-id>/<blake3>.acid` (zstd) + `index.json`.

use std::path::{Path, PathBuf};

use acidtrip_core::Document;
use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use crate::library::write_atomic;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VersionInfo {
    /// blake3 hex of the serialized doc.
    pub hash: String,
    /// RFC3339.
    pub timestamp: String,
    /// "save", "auto", or a user name.
    pub label: String,
    /// Path the doc was saved to at the time, if any.
    pub file: Option<String>,
    /// First few non-blank rows as plain text, for the browser list.
    pub preview: String,
}

pub struct VersionStore {
    dir: PathBuf,
    /// Oldest first (as stored in index.json).
    index: Vec<VersionInfo>,
}

impl VersionStore {
    pub fn open(versions_dir: &Path, doc_id: uuid::Uuid) -> anyhow::Result<VersionStore> {
        let dir = versions_dir.join(doc_id.to_string());
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let index = match std::fs::read(dir.join(INDEX)) {
            Ok(b) => serde_json::from_slice(&b).with_context(|| format!("corrupt {}", dir.join(INDEX).display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(VersionStore { dir, index })
    }

    /// Snapshot; identical content is deduplicated (returns the existing info
    /// with the new label appended).
    pub fn snapshot(&mut self, doc: &Document, label: &str, file: Option<&Path>) -> anyhow::Result<VersionInfo> {
        let bytes = crate::native::to_bytes(doc)?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let file = file.map(|f| f.display().to_string());
        let info = if let Some(pos) = self.index.iter().position(|v| v.hash == hash) {
            let mut v = self.index.remove(pos);
            if !label.is_empty() && !v.label.split(", ").any(|l| l == label) {
                v.label = if v.label.is_empty() { label.to_string() } else { format!("{}, {label}", v.label) };
            }
            v.timestamp = now;
            if file.is_some() {
                v.file = file;
            }
            v
        } else {
            write_atomic(&self.dir.join(format!("{hash}.acid")), &bytes)?;
            VersionInfo { hash, timestamp: now, label: label.to_string(), file, preview: preview(doc) }
        };
        self.index.push(info.clone());
        self.save_index()?;
        Ok(info)
    }

    /// Newest first.
    pub fn list(&self) -> Vec<VersionInfo> {
        self.index.iter().rev().cloned().collect()
    }

    pub fn load(&self, hash: &str) -> anyhow::Result<Document> {
        if hash.is_empty() || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("invalid version hash {hash:?}");
        }
        let path = self.dir.join(format!("{hash}.acid"));
        let bytes = std::fs::read(&path).with_context(|| format!("version {hash} not found"))?;
        crate::native::from_bytes(&bytes)
    }

    pub fn rename(&mut self, hash: &str, label: &str) -> anyhow::Result<()> {
        let v = self
            .index
            .iter_mut()
            .find(|v| v.hash == hash)
            .ok_or_else(|| anyhow::anyhow!("version {hash} not found"))?;
        v.label = label.to_string();
        self.save_index()
    }

    fn save_index(&self) -> anyhow::Result<()> {
        write_atomic(&self.dir.join(INDEX), &serde_json::to_vec_pretty(&self.index)?)
    }
}

/// The version store id for art kept in a file that has no document id of
/// its own (.ans, .xb, …): the same file gets the same id every session, so
/// its history is there when it's reopened.
pub fn file_doc_id(path: &Path) -> uuid::Uuid {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map(|d| d.join(path)).unwrap_or_else(|_| path.to_path_buf())
    };
    // Resolve the folder (it exists even before the first save; the file may not).
    let key = match (abs.parent().and_then(|d| d.canonicalize().ok()), abs.file_name()) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => abs,
    };
    let hash = blake3::hash(format!("acidtrip-file:{}", key.display()).as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash.as_bytes()[..16]);
    uuid::Builder::from_custom_bytes(bytes).into_uuid()
}

/// Which version store holds `doc`'s history: a .acid keeps its document id
/// inside; any other art file is known by its path.
pub fn store_id(doc: &Document, file: Option<&Path>) -> uuid::Uuid {
    use crate::format::Format;
    match file {
        Some(p) if Format::from_path(p).is_some_and(|f| f.reopens() && f != Format::Acid) => file_doc_id(p),
        _ => doc.meta.id,
    }
}

const INDEX: &str = "index.json";
const PREVIEW_ROWS: usize = 6;
const PREVIEW_COLS: usize = 80;

/// First few non-blank rows of the flattened doc as plain text.
pub fn preview(doc: &Document) -> String {
    let g = doc.flatten();
    (0..g.height)
        .map(|y| {
            let row: String =
                g.row(y).iter().map(|c| if c.ch.is_control() { ' ' } else { c.ch }).take(PREVIEW_COLS).collect();
            row.trim_end().to_string()
        })
        .filter(|r| !r.trim().is_empty())
        .take(PREVIEW_ROWS)
        .collect::<Vec<_>>()
        .join("\n")
}
