//! The user's patterns (pattern brush): one small JSON file per pattern in
//! the library's `patterns/` dir, next to the built-in ones.

use std::path::{Path, PathBuf};

use acidtrip_core::tools::pattern::Pattern;

use crate::library::write_atomic;
use crate::stencils::slug;

fn file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.pattern.json", slug(name)))
}

/// Every readable pattern in `dir`, sorted by name. Broken files are skipped.
pub fn load(dir: &Path) -> Vec<Pattern> {
    let mut out: Vec<Pattern> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".pattern.json"))
        .filter_map(|p| std::fs::read(&p).ok())
        .filter_map(|t| serde_json::from_slice::<Pattern>(&t).ok())
        .map(Pattern::sanitized)
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// Save (or overwrite) a pattern under its name.
pub fn save(dir: &Path, p: &Pattern) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = file(dir, &p.name);
    write_atomic(&path, &serde_json::to_vec(p)?)?;
    Ok(path)
}

pub fn delete(dir: &Path, name: &str) -> anyhow::Result<()> {
    let path = file(dir, name);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::Color;
    use acidtrip_core::tools::pattern::Tile;

    #[test]
    fn save_load_delete() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = Pattern::from_rows("My Bricks!", &["▄▄ ", "▀▀▀"]);
        p.cells[0] = Some(Tile { ch: '▄', fg: Some(Color::Pal(4)), bg: Some(Color::Rgb(9, 8, 7)) });
        let path = save(dir.path(), &p).unwrap();
        assert!(path.ends_with("my-bricks.pattern.json"));
        std::fs::write(dir.path().join("broken.pattern.json"), "{\"name\": 3}").unwrap();
        std::fs::write(dir.path().join("short.pattern.json"), r#"{"name":"Short","width":2,"height":2,"cells":[]}"#)
            .unwrap();
        let all = load(dir.path());
        assert_eq!(all.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["My Bricks!", "Short"]);
        assert_eq!(all[0], p);
        assert_eq!(all[1].cells.len(), 4, "padded");
        delete(dir.path(), "My Bricks!").unwrap();
        assert_eq!(load(dir.path()).len(), 1);
    }
}
