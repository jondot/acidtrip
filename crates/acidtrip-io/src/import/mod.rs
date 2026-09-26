//! Image → ANSI conversion (import, reference layers, AI images).
//!
//! 1. Crop, then resample to a grid of samples per cell: an area filter in
//!    linear light (alpha composited over black), or nearest-neighbour for
//!    pixel art, which is detected from its colors and block size.
//! 2. Adjust in OKLab: levels, contrast, brightness, saturation, sharpening.
//! 3. Pick each cell's glyph and colors by least OKLab error:
//!    - half blocks: two square pixels per cell;
//!    - blocks: every glyph of the set (halves; in Modern also quadrants,
//!      eighths, sextants) with its best fg/bg, found in O(1) per glyph from
//!      a summed-area table, plus ░▒▓ as color mixes that pay for their
//!      texture;
//!    - ASCII: a lightness ramp.
//!
//!    Classic uses the document's 16 colors (or 16 fitted to the image);
//!    Modern uses truecolor. Dithering works per pixel (half blocks) or per
//!    cell (blocks): error diffusion or an ordered pattern.
//! 4. Optionally remove lone specks.
//!
//! Cells are 8x16, so half blocks give square pixels. The first version
//! ported ideas from ansidraw's importer and ansimake (both MIT).

pub(crate) mod lab;
mod lines;
mod masks;

use acidtrip_core::render::glyph_pixel;
use acidtrip_core::{Cell, Clip, Color, DocKind, Document, Palette};
use anyhow::ensure;
use image::imageops::{self, FilterType};
use image::{ImageBuffer, Luma, Rgba, RgbaImage};
use lab::Lab;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ImportStyle {
    /// Half-block pixels (▀▄█): 2 square pixels per cell. Best for pixel art.
    #[default]
    HalfBlock,
    /// Best-fit glyph per cell from the glyph set. Best for pictures.
    Blocks,
    /// Plain ASCII lightness ramp.
    Ascii,
}

/// Which glyphs the block search may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Glyphs {
    /// ▀▄▌▐ (and ░▒▓ when shades are on): opens anywhere, scene-style.
    #[default]
    Cp437,
    /// Also quadrants, eighths and sextants (Modern documents).
    Extended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Dither {
    #[default]
    None,
    /// Error diffusion (Floyd–Steinberg): smooth, a little noisy.
    Diffuse,
    /// A fixed 4x4 pattern: the regular "scene" look.
    Ordered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Scaling {
    /// Pixel art when the image looks like it, smooth otherwise.
    #[default]
    Auto,
    Smooth,
    /// Nearest neighbour: hard pixel edges.
    Pixel,
}

/// Picture adjustments, applied after scaling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Adjust {
    /// -100..=100.
    pub brightness: i32,
    /// -100..=100.
    pub contrast: i32,
    /// Percent; 100 leaves colors alone, 0 is grey.
    pub saturation: i32,
    /// 0..=100.
    pub sharpen: u32,
    /// Stretch lightness so the darkest 1% is black and the lightest 1% white.
    pub auto_levels: bool,
}

impl Default for Adjust {
    fn default() -> Self {
        Adjust { brightness: 0, contrast: 0, saturation: 100, sharpen: 0, auto_levels: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportOptions {
    /// Target width in cells (height follows the aspect ratio).
    pub width: usize,
    /// Fit within this many rows, narrowing the width to keep the aspect.
    pub height: Option<usize>,
    pub style: ImportStyle,
    /// Classic → 16 colors (iCE backgrounds allowed); Modern → truecolor.
    pub kind: DocKind,
    pub glyphs: Glyphs,
    /// ░▒▓ as color mixes (Classic blocks).
    pub shades: bool,
    pub dither: Dither,
    /// Snap mostly-dark sample boxes to black so line art survives.
    pub ink: bool,
    /// 0..=100: keep drawn lines (outlines, ink strokes) dark and let the
    /// glyphs follow them, instead of averaging them away.
    pub lines: u32,
    /// Source-pixel crop (x, y, w, h).
    pub crop: Option<(u32, u32, u32, u32)>,
    pub adjust: Adjust,
    pub scaling: Scaling,
    /// Classic colors to use (the document's); VGA when unset.
    pub palette: Option<Palette>,
    /// Classic: fit 16 colors to the image instead (the result carries them).
    pub fit_palette: bool,
    /// Remove lone specks.
    pub cleanup: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            width: 80,
            height: None,
            style: ImportStyle::HalfBlock,
            kind: DocKind::Classic,
            glyphs: Glyphs::Cp437,
            shades: true,
            dither: Dither::None,
            ink: false,
            lines: 0,
            crop: None,
            adjust: Adjust::default(),
            scaling: Scaling::Auto,
            palette: None,
            fit_palette: false,
            cleanup: false,
        }
    }
}

/// Starting points for the kinds of images people bring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    /// Most faithful: blocks (extended glyphs in Modern), dithered in Classic.
    Photo,
    /// Classic scene look: CP437 blocks and shades, ordered dither, punchy.
    Scene,
    /// Half blocks, nearest-neighbour scaling, no dither.
    PixelArt,
    /// Anime and cartoons: outlines kept dark and whole, fills kept flat.
    Cel,
    /// Ink drawings and comics: every line kept, washes shaded.
    Comic,
    /// Dark lines kept, high contrast, no shades, specks removed.
    LineArt,
    Ascii,
}

impl Preset {
    pub const ALL: [Preset; 7] =
        [Preset::Photo, Preset::Scene, Preset::PixelArt, Preset::Cel, Preset::Comic, Preset::LineArt, Preset::Ascii];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Photo => "photo",
            Preset::Scene => "scene",
            Preset::PixelArt => "pixel art",
            Preset::Cel => "cel",
            Preset::Comic => "comic",
            Preset::LineArt => "line art",
            Preset::Ascii => "ascii",
        }
    }

    /// `base` with this preset's style settings (size, crop, kind and
    /// palette are kept).
    pub fn apply(self, base: &ImportOptions) -> ImportOptions {
        let classic = base.kind == DocKind::Classic;
        let o = ImportOptions {
            width: base.width,
            height: base.height,
            kind: base.kind,
            crop: base.crop,
            palette: base.palette.clone(),
            fit_palette: base.fit_palette,
            ..ImportOptions::default()
        };
        match self {
            Preset::Photo => ImportOptions {
                style: ImportStyle::Blocks,
                glyphs: if classic { Glyphs::Cp437 } else { Glyphs::Extended },
                dither: if classic { Dither::Diffuse } else { Dither::None },
                scaling: Scaling::Smooth,
                adjust: Adjust { sharpen: 20, ..Adjust::default() },
                ..o
            },
            Preset::Scene => ImportOptions {
                style: ImportStyle::Blocks,
                dither: Dither::Ordered,
                scaling: Scaling::Smooth,
                adjust: Adjust { contrast: 10, saturation: 120, sharpen: 30, ..Adjust::default() },
                cleanup: true,
                ..o
            },
            Preset::PixelArt => ImportOptions { style: ImportStyle::HalfBlock, scaling: Scaling::Pixel, ..o },
            Preset::Cel => ImportOptions {
                style: ImportStyle::Blocks,
                glyphs: if classic { Glyphs::Cp437 } else { Glyphs::Extended },
                dither: if classic { Dither::Diffuse } else { Dither::None },
                scaling: Scaling::Smooth,
                lines: 80,
                adjust: Adjust { saturation: 110, ..Adjust::default() },
                ..o
            },
            Preset::Comic => ImportOptions {
                style: ImportStyle::Blocks,
                glyphs: if classic { Glyphs::Cp437 } else { Glyphs::Extended },
                dither: if classic { Dither::Diffuse } else { Dither::None },
                scaling: Scaling::Smooth,
                lines: 100,
                adjust: Adjust { contrast: 10, ..Adjust::default() },
                ..o
            },
            Preset::LineArt => ImportOptions {
                style: ImportStyle::Blocks,
                shades: false,
                ink: true,
                lines: 100,
                scaling: Scaling::Smooth,
                adjust: Adjust { contrast: 30, auto_levels: true, ..Adjust::default() },
                cleanup: true,
                ..o
            },
            Preset::Ascii => ImportOptions { style: ImportStyle::Ascii, scaling: Scaling::Smooth, ..o },
        }
    }
}

/// What the importer made.
#[derive(Clone, Debug)]
pub struct Converted {
    pub clip: Clip,
    /// The colors `Color::Pal` cells refer to.
    pub palette: Palette,
    /// The palette was fitted to the image (the document needs it, and iCE).
    pub fitted: bool,
    /// Scaled as pixel art.
    pub pixel: bool,
}

/// What the image looks like, before converting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Analysis {
    pub width: u32,
    pub height: u32,
    /// Distinct colors, up to 257.
    pub colors: usize,
    /// Size with any integer upscaling undone (pixel art saved at 4x).
    pub native: (u32, u32),
    pub pixel_art: bool,
}

/// Decode any format `image` supports (PNG, GIF, JPEG, WebP, BMP).
pub fn decode(bytes: &[u8]) -> anyhow::Result<RgbaImage> {
    Ok(image::load_from_memory(bytes)?.to_rgba8())
}

pub fn image_to_clip(bytes: &[u8], opts: &ImportOptions) -> anyhow::Result<Clip> {
    image_to_clip_ice(bytes, opts, true)
}

/// Like [`image_to_clip`], choosing the Classic background limit: `ice` allows
/// all 16 background colors, otherwise only the dark 8 (blink mode).
pub fn image_to_clip_ice(bytes: &[u8], opts: &ImportOptions, ice: bool) -> anyhow::Result<Clip> {
    Ok(convert(&decode(bytes)?, opts, ice)?.clip)
}

pub fn image_to_doc(bytes: &[u8], opts: &ImportOptions) -> anyhow::Result<Document> {
    Ok(converted_doc(convert(&decode(bytes)?, opts, true)?, opts.kind))
}

/// A new document holding `c`, with its palette.
pub fn converted_doc(c: Converted, kind: DocKind) -> Document {
    let mut d = Document::from_grid(kind, &c.clip.to_grid());
    if kind == DocKind::Classic {
        d.meta.palette = c.palette;
        d.meta.ice = true;
    }
    d
}

/// The crop `opts` asks for.
pub fn cropped(img: &RgbaImage, crop: Option<(u32, u32, u32, u32)>) -> RgbaImage {
    match crop {
        Some((x, y, w, h)) if img.width() > 0 && img.height() > 0 => {
            let x = x.min(img.width() - 1);
            let y = y.min(img.height() - 1);
            imageops::crop_imm(img, x, y, w.clamp(1, img.width() - x), h.clamp(1, img.height() - y)).to_image()
        }
        _ => img.clone(),
    }
}

pub fn analyze(img: &RgbaImage) -> Analysis {
    let (w, h) = img.dimensions();
    let mut seen = std::collections::HashSet::new();
    for p in img.pixels() {
        seen.insert(if p[3] < 128 { [0; 4] } else { p.0 });
        if seen.len() > 256 {
            break;
        }
    }
    let colors = seen.len();
    // The largest k with every k×k block one color.
    let mut k = 1;
    if colors <= 256 {
        for n in (2..=16u32).rev() {
            if w % n == 0 && h % n == 0 && uniform_blocks(img, n) {
                k = n;
                break;
            }
        }
    }
    let native = (w / k, h / k);
    let pixel_art = colors <= 256 && (k >= 2 || (colors <= 64 && native.0 * native.1 <= 256 * 256));
    Analysis { width: w, height: h, colors, native, pixel_art }
}

/// What kind of picture an image is, measured on a small copy.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Features {
    /// Share of pixels that are thin dark lines.
    pub ink: f32,
    /// Share of pixels in flat (untextured) areas.
    pub flat: f32,
    /// Mean chroma (OKLab ×100).
    pub chroma: f32,
    /// Share of pixels that are light and nearly grey (paper).
    pub paper: f32,
    /// Share of pixels in the 16 most common colors (coarsely binned):
    /// high for cel shading and flat art, low for photos.
    pub few: f32,
    /// Of the pixels just beside lines, the share that are flat: lines
    /// drawn around fills, rather than dark detail in texture.
    pub outlined: f32,
}

pub fn features(img: &RgbaImage) -> Features {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Features::default();
    }
    let k = (320.0 / w.max(h) as f32).min(1.0);
    let (sw, sh) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
    let small = imageops::resize(img, sw, sh, FilterType::Triangle);
    let (sw, sh) = (sw as usize, sh as usize);
    let px: Vec<[f32; 3]> = small.pixels().map(|p| [p[0], p[1], p[2]].map(lab::linear)).collect();
    let labs: Vec<Lab> = px.iter().map(|p| lab::from_linear(*p)).collect();
    let n = labs.len() as f32;
    let inky: Vec<f32> = lines::ink(&px, sw, sh, 2).iter().map(|&v| if v > 0.5 { 1.0 } else { 0.0 }).collect();
    let ink = inky.iter().sum::<f32>() / n;
    let l: Vec<f32> = labs.iter().map(|c| c[0]).collect();
    let l2: Vec<f32> = l.iter().map(|v| v * v).collect();
    let (m, m2) = (lines::box_mean(&l, sw, sh, 1), lines::box_mean(&l2, sw, sh, 1));
    let is_flat: Vec<bool> = m.iter().zip(&m2).map(|(m, m2)| (*m2 - *m * *m).max(0.0).sqrt() < 2.0).collect();
    let flat = is_flat.iter().filter(|&&f| f).count() as f32 / n;
    let near = lines::box_mean(&inky, sw, sh, 2);
    let (mut beside, mut beside_flat) = (0f32, 0f32);
    for i in 0..labs.len() {
        if inky[i] == 0.0 && near[i] > 0.0 {
            beside += 1.0;
            if is_flat[i] {
                beside_flat += 1.0;
            }
        }
    }
    let outlined = if beside > 0.0 { beside_flat / beside } else { 0.0 };
    let mut bins = std::collections::HashMap::<u16, u32>::new();
    for p in small.pixels() {
        *bins.entry(((p[0] as u16 >> 4) << 8) | ((p[1] as u16 >> 4) << 4) | (p[2] as u16 >> 4)).or_default() += 1;
    }
    let mut counts: Vec<u32> = bins.into_values().collect();
    counts.sort_unstable_by(|a, b| b.cmp(a));
    let few = counts.iter().take(16).sum::<u32>() as f32 / n;
    let chroma = labs.iter().map(|c| (c[1] * c[1] + c[2] * c[2]).sqrt()).sum::<f32>() / n;
    let paper = labs.iter().filter(|c| c[0] > 80.0 && (c[1] * c[1] + c[2] * c[2]).sqrt() < 6.0).count() as f32 / n;
    Features { ink, flat, chroma, paper, few, outlined }
}

/// The preset that suits `img` best.
pub fn suggest(img: &RgbaImage) -> Preset {
    if analyze(img).pixel_art {
        return Preset::PixelArt;
    }
    let f = features(img);
    if f.chroma < 3.0 && f.ink >= 0.01 && f.few >= 0.8 {
        Preset::Comic
    } else if f.few >= 0.6 && f.outlined >= 0.18 && f.ink >= 0.01 {
        Preset::Cel
    } else {
        Preset::Photo
    }
}

fn uniform_blocks(img: &RgbaImage, n: u32) -> bool {
    let (w, h) = img.dimensions();
    (0..h).all(|y| (0..w).all(|x| img.get_pixel(x, y) == img.get_pixel(x - x % n, y - y % n)))
}

/// Cells the result will have.
pub fn target_size(w: u32, h: u32, opts: &ImportOptions) -> (usize, usize) {
    let aspect = h.max(1) as f64 / w.max(1) as f64;
    let rows_for = |cols: usize| match opts.style {
        ImportStyle::HalfBlock => ((aspect * cols as f64).round().max(1.0) as usize).div_ceil(2),
        _ => (aspect * cols as f64 / 2.0).round().max(1.0) as usize,
    };
    let mut cols = opts.width.clamp(1, 2000);
    let mut rows = rows_for(cols);
    if let Some(max) = opts.height.filter(|&m| m > 0 && rows > m) {
        cols = ((cols as f64 * max as f64 / rows as f64).floor() as usize).max(1);
        rows = rows_for(cols).clamp(1, max);
    }
    (cols, rows)
}

pub fn convert(img: &RgbaImage, opts: &ImportOptions, ice: bool) -> anyhow::Result<Converted> {
    ensure!(img.width() > 0 && img.height() > 0, "empty image");
    let src = cropped(img, opts.crop);
    let (cols, rows) = target_size(src.width(), src.height(), opts);
    let (sw, sh) = match opts.style {
        ImportStyle::HalfBlock => (cols, rows * 2),
        ImportStyle::Blocks => (cols * 8, rows * 16),
        ImportStyle::Ascii => (cols, rows),
    };
    let pixel = match opts.scaling {
        Scaling::Smooth => false,
        Scaling::Pixel => true,
        // Pixel art shrunk a lot looks better smoothed.
        Scaling::Auto => {
            let a = analyze(&src);
            a.pixel_art && sw as f64 >= a.native.0 as f64 * 0.9
        }
    };
    let filter = if opts.style == ImportStyle::Blocks { FilterType::Triangle } else { FilterType::CatmullRom };
    // How far lines grow, in samples: to about half a glyph's smallest part.
    let grow = match (opts.style, opts.glyphs) {
        (ImportStyle::Blocks, Glyphs::Cp437) => 0.5,
        (ImportStyle::Blocks, Glyphs::Extended) => 0.25,
        _ => 0.0,
    };
    let lines = (!pixel && opts.lines > 0).then_some((opts.lines.min(100) as f32 / 100.0, grow));
    let mut s = sample(&src, sw, sh, pixel, opts.ink, filter, lines);
    let sigma = if opts.style == ImportStyle::Blocks { 4.0 } else { 1.0 };
    adjust(&mut s, &opts.adjust, sigma);
    let classic = opts.kind == DocKind::Classic;
    let fitted = classic && opts.fit_palette;
    let palette = if fitted { fit_palette(&src, &s) } else { opts.palette.clone().unwrap_or_default() };
    let pal: Vec<Lab> = (0..16).map(|i| lab::from_rgb(palette.get(i))).collect();
    let limit = if ice || fitted { 16 } else { 8 };
    let ctx = Ctx { classic, pal, limit, dither: opts.dither };
    let (mut clip, shown) = match opts.style {
        ImportStyle::HalfBlock => half(&s, cols, rows, &ctx),
        ImportStyle::Blocks => blocks(&s, cols, rows, &ctx, masks::masks(opts.glyphs), classic && opts.shades),
        ImportStyle::Ascii => ascii(&s, &ctx),
    };
    if opts.cleanup && opts.style != ImportStyle::Ascii {
        despeckle(&mut clip, &shown);
    }
    Ok(Converted { clip, palette, fitted, pixel })
}

// --- sampling --------------------------------------------------------------------

/// Samples in OKLab (composited over black) with their alpha.
struct Samples {
    w: usize,
    h: usize,
    lab: Vec<Lab>,
    alpha: Vec<u8>,
    ink: Option<Vec<bool>>,
    /// How much each sample counts in the glyph search (lines count more).
    weight: Option<Vec<f32>>,
}

impl Samples {
    fn get(&self, x: usize, y: usize) -> Lab {
        if x < self.w && y < self.h { self.lab[y * self.w + x] } else { [0.0; 3] }
    }

    fn weight(&self, x: usize, y: usize) -> f32 {
        match &self.weight {
            Some(v) if x < self.w && y < self.h => v[y * self.w + x],
            _ => 1.0,
        }
    }

    fn alpha(&self, x: usize, y: usize) -> u8 {
        if x < self.w && y < self.h { self.alpha[y * self.w + x] } else { 0 }
    }
}

type Linear = ImageBuffer<Rgba<f32>, Vec<f32>>;

/// `filter` for smooth scaling: a sharp one for few samples per cell (half
/// blocks), a soft one when the glyph search sees every pixel of the cell.
fn sample(
    src: &RgbaImage,
    w: usize,
    h: usize,
    pixel: bool,
    ink: bool,
    filter: FilterType,
    lines: Option<(f32, f32)>,
) -> Samples {
    let (w32, h32) = (w as u32, h as u32);
    let mut weight = None;
    let (lab, alpha): (Vec<Lab>, Vec<u8>) = if pixel {
        imageops::resize(src, w32, h32, FilterType::Nearest)
            .pixels()
            .map(|p| {
                let a = p[3] as f32 / 255.0;
                (lab::from_linear([0, 1, 2].map(|i| lab::linear(p[i]) * a)), p[3])
            })
            .unzip()
    } else {
        // Box-shrink big images in linear light first (to 4x the size; that
        // costs nothing in quality), then filter to size.
        let f = (src.width() / (4 * w32).max(1)).min(src.height() / (4 * h32).max(1)).max(1);
        let (bw, bh) = (src.width() / f, src.height() / f);
        let mut lin = Linear::new(bw, bh);
        let n = (f * f) as f32;
        for (x, y, px) in lin.enumerate_pixels_mut() {
            let mut acc = [0f32; 4];
            for yy in y * f..(y + 1) * f {
                for xx in x * f..(x + 1) * f {
                    let p = src.get_pixel(xx, yy);
                    let a = p[3] as f32 / 255.0;
                    for c in 0..3 {
                        acc[c] += lab::linear(p[c]) * a;
                    }
                    acc[3] += a;
                }
            }
            *px = Rgba(acc.map(|v| v / n));
        }
        let (mut lab, alpha): (Vec<Lab>, Vec<u8>) = imageops::resize(&lin, w32, h32, filter)
            .pixels()
            .map(|p| (lab::from_linear([p[0], p[1], p[2]]), (p[3].clamp(0.0, 1.0) * 255.0).round() as u8))
            .unzip();
        if let Some((strength, grow)) = lines {
            let (bw, bh) = (bw as usize, bh as usize);
            let px: Vec<[f32; 3]> = lin.pixels().map(|p| [p[0], p[1], p[2]]).collect();
            // Lines are what is thinner than about two samples.
            let r = ((bw as f32 / w as f32).round() as usize).max(2);
            let k = lines::ink(&px, bw, bh, r);
            let pooled = lines::pool(&px, &k, bw, bh, w, h, grow);
            // Where most samples around are ink, the lines are packed tighter
            // than the grid can draw: keep their average instead of a blot.
            let strong: Vec<f32> = pooled.iter().map(|p| p.0).collect();
            let packed = lines::box_mean(&strong, w, h, 4);
            let mut wt = vec![1f32; w * h];
            for (((c, (p, dark)), wt), d) in lab.iter_mut().zip(pooled).zip(&mut wt).zip(packed) {
                let t = p * strength * ((0.5 - d) / 0.25).clamp(0.0, 1.0);
                if t > 0.0 {
                    *c = std::array::from_fn(|i| c[i] + (dark[i] - c[i]) * t);
                    *wt = 1.0 + 6.0 * t;
                }
            }
            weight = Some(wt);
        }
        (lab, alpha)
    };
    let ink = ink.then(|| {
        let mask = image::GrayImage::from_fn(src.width(), src.height(), |x, y| {
            let p = src.get_pixel(x, y);
            Luma([if p[3] >= 128 && p[0] < 80 && p[1] < 80 && p[2] < 80 { 255 } else { 0 }])
        });
        imageops::resize(&mask, w32, h32, FilterType::Triangle).pixels().map(|m| m[0] >= 64).collect()
    });
    Samples { w, h, lab, alpha, ink, weight }
}

fn adjust(s: &mut Samples, a: &Adjust, sigma: f32) {
    if a.auto_levels {
        let mut ls: Vec<f32> = s.lab.iter().zip(&s.alpha).filter(|(_, a)| **a >= 128).map(|(l, _)| l[0]).collect();
        if ls.len() > 20 {
            ls.sort_by(f32::total_cmp);
            let (lo, hi) = (ls[ls.len() / 100], ls[ls.len() - 1 - ls.len() / 100]);
            if hi - lo > 5.0 {
                for p in &mut s.lab {
                    p[0] = (p[0] - lo) * 100.0 / (hi - lo);
                }
            }
        }
    }
    let k = 1.0 + a.contrast.clamp(-100, 100) as f32 / 100.0;
    let lift = a.brightness.clamp(-100, 100) as f32 / 2.0;
    let sat = a.saturation.clamp(0, 400) as f32 / 100.0;
    for p in &mut s.lab {
        p[0] = (p[0] - 50.0) * k + 50.0 + lift;
        p[1] *= sat;
        p[2] *= sat;
    }
    if a.sharpen > 0 {
        let l = ImageBuffer::<Luma<f32>, Vec<f32>>::from_fn(s.w as u32, s.h as u32, |x, y| {
            Luma([s.lab[y as usize * s.w + x as usize][0]])
        });
        let blurred = imageops::blur(&l, sigma);
        let amt = a.sharpen.min(100) as f32 / 100.0 * 1.5;
        for (p, b) in s.lab.iter_mut().zip(blurred.pixels()) {
            p[0] += amt * (p[0] - b[0]);
        }
    }
    for p in &mut s.lab {
        p[0] = p[0].clamp(0.0, 100.0);
    }
    if let Some(ink) = &s.ink {
        for (p, &k) in s.lab.iter_mut().zip(ink) {
            if k {
                *p = [0.0; 3];
            }
        }
    }
}

/// 16 colors for the image: its own, when it has that few (pixel art),
/// else k-means clusters. Black is always color 0, the usual background.
fn fit_palette(src: &RgbaImage, s: &Samples) -> Palette {
    let mut own = vec![[0u8; 3]];
    for p in src.pixels().filter(|p| p[3] >= 128) {
        let c = [p[0], p[1], p[2]];
        if !own.contains(&c) {
            own.push(c);
            if own.len() > 16 {
                break;
            }
        }
    }
    if own.len() <= 16 {
        own.resize(16, [0; 3]);
        return Palette { name: "fitted".into(), colors: own };
    }
    // Kept lines count as much as they weigh in the glyph search.
    let step = (s.lab.len() / 20_000).max(1);
    let (mut pts, mut wts, mut ink) = (vec![], vec![], ([0f32; 3], 0f32));
    for i in (0..s.lab.len()).step_by(step) {
        if s.alpha[i] < 128 {
            continue;
        }
        let (c, line) = (s.lab[i], s.weight(i % s.w, i / s.w));
        pts.push(c);
        wts.push(line);
        if line > 2.0 {
            for (sum, v) in ink.0.iter_mut().zip(c) {
                *sum += v * line;
            }
            ink.1 += line;
        }
    }
    let mut centers = lab::kmeans(&pts, &wts, 16, 12);
    centers[0] = [0.0; 3];
    // Outlines keep their own ink instead of the nearest dark average.
    if ink.1 > 0.0 {
        let ink = ink.0.map(|v| v / ink.1);
        if lab::d2(ink, [0.0; 3]) > 25.0 {
            let i = 1 + lab::nearest(&centers[1..], ink);
            centers[i] = ink;
        }
    }
    Palette { name: "fitted".into(), colors: centers.into_iter().map(lab::to_rgb).collect() }
}

// --- shared --------------------------------------------------------------------------

struct Ctx {
    classic: bool,
    pal: Vec<Lab>,
    /// Background colors allowed (8 without iCE).
    limit: usize,
    dither: Dither,
}

impl Ctx {
    fn near(&self, c: Lab, n: usize) -> u8 {
        lab::nearest(&self.pal[..n], c) as u8
    }
}

pub(crate) const BAYER: [[f32; 4]; 4] = [[0.0, 8.0, 2.0, 10.0], [12.0, 4.0, 14.0, 6.0], [3.0, 11.0, 1.0, 9.0], [15.0, 7.0, 13.0, 5.0]];

/// Ordered dither offset for lightness `l`: about half the gap between
/// shades, fading out near black and white so flat ends stay clean.
fn bayer(x: usize, y: usize, l: f32) -> f32 {
    let fade = (l.min(100.0 - l) / 10.0).clamp(0.0, 1.0);
    ((BAYER[y % 4][x % 4] + 0.5) / 16.0 * 14.0 - 7.0) * fade
}

fn rgb(c: Lab) -> Color {
    let [r, g, b] = lab::to_rgb(c);
    Color::Rgb(r, g, b)
}

/// Flat or two-color cell, with a black full block or background as blank.
fn cell(ch: char, fg: Color, bg: Color) -> Cell {
    let blank = matches!((ch, fg), ('█', Color::Pal(0)) | ('█', Color::Rgb(0, 0, 0)))
        || (ch == ' ' && matches!(bg, Color::Pal(0) | Color::Rgb(0, 0, 0)));
    if blank { Cell::BLANK } else { Cell::new(ch, fg, bg) }
}

/// Floyd–Steinberg over `px` (w×h), quantizing with `quant`.
fn diffuse(px: &mut [Lab], w: usize, h: usize, quant: impl Fn(Lab) -> Lab) {
    for y in 0..h {
        for x in 0..w {
            let old = px[y * w + x];
            let new = quant(old);
            px[y * w + x] = new;
            let e = [old[0] - new[0], old[1] - new[1], old[2] - new[2]];
            spread(px, w, h, x, y, e, 0.6);
        }
    }
}

fn spread(px: &mut [Lab], w: usize, h: usize, x: usize, y: usize, e: Lab, k: f32) {
    spread_to(px, w, h, x, y, e, k, |_| true);
}

/// `spread`, only onto neighbours (by index) that `ok` accepts.
#[allow(clippy::too_many_arguments)]
fn spread_to(px: &mut [Lab], w: usize, h: usize, x: usize, y: usize, e: Lab, k: f32, ok: impl Fn(usize) -> bool) {
    for (dx, dy, f) in [(1isize, 0usize, 7.0 / 16.0), (-1, 1, 3.0 / 16.0), (0, 1, 5.0 / 16.0), (1, 1, 1.0 / 16.0)] {
        let nx = x as isize + dx;
        if nx >= 0 && (nx as usize) < w && y + dy < h && ok((y + dy) * w + nx as usize) {
            let p = &mut px[(y + dy) * w + nx as usize];
            for i in 0..3 {
                p[i] += e[i] * f * k;
            }
        }
    }
}

// --- half blocks ---------------------------------------------------------------------

fn half(s: &Samples, cols: usize, rows: usize, ctx: &Ctx) -> (Clip, Vec<Option<Lab>>) {
    let mut px = s.lab.clone();
    if ctx.classic {
        match ctx.dither {
            Dither::Diffuse => diffuse(&mut px, s.w, s.h, |c| ctx.pal[ctx.near(c, 16) as usize]),
            Dither::Ordered => {
                for (i, p) in px.iter_mut().enumerate() {
                    p[0] += bayer(i % s.w, i / s.w, p[0]);
                }
            }
            Dither::None => {}
        }
    }
    let mut clip = Clip::new(cols, rows);
    let mut shown = vec![None; cols * rows];
    let get = |x: usize, y: usize| if x < s.w && y < s.h { px[y * s.w + x] } else { [0.0; 3] };
    for y in 0..rows {
        for x in 0..cols {
            if s.alpha(x, 2 * y) < 128 && s.alpha(x, 2 * y + 1) < 128 {
                continue;
            }
            let (t, b) = (get(x, 2 * y), get(x, 2 * y + 1));
            let (c, m) = if ctx.classic { half_classic(t, b, ctx) } else { half_rgb(t, b) };
            clip.set(x, y, Some(c));
            shown[y * cols + x] = Some(m);
        }
    }
    (clip, shown)
}

fn half_rgb(t: Lab, b: Lab) -> (Cell, Lab) {
    let mean = [(t[0] + b[0]) / 2.0, (t[1] + b[1]) / 2.0, (t[2] + b[2]) / 2.0];
    let (ft, fb) = (rgb(t), rgb(b));
    let c = if ft == fb { cell('█', ft, Color::BLACK) } else { Cell::new('▀', ft, fb) };
    (c, mean)
}

/// Best of full block, upper half and lower half, with fg from all 16
/// colors and bg from the allowed ones.
fn half_classic(t: Lab, b: Lab, ctx: &Ctx) -> (Cell, Lab) {
    let p = |i: u8| ctx.pal[i as usize];
    let avg = [(t[0] + b[0]) / 2.0, (t[1] + b[1]) / 2.0, (t[2] + b[2]) / 2.0];
    let f = ctx.near(avg, 16);
    let (uf, ub) = (ctx.near(t, 16), ctx.near(b, ctx.limit));
    let (lf, lb) = (ctx.near(b, 16), ctx.near(t, ctx.limit));
    let cands = [
        (lab::d2(t, p(f)) + lab::d2(b, p(f)), '█', f, 0),
        (lab::d2(t, p(uf)) + lab::d2(b, p(ub)), '▀', uf, ub),
        (lab::d2(b, p(lf)) + lab::d2(t, p(lb)), '▄', lf, lb),
    ];
    let (_, ch, fg, bg) = cands.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap_or(cands[0]);
    let (ch, bg) = if ch == '█' || fg == bg { ('█', 0) } else { (ch, bg) };
    let shown = if ch == '█' { p(fg) } else { std::array::from_fn(|i| (p(fg)[i] + p(bg)[i]) / 2.0) };
    (cell(ch, Color::Pal(fg), Color::Pal(bg)), shown)
}

// --- block search ------------------------------------------------------------------------

/// Sums over a region: squared error against a flat color in O(1).
#[derive(Clone, Copy, Default)]
struct Stats {
    n: f64,
    s: [f64; 3],
    s2: f64,
}

impl Stats {
    fn sub(self, o: Stats) -> Stats {
        Stats { n: self.n - o.n, s: std::array::from_fn(|i| self.s[i] - o.s[i]), s2: self.s2 - o.s2 }
    }

    fn add(self, o: Stats) -> Stats {
        Stats { n: self.n + o.n, s: std::array::from_fn(|i| self.s[i] + o.s[i]), s2: self.s2 + o.s2 }
    }

    /// Σ|p − c|².
    fn err(&self, c: Lab) -> f64 {
        let c = c.map(f64::from);
        self.s2 - 2.0 * (c[0] * self.s[0] + c[1] * self.s[1] + c[2] * self.s[2])
            + self.n * (c[0] * c[0] + c[1] * c[1] + c[2] * c[2])
    }

    fn mean(&self) -> Lab {
        if self.n <= 0.0 { [0.0; 3] } else { self.s.map(|v| (v / self.n) as f32) }
    }

    /// Error of the region's own mean.
    fn spread(&self) -> f64 {
        if self.n <= 0.0 { 0.0 } else { (self.s2 - (self.s.iter().map(|v| v * v).sum::<f64>()) / self.n).max(0.0) }
    }

    /// Best of the first `n` palette colors.
    fn best(&self, pal: &[Lab], n: usize) -> (f64, u8) {
        let mut best = (f64::MAX, 0);
        for (i, &c) in pal.iter().take(n).enumerate() {
            let e = self.err(c);
            if e < best.0 {
                best = (e, i as u8);
            }
        }
        best
    }
}

/// Summed-area table of one 8x16 cell.
struct Sat([Stats; 9 * 17]);

impl Sat {
    /// Pixels `px`, each counting `wt` times.
    fn new(px: &[Lab; 128], wt: &[f32; 128]) -> Sat {
        let mut t = [Stats::default(); 9 * 17];
        for y in 0..16 {
            let mut row = Stats::default();
            for x in 0..8 {
                let p = px[y * 8 + x].map(f64::from);
                let k = wt[y * 8 + x] as f64;
                row = row.add(Stats { n: k, s: p.map(|v| v * k), s2: k * (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]) });
                t[(y + 1) * 9 + x + 1] = t[y * 9 + x + 1].add(row);
            }
        }
        Sat(t)
    }

    fn rect(&self, (x0, y0, x1, y1): (usize, usize, usize, usize)) -> Stats {
        let t = &self.0;
        t[y1 * 9 + x1].sub(t[y0 * 9 + x1]).sub(t[y1 * 9 + x0]).add(t[y0 * 9 + x0])
    }
}

const SHADES: [char; 3] = ['░', '▒', '▓'];

/// How much a shade's texture counts against it: 1 would judge it pixel by
/// pixel, 0 only by its average color. Lower scores better on the test set,
/// but under ~0.1 shades of the wrong hue creep in up close.
const TEXTURE: f32 = 0.15;

struct ShadeMix {
    /// [shade][fg][bg] → the mixed color and its texture penalty.
    mix: Vec<(Lab, f64)>,
}

impl ShadeMix {
    fn new(pal: &[Lab]) -> ShadeMix {
        let mut mix = Vec::with_capacity(3 * 256);
        for ch in SHADES {
            let a = (0..128).filter(|&i| glyph_pixel(ch, i % 8, i / 8)).count() as f32 / 128.0;
            for f in 0..16 {
                for b in 0..16 {
                    let (lf, lb) = (lab::to_linear(pal[f]), lab::to_linear(pal[b]));
                    let m = lab::from_linear(std::array::from_fn(|i| a * lf[i] + (1.0 - a) * lb[i]));
                    // A two-color pattern's own error against its average, per pixel.
                    let tex = 128.0 * a * (1.0 - a) * lab::d2(pal[f], pal[b]) * TEXTURE;
                    mix.push((m, tex as f64));
                }
            }
        }
        ShadeMix { mix }
    }

    fn get(&self, k: usize, f: usize, b: usize) -> (Lab, f64) {
        self.mix[k * 256 + f * 16 + b]
    }
}

fn blocks(
    s: &Samples,
    cols: usize,
    rows: usize,
    ctx: &Ctx,
    masks: &[masks::Mask],
    shades: bool,
) -> (Clip, Vec<Option<Lab>>) {
    let mix = shades.then(|| ShadeMix::new(&ctx.pal));
    let mut carry = vec![[0f32; 3]; cols * rows];
    let mut clip = Clip::new(cols, rows);
    let mut shown = vec![None; cols * rows];
    let all_rect = (0, 0, 8, 16);
    // Each cell's own color, to keep diffusion from crossing edges.
    let means: Vec<Lab> = (0..cols * rows)
        .map(|i| {
            let (cx, cy) = (i % cols, i / cols);
            let mut m = [0f32; 3];
            for y in cy * 16..cy * 16 + 16 {
                for x in cx * 8..cx * 8 + 8 {
                    let c = s.get(x, y);
                    for k in 0..3 {
                        m[k] += c[k] / 128.0;
                    }
                }
            }
            m
        })
        .collect();
    for cy in 0..rows {
        for cx in 0..cols {
            let e = carry[cy * cols + cx];
            let ordered = ctx.classic && ctx.dither == Dither::Ordered;
            let mut px = [[0f32; 3]; 128];
            let mut wt = [1f32; 128];
            let mut alpha = 0u32;
            let mut plain = [0f32; 3];
            for (i, v) in px.iter_mut().enumerate() {
                let (x, y) = (cx * 8 + i % 8, cy * 16 + i / 8);
                let c = s.get(x, y);
                let bias = if ordered { bayer(cx, cy, c[0]) } else { 0.0 };
                *v = [c[0] + e[0] + bias, c[1] + e[1], c[2] + e[2]];
                wt[i] = s.weight(x, y);
                alpha += s.alpha(x, y) as u32;
                for k in 0..3 {
                    plain[k] += v[k] / 128.0;
                }
            }
            if alpha < 128 * 128 {
                continue;
            }
            let sat = Sat::new(&px, &wt);
            let all = sat.rect(all_rect);
            // (error, char, fg, bg, color seen from afar)
            let mut best: (f64, char, Color, Color, Lab);
            if ctx.classic {
                let (ef, f) = all.best(&ctx.pal, 16);
                let (eb, b) = all.best(&ctx.pal, ctx.limit);
                best = if ef <= eb {
                    (ef, '█', Color::Pal(f), Color::BLACK, ctx.pal[f as usize])
                } else {
                    (eb, ' ', Color::LIGHT_GRAY, Color::Pal(b), ctx.pal[b as usize])
                };
                for m in masks {
                    let on = m.rects.iter().fold(Stats::default(), |acc, &r| acc.add(sat.rect(r)));
                    let off = all.sub(on);
                    let ((ea, f), (eb, b)) = (on.best(&ctx.pal, 16), off.best(&ctx.pal, ctx.limit));
                    if ea + eb < best.0 && f != b {
                        let (pf, pb) = (ctx.pal[f as usize], ctx.pal[b as usize]);
                        let a = m.lit / 128.0;
                        let seen = std::array::from_fn(|i| pf[i] * a + pb[i] * (1.0 - a));
                        best = (ea + eb, m.ch, Color::Pal(f), Color::Pal(b), seen);
                    }
                }
                if let Some(mix) = &mix {
                    for (k, &shade) in SHADES.iter().enumerate() {
                        for f in 0..16 {
                            for b in 0..ctx.limit {
                                if f == b {
                                    continue;
                                }
                                let (q, tex) = mix.get(k, f, b);
                                let err = all.err(q) + tex;
                                if err < best.0 {
                                    best = (err, shade, Color::Pal(f as u8), Color::Pal(b as u8), q);
                                }
                            }
                        }
                    }
                }
            } else {
                let m = all.mean();
                best = (all.spread(), '█', rgb(m), Color::BLACK, m);
                for mask in masks {
                    let on = mask.rects.iter().fold(Stats::default(), |acc, &r| acc.add(sat.rect(r)));
                    let off = all.sub(on);
                    let (a, b) = (on.mean(), off.mean());
                    let err = on.spread() + off.spread();
                    // Two near-identical colors are just a flat cell.
                    if err < best.0 * 0.95 && lab::d2(a, b) > 4.0 {
                        best = (err, mask.ch, rgb(a), rgb(b), m);
                    }
                }
            }
            if ctx.classic && ctx.dither == Dither::Diffuse {
                // Small misses are left alone (that is what keeps flat fills
                // flat instead of speckled), and error never flows across an
                // edge into a differently colored area.
                // Inside a flat fill one steady shade glyph reads better than
                // confetti, so error only moves where the image changes.
                let d: Lab = std::array::from_fn(|i| plain[i] - best.4[i]);
                let size = lab::d2(d, [0.0; 3]).sqrt();
                let here = means[cy * cols + cx];
                let change = [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)]
                    .iter()
                    .filter_map(|&(dx, dy)| {
                        let (x, y) = (cx as isize + dx, cy as isize + dy);
                        (x >= 0 && y >= 0 && (x as usize) < cols && (y as usize) < rows)
                            .then(|| lab::d2(means[y as usize * cols + x as usize], here).sqrt())
                    })
                    .fold(all.spread().sqrt() as f32 / 128f32.sqrt(), f32::max);
                let k = 0.65 * ((size - 2.0) / 4.0).clamp(0.0, 1.0) * ((change - FLAT) / FLAT).clamp(0.0, 1.0);
                if k > 0.0 {
                    spread_to(&mut carry, cols, rows, cx, cy, d, k, |n| lab::d2(means[n], here) < 15.0 * 15.0);
                }
            }
            let (_, ch, fg, bg, seen) = best;
            clip.set(cx, cy, Some(cell(ch, fg, bg)));
            shown[cy * cols + cx] = Some(seen);
        }
    }
    (clip, shown)
}

/// How much (ΔE) a cell may differ from its neighbours and still be flat.
const FLAT: f32 = 1.5;

// --- ASCII ---------------------------------------------------------------------------------

const RAMP: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];

fn ascii(s: &Samples, ctx: &Ctx) -> (Clip, Vec<Option<Lab>>) {
    let (w, h) = (s.w, s.h);
    let mut lum: Vec<Lab> = s.lab.iter().map(|c| [c[0], 0.0, 0.0]).collect();
    let step = 100.0 / (RAMP.len() - 1) as f32;
    let level = |v: f32| (v.clamp(0.0, 100.0) / step).round();
    match ctx.dither {
        Dither::Diffuse => diffuse(&mut lum, w, h, |c| [level(c[0]) * step, 0.0, 0.0]),
        Dither::Ordered => {
            for (i, p) in lum.iter_mut().enumerate() {
                p[0] += bayer(i % w, i / w, p[0]);
            }
        }
        Dither::None => {}
    }
    let mut clip = Clip::new(w, h);
    for y in 0..h {
        for x in 0..w {
            if s.alpha(x, y) < 128 {
                continue;
            }
            let ch = RAMP[level(lum[y * w + x][0]) as usize];
            let c = s.get(x, y);
            // Full-strength color: lightness is carried by the glyph.
            let fg = if ch == ' ' || ctx.classic { Color::LIGHT_GRAY } else { rgb([75.0, c[1], c[2]]) };
            clip.set(x, y, Some(if ch == ' ' { Cell::BLANK } else { Cell::new(ch, fg, Color::BLACK) }));
        }
    }
    (clip, vec![None; w * h])
}

// --- cleanup ----------------------------------------------------------------------------------

/// A cell unlike its four neighbours, which all look alike, becomes its
/// left neighbour.
fn despeckle(clip: &mut Clip, shown: &[Option<Lab>]) {
    let (w, h) = (clip.width, clip.height);
    if w < 3 || h < 3 {
        return;
    }
    let orig = clip.clone();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let Some(c) = shown[y * w + x] else { continue };
            let n: Vec<Lab> =
                [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)].iter().filter_map(|&(a, b)| shown[b * w + a]).collect();
            if n.len() < 4 {
                continue;
            }
            let alike = n.iter().all(|a| n.iter().all(|b| lab::d2(*a, *b) < 16.0));
            if alike && lab::d2(c, n[0]) > 144.0 {
                clip.set(x, y, orig.get(x - 1, y));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_fn(w, h, |x, y| Rgba(f(x, y)));
        acidtrip_core::render::png_bytes(&img)
    }

    #[test]
    fn halfblock_square_pixels_and_colors() {
        // 4x4: top half yellow, bottom half blue.
        let b = png(4, 4, |_, y| if y < 2 { [255, 255, 85, 255] } else { [0, 0, 170, 255] });
        let opts = ImportOptions { width: 4, ..ImportOptions::default() };
        let c = image_to_clip(&b, &opts).unwrap();
        assert_eq!((c.width, c.height), (4, 2));
        assert_eq!(c.get(0, 0).unwrap(), Cell::new('█', Color::Pal(14), Color::BLACK));
        assert_eq!(c.get(0, 1).unwrap(), Cell::new('█', Color::Pal(1), Color::BLACK));
        let m = image_to_clip(&b, &ImportOptions { kind: DocKind::Modern, ..opts.clone() }).unwrap();
        assert_eq!(m.get(1, 0).unwrap().fg, Color::Rgb(255, 255, 85));
    }

    #[test]
    fn non_ice_backgrounds_are_dark() {
        let b = png(2, 2, |_, y| if y == 0 { [255, 85, 85, 255] } else { [85, 255, 255, 255] });
        let opts = ImportOptions { width: 2, ..ImportOptions::default() };
        let c = image_to_clip_ice(&b, &opts, false).unwrap().get(0, 0).unwrap();
        assert!(matches!(c.bg, Color::Pal(0..=7)), "{c:?}");
        let c = image_to_clip_ice(&b, &opts, true).unwrap().get(0, 0).unwrap();
        assert_eq!(c, Cell::new('▀', Color::Pal(12), Color::Pal(11)));
    }

    #[test]
    fn transparency_and_ink() {
        let b = png(32, 32, |x, _| {
            if x < 16 {
                [0, 0, 0, 0]
            } else if (22..24).contains(&x) {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let opts = ImportOptions { width: 8, ..ImportOptions::default() };
        let c = image_to_clip(&b, &opts).unwrap();
        assert_eq!(c.get(0, 0), None, "transparent → no cell");
        let plain = c.get(5, 1).unwrap();
        let inked = image_to_clip(&b, &ImportOptions { ink: true, ..opts }).unwrap().get(5, 1).unwrap();
        assert_ne!(plain, Cell::BLANK);
        assert_eq!(inked, Cell::BLANK, "ink keeps the thin dark line black");
    }

    #[test]
    fn suggests_by_kind_of_picture() {
        // Black circles on white paper: an ink drawing.
        let ring = |x: u32, y: u32, cx: f32, cy: f32, r: f32| {
            ((x as f32 - cx).hypot(y as f32 - cy) - r).abs() < 1.2
        };
        let ink = RgbaImage::from_fn(300, 300, |x, y| {
            if ring(x, y, 150.0, 150.0, 90.0) || ring(x, y, 100.0, 120.0, 30.0) || ring(x, y, 200.0, 120.0, 30.0) {
                image::Rgba([10, 10, 10, 255])
            } else {
                image::Rgba([250, 250, 248, 255])
            }
        });
        assert_eq!(suggest(&ink), Preset::Comic, "{:?}", features(&ink));
        // Colored flat fills with outlines: cel shading.
        let cel = RgbaImage::from_fn(300, 300, |x, y| {
            if ring(x, y, 150.0, 150.0, 90.0) || ring(x, y, 150.0, 150.0, 40.0) {
                image::Rgba([20, 15, 30, 255])
            } else if (x as f32 - 150.0).hypot(y as f32 - 150.0) < 40.0 {
                image::Rgba([240, 190, 160, 255])
            } else if (x as f32 - 150.0).hypot(y as f32 - 150.0) < 90.0 {
                image::Rgba([60, 90, 200, 255])
            } else {
                image::Rgba([120, 190, 240, 255])
            }
        });
        assert_eq!(suggest(&cel), Preset::Cel, "{:?}", features(&cel));
        // A textured, noisy scene: a photo.
        let mut seed = 7u32;
        let photo = RgbaImage::from_fn(300, 300, |x, y| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = (seed >> 24) as u8 / 4;
            image::Rgba([(x / 2) as u8 + n, 100 + n, (y / 2) as u8 + n, 255])
        });
        assert_eq!(suggest(&photo), Preset::Photo, "{:?}", features(&photo));
    }

    #[test]
    fn blocks_pick_structure() {
        // Left half red, right half blue per 8x16 cell.
        let b = png(16, 16, |x, _| if x % 8 < 4 { [170, 0, 0, 255] } else { [0, 0, 170, 255] });
        let opts = ImportOptions { width: 2, style: ImportStyle::Blocks, ..ImportOptions::default() };
        let c = image_to_clip(&b, &opts).unwrap();
        assert_eq!(c.height, 1);
        let cell = c.get(0, 0).unwrap();
        assert!(matches!(cell.ch, '▌' | '▐'), "{cell:?}");
        let modern = ImportOptions { kind: DocKind::Modern, dither: Dither::Diffuse, ..opts };
        assert!(matches!(image_to_clip(&b, &modern).unwrap().get(1, 0).unwrap().ch, '▌' | '▐'));
    }

    #[test]
    fn extended_glyphs_find_quadrants() {
        // Each 8x16 cell: top-left quarter white, the rest black.
        let b = png(16, 16, |x, y| if x % 8 < 4 && y < 8 { [255, 255, 255, 255] } else { [0, 0, 0, 255] });
        let opts = ImportOptions {
            width: 2,
            style: ImportStyle::Blocks,
            kind: DocKind::Modern,
            glyphs: Glyphs::Extended,
            scaling: Scaling::Pixel,
            ..ImportOptions::default()
        };
        assert_eq!(image_to_clip(&b, &opts).unwrap().get(0, 0).unwrap().ch, '▘');
        let cp = ImportOptions { glyphs: Glyphs::Cp437, ..opts };
        assert_ne!(image_to_clip(&b, &cp).unwrap().get(0, 0).unwrap().ch, '▘');
    }

    #[test]
    fn shades_mix_colors() {
        // A flat color between blue and black: a shade of blue.
        let b = png(8, 16, |_, _| [0, 0, 70, 255]);
        let opts = ImportOptions { width: 1, style: ImportStyle::Blocks, ..ImportOptions::default() };
        let c = image_to_clip(&b, &opts).unwrap().get(0, 0).unwrap();
        assert!(SHADES.contains(&c.ch), "{c:?}");
        let flat = image_to_clip(&b, &ImportOptions { shades: false, ..opts }).unwrap().get(0, 0).unwrap();
        assert!(!SHADES.contains(&flat.ch));
    }

    #[test]
    fn fitted_palette_and_doc() {
        let b = png(32, 32, |x, _| if x < 16 { [200, 120, 40, 255] } else { [40, 90, 200, 255] });
        let opts = ImportOptions { width: 4, fit_palette: true, scaling: Scaling::Smooth, ..ImportOptions::default() };
        let c = convert(&decode(&b).unwrap(), &opts, false).unwrap();
        assert!(c.fitted);
        assert_eq!(c.palette.colors[0], [0, 0, 0]);
        let fg = c.clip.get(0, 0).unwrap().fg;
        let Color::Pal(i) = fg else { panic!("{fg:?}") };
        let got = c.palette.colors[i as usize];
        assert!(got.iter().zip([200u8, 120, 40]).all(|(a, b)| a.abs_diff(b) < 6), "{got:?}");
        let d = image_to_doc(&b, &opts).unwrap();
        assert_eq!(d.meta.palette.name, "fitted");
    }

    #[test]
    fn pixel_art_is_found_and_kept_sharp() {
        // An 8x8 checker saved at 4x.
        let b = png(32, 32, |x, y| if (x / 4 + y / 4) % 2 == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
        let a = analyze(&decode(&b).unwrap());
        assert!(a.pixel_art);
        assert_eq!(a.native, (8, 8));
        let opts = ImportOptions { width: 8, ..ImportOptions::default() };
        let c = convert(&decode(&b).unwrap(), &opts, true).unwrap();
        assert!(c.pixel);
        assert_eq!(c.clip.get(0, 0).unwrap(), Cell::new('▀', Color::Pal(12), Color::Pal(9)));
    }

    #[test]
    fn size_fits_height() {
        let o = ImportOptions { width: 80, height: Some(25), style: ImportStyle::Blocks, ..ImportOptions::default() };
        assert_eq!(target_size(100, 100, &o), (50, 25));
        assert_eq!(target_size(200, 100, &o), (80, 20));
        let h = ImportOptions { style: ImportStyle::HalfBlock, height: None, ..o };
        assert_eq!(target_size(100, 100, &h), (80, 40));
    }

    #[test]
    fn presets_and_adjustments_run() {
        let b = png(64, 48, |x, y| [(x * 4) as u8, (y * 5) as u8, 128, 255]);
        for p in Preset::ALL {
            for kind in [DocKind::Classic, DocKind::Modern] {
                let o = p.apply(&ImportOptions { width: 16, kind, ..ImportOptions::default() });
                let c = image_to_clip(&b, &o).unwrap();
                assert_eq!(c.width, 16, "{p:?}");
            }
        }
    }

    #[test]
    fn ascii_ramp_and_dither_runs() {
        let b = png(64, 32, |x, _| {
            let v = (x * 4) as u8;
            [v, v, v, 255]
        });
        for dither in [Dither::None, Dither::Diffuse, Dither::Ordered] {
            let opts = ImportOptions {
                width: 16,
                style: ImportStyle::Ascii,
                dither,
                scaling: Scaling::Smooth,
                ..ImportOptions::default()
            };
            let c = image_to_clip(&b, &opts).unwrap();
            assert_eq!((c.width, c.height), (16, 4));
            assert_eq!(c.get(15, 0).unwrap().ch, '@');
            let d = image_to_doc(&b, &ImportOptions { style: ImportStyle::HalfBlock, ..opts }).unwrap();
            assert_eq!(d.width(), 16);
        }
    }
}
