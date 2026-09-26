//! Downscaled previews drawn with half blocks: the canvas minimap (with the
//! viewport highlighted, click to jump) and layer thumbnails.

use std::sync::OnceLock;

use acidtrip_core::{Canvas, Cell, Palette, cp437, render};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

/// Fraction of lit pixels for each CP437 glyph (█ = 1.0, space = 0.0).
fn ink_table() -> &'static [f32; 256] {
    static T: OnceLock<[f32; 256]> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0.0; 256];
        for (b, v) in t.iter_mut().enumerate() {
            let ch = cp437::to_char(b as u8);
            let lit = (0..16)
                .flat_map(|y| (0..8).map(move |x| (x, y)))
                .filter(|&(x, y)| render::glyph_pixel(ch, x, y))
                .count();
            *v = lit as f32 / 128.0;
        }
        t
    })
}

/// The color a cell reads as from a distance: fg and bg mixed by ink coverage.
pub fn cell_rgb(c: &Cell, pal: &Palette) -> [u8; 3] {
    let ink = cp437::from_char(c.ch).map(|b| ink_table()[b as usize]).unwrap_or(0.5);
    let (f, b) = (c.fg.rgb(pal), c.bg.rgb(pal));
    [0, 1, 2].map(|i| (f[i] as f32 * ink + b[i] as f32 * (1.0 - ink)).round() as u8)
}

/// Rows of half-block cells, each (top, bottom) pixel color; None = empty.
type Pixels = Vec<Vec<(Option<[u8; 3]>, Option<[u8; 3]>)>>;

pub fn draw_samples(buf: &mut Buffer, area: Rect, px: &Pixels, empty: [u8; 3], shade: impl Fn(usize, usize) -> bool) {
    for (r, row) in px.iter().enumerate() {
        for (c, &(t, b)) in row.iter().enumerate() {
            let (x, y) = (area.x + c as u16, area.y + r as u16);
            if x >= area.right() || y >= area.bottom() {
                continue;
            }
            let dim = shade(c, r);
            let col = |p: Option<[u8; 3]>| {
                let [r0, g0, b0] = p.unwrap_or(empty);
                if dim { Color::Rgb(r0 / 3, g0 / 3, b0 / 3) } else { Color::Rgb(r0, g0, b0) }
            };
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char('▀').set_style(Style::new().fg(col(t)).bg(col(b)));
            }
        }
    }
}

/// Mean color of the cells in a `cols` x `rows` block (None if all empty).
fn average(
    x0: usize,
    y0: usize,
    cols: usize,
    rows: usize,
    w: usize,
    h: usize,
    get: impl Fn(usize, usize) -> Option<[u8; 3]>,
) -> Option<[u8; 3]> {
    let (mut sum, mut n) = ([0u32; 3], 0u32);
    for y in y0..(y0 + rows).min(h) {
        for x in x0..(x0 + cols).min(w) {
            if let Some(c) = get(x, y) {
                for i in 0..3 {
                    sum[i] += c[i] as u32;
                }
                n += 1;
            }
        }
    }
    (n > 0).then(|| sum.map(|v| (v / n) as u8))
}

/// Where the minimap was drawn and how it maps back to the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MiniGeom {
    pub rect: Rect,
    /// Canvas columns per minimap terminal column.
    pub cols_per_cell: f32,
    /// Canvas rows per minimap terminal row.
    pub rows_per_cell: f32,
    /// First canvas row shown (tall art scrolls the minimap).
    pub row0: usize,
}

/// Composite minimap, VS Code style: the canvas width fills the panel, and
/// tall art scrolls so the viewport stays in the middle.
pub fn draw_canvas(
    buf: &mut Buffer,
    area: Rect,
    canvas: &Canvas,
    pal: &Palette,
    view: (usize, usize, usize, usize),
) -> MiniGeom {
    let (w, h) = (canvas.width, canvas.height);
    let scale = w.div_ceil(area.width.max(1) as usize).max(1);
    let cols = w.div_ceil(scale).min(area.width as usize);
    // Each text row holds 2 pixel rows; a pixel row is `scale` canvas rows / 2
    // (cells are twice as tall as wide), so one text row = `scale` canvas rows.
    let total_rows = h.div_ceil(scale);
    let rows = total_rows.min(area.height as usize).max(1);
    let (vx, vy, vw, vh) = view;
    let center_row = (vy + vh / 2) / scale;
    let first = center_row.saturating_sub(rows / 2).min(total_rows.saturating_sub(rows));
    let row0 = first * scale;
    let px: Pixels = (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    // Average every cell the pixel covers: a true zoom-out, not a sample.
                    let at = |half: usize| {
                        let (x0, y0) = (c * scale, row0 + r * scale + half * scale / 2);
                        let rows = (scale / 2).max(1);
                        average(x0, y0, scale, rows, w, h, |x, y| Some(cell_rgb(&canvas.composite(x, y), pal)))
                    };
                    (at(0), at(1))
                })
                .collect()
        })
        .collect();
    let inside = |c: usize, r: usize| {
        let (x, y) = (c * scale, row0 + r * scale);
        x + scale > vx && x < vx + vw && y + scale > vy && y < vy + vh
    };
    let rect = Rect::new(area.x, area.y, cols as u16, rows as u16);
    draw_samples(buf, rect, &px, [0, 0, 0], |c, rr| !inside(c, rr));
    MiniGeom { rect, cols_per_cell: scale as f32, rows_per_cell: scale as f32, row0 }
}

/// Thumbnail of one layer alone, filling the width; tall art shows the
/// rows starting at `row0` (the viewport). Transparent cells show dark gray.
pub fn draw_layer(buf: &mut Buffer, area: Rect, canvas: &Canvas, layer: usize, pal: &Palette, row0: usize) {
    let (w, h) = (canvas.width, canvas.height);
    let scale = w.div_ceil(area.width.max(1) as usize).max(1);
    let cols = w.div_ceil(scale).min(area.width as usize);
    let rows = h.div_ceil(scale).min(area.height as usize).max(1);
    let row0 = row0.min(h.saturating_sub(rows * scale));
    let px: Pixels = (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    let at = |half: usize| {
                        let (x0, y0) = (c * scale, row0 + r * scale + half * scale / 2);
                        let rows = (scale / 2).max(1);
                        average(x0, y0, scale, rows, w, h, |x, y| canvas.get(layer, x, y).map(|c| cell_rgb(&c, pal)))
                    };
                    (at(0), at(1))
                })
                .collect()
        })
        .collect();
    draw_samples(buf, Rect::new(area.x, area.y, cols as u16, rows as u16), &px, [30, 30, 40], |_, _| false);
}

/// The whole width of the art from its top, filling `area` (the sidebar's
/// gallery strip); `dim` darkens it.
pub fn draw_fit(buf: &mut Buffer, area: Rect, canvas: &Canvas, pal: &Palette, dim: bool) {
    let (w, h) = (canvas.width, canvas.height);
    let scale = w.div_ceil(area.width.max(1) as usize).max(1);
    let cols = w.div_ceil(scale).min(area.width as usize);
    let rows = h.div_ceil(scale).min(area.height as usize).max(1);
    let px: Pixels = (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    let at = |half: usize| {
                        let (x0, y0) = (c * scale, r * scale + half * scale / 2);
                        let rows = (scale / 2).max(1);
                        average(x0, y0, scale, rows, w, h, |x, y| Some(cell_rgb(&canvas.composite(x, y), pal)))
                    };
                    (at(0), at(1))
                })
                .collect()
        })
        .collect();
    draw_samples(buf, Rect::new(area.x, area.y, cols as u16, rows as u16), &px, [0, 0, 0], |_, _| dim);
}

/// A poster of the art from its top: at most 2 art columns per poster
/// column (sharper than fitting the whole width), centered, so wide art
/// shows its middle, where logos usually are. Returns rows drawn.
pub fn draw_thumb(buf: &mut Buffer, area: Rect, canvas: &Canvas, pal: &Palette) -> u16 {
    let (w, h) = (canvas.width, canvas.height);
    let scale = w.div_ceil(area.width.max(1) as usize).clamp(1, 2);
    let cols = w.div_ceil(scale).min(area.width as usize);
    let x0 = (w.saturating_sub(cols * scale)) / 2;
    let rows = h.div_ceil(scale).min(area.height as usize).max(1);
    let px: Pixels = (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    let at = |half: usize| {
                        let (x, y0) = (x0 + c * scale, r * scale + half * scale / 2);
                        let rows = (scale / 2).max(1);
                        average(x, y0, scale, rows, w, h, |x, y| Some(cell_rgb(&canvas.composite(x, y), pal)))
                    };
                    (at(0), at(1))
                })
                .collect()
        })
        .collect();
    // Narrow art sits in the middle of the poster.
    let left = area.x + (area.width - cols as u16) / 2;
    draw_samples(buf, Rect::new(left, area.y, cols as u16, rows as u16), &px, [0, 0, 0], |_, _| false);
    rows as u16
}

/// Rows `y0..y0+h` of a canvas as a new canvas (for rendering a window).
pub fn crop_rows(c: &Canvas, y0: usize, h: usize) -> Canvas {
    let mut out = c.resized(c.width, 0);
    out.height = h;
    for (l, src) in out.layers.iter_mut().zip(&c.layers) {
        l.cells = src.cells[(y0 * c.width).min(src.cells.len())..((y0 + h) * c.width).min(src.cells.len())].to_vec();
        l.cells.resize(c.width * h, None);
    }
    out
}

/// Map a click inside the minimap back to a canvas cell.
pub fn click_to_cell(g: MiniGeom, sx: u16, sy: u16, w: usize, h: usize) -> Option<(usize, usize)> {
    let r = g.rect;
    if sx < r.x || sy < r.y || sx >= r.right() || sy >= r.bottom() {
        return None;
    }
    let x = (((sx - r.x) as f32 + 0.5) * g.cols_per_cell) as usize;
    let y = g.row0 + (((sy - r.y) as f32 + 0.5) * g.rows_per_cell) as usize;
    Some((x.min(w - 1), y.min(h - 1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_mixes() {
        let pal = Palette::default();
        let full = Cell::new('█', acidtrip_core::Color::WHITE, acidtrip_core::Color::BLACK);
        assert_eq!(cell_rgb(&full, &pal), [255, 255, 255]);
        let blank = Cell::BLANK;
        assert_eq!(cell_rgb(&blank, &pal), [0, 0, 0]);
    }
}
