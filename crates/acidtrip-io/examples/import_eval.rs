//! Scores the image importer: converts every image in a folder with a few
//! setups, renders the result with the real 8x16 font, and compares it to
//! the source as seen from a normal viewing distance.
//!
//!   cargo run --release -p acidtrip-io --example import_eval -- <images> <out> [width]
//!
//! Prints ΔE (mean OKLab distance ×100 after a slight blur; lower is better),
//! SSIM on lightness (higher is better) and the time per conversion, and
//! writes one contact sheet per image: the source, then each setup.

use std::path::Path;
use std::time::Instant;

use acidtrip_core::render::{RenderOptions, render_grid};
use acidtrip_core::DocKind;
use acidtrip_io::import::{Dither, Glyphs, ImportOptions, ImportStyle, Preset, Scaling, convert};
use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};

fn setups(width: usize) -> Vec<(&'static str, ImportOptions)> {
    let base = ImportOptions { width, scaling: Scaling::Smooth, ..ImportOptions::default() };
    let modern = ImportOptions { kind: DocKind::Modern, ..base.clone() };
    let blocks = ImportOptions { style: ImportStyle::Blocks, ..base.clone() };
    let only = std::env::var("SETUPS").unwrap_or_default();
    let all = vec![
        ("classic-half", base.clone()),
        ("classic-half-dither", ImportOptions { dither: Dither::Diffuse, ..base.clone() }),
        ("classic-blocks-noshade", ImportOptions { shades: false, ..blocks.clone() }),
        ("classic-blocks", blocks.clone()),
        ("classic-blocks-diffuse", ImportOptions { dither: Dither::Diffuse, ..blocks.clone() }),
        ("classic-blocks-ordered", ImportOptions { dither: Dither::Ordered, ..blocks.clone() }),
        ("classic-blocks-fitted", ImportOptions { fit_palette: true, ..blocks.clone() }),
        ("classic-blocks-diffuse-lines", ImportOptions { dither: Dither::Diffuse, lines: 100, ..blocks.clone() }),
        ("classic-blocks-fitted-lines", ImportOptions { fit_palette: true, lines: 100, ..blocks.clone() }),
        ("modern-extended-lines", ImportOptions { style: ImportStyle::Blocks, glyphs: Glyphs::Extended, lines: 100, ..modern.clone() }),
        ("modern-half", modern.clone()),
        ("modern-blocks", ImportOptions { style: ImportStyle::Blocks, ..modern.clone() }),
        ("modern-extended", ImportOptions { style: ImportStyle::Blocks, glyphs: Glyphs::Extended, ..modern.clone() }),
        ("preset-photo", Preset::Photo.apply(&base)),
        ("preset-scene", Preset::Scene.apply(&base)),
        ("preset-cel", Preset::Cel.apply(&base)),
        ("fitted-cel", Preset::Cel.apply(&ImportOptions { fit_palette: true, ..base.clone() })),
        ("fitted-photo", Preset::Photo.apply(&ImportOptions { fit_palette: true, ..base.clone() })),
        ("preset-comic", Preset::Comic.apply(&base)),
        ("modern-cel", Preset::Cel.apply(&modern)),
        ("modern-comic", Preset::Comic.apply(&modern)),
        ("modern-photo", Preset::Photo.apply(&modern)),
        ("preset-pixel", Preset::PixelArt.apply(&ImportOptions { scaling: Scaling::Auto, ..base.clone() })),
        ("preset-pixel-fitted", Preset::PixelArt.apply(&ImportOptions { scaling: Scaling::Auto, fit_palette: true, ..base.clone() })),
        ("preset-lineart", Preset::LineArt.apply(&base)),
        ("preset-photo-modern", Preset::Photo.apply(&modern)),
    ];
    all.into_iter().filter(|(n, _)| only.is_empty() || only.split(',').any(|o| n.starts_with(o))).collect()
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, out) = (Path::new(&args[0]), Path::new(&args[1]));
    let width: usize = args.get(2).and_then(|w| w.parse().ok()).unwrap_or(80);
    std::fs::create_dir_all(out)?;
    synth(dir)?;
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| ["png", "jpg", "jpeg", "gif", "webp"].iter().any(|x| e.eq_ignore_ascii_case(x))))
        .collect();
    files.sort();
    let setups = setups(width);
    let mut totals = vec![(0f64, 0f64, 0f64, 0f64, 0f64); setups.len()];
    println!("{:<22} {:<24} {:>6} {:>6} {:>8}", "image", "setup", "ΔE", "SSIM", "ms");
    for f in &files {
        let bytes = std::fs::read(f)?;
        let src = image::load_from_memory(&bytes)?.to_rgba8();
        let name = f.file_stem().unwrap().to_string_lossy().into_owned();
        let mut sheet: Vec<RgbaImage> = vec![];
        for (k, (label, opts)) in setups.iter().enumerate() {
            let t = Instant::now();
            let c = convert(&src, opts, true)?;
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            let shown = render_grid(&c.clip.to_grid(), &c.palette, RenderOptions::default());
            let reference = to_srgb(&imageops::resize(&to_linear(&src), shown.width(), shown.height(), FilterType::Triangle));
            let (de, ssim) = score(&reference, &shown);
            let ink = ink_f1(&src, &shown);
            println!("{name:<22} {label:<24} {de:>6.2} {ssim:>6.3} {:>6} {ms:>8.1}", ink.map_or("-".into(), |v| format!("{v:.3}")));
            totals[k].0 += de;
            totals[k].1 += ssim;
            totals[k].2 += ms;
            if let Some(v) = ink {
                totals[k].3 += v;
                totals[k].4 += 1.0;
            }
            if std::env::var_os("FULL").is_some() {
                shown.save(out.join(format!("{name}.{label}.png")))?;
            }
            if sheet.is_empty() {
                sheet.push(reference);
            }
            sheet.push(shown);
        }
        contact(&sheet).save(out.join(format!("{name}.png")))?;
    }
    let n = files.len().max(1) as f64;
    println!("\n{:<24} {:>6} {:>6} {:>6} {:>8}", "mean", "ΔE", "SSIM", "ink", "ms");
    for ((label, _), (de, ss, ms, ink, ni)) in setups.iter().zip(totals) {
        println!("{label:<24} {:>6.2} {:>6.3} {:>6.3} {:>8.1}", de / n, ss / n, ink / ni.max(1.0), ms / n);
    }
    Ok(())
}

/// Do the dark lines survive? Ink = source pixels clearly darker than
/// their surroundings (thin strokes, outlines). Scored on a grid of
/// quarter cells (4×8 output pixels): a block "has a stroke" in the output
/// when its darkest pixel is well under the local median. F1 of output
/// strokes against source ink, so lost lines and dither noise both cost.
/// None when the source has too little ink to say.
fn ink_f1(src: &RgbaImage, shown: &RgbaImage) -> Option<f64> {
    let luma = |img: &RgbaImage| -> (usize, usize, Vec<f32>) {
        let lin = to_linear(img);
        let v = lin.pixels().map(|p| oklab(p)[0] * 100.0).collect();
        (img.width() as usize, img.height() as usize, v)
    };
    let (bw, bh) = (shown.width() as usize / 4, shown.height() as usize / 8);
    // Source ink: darker than the local mean (window ~ a quarter cell) by 20 L.
    let (sw, sh, sl) = luma(src);
    let r = ((sw as f32 / bw as f32) * 1.5).max(2.0) as usize;
    let mean = box_mean(&sl, sw, sh, r);
    let mut ink = vec![0u32; bw * bh];
    for y in 0..sh {
        for x in 0..sw {
            let i = y * sw + x;
            if sl[i] < mean[i] - 20.0 && sl[i] < 60.0 {
                let (bx, by) = ((x * bw / sw).min(bw - 1), (y * bh / sh).min(bh - 1));
                ink[by * bw + bx] += 1;
            }
        }
    }
    // A block counts as ink when a stroke of at least ~ one block-length crosses it.
    let per = (sw as f32 / bw as f32).max(1.0);
    let src_ink: Vec<bool> = ink.iter().map(|&n| n as f32 >= per * 0.8).collect();
    let total = src_ink.iter().filter(|b| **b).count();
    if total < bw * bh / 50 {
        return None;
    }
    // Output strokes.
    let (ow, oh, ol) = luma(shown);
    let omean = box_mean(&ol, ow, oh, 6);
    let mut out_ink = vec![false; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let mut hit = false;
            for y in by * 8..(by * 8 + 8).min(oh) {
                for x in bx * 4..(bx * 4 + 4).min(ow) {
                    let i = y * ow + x;
                    hit |= ol[i] < omean[i] - 12.0 && ol[i] < 65.0;
                }
            }
            out_ink[by * bw + bx] = hit;
        }
    }
    // Tolerate a one-block shift: a stroke next to where the ink was still reads as the line.
    let near = |m: &[bool], bx: usize, by: usize| {
        (by.saturating_sub(1)..=(by + 1).min(bh - 1))
            .any(|y| (bx.saturating_sub(1)..=(bx + 1).min(bw - 1)).any(|x| m[y * bw + x]))
    };
    let (mut tp_r, mut tp_p, mut outs) = (0usize, 0usize, 0usize);
    for by in 0..bh {
        for bx in 0..bw {
            let i = by * bw + bx;
            if src_ink[i] && near(&out_ink, bx, by) {
                tp_r += 1;
            }
            if out_ink[i] {
                outs += 1;
                if near(&src_ink, bx, by) {
                    tp_p += 1;
                }
            }
        }
    }
    let recall = tp_r as f64 / total as f64;
    let precision = if outs == 0 { 0.0 } else { tp_p as f64 / outs as f64 };
    Some(if recall + precision == 0.0 { 0.0 } else { 2.0 * recall * precision / (recall + precision) })
}

/// Mean over a (2r+1)² window, via a summed-area table.
fn box_mean(v: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
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
        for x in 0..w {
            let (x0, y0, x1, y1) = (x.saturating_sub(r), y.saturating_sub(r), (x + r + 1).min(w), (y + r + 1).min(h));
            let s = sat[y1 * (w + 1) + x1] - sat[y0 * (w + 1) + x1] - sat[y1 * (w + 1) + x0] + sat[y0 * (w + 1) + x0];
            out[y * w + x] = (s / ((x1 - x0) * (y1 - y0)) as f64) as f32;
        }
    }
    out
}

type Linear = image::ImageBuffer<Rgba<f32>, Vec<f32>>;

/// Linear light, composited over black like the importer and a terminal:
/// averages here are what the eye sees from a distance.
fn to_linear(img: &RgbaImage) -> Linear {
    Linear::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y);
        let a = p[3] as f32 / 255.0;
        Rgba([lin(p[0]) * a, lin(p[1]) * a, lin(p[2]) * a, 1.0])
    })
}

fn to_srgb(img: &Linear) -> RgbaImage {
    let enc = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        ((if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }) * 255.0).round() as u8
    };
    RgbaImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y);
        Rgba([enc(p[0]), enc(p[1]), enc(p[2]), 255])
    })
}

fn lin(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn oklab(p: &Rgba<f32>) -> [f32; 3] {
    let (r, g, b) = (p[0].max(0.0), p[1].max(0.0), p[2].max(0.0));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// Mean ΔE (×100) and SSIM on L, both after a blur (in linear light) of about a quarter cell:
/// the art as seen from a normal distance, not pixel by pixel.
fn score(a: &RgbaImage, b: &RgbaImage) -> (f64, f64) {
    let (a, b) = (imageops::blur(&to_linear(a), 2.0), imageops::blur(&to_linear(b), 2.0));
    let la: Vec<[f32; 3]> = a.pixels().map(oklab).collect();
    let lb: Vec<[f32; 3]> = b.pixels().map(oklab).collect();
    let de = la
        .iter()
        .zip(&lb)
        .map(|(p, q)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt() as f64)
        .sum::<f64>()
        / la.len() as f64
        * 100.0;
    let (w, h) = (a.width() as usize, a.height() as usize);
    let (win, step) = (8, 4);
    let (c1, c2) = (0.0001f64, 0.0009f64);
    let (mut sum, mut n) = (0.0, 0.0);
    let mut y = 0;
    while y + win <= h {
        let mut x = 0;
        while x + win <= w {
            let (mut ma, mut mb, mut va, mut vb, mut cov) = (0.0, 0.0, 0.0, 0.0, 0.0);
            let k = (win * win) as f64;
            for yy in y..y + win {
                for xx in x..x + win {
                    ma += la[yy * w + xx][0] as f64;
                    mb += lb[yy * w + xx][0] as f64;
                }
            }
            ma /= k;
            mb /= k;
            for yy in y..y + win {
                for xx in x..x + win {
                    let (p, q) = (la[yy * w + xx][0] as f64 - ma, lb[yy * w + xx][0] as f64 - mb);
                    va += p * p;
                    vb += q * q;
                    cov += p * q;
                }
            }
            let (va, vb, cov) = (va / k, vb / k, cov / k);
            sum += ((2.0 * ma * mb + c1) * (2.0 * cov + c2)) / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            n += 1.0;
            x += step;
        }
        y += step;
    }
    (de, if n > 0.0 { sum / n } else { 1.0 })
}

/// Side by side, 4 px apart.
fn contact(imgs: &[RgbaImage]) -> RgbaImage {
    let w: u32 = imgs.iter().map(|i| i.width() + 4).sum();
    let h = imgs.iter().map(|i| i.height()).max().unwrap_or(1);
    let mut out = RgbaImage::from_pixel(w, h, Rgba([40, 40, 48, 255]));
    let mut x = 0;
    for i in imgs {
        imageops::replace(&mut out, i, x as i64, 0);
        x += i.width() + 4;
    }
    out
}

/// Generated test images: a hue/lightness gradient, pixel art (a sprite
/// scaled 4x) and line art (rings and bars, black on white).
fn synth(dir: &Path) -> anyhow::Result<()> {
    let grad = dir.join("synth_gradient.png");
    if !grad.exists() {
        RgbaImage::from_fn(512, 256, |x, y| {
            let h = x as f32 / 512.0 * 6.0;
            let c = |o: f32| ((((h + o) % 6.0) - 3.0).abs() - 1.0).clamp(0.0, 1.0);
            let (r, g, b) = (c(0.0), c(4.0), c(2.0));
            let v = y as f32 / 255.0;
            let mix = |k: f32| if v < 0.5 { k * v * 2.0 } else { k + (1.0 - k) * (v - 0.5) * 2.0 };
            Rgba([(mix(r) * 255.0) as u8, (mix(g) * 255.0) as u8, (mix(b) * 255.0) as u8, 255])
        })
        .save(&grad)?;
    }
    let pix = dir.join("synth_pixel.png");
    if !pix.exists() {
        const SPRITE: [&str; 12] = [
            "....RRRRR...",
            "...RRRRRRRRR",
            "...BBBSSKS..",
            "..BSBSSSKSSS",
            "..BSBBSSSKSS",
            "..BBSSSSKKKK",
            "....SSSSSSS.",
            "...RRURRR...",
            "..RRRURRURRR",
            ".RRRRUUUURRR",
            ".SSRUYUUYURS",
            ".SSSUUUUUUSS",
        ];
        let col = |c: u8| match c {
            b'R' => [216, 40, 0],
            b'B' => [136, 112, 0],
            b'S' => [252, 152, 56],
            b'K' => [0, 0, 0],
            b'U' => [32, 56, 236],
            b'Y' => [252, 216, 168],
            _ => [92, 148, 252],
        };
        RgbaImage::from_fn(48, 48, |x, y| {
            let [r, g, b] = col(SPRITE[(y / 4) as usize].as_bytes()[(x / 4) as usize]);
            Rgba([r, g, b, 255])
        })
        .save(&pix)?;
    }
    let line = dir.join("synth_lineart.png");
    if !line.exists() {
        RgbaImage::from_fn(400, 300, |x, y| {
            let (dx, dy) = (x as f32 - 150.0, y as f32 - 150.0);
            let d = (dx * dx + dy * dy).sqrt();
            let ring = [40.0, 80.0, 120.0].iter().any(|r| (d - r).abs() < 2.5);
            let bars = x > 290 && x < 380 && (y / 12) % 2 == 0 && y > 40 && y < 260;
            let diag = x > 280 && ((x as i32 - y as i32 - 100).abs() < 2);
            let v = if ring || bars || diag { 0 } else { 255 };
            Rgba([v, v, v, 255])
        })
        .save(&line)?;
    }
    Ok(())
}
