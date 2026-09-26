//! Clipboard and sharing: system clipboard (arboard, OSC52 fallback over
//! SSH), PNG to clipboard, GitHub gist (`gh`), paste host (`curl`).

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub fn over_ssh() -> bool {
    std::env::var_os("SSH_TTY").is_some() || std::env::var_os("SSH_CONNECTION").is_some()
}

fn osc52(text: &str) -> anyhow::Result<()> {
    crossterm::execute!(std::io::stdout(), crossterm::clipboard::CopyToClipboard::to_clipboard_from(text))?;
    Ok(())
}

pub fn copy_text(text: &str) -> anyhow::Result<()> {
    if over_ssh() {
        return osc52(text);
    }
    match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())) {
        Ok(()) => Ok(()),
        Err(_) => osc52(text),
    }
}

pub fn paste_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok().filter(|s| !s.is_empty())
}

pub fn copy_image(png: &[u8]) -> anyhow::Result<()> {
    // arboard would reach the remote machine's clipboard (or none at all),
    // and terminals only take text over OSC 52.
    if over_ssh() {
        anyhow::bail!("images can't be copied over SSH: use 5 (upload) or 6 (export)");
    }
    let img = image::load_from_memory(png)?.to_rgba8();
    let (w, h) = img.dimensions();
    arboard::Clipboard::new()?.set_image(arboard::ImageData {
        width: w as usize,
        height: h as usize,
        bytes: img.into_raw().into(),
    })?;
    Ok(())
}

/// Create a public gist with the given files via the GitHub CLI; returns the URL.
pub fn gist(files: &[&Path], description: &str) -> anyhow::Result<String> {
    let mut cmd = Command::new("gh");
    cmd.args(["gist", "create", "--public", "-d", description]);
    for f in files {
        cmd.arg(f);
    }
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| anyhow::anyhow!("GitHub CLI `gh` not available ({e}); install it and run `gh auth login`"))?;
    if !out.status.success() {
        anyhow::bail!("gh: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().lines().last().unwrap_or_default().to_string())
}

/// Upload a file to a paste host (`curl -F file=@path URL`); returns the URL.
pub fn paste_host(file: &Path, url: &str) -> anyhow::Result<String> {
    let out = Command::new("curl")
        .args(["-sS", "-f", "-A", concat!("acidtrip/", env!("CARGO_PKG_VERSION")), "-F"])
        .arg(format!("file=@{}", file.display()))
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| anyhow::anyhow!("curl not available: {e}"))?;
    if !out.status.success() {
        anyhow::bail!("upload failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let link = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !link.starts_with("http") {
        anyhow::bail!("unexpected response from {url}: {link}");
    }
    Ok(link)
}

/// Write bytes to a temp file with the given name (kept until process exit).
pub fn temp_file(name: &str, bytes: &[u8]) -> anyhow::Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("acidtrip-share-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let p = dir.join(name);
    std::fs::File::create(&p)?.write_all(bytes)?;
    Ok(p)
}
