//! Keeping drawn lines. Outlines and ink strokes are often a pixel or two
//! wide: averaged into 8×16 cells they fade to a tint and vanish. Here
//! they are found at source resolution (clearly darker than what is around
//! them), carried through the shrink at full darkness and a little thicker
//! (PixelOE's idea: pick the dark pixel where there is ink), and weighted
//! up so the glyph search spends its shapes on them.

use super::lab::{self, Lab};

/// Ink strength 0..1 per pixel of a linear-light image (`w`×`h`, RGB):
/// how much a pixel sits in a dark feature thinner than about `r` pixels
/// (a black top-hat: the closing fills lines in, the difference is the
/// line). Broad dark areas are shapes, not lines; where "ink" is dense it
/// is texture (foliage, hair shading, dithering) and is left alone.
pub fn ink(px: &[[f32; 3]], w: usize, h: usize, r: usize) -> Vec<f32> {
    let l: Vec<f32> = px.iter().map(|p| lab::from_linear(*p)[0]).collect();
    let closed = extreme(&extreme(&l, w, h, r, f32::max), w, h, r, f32::min);
    let raw: Vec<f32> = l
        .iter()
        .zip(&closed)
        .map(|(&l, &c)| {
            // Fully ink 24 L under what closes over it; nothing light counts.
            let k = ((c - l - 8.0) / 16.0).clamp(0.0, 1.0);
            let dark = ((75.0 - l) / 15.0).clamp(0.0, 1.0);
            k * dark
        })
        .collect();
    let density = box_mean(&raw, w, h, 3 * r);
    raw.iter().zip(&density).map(|(&k, &d)| k * ((0.4 - d) / 0.2).clamp(0.0, 1.0)).collect()
}

/// Separable square max or min filter of radius `r`.
fn extreme(v: &[f32], w: usize, h: usize, r: usize, f: fn(f32, f32) -> f32) -> Vec<f32> {
    let mut a = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let row = &v[y * w..y * w + w];
            a[y * w + x] = row[x.saturating_sub(r)..(x + r + 1).min(w)].iter().copied().reduce(f).unwrap();
        }
    }
    let mut b = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            b[y * w + x] = (y.saturating_sub(r)..(y + r + 1).min(h)).map(|y| a[y * w + x]).reduce(f).unwrap();
        }
    }
    b
}

/// Mean over a (2r+1)² window, via a summed-area table.
pub fn box_mean(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut sat = vec![0f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0f64;
        for x in 0..w {
            row += v[y * w + x] as f64;
            sat[(y + 1) * (w + 1) + x + 1] = sat[y * (w + 1) + x + 1] + row;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for x in 0..w {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(w));
            let s = sat[y1 * (w + 1) + x1] - sat[y0 * (w + 1) + x1] - sat[y1 * (w + 1) + x0] + sat[y0 * (w + 1) + x0];
            out[y * w + x] = (s / ((x1 - x0) * (y1 - y0)) as f64) as f32;
        }
    }
    out
}

/// For each of `sw`×`sh` samples over a `w`×`h` image: the strongest ink
/// in its footprint grown by `grow` samples, and that ink's color.
pub fn pool(
    px: &[[f32; 3]],
    ink: &[f32],
    w: usize,
    h: usize,
    sw: usize,
    sh: usize,
    grow: f32,
) -> Vec<(f32, Lab)> {
    let (fx, fy) = (w as f32 / sw as f32, h as f32 / sh as f32);
    // Separable max: first along x per source row, then along y.
    let mut rows: Vec<(f32, usize)> = vec![(0.0, 0); sw * h];
    for sx in 0..sw {
        let x0 = (((sx as f32 - grow) * fx).floor().max(0.0)) as usize;
        let x1 = ((((sx + 1) as f32 + grow) * fx).ceil() as usize).min(w).max(x0 + 1);
        for y in 0..h {
            let mut best = (0.0f32, y * w + x0.min(w - 1));
            for x in x0..x1 {
                let i = y * w + x;
                if ink[i] > best.0 {
                    best = (ink[i], i);
                }
            }
            rows[y * sw + sx] = best;
        }
    }
    let mut out = vec![(0.0, [0.0; 3]); sw * sh];
    for sy in 0..sh {
        let y0 = (((sy as f32 - grow) * fy).floor().max(0.0)) as usize;
        let y1 = ((((sy + 1) as f32 + grow) * fy).ceil() as usize).min(h).max(y0 + 1);
        for sx in 0..sw {
            let mut best = (0.0f32, 0usize);
            for y in y0..y1 {
                let b = rows[y * sw + sx];
                if b.0 > best.0 {
                    best = b;
                }
            }
            if best.0 > 0.0 {
                out[sy * sw + sx] = (best.0, lab::from_linear(px[best.1]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_dark_line_is_ink_and_survives_pooling() {
        let (w, h) = (40, 20);
        let mut px = vec![[0.8f32; 3]; w * h];
        for y in 0..h {
            px[y * w + 13] = [0.01; 3];
        }
        let k = ink(&px, w, h, 4);
        assert!(k[5 * w + 13] > 0.9);
        assert!(k[5 * w + 3] < 0.01);
        // A broad dark area is a shape, not a line.
        let mut blob = vec![[0.8f32; 3]; w * h];
        for y in 0..h {
            for x in 10..30 {
                blob[y * w + x] = [0.01; 3];
            }
        }
        assert!(ink(&blob, w, h, 4).iter().all(|&k| k < 0.01));
        // 40 → 8 samples: the line lands in sample 2, at full darkness.
        let p = pool(&px, &k, w, h, 8, 4, 0.0);
        assert!(p[2].0 > 0.9 && p[2].1[0] < 25.0, "{:?}", p[2]);
        assert_eq!(p[0].0, 0.0);
        // Grown by a sample, it reaches the neighbours too.
        let g = pool(&px, &k, w, h, 8, 4, 1.0);
        assert!(g[1].0 > 0.9 && g[3].0 > 0.9 && g[5].0 == 0.0);
    }
}
