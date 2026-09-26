//! The user's brush presets: one TOML file per brush in the library's
//! `brushes/` dir, next to the built-in ones.

use std::path::{Path, PathBuf};

use acidtrip_core::tools::brush::BrushSpec;

use crate::library::write_atomic;
use crate::stencils::slug;

fn file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.toml", slug(name)))
}

/// Every readable brush in `dir`, sorted by name. Broken files are skipped.
pub fn load(dir: &Path) -> Vec<BrushSpec> {
    let mut out: Vec<BrushSpec> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .filter_map(|p| std::fs::read_to_string(&p).ok())
        .filter_map(|t| toml::from_str::<BrushSpec>(&t).ok())
        .map(BrushSpec::sanitized)
        .collect();
    out.sort_by_key(|b| b.name.to_lowercase());
    out
}

/// Save (or overwrite) a brush under its name.
pub fn save(dir: &Path, spec: &BrushSpec) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = file(dir, &spec.name);
    write_atomic(&path, toml::to_string_pretty(spec)?.as_bytes())?;
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
    use acidtrip_core::tools::brush::GlyphSet;

    #[test]
    fn save_load_delete() {
        let dir = tempfile::tempdir().unwrap();
        let b = BrushSpec { name: "My Chalk!".into(), glyphs: GlyphSet::Shades, grain: 0.4, ..Default::default() };
        let p = save(dir.path(), &b).unwrap();
        assert!(p.ends_with("my-chalk.toml"));
        std::fs::write(dir.path().join("broken.toml"), "size = \"big\"").unwrap();
        std::fs::write(dir.path().join("wild.toml"), "name = \"Wild\"\nsize = 999").unwrap();
        let all = load(dir.path());
        assert_eq!(all.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["My Chalk!", "Wild"]);
        assert_eq!(all[0], b);
        assert_eq!(all[1].size, 64.0, "clamped");
        delete(dir.path(), "My Chalk!").unwrap();
        assert_eq!(load(dir.path()).len(), 1);
    }
}
