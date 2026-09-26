//! Pixel-exact raster rendering with the VGA 8x16 bitmap font (CP437 glyphs)
//! and GNU Unifont as the fallback for any other Unicode char.
//!
//! Font: `assets/vga8x16.bin` from libansilove (BSD-2-Clause), see
//! `assets/LICENSE-vga8x16-libansilove.txt`.

use image::RgbaImage;

use crate::color::Palette;
use crate::cp437;
use crate::model::{Document, Grid};

const VGA_FONT: &[u8; 4096] = include_bytes!("../assets/vga8x16.bin");
pub const CELL_W: u32 = 8;
pub const CELL_H: u32 = 16;

#[derive(Clone, Copy, Debug)]
pub struct RenderOptions {
    /// Integer pixel scale (1 = 8x16 per cell).
    pub scale: u32,
    /// 9px letter spacing (VGA line-graphics extension for 0xC0-0xDF).
    pub nine_px: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions { scale: 1, nine_px: false }
    }
}

/// Is pixel (px, py) of glyph `ch` set? `px` in 0..8, `py` in 0..16.
pub fn glyph_pixel(ch: char, px: u32, py: u32) -> bool {
    if let Some(b) = cp437::from_char(ch) {
        let row = VGA_FONT[b as usize * 16 + py as usize];
        return row & (0x80 >> px) != 0;
    }
    if let Some(bits) = sextant_bits(ch) {
        // 2x3 blocks: columns of 4, rows of 5/6/5 pixels.
        let row = if py < 5 { 0 } else if py < 11 { 1 } else { 2 };
        return bits & (1 << (row * 2 + (px >= 4) as u32)) != 0;
    }
    match unifont::get_glyph(ch) {
        Some(g) => {
            // Fullwidth glyphs are squeezed into one cell by sampling every other column.
            let gx = if g.is_fullwidth() { px * 2 } else { px };
            g.get_pixel(gx as usize, py as usize)
        }
        None => {
            // Replacement box.
            (px == 1 || px == 6) && (2..14).contains(&py) || (py == 2 || py == 13) && (1..7).contains(&px)
        }
    }
}

/// Which of a sextant's six blocks are lit (bit 0 top left, bit 1 top
/// right, … bit 5 bottom right). U+1FB00..=U+1FB3B skip the patterns that
/// other characters already have: empty, full, and the left and right halves.
pub fn sextant_bits(ch: char) -> Option<u32> {
    let i = (ch as u32).checked_sub(0x1FB00).filter(|&i| i <= 0x3B)?;
    let mut bits = i + 1;
    if bits >= 21 {
        bits += 1;
    }
    if bits >= 42 {
        bits += 1;
    }
    Some(bits)
}

/// The 16 rows of a glyph as 8-bit masks (bit 7 = leftmost pixel).
pub fn glyph_rows(ch: char) -> [u8; 16] {
    if let Some(b) = cp437::from_char(ch) {
        let mut r = [0u8; 16];
        r.copy_from_slice(&VGA_FONT[b as usize * 16..b as usize * 16 + 16]);
        return r;
    }
    let mut r = [0u8; 16];
    for (py, row) in r.iter_mut().enumerate() {
        for px in 0..8 {
            if glyph_pixel(ch, px, py as u32) {
                *row |= 0x80 >> px;
            }
        }
    }
    r
}

/// Render any cell source. `cell(x, y)` returns (char, fg, bg).
pub fn render_cells(
    width: usize,
    height: usize,
    opts: RenderOptions,
    cell: impl Fn(usize, usize) -> (char, [u8; 3], [u8; 3]),
) -> RgbaImage {
    let cw = if opts.nine_px { 9 } else { CELL_W };
    let s = opts.scale.max(1);
    let (iw, ih) = (width as u32 * cw * s, height as u32 * CELL_H * s);
    let mut buf = vec![0u8; (iw * ih * 4) as usize];
    // Glyph bitmaps are looked up once per distinct char, not per pixel.
    let mut glyphs: std::collections::HashMap<char, ([u8; 16], bool)> = std::collections::HashMap::new();
    for cy in 0..height {
        for cx in 0..width {
            let (ch, fg, bg) = cell(cx, cy);
            let (rows, line_char) = *glyphs
                .entry(ch)
                .or_insert_with(|| (glyph_rows(ch), matches!(cp437::from_char(ch), Some(0xC0..=0xDF))));
            let fgp = [fg[0], fg[1], fg[2], 255];
            let bgp = [bg[0], bg[1], bg[2], 255];
            for (py, &bits) in rows.iter().enumerate() {
                for px in 0..cw {
                    let on = if px < 8 {
                        bits & (0x80 >> px) != 0
                    } else {
                        // 9th column repeats column 8 for line-drawing chars.
                        line_char && bits & 1 != 0
                    };
                    let p = if on { fgp } else { bgp };
                    let x0 = (cx as u32 * cw + px) * s;
                    let y0 = (cy as u32 * CELL_H + py as u32) * s;
                    for dy in 0..s {
                        let row = ((y0 + dy) * iw + x0) as usize * 4;
                        for dx in 0..s as usize {
                            buf[row + dx * 4..row + dx * 4 + 4].copy_from_slice(&p);
                        }
                    }
                }
            }
        }
    }
    RgbaImage::from_raw(iw, ih, buf).expect("buffer size matches")
}

pub fn render_grid(g: &Grid, pal: &Palette, opts: RenderOptions) -> RgbaImage {
    render_cells(g.width, g.height, opts, |x, y| {
        let c = g.get(x, y);
        (c.ch, c.fg.rgb(pal), c.bg.rgb(pal))
    })
}

/// Render the flattened document. `rows` limits the height (e.g. to the
/// used area); `None` renders the whole canvas.
pub fn render_document(doc: &Document, rows: Option<usize>, opts: RenderOptions) -> RgbaImage {
    let g = doc.flatten();
    let h = rows.unwrap_or(g.height).clamp(1, g.height.max(1));
    let pal = &doc.meta.palette;
    let nine = opts.nine_px || doc.meta.letter_spacing_9px;
    render_cells(g.width, h, RenderOptions { nine_px: nine, ..opts }, |x, y| {
        let c = g.get(x, y);
        (c.ch, c.fg.rgb(pal), c.bg.rgb(pal))
    })
}

pub fn png_bytes(img: &RgbaImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).expect("png encode");
    out.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::model::{Cell, DocKind};

    #[test]
    fn full_block_is_solid() {
        for py in 0..16 {
            for px in 0..8 {
                assert!(glyph_pixel('█', px, py));
            }
        }
        assert!(!glyph_pixel(' ', 3, 3));
    }

    #[test]
    fn unicode_fallback_draws_something() {
        let lit = (0..16).flat_map(|y| (0..8).map(move |x| (x, y))).filter(|&(x, y)| glyph_pixel('λ', x, y)).count();
        assert!(lit > 5);
    }

    #[test]
    fn render_dimensions_and_colors() {
        let mut d = Document::new(DocKind::Classic, 2, 1);
        d.canvas.layers[0].cells[0] = Some(Cell::new('█', Color::Pal(4), Color::BLACK));
        let img = render_document(&d, None, RenderOptions::default());
        assert_eq!(img.dimensions(), (16, 16));
        assert_eq!(img.get_pixel(0, 0).0, [0xAA, 0, 0, 255]);
        assert_eq!(img.get_pixel(12, 8).0, [0, 0, 0, 255]);
        let img9 = render_document(&d, None, RenderOptions { nine_px: true, scale: 2 });
        assert_eq!(img9.dimensions(), (36, 32));
    }
}
