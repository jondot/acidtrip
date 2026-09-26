//! Stencils: reusable clips with metadata, stored as `<id>.stencil.json.zst`
//! under `Paths::stencils_dir()`.

use std::path::{Path, PathBuf};

use acidtrip_core::Clip;
use anyhow::bail;
use serde::{Deserialize, Serialize};

use crate::library::write_atomic;

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct StencilMeta {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    pub author: String,
    pub group: String,
    /// Where it came from (file path, 16colo.rs URL, "drawn", "ai").
    pub source: String,
    pub license: String,
    /// Produced by AI rather than lifted from existing art.
    pub generated: bool,
    pub created: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stencil {
    pub meta: StencilMeta,
    pub clip: Clip,
}

pub struct StencilLibrary {
    builtin: Vec<Stencil>,
    user: Vec<Stencil>,
}

const EXT: &str = ".stencil.json.zst";

impl StencilLibrary {
    /// Load all stencils in `dir` plus the built-in starter set.
    pub fn load(dir: &Path) -> StencilLibrary {
        let mut user: Vec<Stencil> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(EXT)))
            .filter_map(|p| read_stencil(&p).ok())
            .collect();
        user.sort_by(|a, b| a.meta.created.cmp(&b.meta.created).then_with(|| a.meta.id.cmp(&b.meta.id)));
        StencilLibrary { builtin: builtin::starters(), user }
    }

    fn all(&self) -> impl Iterator<Item = &Stencil> {
        self.builtin.iter().chain(&self.user)
    }

    pub fn is_builtin(&self, id: &str) -> bool {
        self.builtin.iter().any(|s| s.meta.id == id)
    }

    /// Built-ins first, then user stencils oldest first.
    pub fn list(&self) -> Vec<StencilMeta> {
        self.all().map(|s| s.meta.clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<Stencil> {
        self.all().find(|s| s.meta.id == id).cloned()
    }

    /// Fuzzy search over name + tags + author (+ group). Every whitespace
    /// separated term must match; best matches first. Empty query lists all.
    pub fn search(&self, query: &str) -> Vec<StencilMeta> {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if terms.is_empty() {
            return self.list();
        }
        let mut hits: Vec<(u32, &StencilMeta)> = self
            .all()
            .filter_map(|s| {
                let total = terms.iter().try_fold(0, |acc, t| match score(&s.meta, t) {
                    0 => None,
                    n => Some(acc + n),
                })?;
                Some((total, &s.meta))
            })
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase())));
        hits.into_iter().map(|(_, m)| m.clone()).collect()
    }

    /// Save (assigns id if empty) and add to the library. Saving over a
    /// built-in id stores a new user copy instead.
    pub fn save(&mut self, dir: &Path, mut stencil: Stencil) -> anyhow::Result<StencilMeta> {
        let m = &mut stencil.meta;
        if m.id.is_empty() || self.is_builtin(&m.id) || !valid_id(&m.id) {
            m.id = new_id(&m.name);
        }
        if m.created.is_empty() {
            m.created = chrono::Utc::now().to_rfc3339();
        }
        let json = serde_json::to_vec(&stencil)?;
        write_atomic(&path_for(dir, &stencil.meta.id), &zstd::encode_all(json.as_slice(), 9)?)?;
        let meta = stencil.meta.clone();
        match self.user.iter_mut().find(|s| s.meta.id == meta.id) {
            Some(slot) => *slot = stencil,
            None => self.user.push(stencil),
        }
        Ok(meta)
    }

    pub fn delete(&mut self, dir: &Path, id: &str) -> anyhow::Result<()> {
        if self.is_builtin(id) {
            bail!("built-in stencil {id:?} can't be deleted");
        }
        let pos = self.user.iter().position(|s| s.meta.id == id).ok_or_else(|| anyhow::anyhow!("no stencil {id:?}"))?;
        match std::fs::remove_file(path_for(dir, id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        self.user.remove(pos);
        Ok(())
    }
}

fn path_for(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}{EXT}"))
}

fn read_stencil(p: &Path) -> anyhow::Result<Stencil> {
    let json = zstd::decode_all(std::fs::read(p)?.as_slice())?;
    Ok(serde_json::from_slice(&json)?)
}

fn valid_id(id: &str) -> bool {
    id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Lowercase ASCII slug of `name` (max 40 chars), "stencil" when empty.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_end_matches('-').chars().take(40).collect();
    let out = out.trim_end_matches('-');
    if out.is_empty() { "stencil".into() } else { out.into() }
}

fn new_id(name: &str) -> String {
    let r = uuid::Uuid::new_v4().simple().to_string();
    format!("{}-{}", slug(name), &r[..6])
}

/// Relevance of one lowercase term against a stencil's metadata (0 = no match).
fn score(m: &StencilMeta, term: &str) -> u32 {
    let name = m.name.to_lowercase();
    let mut best = if name.starts_with(term) {
        100
    } else if name.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(term)) {
        80
    } else if name.contains(term) {
        60
    } else if is_subsequence(term, &name) {
        15
    } else {
        0
    };
    for t in &m.tags {
        let t = t.to_lowercase();
        best = best.max(if t == term {
            50
        } else if t.contains(term) {
            30
        } else if is_subsequence(term, &t) {
            8
        } else {
            0
        });
    }
    for f in [&m.author, &m.group] {
        let f = f.to_lowercase();
        best = best.max(if f.contains(term) {
            25
        } else if is_subsequence(term, &f) {
            5
        } else {
            0
        });
    }
    best
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut h = hay.chars();
    needle.chars().all(|n| h.any(|c| c == n))
}

mod builtin;
