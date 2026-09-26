//! Keymaps: presets + config overrides, mapping normalized key specs
//! ("ctrl-s", "alt-shift-i", "f1", "?") to actions in a context.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::actions::Action;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ctx {
    /// Works everywhere on the canvas.
    Global,
    /// Only when a drawing tool (not the text tool) is active: single-letter keys.
    Tool,
    /// Only while typing with the text tool.
    Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preset {
    Modern,
    Acid,
}

impl Preset {
    pub fn parse(s: &str) -> Preset {
        if s.eq_ignore_ascii_case("acid") || s.eq_ignore_ascii_case("aciddraw") { Preset::Acid } else { Preset::Modern }
    }
}

pub struct Keymap {
    pub preset: Preset,
    map: HashMap<(Ctx, String), Action>,
    /// Action -> keys (first is shown in menus).
    keys: HashMap<Action, Vec<(Ctx, String)>>,
}

const MODERN: &[(Action, Ctx, &[&str])] = {
    use Action::*;
    use Ctx::*;
    &[
        (New, Global, &["ctrl-n"]),
        (Open, Global, &["ctrl-o"]),
        (Save, Global, &["ctrl-s"]),
        (SaveAs, Global, &["ctrl-shift-s", "alt-s"]),
        (Export, Global, &["ctrl-e"]),
        (ExportNow, Global, &["alt-shift-e"]),
        (Share, Global, &["ctrl-shift-e", "alt-e"]),
        (Quit, Global, &["ctrl-q"]),
        (Versions, Global, &["alt-v"]),
        (Undo, Global, &["ctrl-z"]),
        (Redo, Global, &["ctrl-y", "ctrl-shift-z"]),
        (Copy, Global, &["ctrl-c"]),
        (Cut, Global, &["ctrl-x"]),
        (Paste, Global, &["ctrl-v"]),
        (SelectAll, Global, &["ctrl-a"]),
        (Deselect, Tool, &["esc"]),
        (DeleteSelection, Tool, &["delete", "backspace"]),
        (BlockMenu, Tool, &["enter"]),
        (InsertLine, Global, &["alt-i"]),
        (DeleteLine, Global, &["alt-y"]),
        (InsertColumn, Global, &["alt-shift-i"]),
        (DeleteColumn, Global, &["alt-shift-y"]),
        (ToolSelect, Tool, &["v"]),
        (ToolText, Tool, &["t"]),
        (ToolBrush, Tool, &["b"]),
        (ToolPen, Tool, &["p"]),
        (ToolPixel, Tool, &["h"]),
        (ToolLine, Tool, &["l"]),
        (ToolRect, Tool, &["r"]),
        (ToolEllipse, Tool, &["o"]),
        (ToolFill, Tool, &["g"]),
        (ToolGradient, Tool, &["d"]),
        (ToolPicker, Tool, &["i"]),
        (ToolFont, Tool, &["f"]),
        (ToolStencil, Tool, &["n"]),
        (ToolPattern, Tool, &["w"]),
        (PatternFromSelection, Tool, &["shift-w"]),
        (ToolShade, Tool, &["s"]),
        (ToolColorize, Tool, &["c"]),
        (ToolErase, Tool, &["e"]),
        (ToolFilters, Tool, &["shift-f"]),
        (ToolRecolor, Tool, &["shift-c"]),
        (ToolOption, Tool, &["tab"]),
        (ToolStyle, Tool, &["shift-tab", "backtab"]),
        (Mirror, Tool, &["m"]),
        (BrushStudio, Tool, &["shift-p"]),
        (BrushBigger, Tool, &["="]),
        (BrushSmaller, Tool, &["-"]),
        (Zoom, Tool, &["z"]),
        (SwapColors, Tool, &["x"]),
        (FgNext, Tool, &["."]),
        (FgPrev, Tool, &[","]),
        (BgNext, Tool, &["'"]),
        (BgPrev, Tool, &[";"]),
        (FgNext, Global, &["ctrl-down"]),
        (FgPrev, Global, &["ctrl-up"]),
        (BgNext, Global, &["ctrl-right"]),
        (BgPrev, Global, &["ctrl-left"]),
        (PickUnderCursor, Global, &["alt-u"]),
        (ColorDialog, Global, &["alt-c"]),
        (CharPicker, Global, &["alt-k"]),
        (ArtMode, Global, &["ctrl-g"]),
        (Gallery, Global, &["ctrl-f"]),
        (TogetherPanel, Global, &["alt-t"]),
        (CharsetNext, Tool, &["]"]),
        (CharsetPrev, Tool, &["["]),
        (CharsetNext, Global, &["alt-n"]),
        (CharsetPrev, Global, &["alt-p"]),
        (Glyph1, Tool, &["1"]),
        (Glyph2, Tool, &["2"]),
        (Glyph3, Tool, &["3"]),
        (Glyph4, Tool, &["4"]),
        (Glyph5, Tool, &["5"]),
        (Glyph6, Tool, &["6"]),
        (Glyph7, Tool, &["7"]),
        (Glyph8, Tool, &["8"]),
        (Glyph9, Tool, &["9"]),
        (Glyph10, Tool, &["0"]),
        (Glyph1, Global, &["f1", "alt-1"]),
        (Glyph2, Global, &["f2", "alt-2"]),
        (Glyph3, Global, &["f3", "alt-3"]),
        (Glyph4, Global, &["f4", "alt-4"]),
        (Glyph5, Global, &["f5", "alt-5"]),
        (Glyph6, Global, &["f6", "alt-6"]),
        (Glyph7, Global, &["f7", "alt-7"]),
        (Glyph8, Global, &["f8", "alt-8"]),
        (Glyph9, Global, &["f9", "alt-9"]),
        (Glyph10, Global, &["f10", "alt-0"]),
        (DocProperties, Global, &["ctrl-d"]),
        (Sauce, Global, &["alt-d"]),
        (ToggleIce, Global, &["alt-z"]),
        (LayerUp, Global, &["alt-up"]),
        (LayerDown, Global, &["alt-down"]),
        (LayerAdd, Global, &["alt-shift-n"]),
        (Sidebar, Global, &["ctrl-b"]),
        (Minimap, Global, &["alt-m"]),
        (LayersPanel, Global, &["ctrl-l"]),
        (LayerDuplicate, Global, &["alt-shift-d"]),
        // Frames go sideways where layers go up and down.
        (FrameNext, Tool, &[">"]),
        (FramePrev, Tool, &["<"]),
        (FrameNext, Global, &["alt-right"]),
        (FramePrev, Global, &["alt-left"]),
        (FrameAdd, Global, &["alt-f"]),
        (FrameDuplicate, Global, &["alt-shift-f"]),
        (FramePlay, Global, &["alt-shift-p"]),
        (Grid, Global, &["alt-g"]),
        (Preview, Global, &["alt-w"]),
        (PlayBaud, Global, &["alt-shift-w"]),
        (Replay, Tool, &["shift-r"]),
        (Replay, Global, &["alt-shift-r"]),
        (Up, Global, &["up"]),
        (Down, Global, &["down"]),
        (Left, Global, &["left"]),
        (Right, Global, &["right"]),
        (PageUp, Global, &["pageup"]),
        (PageDown, Global, &["pagedown"]),
        (LineStart, Global, &["home"]),
        (LineEnd, Global, &["end"]),
        (FirstChar, Global, &["ctrl-home"]),
        (LastChar, Global, &["ctrl-end"]),
        (TabStop, Text, &["tab"]),
        (Apply, Tool, &["space"]),
        (DrawUp, Tool, &["shift-up"]),
        (DrawDown, Tool, &["shift-down"]),
        (DrawLeft, Tool, &["shift-left"]),
        (DrawRight, Tool, &["shift-right"]),
        (AiPrompt, Tool, &["a"]),
        (AiPrompt, Global, &["ctrl-/", "ctrl-7", "alt-a"]),
        (CommandPalette, Global, &["ctrl-k", "ctrl-p"]),
        (Help, Tool, &["?"]),
        (Help, Global, &["f12", "alt-h"]),
    ]
};

/// ACiDDraw-faithful additions (layered on top of modern, which remains as fallback).
const ACID: &[(Action, Ctx, &[&str])] = {
    use Action::*;
    use Ctx::*;
    &[
        (BlockMenu, Global, &["alt-b"]),
        (Undo, Global, &["alt-r"]),
        (Save, Global, &["alt-s"]),
        (Open, Global, &["alt-l"]),
        (Sauce, Global, &["alt-d"]),
        (ClearCanvas, Global, &["alt-c"]),
        (Quit, Global, &["alt-x"]),
        (Help, Global, &["alt-h"]),
        (ToggleIce, Global, &["alt-z"]),
        (PickUnderCursor, Global, &["alt-u"]),
        (ColorDialog, Text, &["esc"]),
        (DeleteLine, Global, &["ctrl-y"]),
        (InsertLine, Global, &["alt-i"]),
        (Redo, Global, &["ctrl-shift-z"]),
        (DocProperties, Global, &["alt-o"]),
    ]
};

impl Keymap {
    /// The keymap a config asks for: its preset (by name) plus its
    /// overrides, and what in them couldn't be used.
    pub fn from_config(
        preset: &str,
        overrides: &std::collections::BTreeMap<String, Vec<String>>,
    ) -> (Keymap, Vec<String>) {
        let (km, mut errs) = Keymap::new(Preset::parse(preset), overrides);
        let known = ["modern", "acid", "aciddraw", ""];
        if !known.iter().any(|k| preset.trim().eq_ignore_ascii_case(k)) {
            errs.insert(0, format!("keymap: unknown preset \"{preset}\" (modern or acid), using modern"));
        }
        (km, errs)
    }

    pub fn new(preset: Preset, overrides: &std::collections::BTreeMap<String, Vec<String>>) -> (Keymap, Vec<String>) {
        let mut km = Keymap { preset: preset.clone(), map: HashMap::new(), keys: HashMap::new() };
        for (a, ctx, keys) in MODERN {
            for k in *keys {
                km.bind(*a, *ctx, k);
            }
        }
        if preset == Preset::Acid {
            for (a, ctx, keys) in ACID {
                // An ACiDDraw binding steals the key from whatever modern action had it.
                for k in *keys {
                    km.unbind_key(*ctx, k);
                    km.bind(*a, *ctx, k);
                    // ...and is the key menus show, unless it only works while typing.
                    if *ctx == Ctx::Global
                        && let Some(v) = km.keys.get_mut(a)
                    {
                        v.rotate_right(1);
                    }
                }
            }
        }
        let mut errors = vec![];
        for (id, keys) in overrides {
            let Some(action) = Action::from_id(id) else {
                errors.push(format!("keymap: unknown action \"{id}\""));
                continue;
            };
            let home = km.keys.get(&action).and_then(|v| v.first()).map(|(c, _)| *c).unwrap_or(Ctx::Global);
            km.unbind_action(action);
            for k in keys {
                match normalize_spec(k) {
                    Some(n) => {
                        // A key that types a character is a tool key: bound
                        // globally it would be shadowed by the tool letters and
                        // would eat that character while typing text.
                        let ctx = if home == Ctx::Global && types_char(&n) { Ctx::Tool } else { home };
                        if ctx == Ctx::Global {
                            // ...and a global key wins over whatever had it anywhere.
                            for c in [Ctx::Global, Ctx::Tool, Ctx::Text] {
                                km.unbind_key(c, &n);
                            }
                        } else {
                            km.unbind_key(ctx, &n);
                        }
                        km.bind(action, ctx, &n);
                    }
                    None => errors.push(format!("keymap: can't parse key \"{k}\" for {id}")),
                }
            }
        }
        (km, errors)
    }

    fn bind(&mut self, a: Action, ctx: Ctx, key: &str) {
        let key = normalize_spec(key).unwrap_or_else(|| key.to_string());
        self.map.insert((ctx, key.clone()), a);
        self.keys.entry(a).or_default().push((ctx, key));
    }

    fn unbind_key(&mut self, ctx: Ctx, key: &str) {
        if let Some(a) = self.map.remove(&(ctx, key.to_string()))
            && let Some(v) = self.keys.get_mut(&a)
        {
            v.retain(|(c, k)| !(*c == ctx && k == key));
        }
    }

    fn unbind_action(&mut self, a: Action) {
        if let Some(v) = self.keys.remove(&a) {
            for (c, k) in v {
                self.map.remove(&(c, k));
            }
        }
    }

    /// Look up a key in text mode (`typing`) or tool mode.
    pub fn lookup(&self, key: &KeyEvent, typing: bool) -> Option<Action> {
        let spec = key_spec(key)?;
        let ctx = if typing { Ctx::Text } else { Ctx::Tool };
        self.map.get(&(ctx, spec.clone())).or_else(|| self.map.get(&(Ctx::Global, spec))).copied()
    }

    /// First key bound to an action, for display ("Ctrl-S").
    pub fn key_for(&self, a: Action) -> Option<String> {
        self.keys.get(&a).and_then(|v| v.first()).map(|(_, k)| display_key(k))
    }

    pub fn keys_for(&self, a: Action) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        // Two specs can read the same ("shift-tab" and "backtab").
        for (_, k) in self.keys.get(&a).into_iter().flatten() {
            let d = display_key(k);
            if !out.contains(&d) {
                out.push(d);
            }
        }
        out
    }
}

/// Canonical spec for a key event: modifiers in ctrl-alt-shift order, then
/// the key. Uppercase letters become shift-<lower>; shifted symbols drop shift.
pub fn key_spec(key: &KeyEvent) -> Option<String> {
    let mut mods = key.modifiers;
    let name = match key.code {
        KeyCode::Char(c) => {
            if c.is_ascii_uppercase() {
                mods |= KeyModifiers::SHIFT;
                c.to_ascii_lowercase().to_string()
            } else if c == ' ' {
                "space".into()
            } else {
                if !c.is_ascii_alphanumeric() {
                    mods -= KeyModifiers::SHIFT;
                }
                c.to_string()
            }
        }
        KeyCode::F(n) => format!("f{n}"),
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => {
            mods -= KeyModifiers::SHIFT;
            "backtab".into()
        }
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Insert => "insert".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        _ => return None,
    };
    let mut s = String::new();
    if mods.contains(KeyModifiers::CONTROL) {
        s.push_str("ctrl-");
    }
    if mods.contains(KeyModifiers::ALT) || mods.contains(KeyModifiers::META) {
        s.push_str("alt-");
    }
    if mods.contains(KeyModifiers::SHIFT) {
        s.push_str("shift-");
    }
    s.push_str(&name);
    Some(s)
}

/// Whether a normalized spec types a character (no ctrl/alt; a letter,
/// digit, symbol or space).
fn types_char(spec: &str) -> bool {
    if spec.starts_with("ctrl-") || spec.starts_with("alt-") {
        return false;
    }
    let rest = spec.strip_prefix("shift-").filter(|r| !r.is_empty()).unwrap_or(spec);
    rest == "space" || rest.chars().count() == 1
}

/// Normalize a user-written spec ("Ctrl+Shift+S", "alt-B", "F1").
pub fn normalize_spec(spec: &str) -> Option<String> {
    let lower = spec.trim().to_lowercase().replace('+', "-");
    if lower.is_empty() {
        return None;
    }
    // Keep a literal "-" key working ("ctrl--" or "-").
    let (mods_part, key) = match lower.rsplit_once('-') {
        Some((m, "")) => (m.trim_end_matches('-'), "-"),
        Some((m, k)) => (m, k),
        None => ("", lower.as_str()),
    };
    let (mut ctrl, mut alt, mut shift) = (false, false, false);
    for m in mods_part.split('-').filter(|m| !m.is_empty()) {
        match m {
            "ctrl" | "control" | "c" => ctrl = true,
            "alt" | "option" | "opt" | "meta" | "m" => alt = true,
            "shift" | "s" => shift = true,
            _ => return None,
        }
    }
    let key = match key {
        "return" => "enter",
        "escape" => "esc",
        "del" => "delete",
        "ins" => "insert",
        "pgup" => "pageup",
        "pgdn" | "pgdown" => "pagedown",
        " " => "space",
        k => k,
    };
    let valid = key.chars().count() == 1
        || matches!(
            key,
            "enter"
                | "esc"
                | "tab"
                | "backtab"
                | "backspace"
                | "delete"
                | "insert"
                | "home"
                | "end"
                | "pageup"
                | "pagedown"
                | "up"
                | "down"
                | "left"
                | "right"
                | "space"
        )
        || (key.starts_with('f') && key[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)));
    if !valid {
        return None;
    }
    // Shifted symbols don't carry shift (matches key_spec).
    if shift && key.chars().count() == 1 && !key.chars().next().unwrap().is_ascii_alphanumeric() {
        shift = false;
    }
    let mut s = String::new();
    if ctrl {
        s.push_str("ctrl-");
    }
    if alt {
        s.push_str("alt-");
    }
    if shift {
        s.push_str("shift-");
    }
    s.push_str(key);
    Some(s)
}

/// "ctrl-shift-s" -> "Ctrl-Shift-S".
pub fn display_key(spec: &str) -> String {
    let mut parts: Vec<String> = vec![];
    let (mods, key) = match spec.rsplit_once('-') {
        Some((m, "")) => (m.trim_end_matches('-'), "-"),
        Some((m, k)) => (m, k),
        None => ("", spec),
    };
    for m in mods.split('-').filter(|m| !m.is_empty()) {
        parts.push(match m {
            "ctrl" => "Ctrl".into(),
            "alt" => "Alt".into(),
            "shift" => "Shift".into(),
            o => o.into(),
        });
    }
    let k = match key {
        "pageup" => "PgUp".to_string(),
        "pagedown" => "PgDn".to_string(),
        "backtab" => "Shift-Tab".to_string(),
        k if k.len() == 1 => k.to_uppercase(),
        k => {
            let mut c = k.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    };
    parts.push(k);
    parts.join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    #[test]
    fn specs() {
        assert_eq!(key_spec(&ev(KeyCode::Char('s'), KeyModifiers::CONTROL)).unwrap(), "ctrl-s");
        assert_eq!(key_spec(&ev(KeyCode::Char('I'), KeyModifiers::ALT)).unwrap(), "alt-shift-i");
        assert_eq!(key_spec(&ev(KeyCode::Char('?'), KeyModifiers::SHIFT)).unwrap(), "?");
        assert_eq!(key_spec(&ev(KeyCode::F(3), KeyModifiers::NONE)).unwrap(), "f3");
        assert_eq!(normalize_spec("Ctrl+Shift+S").unwrap(), "ctrl-shift-s");
        assert_eq!(normalize_spec("alt-B").unwrap(), "alt-b");
        assert_eq!(normalize_spec("ctrl--").unwrap(), "ctrl--");
        assert!(normalize_spec("hyper-x").is_none());
        assert_eq!(display_key("ctrl-shift-s"), "Ctrl-Shift-S");
    }

    #[test]
    fn keys_read_once() {
        let (km, _) = Keymap::new(Preset::Modern, &Default::default());
        assert_eq!(km.keys_for(Action::ToolStyle), ["Shift-Tab"]);
    }

    #[test]
    fn modern_lookup_by_context() {
        let (km, errs) = Keymap::new(Preset::Modern, &Default::default());
        assert!(errs.is_empty());
        let b = ev(KeyCode::Char('b'), KeyModifiers::NONE);
        assert_eq!(km.lookup(&b, false), Some(Action::ToolBrush));
        assert_eq!(km.lookup(&b, true), None, "letters type in text mode");
        assert_eq!(km.lookup(&ev(KeyCode::Char('s'), KeyModifiers::CONTROL), true), Some(Action::Save));
        let gt = ev(KeyCode::Char('>'), KeyModifiers::SHIFT);
        assert_eq!(km.lookup(&gt, false), Some(Action::FrameNext));
        assert_eq!(km.lookup(&gt, true), None, "> types in text mode");
        assert_eq!(km.lookup(&ev(KeyCode::Left, KeyModifiers::ALT), true), Some(Action::FramePrev));
    }

    #[test]
    fn frame_keys_are_free_in_both_presets() {
        for preset in [Preset::Modern, Preset::Acid] {
            let (km, _) = Keymap::new(preset, &Default::default());
            for a in [Action::FrameNext, Action::FramePrev, Action::FrameAdd, Action::FrameDuplicate, Action::FramePlay] {
                assert!(km.key_for(a).is_some(), "{a:?}");
                assert!(!km.keys_for(a).iter().any(|k| k.starts_with('F') && k.len() > 1 && k[1..].parse::<u8>().is_ok()));
            }
        }
    }

    #[test]
    fn export_keys_are_free_in_both_presets() {
        for preset in [Preset::Modern, Preset::Acid] {
            let (km, _) = Keymap::new(preset, &Default::default());
            assert_eq!(km.lookup(&ev(KeyCode::Char('e'), KeyModifiers::CONTROL), false), Some(Action::Export));
            assert_eq!(
                km.lookup(&ev(KeyCode::Char('E'), KeyModifiers::ALT | KeyModifiers::SHIFT), false),
                Some(Action::ExportNow)
            );
        }
    }

    #[test]
    fn acid_preset_and_overrides() {
        let mut o = std::collections::BTreeMap::new();
        o.insert("save".to_string(), vec!["f11".to_string()]);
        o.insert("bogus".to_string(), vec!["x".to_string()]);
        let (km, errs) = Keymap::new(Preset::Acid, &o);
        assert_eq!(errs.len(), 1);
        assert_eq!(km.lookup(&ev(KeyCode::Char('r'), KeyModifiers::ALT), true), Some(Action::Undo));
        // menus show the preset's own key first
        assert_eq!(km.key_for(Action::Undo).as_deref(), Some("Alt-R"));
        assert_eq!(km.key_for(Action::Sauce).as_deref(), Some("Alt-D"));
        assert_eq!(km.lookup(&ev(KeyCode::F(11), KeyModifiers::NONE), false), Some(Action::Save));
        assert_eq!(km.lookup(&ev(KeyCode::Char('s'), KeyModifiers::CONTROL), false), None);
    }

    #[test]
    fn unknown_preset_is_reported() {
        let none = Default::default();
        let (km, errs) = Keymap::from_config("acidd", &none);
        assert_eq!(km.preset, Preset::Modern);
        assert!(errs[0].contains("acidd"), "{errs:?}");
        let (km, errs) = Keymap::from_config("ACID", &none);
        assert_eq!(km.preset, Preset::Acid);
        assert!(errs.is_empty());
    }

    #[test]
    fn overrides_take_effect_in_the_right_context() {
        let mut o = std::collections::BTreeMap::new();
        // a global command on a plain letter: a tool key, never eaten while typing
        o.insert("help".to_string(), vec!["h".to_string()]);
        // a global command on a key the tools and text mode both use: it wins in both
        o.insert("save".to_string(), vec!["tab".to_string()]);
        let (km, errs) = Keymap::new(Preset::Modern, &o);
        assert!(errs.is_empty(), "{errs:?}");
        let h = ev(KeyCode::Char('h'), KeyModifiers::NONE);
        assert_eq!(km.lookup(&h, false), Some(Action::Help));
        assert_eq!(km.lookup(&h, true), None);
        let tab = ev(KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(km.lookup(&tab, false), Some(Action::Save));
        assert_eq!(km.lookup(&tab, true), Some(Action::Save));
        assert!(types_char("shift-a") && types_char("?") && types_char("space") && types_char("-"));
        assert!(!types_char("ctrl-a") && !types_char("f5") && !types_char("enter"));
    }
}
