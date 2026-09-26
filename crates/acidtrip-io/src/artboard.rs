//! The art tool's keyboard: which glyph each left-hand key types.
//!
//! The number row is always the shade ramp (shades sit next to nearly
//! everything in real ANSI art). The letter keys come from a glyph set you
//! rotate through with `[` `]`: Q W E / A S D / Z X C are the nine places of
//! a cell (a corner key types a corner piece), F V and T G B hold the set's
//! extras, and R erases. Sets were picked from a count of 527 scene files
//! (1994-2023): █▀▄▓░▒▌▐ are ~97% of all glyphs, then box lines, then the
//! details ■ · ∙ ▬ °. Modern-only sets are skipped in Classic documents.
//! The user's key changes live in the library's `artboard.toml`, per set.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::library::write_atomic;

/// The board's keys, top row to bottom, as they sit on a keyboard.
pub const ROWS: [&str; 4] = ["12345", "qwert", "asdfg", "zxcvb"];

/// The left hand's erase key.
pub const ERASE_KEY: char = 'r';

/// The number row, the same in every set.
const SHADES: &str = "░▒▓█■";

/// One set of glyphs for the letter keys: rows "qwert", "asdfg", "zxcvb"
/// (the R slot is the erase key, so its glyph is a placeholder).
pub struct GlyphSet {
    pub name: &'static str,
    /// Unicode-only glyphs: not offered in Classic (CP437) documents.
    pub modern_only: bool,
    rows: [&'static str; 3],
    /// CP437 stand-ins for a Classic document, if the set differs there.
    classic: Option<[&'static str; 3]>,
}

pub const SETS: &[GlyphSet] = &[
    GlyphSet {
        name: "Blocks",
        modern_only: false,
        rows: ["▘▀▝ ▪", "▌█▐▚·", "▖▄▗▞▬"],
        classic: Some(["°▀° ▬", "▌█▐■∙", "·▄·≡÷"]),
    },
    GlyphSet { name: "Single lines", modern_only: false, rows: ["┌┬┐ ·", "├┼┤─∙", "└┴┘│▬"], classic: None },
    GlyphSet { name: "Double lines", modern_only: false, rows: ["╔╦╗ ·", "╠╬╣═∙", "╚╩╝║■"], classic: None },
    GlyphSet { name: "Mixed lines ╓", modern_only: false, rows: ["╓╥╖ ·", "╟╫╢─∙", "╙╨╜║■"], classic: None },
    GlyphSet { name: "Mixed lines ╒", modern_only: false, rows: ["╒╤╕ ·", "╞╪╡═∙", "╘╧╛│■"], classic: None },
    GlyphSet { name: "Quads", modern_only: true, rows: ["▛▀▜ ■", "▌█▐▚▪", "▙▄▟▞·"], classic: None },
    GlyphSet { name: "Eighths", modern_only: true, rows: ["▁▂▃ ▔", "▅▆▇▕▉", "▏▎▍▋▊"], classic: None },
];

impl GlyphSet {
    /// The default glyph for a letter `key` (lowercase) in this set.
    fn glyph(&self, key: char, classic: bool) -> Option<char> {
        if key == ERASE_KEY {
            return None;
        }
        let rows = if classic { self.classic.unwrap_or(self.rows) } else { self.rows };
        ROWS[1..]
            .iter()
            .zip(rows)
            .find_map(|(keys, glyphs)| keys.chars().position(|k| k == key).and_then(|i| glyphs.chars().nth(i)))
    }
}

/// The sets a document can use.
pub fn sets(classic: bool) -> impl Iterator<Item = &'static GlyphSet> {
    SETS.iter().filter(move |s| !(classic && s.modern_only))
}

fn find(name: &str, classic: bool) -> &'static GlyphSet {
    sets(classic).find(|s| s.name == name).unwrap_or(&SETS[0])
}

/// The default glyph for `key` (lowercase) in the set named `set`.
pub fn default_glyph(key: char, set: &str, classic: bool) -> Option<char> {
    match SHADES.chars().zip(ROWS[0].chars()).find(|(_, k)| *k == key) {
        Some((g, _)) => Some(g),
        None => find(set, classic).glyph(key, classic),
    }
}

fn on_board(key: char) -> bool {
    key != ERASE_KEY && ROWS.concat().contains(key)
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtBoard {
    /// Number-row keys the user changed: key → glyph.
    #[serde(default)]
    pub keys: BTreeMap<char, char>,
    /// Letter keys the user changed, per set name.
    #[serde(default)]
    pub sets: BTreeMap<String, BTreeMap<char, char>>,
    /// The set in use (empty: the first).
    #[serde(default)]
    pub set: String,
}

impl ArtBoard {
    /// The board saved at `path` (defaults when missing or unreadable).
    pub fn load(path: &Path) -> ArtBoard {
        let mut b: ArtBoard =
            std::fs::read_to_string(path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default();
        // Before sets, letter-key changes lived with the number row's.
        let letters: Vec<(char, char)> = b.keys.iter().filter(|(k, _)| k.is_ascii_alphabetic()).map(|(k, g)| (*k, *g)).collect();
        for (k, g) in letters {
            b.keys.remove(&k);
            if on_board(k) {
                b.sets.entry(SETS[0].name.into()).or_default().entry(k).or_insert(g);
            }
        }
        b
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        write_atomic(path, toml::to_string_pretty(self)?.as_bytes())
    }

    /// The set in use (Blocks when a Classic document can't use it).
    pub fn current(&self, classic: bool) -> &'static GlyphSet {
        find(&self.set, classic)
    }

    /// Step through the sets a document can use; the new set's name.
    pub fn cycle(&mut self, step: i32, classic: bool) -> &'static str {
        let all: Vec<&GlyphSet> = sets(classic).collect();
        let i = all.iter().position(|s| s.name == self.current(classic).name).unwrap_or(0) as i32;
        let next = all[(i + step).rem_euclid(all.len() as i32) as usize];
        self.set = next.name.into();
        next.name
    }

    fn changes(&self, key: char, classic: bool) -> Option<&BTreeMap<char, char>> {
        if key.is_ascii_digit() { Some(&self.keys) } else { self.sets.get(self.current(classic).name) }
    }

    /// What `key` types (either case) in the current set, if it's on the board.
    pub fn glyph(&self, key: char, classic: bool) -> Option<char> {
        let k = key.to_ascii_lowercase();
        if !on_board(k) {
            return None;
        }
        self.changes(k, classic)
            .and_then(|m| m.get(&k).copied())
            .or_else(|| default_glyph(k, self.current(classic).name, classic))
    }

    /// Put `glyph` on `key` in the current set; the default glyph clears the change.
    pub fn set(&mut self, key: char, glyph: char, classic: bool) {
        let k = key.to_ascii_lowercase();
        if !on_board(k) {
            return;
        }
        let name = self.current(classic).name;
        let is_default = [false, true].iter().any(|&c| default_glyph(k, name, c) == Some(glyph));
        let m = if k.is_ascii_digit() { &mut self.keys } else { self.sets.entry(name.into()).or_default() };
        if is_default {
            m.remove(&k);
        } else {
            m.insert(k, glyph);
        }
        self.sets.retain(|_, m| !m.is_empty());
    }

    pub fn changed(&self, key: char, classic: bool) -> bool {
        let k = key.to_ascii_lowercase();
        self.changes(k, classic).is_some_and(|m| m.contains_key(&k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_board_is_spatial() {
        let b = ArtBoard::default();
        assert_eq!(b.current(false).name, "Blocks");
        // The 3x3 of a cell: top-left quadrant on Q, full block in the middle on S.
        assert_eq!([b.glyph('q', false), b.glyph('w', false), b.glyph('e', false)], [Some('▘'), Some('▀'), Some('▝')]);
        assert_eq!([b.glyph('a', false), b.glyph('s', false), b.glyph('d', false)], [Some('▌'), Some('█'), Some('▐')]);
        assert_eq!([b.glyph('z', false), b.glyph('x', false), b.glyph('c', false)], [Some('▖'), Some('▄'), Some('▗')]);
        assert_eq!(b.glyph('1', false), Some('░'), "the number row is the shade ramp");
        assert_eq!(b.glyph('r', false), None, "R erases");
        assert_eq!(b.glyph('Q', false), Some('▘'), "either case");
        assert_eq!(b.glyph('j', false), None, "the right hand moves");
    }

    #[test]
    fn every_set_fills_the_board_and_classic_is_cp437() {
        for classic in [false, true] {
            for set in sets(classic) {
                for k in ROWS.concat().chars().filter(|&k| k != ERASE_KEY) {
                    let g = default_glyph(k, set.name, classic).unwrap_or_else(|| panic!("{} {k}", set.name));
                    assert_ne!(g, ' ', "{} {k}", set.name);
                    if classic {
                        assert!(acidtrip_core::cp437::from_char(g).is_some(), "{} {k} → {g:?} isn't CP437", set.name);
                    }
                }
            }
        }
    }

    #[test]
    fn sets_rotate_and_classic_skips_unicode_sets() {
        let mut b = ArtBoard::default();
        assert_eq!(b.cycle(1, false), "Single lines");
        assert_eq!([b.glyph('q', false), b.glyph('s', false), b.glyph('c', false)], [Some('┌'), Some('┼'), Some('┘')]);
        assert_eq!(b.glyph('1', false), Some('░'), "the shades stay");
        assert_eq!(b.cycle(-2, false), "Eighths", "wraps around");
        assert_eq!(b.current(true).name, "Blocks", "a Classic document can't use Eighths");
        assert_eq!(b.cycle(-1, true), "Mixed lines ╒");
        assert!(sets(true).all(|s| !s.modern_only));
    }

    #[test]
    fn changes_are_per_set_save_and_the_default_clears_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artboard.toml");
        let mut b = ArtBoard::default();
        b.set('T', '♥', true);
        b.set('5', '•', true);
        b.set('j', 'x', false);
        b.set('r', 'x', false);
        assert_eq!(b.glyph('t', true), Some('♥'));
        assert_eq!(b.glyph('j', false), None, "only board keys change");
        assert_eq!(b.glyph('r', false), None, "R stays the eraser");
        b.cycle(1, false);
        assert_eq!(b.glyph('t', false), Some('·'), "another set has its own keys");
        assert_eq!(b.glyph('5', false), Some('•'), "the number row is shared");
        b.save(&path).unwrap();
        let mut back = ArtBoard::load(&path);
        assert_eq!(back, b);
        back.cycle(-1, false);
        back.set('t', '▪', false);
        assert!(!back.changed('t', false));
    }

    #[test]
    fn old_boards_move_letter_changes_to_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("artboard.toml");
        std::fs::write(&path, "[keys]\nt = \"♥\"\n1 = \"x\"\n").unwrap();
        let b = ArtBoard::load(&path);
        assert_eq!(b.glyph('t', false), Some('♥'));
        assert_eq!(b.glyph('1', false), Some('x'));
    }
}
