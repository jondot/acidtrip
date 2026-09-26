//! `config.toml`. Created with commented defaults on first run.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;

use serde::{Deserialize, Serialize};

use crate::backup::BackupMode;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub keymap: KeymapConfig,
    pub new_doc: NewDocConfig,
    pub autosave_seconds: u64,
    pub version_every_minutes: u64,
    pub backup: BackupMode,
    pub ai: AiConfig,
    pub share: ShareConfig,
    pub ui: UiConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeymapConfig {
    /// "modern" or "acid".
    pub preset: String,
    /// action name -> list of key specs ("ctrl-s", "alt-b", "shift-f1").
    pub bindings: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NewDocConfig {
    /// "classic" or "modern".
    pub kind: String,
    pub width: usize,
    pub height: usize,
    pub ice: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    /// Falls back to the ANTHROPIC_API_KEY env var when empty.
    pub api_key: String,
    pub model: String,
    pub max_tool_rounds: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShareConfig {
    /// Paste host accepting `curl -F file=@x URL` style uploads.
    pub paste_url: String,
    pub png_scale: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub sidebar: bool,
    pub show_grid: bool,
    /// Pixel-exact preview through terminal graphics when supported.
    pub preview: bool,
    pub author: String,
    pub group: String,
    /// Your name in Draw together sessions (empty: your login name).
    pub name: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            keymap: KeymapConfig::default(),
            new_doc: NewDocConfig::default(),
            autosave_seconds: 20,
            version_every_minutes: 5,
            backup: BackupMode::Bak,
            ai: AiConfig::default(),
            share: ShareConfig::default(),
            ui: UiConfig::default(),
        }
    }
}

/// The commented file written on first run. Its values equal `Config::default()`.
pub const DEFAULT_CONFIG_TOML: &str = r#"# acidtrip configuration.
# This file was created with the default values; edit freely. Unknown keys are
# ignored, and anything you delete falls back to its default.

# Seconds between crash-recovery autosaves of unsaved documents (0 = off).
autosave_seconds = 20

# Minutes of editing between automatic version snapshots (0 = only on save).
version_every_minutes = 5

# Backup made before a save overwrites an existing file:
#   "none"     - no backup
#   "bak"      - file.ans -> file.ans.bak (replaced on every save)
#   "numbered" - file.ans -> file.ans.001, .002, ... .999 (keeps them all)
backup = "bak"

[keymap]
# Key preset: "modern" (Ctrl-S save, Ctrl-Z undo, ...) or "acid" (ACiDDraw
# style: Alt-letter commands, F1-F10 character sets).
preset = "modern"

# Per-action overrides on top of the preset. Each action takes a list of key
# specs such as "ctrl-s", "alt-b", "shift-f1", "ctrl-shift-h".
[keymap.bindings]
# save = ["ctrl-s", "f2"]
# undo = ["ctrl-z", "alt-u"]

[new_doc]
# "classic" (CP437 glyphs, 16-color palette; lossless to .ans/.bin/.xb) or
# "modern" (any Unicode char, 24-bit color).
kind = "classic"
width = 80
height = 25
# iCE colors: bright backgrounds instead of blinking text.
ice = true

[ai]
# Anthropic API key. Leave empty to use the ANTHROPIC_API_KEY environment
# variable instead (or set ANTHROPIC_API_KEY).
api_key = ""
# Model used by the prompt bar.
model = "claude-sonnet-5"
# Maximum tool-call rounds per prompt before the AI stops.
max_tool_rounds = 24

[share]
# Paste host that accepts `curl -F file=@art.ans <url>` style uploads.
paste_url = "https://0x0.st"
# Pixel scale for shared PNG renders.
png_scale = 2

[ui]
# Show the tool/palette sidebar.
sidebar = true
# Draw a faint cell grid over the canvas.
show_grid = false
# Pixel-exact preview through terminal graphics (kitty/sixel/iTerm) when supported.
preview = true
# Defaults for the SAUCE author/group fields of new documents.
author = ""
group = ""
# Your name beside your cursor when drawing together (empty: your login name).
name = ""
"#;

impl Default for KeymapConfig {
    fn default() -> Self {
        KeymapConfig { preset: "modern".into(), bindings: BTreeMap::new() }
    }
}

impl Default for NewDocConfig {
    fn default() -> Self {
        NewDocConfig { kind: "classic".into(), width: 80, height: 25, ice: true }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        AiConfig { api_key: String::new(), model: "claude-sonnet-5".into(), max_tool_rounds: 24 }
    }
}

impl Default for ShareConfig {
    fn default() -> Self {
        ShareConfig { paste_url: "https://0x0.st".into(), png_scale: 2 }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            sidebar: true,
            show_grid: false,
            preview: true,
            author: String::new(),
            group: String::new(),
            name: String::new(),
        }
    }
}

impl Config {
    /// Load; if missing, write a commented default file and return defaults.
    /// Unknown keys are ignored; parse errors return the error (the app shows it).
    pub fn load_or_create(path: &Path) -> anyhow::Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text).with_context(|| format!("{}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                crate::library::write_atomic(path, DEFAULT_CONFIG_TOML.as_bytes())?;
                Ok(Config::default())
            }
            Err(e) => Err(anyhow::Error::new(e).context(format!("reading {}", path.display()))),
        }
    }

    /// Parse config text (missing keys default, unknown keys ignored).
    pub fn parse(text: &str) -> anyhow::Result<Config> {
        Ok(toml::from_str(text)?)
    }

    /// The API key from config or `ANTHROPIC_API_KEY`.
    pub fn api_key(&self) -> Option<String> {
        let k = self.ai.api_key.trim();
        if !k.is_empty() {
            return Some(k.to_string());
        }
        std::env::var("ANTHROPIC_API_KEY").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }
}
