//! Smart shapes: rectangles, ellipses and lines that pick their rendering
//! from the brush glyph (see [`shape_style`]).
//!
//! * Solid blocks `█ ▀ ▄ ▌ ▐` draw in half-block pixel space (one cell = two
//!   square pixels), so circles and diagonals get twice the vertical
//!   resolution.
//! * Box-drawing glyphs draw boxes in their family's [`BoxStyle`], straight
//!   lines with the family's `─`/`│`, and diagonals and ellipses with the
//!   [`pen`](super::pen) restricted to the family's glyphs (its line-drawing
//!   mode: a connected `──┐ └──` staircase).
//! * Anything else (shades, letters) keeps the cell-level behaviour.
//!
//! Only [`PaintMode::Char`] uses the smart renderings; other modes (erase,
//! shade, colorize, ...) keep the cell-level tools so they behave as before.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::pen::{self, PenStroke};
use super::{BoxStyle, Ctx, PaintMode, Rect, ShapeFill, Symmetry, geom, pixel};
use crate::tx::TxBuilder;

/// How the smart shape tools render for a brush glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeStyle {
    /// Half-block pixels (`▀ ▄ █`) at 2x vertical resolution.
    HalfBlock,
    /// Box-drawing family.
    Box(BoxStyle),
    /// The brush glyph in every cell (cell-level).
    Glyph,
}

pub fn shape_style(brush: char) -> ShapeStyle {
    match brush {
        '█' | '▀' | '▄' | '▌' | '▐' => ShapeStyle::HalfBlock,
        '─' | '│' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' => {
            ShapeStyle::Box(BoxStyle::Single)
        }
        '═' | '║' | '╔' | '╗' | '╚' | '╝' | '╠' | '╣' | '╦' | '╩' | '╬' => {
            ShapeStyle::Box(BoxStyle::Double)
        }
        '╒' | '╕' | '╘' | '╛' | '╞' | '╡' | '╤' | '╧' | '╪' => ShapeStyle::Box(BoxStyle::DoubleH),
        '╓' | '╖' | '╙' | '╜' | '╟' | '╢' | '╥' | '╨' | '╫' => ShapeStyle::Box(BoxStyle::DoubleV),
        '╭' | '╮' | '╰' | '╯' => ShapeStyle::Box(BoxStyle::Rounded),
        _ => ShapeStyle::Glyph,
    }
}

/// Style for `ctx`: non-Char modes always use the cell-level tools.
fn style_for(ctx: &Ctx) -> ShapeStyle {
    if ctx.mode == PaintMode::Char { shape_style(ctx.brush.ch) } else { ShapeStyle::Glyph }
}

/// (horizontal, vertical) straight-line glyphs of a box family.
fn box_lines(style: BoxStyle) -> (char, char) {
    match style {
        BoxStyle::Double => ('═', '║'),
        BoxStyle::DoubleH => ('═', '│'),
        BoxStyle::DoubleV => ('─', '║'),
        _ => ('─', '│'),
    }
}

/// Glyphs the pen uses for box-family paths: straights, corners, tees and
/// the cross (so paths merge with line work already on the canvas).
pub fn box_candidates(style: BoxStyle) -> Vec<char> {
    let v: &str = match style {
        BoxStyle::Double => "═║╔╗╚╝╠╣╦╩╬",
        BoxStyle::DoubleH => "═│╒╕╘╛╞╡╤╧╪",
        BoxStyle::DoubleV => "─║╓╖╙╜╟╢╥╨╫",
        BoxStyle::Rounded => "─│╭╮╰╯├┤┬┴┼",
        _ => "─│┌┐└┘├┤┬┴┼",
    };
    v.chars().collect()
}

/// Where box glyphs cross inside a cell, in glyph pixels (─ is row 7,
/// │ is columns 3-4 of the VGA font).
fn box_center(x: usize, y: usize) -> (f32, f32) {
    ((x * pen::GW) as f32 + 4.0, (y * pen::GH) as f32 + 7.5)
}

/// Draw a pen path through glyph-pixel points with the box family's glyphs.
fn pen_path(b: &mut TxBuilder, ctx: &Ctx, pts: &[(f32, f32)], style: BoxStyle) {
    let mut s = PenStroke::new(pen::RADIUS_BOX);
    for &(x, y) in pts {
        s.add_point(x, y);
    }
    let mut cells: Vec<_> = s.cells().collect();
    cells.sort_unstable_by_key(|&(x, y)| (y, x));
    pen::apply(b, ctx, &s, cells, &box_candidates(style));
}

/// Set half-block pixels in the brush fg, plus their symmetry mirrors.
fn plot(b: &mut TxBuilder, ctx: &Ctx, pts: impl IntoIterator<Item = (i64, i64)>) {
    let (pw, ph) = (b.width() as i64, b.height() as i64 * 2);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (x, y) in pts {
        if !(0..pw).contains(&x) || !(0..ph).contains(&y) {
            continue;
        }
        let (mx, my) = (pw - 1 - x, ph - 1 - y);
        let mirrors: &[(i64, i64)] = match ctx.symmetry {
            Symmetry::None => &[],
            Symmetry::X => &[(mx, y)],
            Symmetry::Y => &[(x, my)],
            Symmetry::Both => &[(mx, y), (x, my), (mx, my)],
        };
        for &p in std::iter::once(&(x, y)).chain(mirrors) {
            if seen.insert(p) {
                out.push(p);
            }
        }
    }
    for (x, y) in out {
        pixel::set(b, ctx.layer, x as usize, y as usize, ctx.brush.fg);
    }
}

/// Rectangle between two corners, rendered per [`shape_style`] of the brush:
/// half-block outline (`█▀▀█` / `█▄▄█`) or solid fill, the brush's box
/// family, or the brush glyph.
pub fn smart_rect(b: &mut TxBuilder, ctx: &Ctx, r: Rect, fill: ShapeFill) {
    if r.w == 0 || r.h == 0 {
        return;
    }
    match style_for(ctx) {
        ShapeStyle::HalfBlock => {
            let (x0, y0, x1, y1) = (r.x as i64, 2 * r.y as i64, r.right() as i64, 2 * r.bottom() as i64 + 1);
            let filled = fill == ShapeFill::Filled;
            let pts = (y0..=y1)
                .flat_map(|y| (x0..=x1).map(move |x| (x, y)))
                .filter(|&(x, y)| filled || x == x0 || x == x1 || y == y0 || y == y1);
            plot(b, ctx, pts);
        }
        ShapeStyle::Box(style) => super::rect(b, ctx, r, fill, style),
        ShapeStyle::Glyph => super::rect(b, ctx, r, fill, BoxStyle::Brush),
    }
}

/// Ellipse inscribed in `r` (cell space), rendered per [`shape_style`]:
/// half-block pixels at 2x vertical resolution, a pen-fitted outline in the
/// brush's box family, or the brush glyph.
pub fn smart_ellipse(b: &mut TxBuilder, ctx: &Ctx, r: Rect, fill: ShapeFill) {
    if r.w == 0 || r.h == 0 {
        return;
    }
    match style_for(ctx) {
        ShapeStyle::HalfBlock => {
            let (x0, y0, x1, y1) = (r.x as i64, 2 * r.y as i64, r.right() as i64, 2 * r.bottom() as i64 + 1);
            match fill {
                ShapeFill::Outline => plot(b, ctx, geom::ellipse_outline(x0, y0, x1, y1)),
                ShapeFill::Filled => plot(
                    b,
                    ctx,
                    geom::ellipse_spans(x0, y0, x1, y1).into_iter().flat_map(|(y, a, z)| (a..=z).map(move |x| (x, y))),
                ),
            }
        }
        ShapeStyle::Box(style) => {
            if r.w == 1 || r.h == 1 {
                return smart_line(b, ctx, r.x, r.y, r.right(), r.bottom());
            }
            if fill == ShapeFill::Filled {
                // Clear the inside like a filled box does.
                let blank = Ctx { brush: super::Brush { ch: ' ', ..ctx.brush }, ..*ctx };
                super::ellipse(b, &blank, r, ShapeFill::Filled);
            }
            let (ax, ay) = box_center(r.x, r.y);
            let (bx, by) = box_center(r.right(), r.bottom());
            let (cx, cy, rx, ry) = ((ax + bx) / 2.0, (ay + by) / 2.0, (bx - ax) / 2.0, (by - ay) / 2.0);
            // About one point per pixel of perimeter.
            let n = ((std::f32::consts::TAU * rx.max(ry)).ceil() as usize).max(16);
            let pts: Vec<(f32, f32)> = (0..=n)
                .map(|i| {
                    let t = std::f32::consts::TAU * i as f32 / n as f32;
                    (cx + rx * t.cos(), cy + ry * t.sin())
                })
                .collect();
            pen_path(b, ctx, &pts, style);
        }
        ShapeStyle::Glyph => super::ellipse(b, ctx, r, fill),
    }
}

/// Line between two cells, rendered per [`shape_style`] of the brush.
///
/// * Half blocks: a pixel line at 2x vertical resolution. Horizontal lines
///   are full `█` rows (`▀`/`▄` brushes: that half only); `▌`/`▐` brushes
///   draw vertical lines with the brush glyph.
/// * Box family: `─`/`═` horizontally, `│`/`║` vertically, and a pen-fitted
///   path of the family's glyphs for diagonals.
/// * Otherwise: the brush glyph on a cell-level line.
pub fn smart_line(b: &mut TxBuilder, ctx: &Ctx, x0: usize, y0: usize, x1: usize, y1: usize) {
    match style_for(ctx) {
        ShapeStyle::HalfBlock => {
            let ch = ctx.brush.ch;
            if x0 == x1 && matches!(ch, '▌' | '▐') {
                return super::paint_line(b, ctx, x0, y0, x1, y1);
            }
            let (x0, y0, x1, y1) = (x0 as i64, y0 as i64, x1 as i64, y1 as i64);
            let (top, bot) = (|y: i64| 2 * y, |y: i64| 2 * y + 1);
            let segs: Vec<(i64, i64)> = match ch {
                '▀' => vec![(top(y0), top(y1))],
                '▄' => vec![(bot(y0), bot(y1))],
                _ if y0 == y1 => vec![(top(y0), top(y1)), (bot(y0), bot(y1))],
                _ if y1 > y0 => vec![(top(y0), bot(y1))],
                _ => vec![(bot(y0), top(y1))],
            };
            let pts: Vec<(i64, i64)> =
                segs.into_iter().flat_map(|(py0, py1)| geom::line(x0, py0, x1, py1).collect::<Vec<_>>()).collect();
            plot(b, ctx, pts);
        }
        ShapeStyle::Box(style) => {
            let (hz, vt) = box_lines(style);
            if y0 == y1 || x0 == x1 {
                let ch = if y0 == y1 { hz } else { vt };
                let c = Ctx { brush: super::Brush { ch, ..ctx.brush }, ..*ctx };
                return super::paint_line(b, &c, x0, y0, x1, y1);
            }
            pen_path(b, ctx, &[box_center(x0, y0), box_center(x1, y1)], style);
        }
        ShapeStyle::Glyph => super::paint_line(b, ctx, x0, y0, x1, y1),
    }
}
