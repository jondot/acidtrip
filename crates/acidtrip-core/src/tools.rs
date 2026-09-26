//! Drawing tools. Every tool writes into a [`TxBuilder`] so the caller
//! decides when to commit (one undo step per tool use / stroke).
//!
//! Coordinates are cell coordinates unless stated otherwise. Out-of-bounds
//! writes are ignored by the builder.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::color::Color;
use crate::mirror;
use crate::model::{Canvas, Cell, Clip, DocKind, Layer, LayerKind};
use crate::tx::TxBuilder;

mod geom;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Brush {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
}

impl Default for Brush {
    fn default() -> Self {
        Brush { ch: '█', fg: Color::LIGHT_GRAY, bg: Color::BLACK }
    }
}

impl Brush {
    pub fn cell(&self) -> Cell {
        Cell::new(self.ch, self.fg, self.bg)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PaintMode {
    /// Write char + fg + bg.
    #[default]
    Char,
    /// Keep char, set fg + bg.
    Color,
    /// Keep char + bg, set fg.
    Fg,
    /// Keep char + fg, set bg.
    Bg,
    /// Shading brush: step through ` ░▒▓█` (up = denser), fg from brush.
    Shade { up: bool },
    /// Colorize: set fg only where the glyph has ink, keep char and bg.
    Colorize,
    /// Erase to transparent (blank on the background layer).
    Erase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Symmetry {
    #[default]
    None,
    /// Mirror across the vertical center line (left-right).
    X,
    /// Mirror across the horizontal center line (top-bottom).
    Y,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Ctx {
    pub layer: usize,
    pub brush: Brush,
    pub mode: PaintMode,
    pub symmetry: Symmetry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub fn new(x: usize, y: usize, w: usize, h: usize) -> Self {
        Rect { x, y, w, h }
    }

    /// Normalized rect spanning two corner points (inclusive).
    pub fn from_points(x0: usize, y0: usize, x1: usize, y1: usize) -> Self {
        Rect { x: x0.min(x1), y: y0.min(y1), w: x0.abs_diff(x1) + 1, h: y0.abs_diff(y1) + 1 }
    }

    pub fn contains(&self, x: usize, y: usize) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn right(&self) -> usize {
        self.x + self.w.saturating_sub(1)
    }

    pub fn bottom(&self) -> usize {
        self.y + self.h.saturating_sub(1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ShapeFill {
    #[default]
    Outline,
    Filled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BoxStyle {
    /// ┌─┐ (ACiDDraw set 1)
    #[default]
    Single,
    /// ╔═╗ (set 2)
    Double,
    /// ╒═╕ (set 3)
    DoubleH,
    /// ╓─╖ (set 4)
    DoubleV,
    /// Solid █ blocks.
    Block,
    /// Uses the brush char for every edge cell.
    Brush,
    /// ╭─╮ (Modern docs; downsamples to Single in Classic).
    Rounded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillMatch {
    pub ch: bool,
    pub fg: bool,
    pub bg: bool,
}

impl Default for FillMatch {
    fn default() -> Self {
        FillMatch { ch: true, fg: true, bg: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FillWhat {
    /// Brush char + colors.
    All,
    /// Only the char (keep colors).
    Char,
    Fg,
    Bg,
    /// fg + bg, keep chars.
    Colors,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum StampMode {
    /// Every clip cell replaces the target (transparent clip cells too).
    Opaque,
    /// Transparent clip cells (None) and blank cells leave the target alone.
    #[default]
    Transparent,
    /// Only fills target cells that are blank/transparent.
    Under,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Justify {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LayerProps {
    pub name: Option<String>,
    pub visible: Option<bool>,
    pub locked: Option<bool>,
    pub reference: Option<bool>,
}

// ---------------------------------------------------------------- painting

const SHADES: [char; 5] = [' ', '░', '▒', '▓', '█'];

/// What an erased cell holds on `layer`: blank on the background (layer 0),
/// transparent elsewhere.
pub fn empty_cell(layer: usize) -> Option<Cell> {
    (layer == 0).then_some(Cell::BLANK)
}

/// True when a glyph draws any foreground pixels.
pub fn has_ink(ch: char) -> bool {
    !matches!(ch, ' ' | '\u{0}' | '\u{A0}')
}

/// Next shade glyph from `ch`; `None` when there is nothing to do (stepping
/// down from a non-shade glyph).
pub fn shade_step(ch: char, up: bool) -> Option<char> {
    let ch = if has_ink(ch) { ch } else { ' ' };
    match SHADES.iter().position(|&s| s == ch) {
        Some(i) if up => Some(SHADES[(i + 1).min(4)]),
        Some(i) => Some(SHADES[i.saturating_sub(1)]),
        None if up => Some(SHADES[1]),
        None => None,
    }
}

/// Layer cell at (x, y), or the composite where the layer is transparent.
fn current(b: &TxBuilder, layer: usize, x: usize, y: usize) -> Cell {
    b.get(layer, x, y).unwrap_or_else(|| b.composite(x, y))
}

/// Paint a single cell (no symmetry) with glyph `ch` per `ctx.mode`.
fn paint_cell(b: &mut TxBuilder, ctx: &Ctx, x: usize, y: usize, ch: char) {
    if x >= b.width() || y >= b.height() {
        return;
    }
    let br = ctx.brush;
    let out = match ctx.mode {
        PaintMode::Char => Some(Cell::new(ch, br.fg, br.bg)),
        PaintMode::Erase => empty_cell(ctx.layer),
        mode => {
            let c = current(b, ctx.layer, x, y);
            match mode {
                PaintMode::Color => Some(Cell { fg: br.fg, bg: br.bg, ..c }),
                PaintMode::Fg => Some(Cell { fg: br.fg, ..c }),
                PaintMode::Bg => Some(Cell { bg: br.bg, ..c }),
                PaintMode::Shade { up } => match shade_step(c.ch, up) {
                    Some(s) => Some(Cell::new(s, br.fg, c.bg)),
                    None => return,
                },
                PaintMode::Colorize if has_ink(c.ch) => Some(Cell { fg: br.fg, ..c }),
                _ => return,
            }
        }
    };
    b.set(ctx.layer, x, y, out);
}

/// Paint glyph points plus their symmetry mirrors. Each position is painted
/// once; the shape's own points win over mirrored ones.
fn paint_glyphs(b: &mut TxBuilder, ctx: &Ctx, pts: impl IntoIterator<Item = (usize, usize, char)>) {
    let (w, h) = (b.width(), b.height());
    let pts: Vec<_> = pts.into_iter().filter(|&(x, y, _)| x < w && y < h).collect();
    let mut all = pts.clone();
    for &(x, y, ch) in &pts {
        let (mx, my) = (w - 1 - x, h - 1 - y);
        match ctx.symmetry {
            Symmetry::None => {}
            Symmetry::X => all.push((mx, y, mirror::mirror_h(ch))),
            Symmetry::Y => all.push((x, my, mirror::mirror_v(ch))),
            Symmetry::Both => all.extend([
                (mx, y, mirror::mirror_h(ch)),
                (x, my, mirror::mirror_v(ch)),
                (mx, my, mirror::mirror_v(mirror::mirror_h(ch))),
            ]),
        }
    }
    let mut seen = HashSet::with_capacity(all.len());
    for (x, y, ch) in all {
        if seen.insert((x, y)) {
            paint_cell(b, ctx, x, y, ch);
        }
    }
}

/// Paint one cell (plus symmetry mirrors) with the brush in `ctx.mode`.
pub fn paint(b: &mut TxBuilder, ctx: &Ctx, x: usize, y: usize) {
    paint_glyphs(b, ctx, [(x, y, ctx.brush.ch)]);
}

/// Paint every cell on the Bresenham line from (x0,y0) to (x1,y1). Each cell
/// is painted once even where mirrors overlap, so shading steps once.
pub fn paint_line(b: &mut TxBuilder, ctx: &Ctx, x0: usize, y0: usize, x1: usize, y1: usize) {
    let ch = ctx.brush.ch;
    let pts = geom::line(x0 as i64, y0 as i64, x1 as i64, y1 as i64).map(|(x, y)| (x as usize, y as usize, ch));
    paint_glyphs(b, ctx, pts);
}

/// (top-left, horizontal, top-right, vertical, bottom-left, bottom-right).
fn box_glyphs(style: BoxStyle, brush: char) -> [char; 6] {
    match style {
        BoxStyle::Single => ['┌', '─', '┐', '│', '└', '┘'],
        BoxStyle::Double => ['╔', '═', '╗', '║', '╚', '╝'],
        BoxStyle::DoubleH => ['╒', '═', '╕', '│', '╘', '╛'],
        BoxStyle::DoubleV => ['╓', '─', '╖', '║', '╙', '╜'],
        BoxStyle::Block => ['█'; 6],
        BoxStyle::Brush => [brush; 6],
        BoxStyle::Rounded => ['╭', '─', '╮', '│', '╰', '╯'],
    }
}

/// Rectangle between two corners. Box styles pick matching corner/edge glyphs.
/// A 1-tall box is a horizontal edge, a 1-wide box a vertical edge. Filled
/// box styles clear the interior to spaces with the brush bg; Block and
/// Brush fill it with the brush char.
pub fn rect(b: &mut TxBuilder, ctx: &Ctx, r: Rect, fill: ShapeFill, style: BoxStyle) {
    if r.w == 0 || r.h == 0 {
        return;
    }
    let [tl, hz, tr, vt, bl, br] = box_glyphs(style, ctx.brush.ch);
    let (x1, y1) = (r.right(), r.bottom());
    let inner = match style {
        BoxStyle::Block | BoxStyle::Brush => ctx.brush.ch,
        _ => ' ',
    };
    let mut pts = Vec::new();
    for y in r.y..=y1 {
        for x in r.x..=x1 {
            let (top, bot, left, right) = (y == r.y, y == y1, x == r.x, x == x1);
            let ch = if r.h == 1 {
                hz
            } else if r.w == 1 {
                vt
            } else {
                match (top, bot, left, right) {
                    (true, _, true, _) => tl,
                    (true, _, _, true) => tr,
                    (_, true, true, _) => bl,
                    (_, true, _, true) => br,
                    (true, ..) | (_, true, ..) => hz,
                    (_, _, true, _) | (_, _, _, true) => vt,
                    _ if fill == ShapeFill::Filled => inner,
                    _ => continue,
                }
            };
            pts.push((x, y, ch));
        }
    }
    paint_glyphs(b, ctx, pts);
}

/// Ellipse inscribed in `r` (cell space), outline or filled, painted with the brush.
pub fn ellipse(b: &mut TxBuilder, ctx: &Ctx, r: Rect, fill: ShapeFill) {
    if r.w == 0 || r.h == 0 {
        return;
    }
    let (x0, y0, x1, y1) = (r.x as i64, r.y as i64, r.right() as i64, r.bottom() as i64);
    let ch = ctx.brush.ch;
    let pts: Vec<(usize, usize, char)> = match fill {
        ShapeFill::Outline => {
            geom::ellipse_outline(x0, y0, x1, y1).into_iter().map(|(x, y)| (x as usize, y as usize, ch)).collect()
        }
        ShapeFill::Filled => geom::ellipse_spans(x0, y0, x1, y1)
            .into_iter()
            .flat_map(|(y, a, z)| (a..=z).map(move |x| (x as usize, y as usize, ch)))
            .collect(),
    };
    paint_glyphs(b, ctx, pts);
}

/// Scanline flood fill from (x, y) over the composite, matching per `m`,
/// writing with the brush per `ctx.mode` on `ctx.layer`. Symmetry is not
/// applied.
pub fn flood_fill(b: &mut TxBuilder, ctx: &Ctx, x: usize, y: usize, m: FillMatch) {
    let w = b.width();
    let region = fill_region(b, x, y, m);
    for (i, _) in region.iter().enumerate().filter(|(_, r)| **r) {
        paint_cell(b, ctx, i % w, i / w, ctx.brush.ch);
    }
}

/// The cells a flood fill from (x, y) reaches, as a row-major mask over the
/// canvas (all false when (x, y) is outside). Iterative scanline; each cell
/// is visited once.
pub fn fill_region(b: &TxBuilder, x: usize, y: usize, m: FillMatch) -> Vec<bool> {
    let (w, h) = (b.width(), b.height());
    let mut out = vec![false; w * h];
    if x >= w || y >= h {
        return out;
    }
    let start = b.composite(x, y);
    let same = |c: Cell| (!m.ch || c.ch == start.ch) && (!m.fg || c.fg == start.fg) && (!m.bg || c.bg == start.bg);
    // Per-cell state, evaluated lazily: 0 unknown, 1 fillable, 2 not (or done).
    let mut st = vec![0u8; w * h];
    let open = |st: &mut [u8], x: usize, y: usize| {
        let s = &mut st[y * w + x];
        if *s == 0 {
            *s = if same(b.composite(x, y)) { 1 } else { 2 };
        }
        *s == 1
    };
    let mut stack = vec![(x, y)];
    while let Some((sx, sy)) = stack.pop() {
        if !open(&mut st, sx, sy) {
            continue;
        }
        let mut l = sx;
        while l > 0 && open(&mut st, l - 1, sy) {
            l -= 1;
        }
        let mut r = sx;
        while r + 1 < w && open(&mut st, r + 1, sy) {
            r += 1;
        }
        for ny in [sy.wrapping_sub(1), sy + 1] {
            if ny >= h {
                continue;
            }
            let mut in_run = false;
            for nx in l..=r {
                let hit = open(&mut st, nx, ny);
                if hit && !in_run {
                    stack.push((nx, ny));
                }
                in_run = hit;
            }
        }
        st[sy * w + l..=sy * w + r].fill(2);
        out[sy * w + l..=sy * w + r].fill(true);
    }
    out
}

/// Type a char at (x, y) with the brush colors.
pub fn type_char(b: &mut TxBuilder, ctx: &Ctx, x: usize, y: usize, ch: char) {
    b.set(ctx.layer, x, y, Some(Cell::new(ch, ctx.brush.fg, ctx.brush.bg)));
}

/// Multi-line text at (x, y) with brush colors; spaces are transparent when
/// `transparent_spaces`. Returns the (width, height) of the text block in
/// cells (before clipping); a trailing newline adds no row.
pub fn put_text(
    b: &mut TxBuilder,
    ctx: &Ctx,
    x: usize,
    y: usize,
    text: &str,
    transparent_spaces: bool,
) -> (usize, usize) {
    let (mut w, mut h) = (0, 0);
    for (row, line) in text.lines().enumerate() {
        h = row + 1;
        let mut col = 0;
        for ch in line.chars() {
            if !(transparent_spaces && ch == ' ') {
                type_char(b, ctx, x + col, y + row, ch);
            }
            col += 1;
        }
        w = w.max(col);
    }
    (w, h)
}

/// Composite cell under (x, y).
pub fn eyedrop(b: &TxBuilder, x: usize, y: usize) -> Cell {
    b.composite(x, y)
}

// ------------------------------------------------------- half-block pixels

pub mod pixel;

// ------------------------------------------------ smart pen and smart shapes

pub mod brush;
pub mod pattern;
pub mod pen;
pub mod recolor;
mod shapes;

pub use shapes::{ShapeStyle, box_candidates, shape_style, smart_ellipse, smart_line, smart_rect};

// --------------------------------------------------------------- selection

/// `r` clipped to the canvas, or `None` when nothing is left.
fn clip_rect(b: &TxBuilder, r: Rect) -> Option<Rect> {
    let (w, h) = (b.width(), b.height());
    if r.x >= w || r.y >= h || r.w == 0 || r.h == 0 {
        return None;
    }
    Some(Rect::new(r.x, r.y, r.w.min(w - r.x), r.h.min(h - r.y)))
}

/// Copy a rect. `layer: None` copies the composite; `Some(l)` copies that
/// layer including transparency. The clip is `r` clipped to the canvas.
pub fn copy(b: &TxBuilder, layer: Option<usize>, r: Rect) -> Clip {
    let Some(r) = clip_rect(b, r) else {
        return Clip::new(0, 0);
    };
    let mut c = Clip::new(r.w, r.h);
    for y in 0..r.h {
        for x in 0..r.w {
            let (sx, sy) = (r.x + x, r.y + y);
            c.set(
                x,
                y,
                match layer {
                    None => Some(b.composite(sx, sy)),
                    Some(l) => b.get(l, sx, sy),
                },
            );
        }
    }
    c
}

/// Stamp `clip` with its top-left at (x, y). Opaque writes transparent clip
/// cells as erased cells; Transparent skips transparent and blank clip
/// cells; Under skips those too and only writes where the composite is blank.
pub fn stamp(b: &mut TxBuilder, layer: usize, clip: &Clip, x: usize, y: usize, mode: StampMode) {
    for cy in 0..clip.height {
        for cx in 0..clip.width {
            let (tx, ty) = (x + cx, y + cy);
            let src = clip.get(cx, cy);
            let out = match (mode, src) {
                (StampMode::Opaque, None) => empty_cell(layer),
                (StampMode::Opaque, Some(c)) => Some(c),
                (_, None) => continue,
                (_, Some(c)) if c.is_blank() => continue,
                (StampMode::Under, Some(_)) if !b.composite(tx, ty).is_blank() => continue,
                (_, Some(c)) => Some(c),
            };
            b.set(layer, tx, ty, out);
        }
    }
}

/// Clear a rect to transparent (blank on the background layer).
pub fn erase(b: &mut TxBuilder, layer: usize, r: Rect) {
    let Some(r) = clip_rect(b, r) else { return };
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            b.set(layer, x, y, empty_cell(layer));
        }
    }
}

/// Fill a rect on `ctx.layer` with parts of the brush.
pub fn fill_rect(b: &mut TxBuilder, ctx: &Ctx, r: Rect, what: FillWhat) {
    let Some(r) = clip_rect(b, r) else { return };
    let br = ctx.brush;
    for y in r.y..r.y + r.h {
        for x in r.x..r.x + r.w {
            let c = current(b, ctx.layer, x, y);
            let out = match what {
                FillWhat::All => br.cell(),
                FillWhat::Char => Cell { ch: br.ch, ..c },
                FillWhat::Fg => Cell { fg: br.fg, ..c },
                FillWhat::Bg => Cell { bg: br.bg, ..c },
                FillWhat::Colors => Cell { fg: br.fg, bg: br.bg, ..c },
            };
            b.set(ctx.layer, x, y, Some(out));
        }
    }
}

fn map_clip(clip: &Clip, pos: impl Fn(usize, usize) -> (usize, usize), glyph: impl Fn(char) -> char) -> Clip {
    let mut out = Clip::new(clip.width, clip.height);
    for y in 0..clip.height {
        for x in 0..clip.width {
            let (sx, sy) = pos(x, y);
            out.set(x, y, clip.get(sx, sy).map(|c| Cell { ch: glyph(c.ch), ..c }));
        }
    }
    out
}

/// Left-right flip. `mirror_glyphs` also swaps ▌/▐, ┌/┐ and so on.
pub fn flip_x(clip: &Clip, mirror_glyphs: bool) -> Clip {
    let w = clip.width;
    map_clip(
        clip,
        |x, y| (w - 1 - x, y),
        |c| {
            if mirror_glyphs { mirror::mirror_h(c) } else { c }
        },
    )
}

/// Top-bottom flip. `mirror_glyphs` also swaps ▀/▄, ┌/└ and so on.
pub fn flip_y(clip: &Clip, mirror_glyphs: bool) -> Clip {
    let h = clip.height;
    map_clip(
        clip,
        |x, y| (x, h - 1 - y),
        |c| {
            if mirror_glyphs { mirror::mirror_v(c) } else { c }
        },
    )
}

pub fn rotate_180(clip: &Clip) -> Clip {
    flip_y(&flip_x(clip, true), true)
}

/// Draw a box around the inside edge of `r` in `style` with brush colors.
pub fn outline(b: &mut TxBuilder, ctx: &Ctx, r: Rect, style: BoxStyle) {
    rect(b, ctx, r, ShapeFill::Outline, style)
}

/// Justify each row's content within `r`: leading and trailing blank or
/// transparent cells are trimmed, inner spacing is kept.
pub fn justify(b: &mut TxBuilder, layer: usize, r: Rect, j: Justify) {
    let Some(r) = clip_rect(b, r) else { return };
    let empty = |c: &Option<Cell>| c.is_none_or(|c| c.is_blank());
    for y in r.y..r.y + r.h {
        let row: Vec<Option<Cell>> = (r.x..r.x + r.w).map(|x| b.get(layer, x, y)).collect();
        let Some(first) = row.iter().position(|c| !empty(c)) else {
            continue;
        };
        let last = row.iter().rposition(|c| !empty(c)).unwrap_or(first);
        let content = &row[first..=last];
        let off = match j {
            Justify::Left => 0,
            Justify::Center => (r.w - content.len()) / 2,
            Justify::Right => r.w - content.len(),
        };
        for i in 0..r.w {
            let c = if (off..off + content.len()).contains(&i) { content[i - off] } else { empty_cell(layer) };
            b.set(layer, r.x + i, y, c);
        }
    }
}

/// ACiDDraw block delete: erase `r` and shift content to its right leftwards
/// by `r.w`; the vacated cells at the right edge become blank/transparent.
pub fn delete_block(b: &mut TxBuilder, layer: usize, r: Rect) {
    let Some(r) = clip_rect(b, r) else { return };
    let w = b.width();
    for y in r.y..r.y + r.h {
        let row: Vec<Option<Cell>> =
            (r.x..w).map(|x| if x + r.w < w { b.get(layer, x + r.w, y) } else { empty_cell(layer) }).collect();
        for (i, c) in row.into_iter().enumerate() {
            b.set(layer, r.x + i, y, c);
        }
    }
}

// ------------------------------------------------- canvas-wide structure ops

/// Apply `f(layer_index, cells)` to a copy of every layer.
fn map_layers(b: &mut TxBuilder, f: impl Fn(usize, &mut Vec<Option<Cell>>, usize, usize)) {
    b.replace_canvas(|c| {
        let mut out = c.clone();
        for (li, l) in out.layers.iter_mut().enumerate() {
            f(li, &mut l.cells, c.width, c.height);
        }
        out
    });
}

/// Apply `f` to every cell of every layer of every animation frame
/// (doc-wide conversions).
fn map_cells_all(b: &mut TxBuilder, f: impl Fn(&mut Cell)) {
    b.replace_all_canvases(|c| {
        let mut out = c.clone();
        for l in &mut out.layers {
            l.cells.iter_mut().flatten().for_each(&f);
        }
        out
    });
}

/// Insert a blank row at `y`, shifting rows below down. The canvas keeps its
/// height, so the bottom row falls off (as in ACiDDraw).
pub fn insert_line(b: &mut TxBuilder, y: usize) {
    if y >= b.height() {
        return;
    }
    map_layers(b, |li, cells, w, h| {
        cells.copy_within(y * w..(h - 1) * w, (y + 1) * w);
        cells[y * w..(y + 1) * w].fill(empty_cell(li));
    });
}

/// Delete row `y`, shifting rows below up; a blank row is added at the bottom.
pub fn delete_line(b: &mut TxBuilder, y: usize) {
    if y >= b.height() {
        return;
    }
    map_layers(b, |li, cells, w, h| {
        cells.copy_within((y + 1) * w..h * w, y * w);
        cells[(h - 1) * w..h * w].fill(empty_cell(li));
    });
}

/// Insert a blank column at `x`, shifting columns right; the rightmost
/// column falls off.
pub fn insert_column(b: &mut TxBuilder, x: usize) {
    if x >= b.width() {
        return;
    }
    map_layers(b, |li, cells, w, _| {
        for row in cells.chunks_mut(w) {
            row.copy_within(x..w - 1, x + 1);
            row[x] = empty_cell(li);
        }
    });
}

/// Delete column `x`, shifting columns left; a blank column is added at the right.
pub fn delete_column(b: &mut TxBuilder, x: usize) {
    if x >= b.width() {
        return;
    }
    map_layers(b, |li, cells, w, _| {
        for row in cells.chunks_mut(w) {
            row.copy_within(x + 1..w, x);
            row[w - 1] = empty_cell(li);
        }
    });
}

pub fn resize(b: &mut TxBuilder, width: usize, height: usize) {
    b.replace_all_canvases(|c| c.resized(width.max(1), height.max(1)));
}

/// Crop every layer to `r` (clipped to the canvas).
pub fn crop(b: &mut TxBuilder, r: Rect) {
    let Some(r) = clip_rect(b, r) else { return };
    b.replace_all_canvases(|c| Canvas {
        width: r.w,
        height: r.h,
        layers: c
            .layers
            .iter()
            .map(|l| Layer {
                cells: (r.y..r.y + r.h)
                    .flat_map(|y| l.cells[y * c.width + r.x..y * c.width + r.x + r.w].iter().copied())
                    .collect(),
                ..l.clone()
            })
            .collect(),
    });
}

// ------------------------------------------------------------------ layers

/// Insert a new empty layer at `index`, clamped to 1..=len so the
/// background stays at the bottom. Returns its index.
pub fn add_layer(b: &mut TxBuilder, name: &str, index: usize) -> usize {
    let len = b.canvas().layers.len();
    let idx = index.clamp(len.min(1), len);
    let (w, h) = (b.width(), b.height());
    b.replace_canvas(|c| {
        let mut out = c.clone();
        out.layers.insert(idx, Layer::new(name, w, h));
        out
    });
    idx
}

/// Copy layer `index` (cells and flags) into a new layer right above it,
/// named "<name> copy". Returns the new layer's index.
pub fn duplicate_layer(b: &mut TxBuilder, index: usize) -> usize {
    let len = b.canvas().layers.len();
    if index >= len {
        return index.min(len.saturating_sub(1));
    }
    b.replace_canvas(|c| {
        let mut out = c.clone();
        let mut l = out.layers[index].clone();
        l.name = format!("{} copy", l.name);
        // Keep "background" semantics on layer 0 only: a copy of it is a normal layer.
        l.locked = false;
        out.layers.insert(index + 1, l);
        out
    });
    index + 1
}

/// Remove a layer. The last remaining layer is never removed.
pub fn remove_layer(b: &mut TxBuilder, index: usize) {
    let len = b.canvas().layers.len();
    if len <= 1 || index >= len {
        return;
    }
    b.replace_canvas(|c| {
        let mut out = c.clone();
        out.layers.remove(index);
        out
    });
}

/// Move layer `from` to position `to` (clamped).
pub fn move_layer(b: &mut TxBuilder, from: usize, to: usize) {
    let len = b.canvas().layers.len();
    let to = to.min(len.saturating_sub(1));
    if from >= len || from == to {
        return;
    }
    b.replace_canvas(|c| {
        let mut out = c.clone();
        let l = out.layers.remove(from);
        out.layers.insert(to, l);
        out
    });
}

pub fn set_layer_props(b: &mut TxBuilder, index: usize, props: &LayerProps) {
    if index >= b.canvas().layers.len() {
        return;
    }
    b.replace_canvas(|c| {
        let mut out = c.clone();
        let l = &mut out.layers[index];
        if let Some(n) = &props.name {
            l.name = n.clone();
        }
        if let Some(v) = props.visible {
            l.visible = v;
        }
        if let Some(v) = props.locked {
            l.locked = v;
        }
        if let Some(r) = props.reference {
            l.kind = if r { LayerKind::Reference } else { LayerKind::Normal };
        }
        out
    });
}

/// Merge layer `index` into the one below it: its opaque cells overwrite the
/// lower layer, then it is removed.
pub fn merge_down(b: &mut TxBuilder, index: usize) {
    let len = b.canvas().layers.len();
    if index == 0 || index >= len {
        return;
    }
    b.replace_canvas(|c| {
        let mut out = c.clone();
        let upper = out.layers.remove(index);
        for (dst, src) in out.layers[index - 1].cells.iter_mut().zip(upper.cells) {
            if src.is_some() {
                *dst = src;
            }
        }
        out
    });
}

// ---------------------------------------------------------------- doc-wide

/// Switch Classic <-> Modern. Modern -> Classic downsamples every cell.
pub fn set_kind(b: &mut TxBuilder, kind: DocKind) {
    if b.meta().kind == kind {
        return;
    }
    b.replace_meta(|m| m.kind = kind);
    if kind == DocKind::Classic {
        let meta = b.meta().clone();
        map_cells_all(b, |c| *c = crate::model::conform_cell(&meta, *c));
    }
}

/// Toggle iCE colors. Turning it off maps bright backgrounds to their dark
/// counterparts (Classic docs; Modern docs only record the flag).
pub fn set_ice(b: &mut TxBuilder, on: bool) {
    if b.meta().ice == on {
        return;
    }
    b.replace_meta(|m| m.ice = on);
    if !on && b.meta().kind == DocKind::Classic {
        map_cells_all(b, |c| {
            if let Color::Pal(i @ 8..=15) = c.bg {
                c.bg = Color::Pal(i - 8);
            }
        });
    }
}
