//! OKLab (scaled so L runs 0–100): distances in it track how different two
//! colors look, and averages in it make good flat colors. Mixing (what
//! the eye does with shade glyphs) happens in linear light.

use std::sync::LazyLock;

pub type Lab = [f32; 3];

static LINEAR: LazyLock<[f32; 256]> = LazyLock::new(|| {
    std::array::from_fn(|i| {
        let c = i as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    })
});

pub fn linear(v: u8) -> f32 {
    LINEAR[v as usize]
}

pub fn encode(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let c = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (c * 255.0).round() as u8
}

pub fn from_linear([r, g, b]: [f32; 3]) -> Lab {
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).max(0.0).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).max(0.0).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).max(0.0).cbrt();
    [
        100.0 * (0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s),
        100.0 * (1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s),
        100.0 * (0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s),
    ]
}

pub fn to_linear([l, a, b]: Lab) -> [f32; 3] {
    let (l, a, b) = (l / 100.0, a / 100.0, b / 100.0);
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ]
}

pub fn from_rgb(c: [u8; 3]) -> Lab {
    from_linear(c.map(linear))
}

pub fn to_rgb(l: Lab) -> [u8; 3] {
    to_linear(l).map(encode)
}

pub fn d2(a: Lab, b: Lab) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// `k` colors that fit `points` (k-means, seeded k-means++ style with a
/// fixed sequence so results are repeatable), darkest first. Each point
/// counts `weights` times.
pub fn kmeans(points: &[Lab], weights: &[f32], k: usize, iters: usize) -> Vec<Lab> {
    if points.is_empty() || k == 0 {
        return vec![[0.0; 3]; k];
    }
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rand = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut centers = vec![points[0]];
    let mut near: Vec<f32> = points.iter().zip(weights).map(|(p, w)| d2(*p, centers[0]) * w).collect();
    while centers.len() < k {
        let total: f64 = near.iter().map(|&d| d as f64).sum();
        if total <= 0.0 {
            centers.push(centers[centers.len() - 1]);
            continue;
        }
        let mut pick = rand() * total;
        let mut idx = points.len() - 1;
        for (i, &d) in near.iter().enumerate() {
            pick -= d as f64;
            if pick <= 0.0 {
                idx = i;
                break;
            }
        }
        let c = points[idx];
        centers.push(c);
        for ((n, p), w) in near.iter_mut().zip(points).zip(weights) {
            *n = n.min(d2(*p, c) * w);
        }
    }
    for _ in 0..iters {
        let mut sums = vec![([0f64; 3], 0f64); k];
        for (p, &w) in points.iter().zip(weights) {
            let i = nearest(&centers, *p);
            for (s, v) in sums[i].0.iter_mut().zip(p) {
                *s += (*v * w) as f64;
            }
            sums[i].1 += w as f64;
        }
        for (c, (s, n)) in centers.iter_mut().zip(sums) {
            if n > 0.0 {
                *c = s.map(|v| (v / n) as f32);
            }
        }
    }
    centers.sort_by(|a, b| a[0].total_cmp(&b[0]));
    centers
}

pub fn nearest(colors: &[Lab], p: Lab) -> usize {
    let mut best = (f32::MAX, 0);
    for (i, c) in colors.iter().enumerate() {
        let d = d2(*c, p);
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_orders() {
        for c in [[0u8, 0, 0], [255, 255, 255], [170, 85, 0], [12, 200, 99]] {
            let back = to_rgb(from_rgb(c));
            assert!(c.iter().zip(back).all(|(a, b)| a.abs_diff(b) <= 1), "{c:?} → {back:?}");
        }
        assert!((from_rgb([255, 255, 255])[0] - 100.0).abs() < 0.1);
        let k = kmeans(&[[10.0, 0.0, 0.0], [11.0, 0.0, 0.0], [90.0, 5.0, 5.0], [91.0, 5.0, 5.0]], &[1.0; 4], 2, 5);
        assert!(k[0][0] < 12.0 && k[1][0] > 89.0, "{k:?}");
    }
}
