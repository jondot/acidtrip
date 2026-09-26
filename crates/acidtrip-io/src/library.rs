//! Paths. Everything can be redirected with `ACIDTRIP_HOME` (used by tests
//! and the harness) — then config/data/state all live under that dir.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;

#[derive(Clone, Debug)]
pub struct Paths {
    /// config.toml, keymap overrides.
    pub config_dir: PathBuf,
    /// library/, versions/.
    pub data_dir: PathBuf,
    /// recovery/, sockets, logs.
    pub state_dir: PathBuf,
}

impl Paths {
    /// Resolve (ACIDTRIP_HOME or platform dirs via `directories`) and create dirs.
    pub fn resolve() -> anyhow::Result<Paths> {
        let paths = match std::env::var_os("ACIDTRIP_HOME").filter(|v| !v.is_empty()) {
            Some(home) => Paths::under(Path::new(&home)),
            None => {
                let pd =
                    directories::ProjectDirs::from("", "", "acidtrip").context("cannot determine home directory")?;
                let state = pd.state_dir().unwrap_or_else(|| pd.data_local_dir()).to_path_buf();
                Paths {
                    config_dir: pd.config_dir().to_path_buf(),
                    data_dir: pd.data_dir().to_path_buf(),
                    state_dir: state,
                }
            }
        };
        paths.create_dirs()?;
        Ok(paths)
    }

    /// Everything under one root (`<root>/{config,data,state}`), not created.
    pub fn under(root: &Path) -> Paths {
        Paths { config_dir: root.join("config"), data_dir: root.join("data"), state_dir: root.join("state") }
    }

    /// Create every directory this layout uses.
    pub fn create_dirs(&self) -> anyhow::Result<()> {
        for d in [
            self.config_dir.clone(),
            self.fonts_dir(),
            self.stencils_dir(),
            self.palettes_dir(),
            self.charsets_dir(),
            self.brushes_dir(),
            self.patterns_dir(),
            self.versions_dir(),
            self.recovery_dir(),
            self.sockets_dir(),
        ] {
            std::fs::create_dir_all(&d).with_context(|| format!("creating {}", d.display()))?;
        }
        Ok(())
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn fonts_dir(&self) -> PathBuf {
        self.data_dir.join("library").join("fonts")
    }

    pub fn stencils_dir(&self) -> PathBuf {
        self.data_dir.join("library").join("stencils")
    }

    pub fn palettes_dir(&self) -> PathBuf {
        self.data_dir.join("library").join("palettes")
    }

    pub fn brushes_dir(&self) -> PathBuf {
        self.data_dir.join("library").join("brushes")
    }

    /// The pattern brush's saved patterns.
    pub fn patterns_dir(&self) -> PathBuf {
        self.data_dir.join("library").join("patterns")
    }

    /// The art-mode keyboard's changed keys.
    pub fn artboard_file(&self) -> PathBuf {
        self.data_dir.join("library").join("artboard.toml")
    }

    pub fn charsets_dir(&self) -> PathBuf {
        self.data_dir.join("library").join("charsets")
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.data_dir.join("versions")
    }

    pub fn recovery_dir(&self) -> PathBuf {
        self.state_dir.join("recovery")
    }

    /// Unix socket paths are limited to ~104 bytes (macOS). When the state dir
    /// is too deep, use a short per-home dir under /tmp; every process with the
    /// same home computes the same path.
    pub fn sockets_dir(&self) -> PathBuf {
        let dir = self.state_dir.join("sockets");
        if dir.as_os_str().len() + 16 <= 100 {
            return dir;
        }
        let tag = blake3::hash(self.state_dir.as_os_str().as_encoded_bytes()).to_hex();
        PathBuf::from("/tmp").join(format!("acidtrip-{}", &tag[..12]))
    }

    pub fn log_file(&self) -> PathBuf {
        self.state_dir.join("acidtrip.log")
    }
}

/// Write `bytes` to `path` atomically (temp file in the same dir, then rename).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    // The temp file's random name means nothing to anyone: name the folder.
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .map_err(|e| anyhow::anyhow!("can't write in {}: {}", dir.display(), e.kind()))?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    // Temp files are created 0600; keep an existing file's mode, else use 0644.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path).map(|m| m.permissions().mode() & 0o7777).unwrap_or(0o644);
        tmp.as_file().set_permissions(std::fs::Permissions::from_mode(mode))?;
    }
    tmp.persist(path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn write_atomic_names_the_folder_it_cant_write_in() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ro = dir.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        let r = write_atomic(&ro.join("a.ans"), b"x");
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Root writes anywhere.
        if std::env::var("USER").is_ok_and(|u| u == "root") {
            return;
        }
        let e = format!("{:#}", r.unwrap_err());
        assert!(e.starts_with("can't write in ") && e.contains("/ro: "), "{e}");
        assert!(!e.contains("ro/.tmp"), "{e}");
    }
}
