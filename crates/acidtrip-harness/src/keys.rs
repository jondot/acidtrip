//! Key spec -> terminal byte sequence encoding (xterm / legacy).
//!
//! A spec is `[mod-]*key`, with modifiers `ctrl`, `alt` (or `meta`), `shift`.
//! Keys: a single character (`a`, `B`, `/`, `-`), or a name: `esc`, `enter`,
//! `tab`, `space`, `backspace`, `delete`, `insert`, `home`, `end`, `pgup`,
//! `pgdn`, `up`, `down`, `left`, `right`, `f1`..`f12`.
//!
//! Combinations that legacy encoding cannot express (e.g. `ctrl-shift-s`,
//! `shift-enter`, `ctrl-enter`) are sent as CSI-u (`ESC [ code ; mods u`),
//! which crossterm decodes even without the kitty protocol being enabled.

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    /// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }
    fn any(self) -> bool {
        self.ctrl || self.alt || self.shift
    }
}

/// Encode a whitespace-separated list of key specs.
pub fn encode_keys(spec: &str, app_cursor: bool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for s in spec.split_whitespace() {
        out.extend(encode_key(s, app_cursor)?);
    }
    Ok(out)
}

/// Encode one key spec like `ctrl-s`, `alt-b`, `shift-tab`, `f5`, `a`.
pub fn encode_key(spec: &str, app_cursor: bool) -> Result<Vec<u8>> {
    let mut mods = Mods::default();
    let mut rest = spec;
    loop {
        let lower = rest.to_ascii_lowercase();
        let strip = ["ctrl-", "control-", "c-", "alt-", "meta-", "m-", "shift-", "s-"]
            .iter()
            .find(|p| lower.starts_with(**p) && rest.len() > p.len());
        match strip {
            Some(p) => {
                match *p {
                    "ctrl-" | "control-" | "c-" => mods.ctrl = true,
                    "alt-" | "meta-" | "m-" => mods.alt = true,
                    _ => mods.shift = true,
                }
                rest = &rest[p.len()..];
            }
            None => break,
        }
    }
    if rest.is_empty() {
        bail!("empty key in spec {spec:?}");
    }
    let key = rest;
    let name = key.to_ascii_lowercase();

    // CSI with optional modifier param: ESC [ 1 ; m X  or  ESC [ X
    let csi_letter = |c: char, ss3_plain: bool| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[1;{}{}", mods.param(), c).into_bytes()
        } else if ss3_plain {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    let csi_tilde = |n: u8| -> Vec<u8> {
        if mods.any() { format!("\x1b[{n};{}~", mods.param()).into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };
    let csi_u = |code: u32| -> Vec<u8> { format!("\x1b[{code};{}u", mods.param()).into_bytes() };

    let special = match name.as_str() {
        "up" => Some(csi_letter('A', app_cursor)),
        "down" => Some(csi_letter('B', app_cursor)),
        "right" => Some(csi_letter('C', app_cursor)),
        "left" => Some(csi_letter('D', app_cursor)),
        "home" => Some(csi_letter('H', app_cursor)),
        "end" => Some(csi_letter('F', app_cursor)),
        "insert" | "ins" => Some(csi_tilde(2)),
        "delete" | "del" => Some(csi_tilde(3)),
        "pgup" | "pageup" => Some(csi_tilde(5)),
        "pgdn" | "pagedown" | "pgdown" => Some(csi_tilde(6)),
        "f1" => Some(csi_letter('P', true)),
        "f2" => Some(csi_letter('Q', true)),
        "f3" => Some(csi_letter('R', true)),
        "f4" => Some(csi_letter('S', true)),
        "f5" => Some(csi_tilde(15)),
        "f6" => Some(csi_tilde(17)),
        "f7" => Some(csi_tilde(18)),
        "f8" => Some(csi_tilde(19)),
        "f9" => Some(csi_tilde(20)),
        "f10" => Some(csi_tilde(21)),
        "f11" => Some(csi_tilde(23)),
        "f12" => Some(csi_tilde(24)),
        _ => None,
    };
    if let Some(s) = special {
        return Ok(s);
    }

    let alt_prefix = |mut b: Vec<u8>| -> Vec<u8> {
        if mods.alt {
            b.insert(0, 0x1b);
        }
        b
    };

    match name.as_str() {
        "esc" | "escape" => {
            return Ok(if mods.ctrl || mods.shift { csi_u(27) } else { alt_prefix(vec![0x1b]) });
        }
        "enter" | "return" | "cr" => {
            return Ok(if mods.ctrl || mods.shift { csi_u(13) } else { alt_prefix(vec![b'\r']) });
        }
        "tab" => {
            return Ok(match (mods.ctrl, mods.shift, mods.alt) {
                (false, false, _) => alt_prefix(vec![b'\t']),
                (false, true, false) => b"\x1b[Z".to_vec(),
                (false, true, true) => b"\x1b\x1b[Z".to_vec(),
                _ => csi_u(9),
            });
        }
        "backspace" | "bs" => {
            return Ok(match (mods.ctrl, mods.shift) {
                (false, false) => alt_prefix(vec![0x7f]),
                (true, false) => alt_prefix(vec![0x08]),
                _ => csi_u(127),
            });
        }
        "space" | "spc" => {
            return Ok(match (mods.ctrl, mods.shift) {
                (false, false) => alt_prefix(vec![b' ']),
                (true, false) => alt_prefix(vec![0x00]),
                _ => csi_u(32),
            });
        }
        _ => {}
    }

    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        bail!("unknown key {key:?} in spec {spec:?}");
    };

    if mods.ctrl {
        let lc = c.to_ascii_lowercase();
        // ctrl-shift-<letter> (or ctrl-<Upper>) can't be expressed legacy -> CSI u.
        if mods.shift || (c.is_ascii_uppercase()) {
            return Ok(csi_u(lc as u32));
        }
        let b = match c {
            'a'..='z' => Some(c as u8 - b'a' + 1),
            '@' | '2' | ' ' => Some(0x00),
            '[' | '3' => Some(0x1b),
            '\\' | '4' => Some(0x1c),
            ']' | '5' => Some(0x1d),
            '^' | '6' | '~' => Some(0x1e),
            '_' | '/' | '7' | '-' => Some(0x1f),
            '?' | '8' => Some(0x7f),
            _ => None,
        };
        return Ok(match b {
            Some(b) => alt_prefix(vec![b]),
            None => csi_u(c as u32),
        });
    }

    let c = if mods.shift { c.to_uppercase().next().unwrap_or(c) } else { c };
    let mut buf = [0u8; 4];
    Ok(alt_prefix(c.encode_utf8(&mut buf).as_bytes().to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> Vec<u8> {
        encode_keys(s, false).unwrap()
    }

    #[test]
    fn basics() {
        assert_eq!(k("ctrl-s"), vec![0x13]);
        assert_eq!(k("alt-b"), b"\x1bb");
        assert_eq!(k("shift-tab"), b"\x1b[Z");
        assert_eq!(k("esc"), vec![0x1b]);
        assert_eq!(k("enter"), b"\r");
        assert_eq!(k("up"), b"\x1b[A");
        assert_eq!(encode_keys("up", true).unwrap(), b"\x1bOA");
        assert_eq!(k("ctrl-up"), b"\x1b[1;5A");
        assert_eq!(k("f1"), b"\x1bOP");
        assert_eq!(k("shift-f1"), b"\x1b[1;2P");
        assert_eq!(k("f5"), b"\x1b[15~");
        assert_eq!(k("ctrl-shift-s"), b"\x1b[115;6u");
        assert_eq!(k("a"), b"a");
        assert_eq!(k("B"), b"B");
        assert_eq!(k("shift-a"), b"A");
        assert_eq!(k("space"), b" ");
        assert_eq!(k("backspace"), vec![0x7f]);
        assert_eq!(k("delete"), b"\x1b[3~");
        assert_eq!(k("home"), b"\x1b[H");
        assert_eq!(k("end"), b"\x1b[F");
        assert_eq!(k("pgup"), b"\x1b[5~");
        assert_eq!(k("pgdn"), b"\x1b[6~");
        assert_eq!(k("tab"), b"\t");
        assert_eq!(k("ctrl-/"), vec![0x1f]);
        assert_eq!(k("ctrl--"), vec![0x1f]);
        assert_eq!(k("-"), b"-");
        assert_eq!(k("ctrl-k alt-x"), b"\x0b\x1bx");
        assert_eq!(k("shift-enter"), b"\x1b[13;2u");
        assert!(encode_keys("nosuchkey", false).is_err());
    }
}
