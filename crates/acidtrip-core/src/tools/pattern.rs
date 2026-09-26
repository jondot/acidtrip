//! Pattern brush: paint with a small tile that repeats across the canvas.
//!
//! A pattern is a grid of cells; empty cells are holes that leave the art
//! underneath alone. Tiles line up on a fixed grid (the canvas origin by
//! default), so separate strokes, rectangles and fills join seamlessly.
//! Cells without colors of their own (the built-ins) take the brush colors.

use serde::{Deserialize, Serialize};

use super::{FillMatch, Rect, empty_cell, fill_region};
use crate::color::Color;
use crate::cp437;
use crate::model::{Cell, Clip};
use crate::tx::TxBuilder;

/// The largest pattern, in cells (a selection is cut to this).
pub const MAX_W: usize = 64;
pub const MAX_H: usize = 32;

/// One pattern cell. Colors left out come from the brush.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tile {
    pub ch: char,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fg: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<Color>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// Row-major; `None` is a hole.
    pub cells: Vec<Option<Tile>>,
}

impl Pattern {
    /// A glyph-only pattern from text rows; spaces are holes. Short rows are
    /// padded with holes.
    pub fn from_rows(name: &str, rows: &[&str]) -> Pattern {
        let width = rows.iter().map(|r| r.chars().count()).max().unwrap_or(1).max(1);
        let height = rows.len().max(1);
        let mut cells = vec![None; width * height];
        for (y, row) in rows.iter().enumerate() {
            for (x, ch) in row.chars().enumerate() {
                if ch != ' ' {
                    cells[y * width + x] = Some(Tile { ch, fg: None, bg: None });
                }
            }
        }
        Pattern { name: name.into(), width, height, cells }
    }

    /// A pattern from a copied block, colors and all. Transparent and blank
    /// cells become holes; the block is cut to [`MAX_W`] x [`MAX_H`].
    pub fn from_clip(name: &str, clip: &Clip) -> Pattern {
        let (width, height) = (clip.width.clamp(1, MAX_W), clip.height.clamp(1, MAX_H));
        let mut cells = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                cells.push(
                    clip.get(x, y)
                        .filter(|c| !c.is_blank())
                        .map(|c| Tile { ch: c.ch, fg: Some(c.fg), bg: Some(c.bg) }),
                );
            }
        }
        Pattern { name: name.into(), width, height, cells }
    }

    /// Fix what a hand-edited or broken file got wrong: the size is clamped
    /// and the cells padded or cut to match.
    pub fn sanitized(mut self) -> Pattern {
        self.width = self.width.clamp(1, MAX_W);
        self.height = self.height.clamp(1, MAX_H);
        self.cells.resize(self.width * self.height, None);
        self
    }

    /// True when every cell is a hole (nothing to paint with).
    pub fn is_empty(&self) -> bool {
        self.cells.iter().all(Option::is_none)
    }

    /// True when the pattern has colors of its own.
    pub fn has_colors(&self) -> bool {
        self.cells.iter().flatten().any(|t| t.fg.is_some() || t.bg.is_some())
    }

    /// True when every glyph is in CP437 (it draws as-is in Classic docs).
    pub fn is_classic(&self) -> bool {
        self.cells.iter().flatten().all(|t| cp437::is_cp437(t.ch))
    }

    /// The tile cell over canvas cell (x, y), with tile (0, 0) at `origin`.
    pub fn at(&self, x: usize, y: usize, origin: (usize, usize)) -> Option<Tile> {
        let tx = (x as i64 - origin.0 as i64).rem_euclid(self.width as i64) as usize;
        let ty = (y as i64 - origin.1 as i64).rem_euclid(self.height as i64) as usize;
        self.cells.get(ty * self.width + tx).copied().flatten()
    }
}

/// How a pattern goes onto the canvas.
#[derive(Clone, Copy, Debug)]
pub struct PatternCtx<'a> {
    pub pattern: &'a Pattern,
    pub layer: usize,
    /// Canvas cell where tile (0, 0) sits.
    pub origin: (usize, usize),
    /// Colors for tiles without their own.
    pub fg: Color,
    pub bg: Color,
    /// Use `fg`/`bg` everywhere, even where the pattern has colors.
    pub recolor: bool,
    /// Erase the covered cells instead (holes included).
    pub erase: bool,
}

impl PatternCtx<'_> {
    /// What lands on (x, y): `None` leaves the cell alone (a hole).
    pub fn cell(&self, x: usize, y: usize) -> Option<Option<Cell>> {
        if self.erase {
            return Some(empty_cell(self.layer));
        }
        let t = self.pattern.at(x, y, self.origin)?;
        let (fg, bg) = if self.recolor {
            (self.fg, self.bg)
        } else {
            (t.fg.unwrap_or(self.fg), t.bg.unwrap_or(self.bg))
        };
        Some(Some(Cell::new(t.ch, fg, bg)))
    }

    fn put(&self, b: &mut TxBuilder, x: usize, y: usize) {
        if x < b.width()
            && y < b.height()
            && let Some(c) = self.cell(x, y)
        {
            b.set(self.layer, x, y, c);
        }
    }
}

/// A square dab `size` cells wide centered on (x, y).
pub fn dab(b: &mut TxBuilder, p: &PatternCtx, x: usize, y: usize, size: usize) {
    let size = size.max(1);
    let (x0, y0) = (x as i64 - (size as i64 - 1) / 2, y as i64 - (size as i64 - 1) / 2);
    for dy in 0..size as i64 {
        for dx in 0..size as i64 {
            let (cx, cy) = (x0 + dx, y0 + dy);
            if cx >= 0 && cy >= 0 {
                p.put(b, cx as usize, cy as usize);
            }
        }
    }
}

/// Dabs along the line from `a` to `c`. The tile depends only on the
/// position, so overlapping dabs repaint the same cells the same way.
pub fn stroke(b: &mut TxBuilder, p: &PatternCtx, a: (usize, usize), c: (usize, usize), size: usize) {
    for (x, y) in super::geom::line(a.0 as i64, a.1 as i64, c.0 as i64, c.1 as i64) {
        dab(b, p, x as usize, y as usize, size);
    }
}

/// Fill a rectangle.
pub fn fill_rect(b: &mut TxBuilder, p: &PatternCtx, r: Rect) {
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            p.put(b, x, y);
        }
    }
}

/// Flood-fill the area around (x, y) that matches per `m` (like the fill
/// bucket).
pub fn flood_fill(b: &mut TxBuilder, p: &PatternCtx, x: usize, y: usize, m: FillMatch) {
    let w = b.width();
    let region = fill_region(b, x, y, m);
    for (i, _) in region.iter().enumerate().filter(|(_, r)| **r) {
        p.put(b, i % w, i / w);
    }
}

/// The built-in patterns: classic scene textures. Glyph-only, so they take
/// the brush colors; spaces are holes. Some need Modern documents (see
/// [`Pattern::is_classic`]).
pub fn builtin() -> Vec<Pattern> {
    let p = Pattern::from_rows;
    vec![
        p("bricks", &["███████ ", "▀▀▀▀▀▀▀ ", "███ ████", "▀▀▀ ▀▀▀▀"]),
        p("wall", &["───┬───┴", "   │    ", "───┴───┬", "       │"]),
        p("checkers", &["████    ", "████    ", "    ████", "    ████"]),
        p("pixel checker", &["▀▄"]),
        p("basket weave", &["════││││", "════││││", "││││════", "││││════"]),
        p("twill", &["▀▄  ", "  ▀▄"]),
        p("stripes", &["█▄ ▀", " ▀█▄"]),
        p("scales", &["▀▄  ▄▀", " ▀▄▄▀ "]),
        p("waves", &["  ▄▀▀▄  ", "▄▀    ▀▄", ""]),
        p("shade waves", &["░▒▓█▓▒░ "]),
        p("shade bands", &["█", "▓", "▒", "░", " ", "░", "▒", "▓"]),
        p("dither ramp", &["██▓▓▒▒░░  ░░▒▒▓▓"]),
        p("grid", &["┼───", "│   "]),
        p("double grid", &["╬══", "║  "]),
        p("honeycomb", &["__/  \\", "  \\__/"]),
        p("argyle", &["/\\", "\\/"]),
        p("polka dots", &["•   ", "  • "]),
        p("starfield", &["∙     ·  ", "   +     ", " ·     ∙ ", "      *  "]),
        p("card suits", &["♥   ♠   ", "  ♦   ♣ "]),
        p("diamonds", &["╱╲", "╲╱"]),
        p("bubbles", &["╭╮", "╰╯"]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DocKind, Document};

    fn ctx(p: &Pattern) -> PatternCtx<'_> {
        PatternCtx {
            pattern: p,
            layer: 0,
            origin: (0, 0),
            fg: Color::Pal(4),
            bg: Color::Pal(1),
            recolor: false,
            erase: false,
        }
    }

    #[test]
    fn rows_and_holes() {
        let p = Pattern::from_rows("t", &["ab", "c"]);
        assert_eq!((p.width, p.height), (2, 2));
        assert_eq!(p.at(0, 0, (0, 0)).unwrap().ch, 'a');
        assert_eq!(p.at(1, 1, (0, 0)), None, "short rows pad with holes");
        assert_eq!(p.at(2, 2, (0, 0)).unwrap().ch, 'a', "tiles repeat");
        assert_eq!(p.at(4, 5, (1, 1)).unwrap().ch, 'b', "offset origin");
        assert_eq!(p.at(0, 0, (1, 0)).unwrap().ch, 'b', "left of the origin wraps");
    }

    #[test]
    fn separate_strokes_line_up() {
        let p = Pattern::from_rows("t", &["ab", "cd"]);
        let doc = Document::new(DocKind::Classic, 10, 4);
        let mut b = TxBuilder::new(&doc, "t");
        dab(&mut b, &ctx(&p), 1, 1, 1);
        dab(&mut b, &ctx(&p), 6, 2, 1);
        assert_eq!(b.composite(1, 1).ch, 'd');
        assert_eq!(b.composite(6, 2).ch, 'a');
        assert_eq!(b.composite(1, 1).fg, Color::Pal(4), "brush colors");
        assert!(b.composite(0, 0).is_blank(), "a size-1 dab paints one cell");
    }

    #[test]
    fn holes_leave_the_art_alone() {
        let p = Pattern::from_rows("t", &["x "]);
        let doc = Document::new(DocKind::Classic, 4, 1);
        let mut b = TxBuilder::new(&doc, "t");
        b.set(0, 1, 0, Some(Cell::new('Z', Color::WHITE, Color::BLACK)));
        fill_rect(&mut b, &ctx(&p), Rect::new(0, 0, 4, 1));
        let row: String = (0..4).map(|x| b.composite(x, 0).ch).collect();
        assert_eq!(row, "xZx ");
    }

    #[test]
    fn dab_size_and_erase() {
        let p = Pattern::from_rows("t", &["#"]);
        let doc = Document::new(DocKind::Classic, 6, 6);
        let mut b = TxBuilder::new(&doc, "t");
        dab(&mut b, &ctx(&p), 2, 2, 3);
        let n = (0..36).filter(|i| b.composite(i % 6, i / 6).ch == '#').count();
        assert_eq!(n, 9);
        assert_eq!(b.composite(1, 1).ch, '#');
        let erase = PatternCtx { erase: true, ..ctx(&p) };
        dab(&mut b, &erase, 2, 2, 1);
        assert!(b.composite(2, 2).is_blank());
    }

    #[test]
    fn flood_fill_stays_inside() {
        let p = Pattern::from_rows("t", &["▒"]);
        let doc = Document::new(DocKind::Classic, 5, 3);
        let mut b = TxBuilder::new(&doc, "t");
        for y in 0..3 {
            b.set(0, 2, y, Some(Cell::new('│', Color::WHITE, Color::BLACK)));
        }
        flood_fill(&mut b, &ctx(&p), 0, 0, FillMatch::default());
        assert_eq!(b.composite(1, 2).ch, '▒');
        assert_eq!(b.composite(2, 1).ch, '│');
        assert_eq!(b.composite(3, 1).ch, ' ', "the wall stops the fill");
    }

    #[test]
    fn captured_colors_and_recolor() {
        let mut clip = Clip::new(2, 1);
        clip.set(0, 0, Some(Cell::new('A', Color::Pal(10), Color::Pal(2))));
        clip.set(1, 0, Some(Cell::BLANK));
        let p = Pattern::from_clip("sel", &clip);
        assert!(p.has_colors());
        assert_eq!(p.cells[1], None, "blank cells are holes");
        let c = ctx(&p);
        assert_eq!(c.cell(0, 0), Some(Some(Cell::new('A', Color::Pal(10), Color::Pal(2)))));
        assert_eq!(c.cell(1, 0), None);
        let re = PatternCtx { recolor: true, ..c };
        assert_eq!(re.cell(2, 3), Some(Some(Cell::new('A', Color::Pal(4), Color::Pal(1)))));
    }

    #[test]
    fn builtins_are_sound() {
        let all = builtin();
        assert!(all.len() >= 15);
        for p in &all {
            assert!(!p.is_empty(), "{}", p.name);
            assert_eq!(p.cells.len(), p.width * p.height, "{}", p.name);
            assert!(!p.has_colors(), "{} takes the brush colors", p.name);
        }
        let classic = all.iter().filter(|p| p.is_classic()).count();
        assert!(classic >= all.len() - 3, "most built-ins draw in CP437");
        assert!(!all.iter().find(|p| p.name == "diamonds").unwrap().is_classic());
    }

    #[test]
    fn sanitize_fixes_sizes() {
        let p = Pattern { name: "x".into(), width: 0, height: 999, cells: vec![] }.sanitized();
        assert_eq!((p.width, p.height), (1, MAX_H));
        assert_eq!(p.cells.len(), MAX_H);
    }
}
