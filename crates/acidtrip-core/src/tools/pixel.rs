//! Half-block "pixel space": each cell is two square pixels (top/bottom)
//! drawn with ▀ ▄ █ and space. Pixel y ranges over 0..height*2.
//!
//! Setting a pixel preserves the other half of the cell. Encoding prefers
//! black in the background and bright colors in the foreground, because
//! Classic docs without iCE can only hold colors 0-7 in the background.
//! When two different bright colors must share a cell there, the one not
//! just drawn is dimmed to its dark counterpart (color - 8).

use super::geom;
use crate::color::Color;
use crate::model::{Cell, DocKind};
use crate::tx::TxBuilder;

fn is_black(c: Color) -> bool {
    matches!(c, Color::Pal(0) | Color::Rgb(0, 0, 0))
}

fn norm_black(c: Color) -> Color {
    if is_black(c) { Color::BLACK } else { c }
}

/// Decode a cell into its (top, bottom) pixel colors. Glyphs other than
/// half/full blocks count as their ink color (fg), except ░ and blanks which
/// count as background.
pub fn cell_pixels(cell: Cell) -> (Color, Color) {
    let (fg, bg) = (norm_black(cell.fg), norm_black(cell.bg));
    match cell.ch {
        '▀' => (fg, bg),
        '▄' => (bg, fg),
        ' ' | '\u{0}' | '\u{A0}' | '░' => (bg, bg),
        _ => (fg, fg),
    }
}

/// Encode a (top, bottom) pair as a cell. `bright_bg` allows colors 8-15 in
/// the background; without it two different brights dim the half that is
/// not `keep_top`'s choice.
pub fn pixels_to_cell(top: Color, bottom: Color, keep_top: bool, bright_bg: bool) -> Cell {
    let (top, bottom) = (norm_black(top), norm_black(bottom));
    if top == bottom {
        return if is_black(top) { Cell::BLANK } else { Cell::new('█', top, Color::BLACK) };
    }
    let bg_ok = |c: Color| bright_bg || matches!(c, Color::Pal(i) if i < 8);
    let upper = |fg, bg| Cell::new('▀', fg, bg);
    let lower = |fg, bg| Cell::new('▄', fg, bg);
    if is_black(bottom) {
        upper(top, Color::BLACK)
    } else if is_black(top) {
        lower(bottom, Color::BLACK)
    } else if bg_ok(bottom) {
        upper(top, bottom)
    } else if bg_ok(top) {
        lower(bottom, top)
    } else if keep_top {
        upper(top, dim(bottom))
    } else {
        lower(bottom, dim(top))
    }
}

fn dim(c: Color) -> Color {
    match c {
        Color::Pal(i @ 8..=15) => Color::Pal(i - 8),
        other => other,
    }
}

/// Map a color to what the doc can hold (Classic: palette index < 16).
fn norm(b: &TxBuilder, c: Color) -> Color {
    let m = b.meta();
    let c = match (m.kind, c) {
        (DocKind::Modern, c) => c,
        (DocKind::Classic, Color::Pal(i)) if i < 16 => c,
        (DocKind::Classic, c) => Color::Pal(m.palette.nearest(c.rgb(&m.palette), 16)),
    };
    norm_black(c)
}

fn bright_bg(b: &TxBuilder) -> bool {
    let m = b.meta();
    m.kind == DocKind::Modern || m.ice
}

fn pix_size(b: &TxBuilder) -> (usize, usize) {
    (b.width(), b.height() * 2)
}

/// Color of a pixel, reading the composite (None = black/empty).
pub fn get(b: &TxBuilder, px: usize, py: usize) -> Option<Color> {
    let (w, h) = pix_size(b);
    if px >= w || py >= h {
        return None;
    }
    let (t, bo) = cell_pixels(b.composite(px, py / 2));
    let c = if py.is_multiple_of(2) { t } else { bo };
    (!is_black(c)).then_some(c)
}

/// Set one pixel, preserving the other half of the cell. Classic docs
/// resolve two bright colors in one cell by dimming the other half.
pub fn set(b: &mut TxBuilder, layer: usize, px: usize, py: usize, color: Color) {
    let (w, h) = pix_size(b);
    if px >= w || py >= h {
        return;
    }
    let color = norm(b, color);
    let cy = py / 2;
    let cur = b.get(layer, px, cy).unwrap_or_else(|| b.composite(px, cy));
    let (mut t, mut bo) = cell_pixels(cur);
    let top = py.is_multiple_of(2);
    if top {
        t = color;
    } else {
        bo = color;
    }
    let cell = pixels_to_cell(t, bo, top, bright_bg(b));
    b.set(layer, px, cy, Some(cell));
}

fn set_i(b: &mut TxBuilder, layer: usize, x: i64, y: i64, color: Color) {
    if x >= 0 && y >= 0 {
        set(b, layer, x as usize, y as usize, color);
    }
}

/// Bresenham line in pixel space; off-canvas parts are clipped.
pub fn line(b: &mut TxBuilder, layer: usize, x0: i64, y0: i64, x1: i64, y1: i64, color: Color) {
    for (x, y) in geom::line(x0, y0, x1, y1) {
        set_i(b, layer, x, y, color);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn rect(b: &mut TxBuilder, layer: usize, x: usize, y: usize, w: usize, h: usize, color: Color, filled: bool) {
    if w == 0 || h == 0 {
        return;
    }
    let (pw, ph) = pix_size(b);
    let (x1, y1) = (x + w - 1, y + h - 1);
    for py in y..=y1.min(ph.saturating_sub(1)) {
        for px in x..=x1.min(pw.saturating_sub(1)) {
            if filled || py == y || py == y1 || px == x || px == x1 {
                set(b, layer, px, py, color);
            }
        }
    }
}

/// Midpoint ellipse centered at (cx, cy) with radii (rx, ry) in pixels.
#[allow(clippy::too_many_arguments)]
pub fn ellipse(b: &mut TxBuilder, layer: usize, cx: i64, cy: i64, rx: i64, ry: i64, color: Color, filled: bool) {
    let (rx, ry) = (rx.abs(), ry.abs());
    let (x0, y0, x1, y1) = (cx - rx, cy - ry, cx + rx, cy + ry);
    if filled {
        let (pw, ph) = pix_size(b);
        for (y, a, z) in geom::ellipse_spans(x0, y0, x1, y1) {
            if y < 0 || y >= ph as i64 {
                continue;
            }
            for x in a.max(0)..=z.min(pw as i64 - 1) {
                set_i(b, layer, x, y, color);
            }
        }
    } else {
        for (x, y) in geom::ellipse_outline(x0, y0, x1, y1) {
            set_i(b, layer, x, y, color);
        }
    }
}

/// 4x4 Bayer ordered-dither matrix (values 0-15).
pub const BAYER4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Bayer threshold in (0, 1) for a pixel position.
pub fn bayer(x: usize, y: usize) -> f32 {
    (BAYER4[y % 4][x % 4] as f32 + 0.5) / 16.0
}

/// Bayer 4x4 ordered dither between two colors; `mix` 0.0 = all c1, 1.0 = all c2.
#[allow(clippy::too_many_arguments)]
pub fn dither_rect(
    b: &mut TxBuilder,
    layer: usize,
    x: usize,
    y: usize,
    w: usize,
    h: usize,
    c1: Color,
    c2: Color,
    mix: f32,
) {
    let (pw, ph) = pix_size(b);
    for py in y..y.saturating_add(h).min(ph) {
        for px in x..x.saturating_add(w).min(pw) {
            let c = if mix > bayer(px, py) { c2 } else { c1 };
            set(b, layer, px, py, c);
        }
    }
}

/// 4-connected fill of the pixel region sharing the start pixel's color.
pub fn flood_fill(b: &mut TxBuilder, layer: usize, px: usize, py: usize, color: Color) {
    let (w, h) = pix_size(b);
    if px >= w || py >= h {
        return;
    }
    let start = get(b, px, py);
    let target = norm(b, color);
    if start == (!is_black(target)).then_some(target) {
        return;
    }
    let mut seen = vec![false; w * h];
    let mut stack = vec![(px, py)];
    let matches = |b: &TxBuilder, seen: &[bool], x: usize, y: usize| !seen[y * w + x] && get(b, x, y) == start;
    while let Some((sx, sy)) = stack.pop() {
        if !matches(b, &seen, sx, sy) {
            continue;
        }
        let mut l = sx;
        while l > 0 && matches(b, &seen, l - 1, sy) {
            l -= 1;
        }
        let mut r = sx;
        while r + 1 < w && matches(b, &seen, r + 1, sy) {
            r += 1;
        }
        for x in l..=r {
            seen[sy * w + x] = true;
        }
        for ny in [sy.wrapping_sub(1), sy + 1] {
            if ny >= h {
                continue;
            }
            let mut in_run = false;
            for x in l..=r {
                let ok = matches(b, &seen, x, ny);
                if ok && !in_run {
                    stack.push((x, ny));
                }
                in_run = ok;
            }
        }
        for x in l..=r {
            set(b, layer, x, sy, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Document;

    const RED: Color = Color::Pal(4);
    const YELLOW: Color = Color::Pal(14);
    const LRED: Color = Color::Pal(12);
    const WHITE: Color = Color::WHITE;

    fn doc(kind: DocKind, ice: bool) -> Document {
        let mut d = Document::new(kind, 8, 4);
        d.meta.ice = ice;
        d
    }

    #[test]
    fn decode_table() {
        let c = |ch| Cell::new(ch, YELLOW, RED);
        assert_eq!(cell_pixels(c('▀')), (YELLOW, RED));
        assert_eq!(cell_pixels(c('▄')), (RED, YELLOW));
        assert_eq!(cell_pixels(c('█')), (YELLOW, YELLOW));
        assert_eq!(cell_pixels(c(' ')), (RED, RED));
        assert_eq!(cell_pixels(c('A')), (YELLOW, YELLOW));
        assert_eq!(cell_pixels(Cell::BLANK), (Color::BLACK, Color::BLACK));
    }

    #[test]
    fn encode_prefers_black_background() {
        assert_eq!(pixels_to_cell(Color::BLACK, LRED, true, true), Cell::new('▄', LRED, Color::BLACK));
        assert_eq!(pixels_to_cell(LRED, Color::BLACK, true, true), Cell::new('▀', LRED, Color::BLACK));
        assert_eq!(pixels_to_cell(LRED, LRED, true, false), Cell::new('█', LRED, Color::BLACK));
        assert!(pixels_to_cell(Color::BLACK, Color::Rgb(0, 0, 0), true, true).is_blank());
    }

    #[test]
    fn encode_puts_bright_in_fg_without_ice() {
        assert_eq!(pixels_to_cell(RED, LRED, true, false), Cell::new('▄', LRED, RED));
        assert_eq!(pixels_to_cell(LRED, RED, true, false), Cell::new('▀', LRED, RED));
        // Two brights: the half not being kept is dimmed.
        assert_eq!(pixels_to_cell(WHITE, LRED, true, false), Cell::new('▀', WHITE, RED));
        assert_eq!(pixels_to_cell(WHITE, LRED, false, false), Cell::new('▄', LRED, Color::LIGHT_GRAY));
        // With bright backgrounds allowed nothing is lost.
        assert_eq!(cell_pixels(pixels_to_cell(WHITE, LRED, true, true)), (WHITE, LRED));
    }

    #[test]
    fn set_preserves_other_half_and_roundtrips() {
        let d = doc(DocKind::Classic, false);
        let mut b = TxBuilder::new(&d, "p");
        set(&mut b, 0, 3, 0, YELLOW);
        set(&mut b, 0, 3, 1, RED);
        assert_eq!(get(&b, 3, 0), Some(YELLOW));
        assert_eq!(get(&b, 3, 1), Some(RED));
        assert_eq!(b.get(0, 3, 0), Some(Cell::new('▀', YELLOW, RED)));
        set(&mut b, 0, 3, 0, WHITE);
        assert_eq!((get(&b, 3, 0), get(&b, 3, 1)), (Some(WHITE), Some(RED)));
        set(&mut b, 0, 3, 0, Color::BLACK);
        assert_eq!(b.get(0, 3, 0), Some(Cell::new('▄', RED, Color::BLACK)));
        assert_eq!(get(&b, 3, 0), None);
    }

    #[test]
    fn classic_no_ice_keeps_new_pixel_exact() {
        let d = doc(DocKind::Classic, false);
        let mut b = TxBuilder::new(&d, "p");
        set(&mut b, 0, 0, 0, WHITE);
        set(&mut b, 0, 0, 1, LRED);
        assert_eq!(get(&b, 0, 1), Some(LRED));
        assert_eq!(get(&b, 0, 0), Some(Color::LIGHT_GRAY));
        let c = b.get(0, 0, 0).unwrap();
        assert!(matches!(c.bg, Color::Pal(i) if i < 8));
    }

    #[test]
    fn ice_allows_two_brights() {
        let d = doc(DocKind::Classic, true);
        let mut b = TxBuilder::new(&d, "p");
        set(&mut b, 0, 0, 0, WHITE);
        set(&mut b, 0, 0, 1, LRED);
        assert_eq!((get(&b, 0, 0), get(&b, 0, 1)), (Some(WHITE), Some(LRED)));
    }

    #[test]
    fn modern_rgb_pixels() {
        let d = doc(DocKind::Modern, false);
        let mut b = TxBuilder::new(&d, "p");
        let (p, q) = (Color::Rgb(10, 200, 30), Color::Rgb(250, 0, 90));
        set(&mut b, 0, 1, 2, p);
        set(&mut b, 0, 1, 3, q);
        assert_eq!((get(&b, 1, 2), get(&b, 1, 3)), (Some(p), Some(q)));
    }

    #[test]
    fn classic_rgb_is_quantized() {
        let d = doc(DocKind::Classic, true);
        let mut b = TxBuilder::new(&d, "p");
        set(&mut b, 0, 0, 0, Color::Rgb(250, 250, 90));
        assert_eq!(get(&b, 0, 0), Some(YELLOW));
    }

    #[test]
    fn out_of_bounds_is_ignored() {
        let d = doc(DocKind::Classic, true);
        let mut b = TxBuilder::new(&d, "p");
        set(&mut b, 0, 8, 0, RED);
        set(&mut b, 0, 0, 8, RED);
        line(&mut b, 0, -10, -10, 100, 50, RED);
        ellipse(&mut b, 0, 0, 0, 40, 40, RED, true);
        ellipse(&mut b, 0, -3, -3, 40, 40, RED, false);
        rect(&mut b, 0, 5, 5, 100, 100, RED, true);
        dither_rect(&mut b, 0, 0, 0, 100, 100, RED, YELLOW, 0.5);
        assert_eq!(get(&b, 100, 100), None);
    }
}
