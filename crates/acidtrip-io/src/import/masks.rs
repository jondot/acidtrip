//! Glyphs the block search tries, each as the rectangles its lit pixels
//! make in the 8x16 cell, so a region's color sums cost a few lookups.

use std::sync::LazyLock;

use acidtrip_core::render::glyph_pixel;

use super::Glyphs;

pub struct Mask {
    pub ch: char,
    /// (x0, y0, x1, y1), ends exclusive.
    pub rects: Vec<(usize, usize, usize, usize)>,
    /// Lit pixels.
    pub lit: f32,
}

/// Halves: the four every set has.
const HALVES: &str = "▀▄▌▐";
/// Quadrants and eighths (lower, upper, left, right).
const EXTRA: &str = "▘▝▖▗▚▞▙▛▜▟▁▂▃▅▆▇▔▏▎▍▋▊▉▕";

fn mask(ch: char) -> Mask {
    let mut rects = vec![];
    // Runs per row; a run that repeats on the next row grows its rectangle.
    let mut open: Vec<(usize, usize, usize)> = vec![]; // (x0, x1, y0)
    for y in 0..=16 {
        let mut runs = vec![];
        if y < 16 {
            let mut x = 0;
            while x < 8 {
                if glyph_pixel(ch, x as u32, y as u32) {
                    let x0 = x;
                    while x < 8 && glyph_pixel(ch, x as u32, y as u32) {
                        x += 1;
                    }
                    runs.push((x0, x));
                } else {
                    x += 1;
                }
            }
        }
        let mut next = vec![];
        for &(x0, x1, y0) in &open {
            if runs.contains(&(x0, x1)) {
                next.push((x0, x1, y0));
            } else {
                rects.push((x0, y0, x1, y));
            }
        }
        for &(x0, x1) in &runs {
            if !open.iter().any(|&(a, b, _)| (a, b) == (x0, x1)) {
                next.push((x0, x1, y));
            }
        }
        open = next;
    }
    let lit = rects.iter().map(|&(x0, y0, x1, y1)| ((x1 - x0) * (y1 - y0)) as f32).sum();
    Mask { ch, rects, lit }
}

static CP437: LazyLock<Vec<Mask>> = LazyLock::new(|| HALVES.chars().map(mask).collect());

static EXTENDED: LazyLock<Vec<Mask>> = LazyLock::new(|| {
    HALVES
        .chars()
        .chain(EXTRA.chars())
        .chain((0x1FB00..=0x1FB3B).filter_map(char::from_u32))
        .map(mask)
        .collect()
});

pub fn masks(set: Glyphs) -> &'static [Mask] {
    match set {
        Glyphs::Cp437 => &CP437,
        Glyphs::Extended => &EXTENDED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles_cover_the_glyph() {
        for m in masks(Glyphs::Extended) {
            let mut n = 0;
            for y in 0..16 {
                for x in 0..8 {
                    let inside = m.rects.iter().any(|&(x0, y0, x1, y1)| x >= x0 && x < x1 && y >= y0 && y < y1);
                    assert_eq!(inside, glyph_pixel(m.ch, x as u32, y as u32), "{} at {x},{y}", m.ch);
                    n += inside as usize;
                }
            }
            assert_eq!(n as f32, m.lit);
            assert!(m.lit > 0.0 && m.lit < 128.0, "{}", m.ch);
        }
        assert_eq!(masks(Glyphs::Extended).len(), 4 + 24 + 60);
        let q = mask('▚');
        assert_eq!(q.rects.len(), 2);
    }
}
