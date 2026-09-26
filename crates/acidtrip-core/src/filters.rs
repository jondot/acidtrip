//! Photo filters, iPhone and Instagram style, on plain RGB: the Filters
//! tool runs them per cell (fg and bg), the image importer on the source
//! picture.
//!
//! A filter is a preset plus adjustments on top. Instagram presets follow
//! CSSgram (MIT, Una Kravets) and instagram.css (MIT, picturepan2) for the
//! four CSSgram lacks: color layers blended over the picture (flat, linear
//! or radial gradients, CSS blend modes), then CSS filter functions, all in
//! sRGB as a browser does. iPhone presets are adjustment recipes that
//! approximate Apple's looks. Adjustments work where they behave: exposure,
//! white balance and vignette in linear light; tone, saturation and hue in
//! OKLab.
//!
//! Positions (for vignettes and gradients) are in square units: a cell is
//! 1 wide and 2 tall.

use image::RgbaImage;
use image::imageops::{self, FilterType};
use serde::{Deserialize, Serialize};

/// The adjustment sliders, in panel order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Knob {
    Exposure,
    Brightness,
    Contrast,
    Highlights,
    Shadows,
    Saturation,
    Vibrance,
    Warmth,
    Tint,
    Hue,
    Fade,
    Vignette,
    /// How much of the preset (the adjustments always apply fully).
    Strength,
}

impl Knob {
    pub const ALL: [Knob; 13] = [
        Knob::Exposure,
        Knob::Brightness,
        Knob::Contrast,
        Knob::Highlights,
        Knob::Shadows,
        Knob::Saturation,
        Knob::Vibrance,
        Knob::Warmth,
        Knob::Tint,
        Knob::Hue,
        Knob::Fade,
        Knob::Vignette,
        Knob::Strength,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Knob::Exposure => "exposure",
            Knob::Brightness => "brightness",
            Knob::Contrast => "contrast",
            Knob::Highlights => "highlights",
            Knob::Shadows => "shadows",
            Knob::Saturation => "saturation",
            Knob::Vibrance => "vibrance",
            Knob::Warmth => "warmth",
            Knob::Tint => "tint",
            Knob::Hue => "hue",
            Knob::Fade => "fade",
            Knob::Vignette => "vignette",
            Knob::Strength => "strength",
        }
    }

    /// What the slider does (hover tips).
    pub fn blurb(self) -> &'static str {
        match self {
            Knob::Exposure => "brighten or darken like a camera stop",
            Knob::Brightness => "lift or lower the midtones",
            Knob::Contrast => "spread or squeeze the tones",
            Knob::Highlights => "recover or boost the bright parts",
            Knob::Shadows => "open up or deepen the dark parts",
            Knob::Saturation => "more or less color everywhere",
            Knob::Vibrance => "boost the dull colors, spare the vivid ones",
            Knob::Warmth => "warmer (orange) or cooler (blue) light",
            Knob::Tint => "toward magenta or toward green",
            Knob::Hue => "turn every color around the wheel",
            Knob::Fade => "lift the blacks for a washed-out print",
            Knob::Vignette => "darken the edges (left of center lightens)",
            Knob::Strength => "how much of the preset",
        }
    }

    /// (min, max).
    pub fn range(self) -> (i32, i32) {
        match self {
            Knob::Fade | Knob::Strength => (0, 100),
            _ => (-100, 100),
        }
    }
}

/// Adjustment values, -100..=100 (fade 0..=100); 0 leaves the picture alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Adjust {
    pub exposure: i32,
    pub brightness: i32,
    pub contrast: i32,
    pub highlights: i32,
    pub shadows: i32,
    pub saturation: i32,
    pub vibrance: i32,
    pub warmth: i32,
    pub tint: i32,
    pub hue: i32,
    pub fade: i32,
    pub vignette: i32,
}

impl Adjust {
    pub const ZERO: Adjust = Adjust {
        exposure: 0,
        brightness: 0,
        contrast: 0,
        highlights: 0,
        shadows: 0,
        saturation: 0,
        vibrance: 0,
        warmth: 0,
        tint: 0,
        hue: 0,
        fade: 0,
        vignette: 0,
    };

    pub fn is_zero(&self) -> bool {
        *self == Adjust::ZERO
    }

    fn field(&mut self, k: Knob) -> Option<&mut i32> {
        Some(match k {
            Knob::Exposure => &mut self.exposure,
            Knob::Brightness => &mut self.brightness,
            Knob::Contrast => &mut self.contrast,
            Knob::Highlights => &mut self.highlights,
            Knob::Shadows => &mut self.shadows,
            Knob::Saturation => &mut self.saturation,
            Knob::Vibrance => &mut self.vibrance,
            Knob::Warmth => &mut self.warmth,
            Knob::Tint => &mut self.tint,
            Knob::Hue => &mut self.hue,
            Knob::Fade => &mut self.fade,
            Knob::Vignette => &mut self.vignette,
            Knob::Strength => return None,
        })
    }
}

/// A preset, plus adjustments stacked on top.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Filter {
    /// Index into [`PRESETS`]; 0 is Original.
    pub preset: usize,
    pub adjust: Adjust,
    /// 0..=100: how much of the preset.
    pub strength: i32,
}

impl Default for Filter {
    fn default() -> Self {
        Filter { preset: 0, adjust: Adjust::ZERO, strength: 100 }
    }
}

impl Filter {
    pub fn preset(p: usize) -> Filter {
        Filter { preset: p.min(PRESETS.len() - 1), ..Filter::default() }
    }

    pub fn get(&self, k: Knob) -> i32 {
        match k {
            Knob::Strength => self.strength,
            _ => *{ self.adjust }.field(k).expect("an adjustment"),
        }
    }

    pub fn set(&mut self, k: Knob, v: i32) {
        let (lo, hi) = k.range();
        let v = v.clamp(lo, hi);
        match self.adjust.field(k) {
            Some(f) => *f = v,
            None => self.strength = v,
        }
    }

    pub fn current(&self) -> &'static Preset {
        &PRESETS[self.preset.min(PRESETS.len() - 1)]
    }

    /// Changes nothing.
    pub fn is_identity(&self) -> bool {
        (self.preset == 0 || self.strength == 0) && self.adjust.is_zero()
    }

    /// Filter one color at `at` inside an area of `size` (square units).
    pub fn apply(&self, rgb: [u8; 3], at: (f32, f32), size: (f32, f32)) -> [u8; 3] {
        if self.is_identity() {
            return rgb;
        }
        let p = Place::new(at, size);
        let src = unit(rgb);
        let mut c = src;
        let preset = self.current();
        if self.preset != 0 && self.strength > 0 {
            c = preset.run(c, &p);
            let s = self.strength as f32 / 100.0;
            c = [0, 1, 2].map(|i| src[i] + (c[i] - src[i]) * s);
        }
        if !self.adjust.is_zero() {
            c = adjust(c, &self.adjust, &p);
        }
        c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
    }

    /// Filter a picture in place. Huge pictures are shrunk first (to 2048 on
    /// the long side), since an import samples far fewer pixels anyway.
    pub fn apply_image(&self, img: &RgbaImage) -> RgbaImage {
        let mut out = if img.width() as u64 * img.height() as u64 > 4_000_000 {
            let k = 2048.0 / img.width().max(img.height()) as f32;
            let (w, h) = ((img.width() as f32 * k).round() as u32, (img.height() as f32 * k).round() as u32);
            imageops::resize(img, w.max(1), h.max(1), FilterType::Triangle)
        } else {
            img.clone()
        };
        if self.is_identity() {
            return out;
        }
        let size = (out.width() as f32, out.height() as f32);
        for (x, y, px) in out.enumerate_pixels_mut() {
            let [r, g, b] = self.apply([px[0], px[1], px[2]], (x as f32 + 0.5, y as f32 + 0.5), size);
            px.0 = [r, g, b, px[3]];
        }
        out
    }
}

/// Where a color sits, for position-dependent effects.
struct Place {
    /// 0..1 across and down.
    u: f32,
    v: f32,
    /// Distance from the center over the distance to a corner (0..1).
    d: f32,
    at: (f32, f32),
    size: (f32, f32),
}

impl Place {
    fn new(at: (f32, f32), size: (f32, f32)) -> Place {
        let (w, h) = (size.0.max(1e-3), size.1.max(1e-3));
        let (dx, dy) = (at.0 - w / 2.0, at.1 - h / 2.0);
        let d = (dx * dx + dy * dy).sqrt() / (w * w + h * h).sqrt() * 2.0;
        Place { u: at.0 / w, v: at.1 / h, d, at, size: (w, h) }
    }
}

// ------------------------------------------------------------------ presets

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Original,
    Iphone,
    Instagram,
}

/// A named look: color layers, then CSS filter functions, then adjustments.
pub struct Preset {
    pub name: &'static str,
    pub family: Family,
    layers: &'static [Layer],
    css: &'static [Css],
    adjust: Adjust,
}

impl Preset {
    fn run(&self, mut c: [f32; 3], p: &Place) -> [f32; 3] {
        for l in self.layers {
            c = l.blend(c, p);
        }
        for f in self.css {
            c = f.apply(c).map(|v| v.clamp(0.0, 1.0));
        }
        if !self.adjust.is_zero() {
            c = adjust(c, &self.adjust, p);
        }
        c
    }
}

/// CSS filter functions (Filter Effects Level 1 matrices, sRGB values).
#[derive(Clone, Copy, Debug)]
enum Css {
    Contrast(f32),
    Brightness(f32),
    Saturate(f32),
    Sepia(f32),
    Grayscale(f32),
    HueRotate(f32),
}

impl Css {
    fn apply(self, [r, g, b]: [f32; 3]) -> [f32; 3] {
        let m = |m: [[f32; 3]; 3]| {
            [0, 1, 2].map(|i| m[i][0] * r + m[i][1] * g + m[i][2] * b)
        };
        match self {
            Css::Contrast(k) => [r, g, b].map(|v| (v - 0.5) * k + 0.5),
            Css::Brightness(k) => [r, g, b].map(|v| v * k),
            Css::Saturate(s) => m([
                [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
                [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
                [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
            ]),
            Css::Sepia(k) => {
                let a = 1.0 - k.min(1.0);
                m([
                    [0.393 + 0.607 * a, 0.769 - 0.769 * a, 0.189 - 0.189 * a],
                    [0.349 - 0.349 * a, 0.686 + 0.314 * a, 0.168 - 0.168 * a],
                    [0.272 - 0.272 * a, 0.534 - 0.534 * a, 0.131 + 0.869 * a],
                ])
            }
            Css::Grayscale(k) => {
                let a = 1.0 - k.min(1.0);
                m([
                    [0.2126 + 0.7874 * a, 0.7152 - 0.7152 * a, 0.0722 - 0.0722 * a],
                    [0.2126 - 0.2126 * a, 0.7152 + 0.2848 * a, 0.0722 - 0.0722 * a],
                    [0.2126 - 0.2126 * a, 0.7152 - 0.7152 * a, 0.0722 + 0.9278 * a],
                ])
            }
            Css::HueRotate(deg) => {
                let (s, c) = deg.to_radians().sin_cos();
                m([
                    [0.213 + c * 0.787 - s * 0.213, 0.715 - c * 0.715 - s * 0.715, 0.072 - c * 0.072 + s * 0.928],
                    [0.213 - c * 0.213 + s * 0.143, 0.715 + c * 0.285 + s * 0.140, 0.072 - c * 0.072 - s * 0.283],
                    [0.213 - c * 0.213 - s * 0.787, 0.715 - c * 0.715 + s * 0.715, 0.072 + c * 0.928 + s * 0.072],
                ])
            }
        }
    }
}

/// CSS mix-blend-mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Blend {
    Multiply,
    Screen,
    Overlay,
    SoftLight,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    Exclusion,
    Color,
}

impl Blend {
    fn mix(self, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
        let each = |f: fn(f32, f32) -> f32| [0, 1, 2].map(|i| f(b[i], s[i]));
        match self {
            Blend::Multiply => each(|b, s| b * s),
            Blend::Screen => each(screen),
            Blend::Overlay => each(|b, s| hard_light(s, b)),
            Blend::SoftLight => each(|b, s| {
                if s <= 0.5 {
                    b - (1.0 - 2.0 * s) * b * (1.0 - b)
                } else {
                    let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
                    b + (2.0 * s - 1.0) * (d - b)
                }
            }),
            Blend::Darken => each(f32::min),
            Blend::Lighten => each(f32::max),
            Blend::ColorDodge => each(|b, s| {
                if b <= 0.0 {
                    0.0
                } else if s >= 1.0 {
                    1.0
                } else {
                    (b / (1.0 - s)).min(1.0)
                }
            }),
            Blend::ColorBurn => each(|b, s| {
                if b >= 1.0 {
                    1.0
                } else if s <= 0.0 {
                    0.0
                } else {
                    1.0 - ((1.0 - b) / s).min(1.0)
                }
            }),
            Blend::Exclusion => each(|b, s| b + s - 2.0 * b * s),
            Blend::Color => set_lum(s, lum(b)),
        }
    }
}

fn screen(b: f32, s: f32) -> f32 {
    b + s - b * s
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 { b * 2.0 * s } else { screen(b, 2.0 * s - 1.0) }
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    let c = c.map(|v| v + d);
    let l = lum(c);
    let (n, x) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
    c.map(|v| {
        let mut v = v;
        if n < 0.0 {
            v = l + (v - l) * l / (l - n);
        }
        if x > 1.0 {
            v = l + (v - l) * (1.0 - l) / (x - l);
        }
        v
    })
}

/// A color stop: position (1.0 = the gradient's end), sRGB color, alpha.
#[derive(Clone, Copy, Debug)]
struct Stop(f32, [u8; 3], f32);

#[derive(Clone, Copy, Debug)]
enum Paint {
    Flat([u8; 3], f32),
    /// Left to right.
    Across(&'static [Stop]),
    /// Top to bottom.
    Down(&'static [Stop]),
    /// A circle from `center` (fractions of the area) to the farthest
    /// corner, or to the closest one.
    Radial { center: (f32, f32), closest: bool, stops: &'static [Stop] },
}

/// A color layer blended over the picture.
#[derive(Clone, Copy, Debug)]
struct Layer(Blend, Paint, f32);

impl Layer {
    fn blend(&self, c: [f32; 3], p: &Place) -> [f32; 3] {
        let Layer(mode, paint, opacity) = *self;
        let (color, alpha) = match paint {
            Paint::Flat(rgb, a) => (unit(rgb), a),
            Paint::Across(stops) => gradient(stops, p.u),
            Paint::Down(stops) => gradient(stops, p.v),
            Paint::Radial { center, closest, stops } => {
                let (w, h) = p.size;
                let (cx, cy) = (center.0 * w, center.1 * h);
                let corners = [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(x, y): (f32, f32)| (x - cx).hypot(y - cy));
                let r = if closest {
                    corners.iter().copied().fold(f32::MAX, f32::min)
                } else {
                    corners.iter().copied().fold(0.0, f32::max)
                };
                gradient(stops, (p.at.0 - cx).hypot(p.at.1 - cy) / r.max(1e-3))
            }
        };
        let a = (alpha * opacity).clamp(0.0, 1.0);
        if a <= 0.0 {
            return c;
        }
        let mixed = mode.mix(c, color);
        [0, 1, 2].map(|i| c[i] + (mixed[i] - c[i]) * a)
    }
}

/// Color and alpha at `t`, interpolated premultiplied as CSS does (so a
/// transparent stop fades without darkening).
fn gradient(stops: &[Stop], t: f32) -> ([f32; 3], f32) {
    let first = stops[0];
    let last = stops[stops.len() - 1];
    let pick = |s: Stop| (unit(s.1), s.2);
    if t <= first.0 {
        return pick(first);
    }
    if t >= last.0 {
        return pick(last);
    }
    let i = stops.windows(2).position(|w| t < w[1].0).unwrap_or(0);
    let (a, b) = (stops[i], stops[i + 1]);
    let k = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 1.0 };
    let alpha = a.2 + (b.2 - a.2) * k;
    let (ca, cb) = (unit(a.1), unit(b.1));
    let pre = [0, 1, 2].map(|j| ca[j] * a.2 + (cb[j] * b.2 - ca[j] * a.2) * k);
    let color = if alpha > 1e-4 { pre.map(|v| v / alpha) } else { ca };
    (color, alpha)
}

const fn rgb(hex: u32) -> [u8; 3] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

const fn flat(mode: Blend, hex: u32, alpha: f32) -> Layer {
    Layer(mode, Paint::Flat(rgb(hex), alpha), 1.0)
}

const fn radial(mode: Blend, stops: &'static [Stop], opacity: f32) -> Layer {
    Layer(mode, Paint::Radial { center: (0.5, 0.5), closest: false, stops }, opacity)
}

const A: Adjust = Adjust::ZERO;

const fn iphone(name: &'static str, css: &'static [Css], adjust: Adjust) -> Preset {
    Preset { name, family: Family::Iphone, layers: &[], css, adjust }
}

const fn insta(name: &'static str, layers: &'static [Layer], css: &'static [Css]) -> Preset {
    Preset { name, family: Family::Instagram, layers, css, adjust: A }
}

use Blend::*;
use Css::*;

const VIVID: Adjust = Adjust { contrast: 14, saturation: 22, vibrance: 25, shadows: 5, ..A };
const DRAMATIC: Adjust = Adjust { contrast: 30, highlights: -40, shadows: 15, saturation: -22, exposure: -8, ..A };
const MONO: [Css; 1] = [Grayscale(1.0)];

/// Every preset: Original, the iPhone set, then Instagram's.
pub const PRESETS: &[Preset] = &[
    Preset { name: "Original", family: Family::Original, layers: &[], css: &[], adjust: A },
    // iPhone (Photos app): adjustment recipes.
    iphone("Vivid", &[], VIVID),
    iphone("Vivid Warm", &[], Adjust { warmth: 30, tint: 5, ..VIVID }),
    iphone("Vivid Cool", &[], Adjust { warmth: -30, ..VIVID }),
    iphone("Dramatic", &[], DRAMATIC),
    iphone("Dramatic Warm", &[], Adjust { warmth: 40, tint: 8, ..DRAMATIC }),
    iphone("Dramatic Cool", &[], Adjust { warmth: -40, tint: -5, ..DRAMATIC }),
    iphone("Mono", &MONO, Adjust { contrast: 10, ..A }),
    iphone("Silvertone", &MONO, Adjust { contrast: 18, brightness: 10, highlights: 10, warmth: -10, ..A }),
    iphone("Noir", &MONO, Adjust { contrast: 60, shadows: -30, highlights: 15, exposure: -5, ..A }),
    iphone("Chrome", &[], Adjust { contrast: 20, saturation: 18, warmth: -6, shadows: -5, fade: 5, ..A }),
    iphone("Fade", &[], Adjust { saturation: -35, fade: 35, contrast: -10, ..A }),
    iphone("Instant", &[], Adjust { warmth: 22, tint: 6, fade: 25, saturation: -12, highlights: -15, ..A }),
    iphone("Process", &[], Adjust { warmth: -25, tint: -18, fade: 22, saturation: -10, contrast: 6, ..A }),
    iphone("Transfer", &[], Adjust { warmth: 28, tint: -10, fade: 28, contrast: -12, saturation: -8, ..A }),
    iphone("Tonal", &MONO, Adjust { contrast: -18, fade: 12, brightness: 5, ..A }),
    // Instagram, after CSSgram and instagram.css.
    insta("Clarendon", &[flat(Overlay, 0x7fbbe3, 0.2)], &[Contrast(1.2), Saturate(1.35)]),
    insta("Gingham", &[flat(SoftLight, 0xe6e6fa, 1.0)], &[Brightness(1.05), HueRotate(-10.0)]),
    insta("Juno", &[flat(Overlay, 0x7fbbe3, 0.2)], &[Sepia(0.35), Contrast(1.15), Brightness(1.15), Saturate(1.8)]),
    insta("Lark", &[flat(ColorDodge, 0x22253f, 1.0), flat(Darken, 0xf2f2f2, 0.8)], &[Contrast(0.9)]),
    insta("Reyes", &[Layer(SoftLight, Paint::Flat(rgb(0xefcdad), 1.0), 0.5)], &[
        Sepia(0.22),
        Brightness(1.1),
        Contrast(0.85),
        Saturate(0.75),
    ]),
    insta("Moon", &[flat(SoftLight, 0xa0a0a0, 1.0), flat(Lighten, 0x383838, 1.0)], &[
        Grayscale(1.0),
        Contrast(1.1),
        Brightness(1.1),
    ]),
    insta("Valencia", &[Layer(Exclusion, Paint::Flat(rgb(0x3a0339), 1.0), 0.5)], &[
        Contrast(1.08),
        Brightness(1.08),
        Sepia(0.08),
    ]),
    insta("Nashville", &[flat(Darken, 0xf7b099, 0.56), flat(Lighten, 0x004696, 0.4)], &[
        Sepia(0.2),
        Contrast(1.2),
        Brightness(1.05),
        Saturate(1.2),
    ]),
    insta(
        "X-Pro II",
        &[radial(ColorBurn, &[Stop(0.4, rgb(0xe6e7e0), 1.0), Stop(1.1, rgb(0x2b2aa1), 0.6)], 1.0)],
        &[Sepia(0.3)],
    ),
    insta("Lo-Fi", &[radial(Multiply, &[Stop(0.7, rgb(0x222222), 0.0), Stop(1.5, rgb(0x222222), 1.0)], 1.0)], &[
        Saturate(1.1),
        Contrast(1.5),
    ]),
    insta(
        "Earlybird",
        &[radial(
            Overlay,
            &[Stop(0.2, rgb(0xd0ba8e), 1.0), Stop(0.85, rgb(0x360309), 1.0), Stop(1.0, rgb(0x1d0210), 1.0)],
            1.0,
        )],
        &[Contrast(0.9), Sepia(0.2)],
    ),
    insta("Hudson", &[radial(Multiply, &[Stop(0.5, rgb(0xa6b1ff), 1.0), Stop(1.0, rgb(0x342134), 1.0)], 0.5)], &[
        Brightness(1.2),
        Contrast(0.9),
        Saturate(1.1),
    ]),
    insta("Amaro", &[flat(Multiply, 0x7d6918, 0.2)], &[
        HueRotate(-10.0),
        Contrast(0.9),
        Brightness(1.1),
        Saturate(1.5),
    ]),
    insta(
        "Rise",
        &[
            radial(Multiply, &[Stop(0.55, rgb(0xeccda9), 0.15), Stop(1.0, rgb(0x321e07), 0.4)], 1.0),
            radial(Overlay, &[Stop(0.0, rgb(0xe8c598), 0.8), Stop(0.9, rgb(0xe8c598), 0.0)], 0.6),
        ],
        &[Brightness(1.05), Sepia(0.2), Contrast(0.9), Saturate(0.9)],
    ),
    insta(
        "Sierra",
        &[Layer(
            Screen,
            Paint::Radial {
                center: (0.5, 0.5),
                closest: true,
                stops: &[Stop(0.0, rgb(0x804e0f), 0.5), Stop(1.0, rgb(0x000000), 0.65)],
            },
            1.0,
        )],
        &[Sepia(0.25), Contrast(1.5), Brightness(0.9), HueRotate(-15.0)],
    ),
    insta("Inkwell", &[], &[Sepia(0.3), Contrast(1.1), Brightness(1.1), Grayscale(1.0)]),
    insta("1977", &[flat(Screen, 0xf36abc, 0.3)], &[Contrast(1.1), Brightness(1.1), Saturate(1.3)]),
    insta("Toaster", &[radial(Screen, &[Stop(0.0, rgb(0x804e0f), 1.0), Stop(1.0, rgb(0x3b003b), 1.0)], 1.0)], &[
        Contrast(1.5),
        Brightness(0.9),
    ]),
    insta("Kelvin", &[flat(ColorDodge, 0x382c34, 1.0), flat(Overlay, 0xb77d21, 1.0)], &[]),
    insta("Walden", &[Layer(Screen, Paint::Flat(rgb(0x0044cc), 1.0), 0.3)], &[
        Brightness(1.1),
        HueRotate(-10.0),
        Sepia(0.3),
        Saturate(1.6),
    ]),
    insta(
        "Willow",
        &[
            radial(Overlay, &[Stop(0.55, rgb(0xd4a9af), 1.0), Stop(1.5, rgb(0x000000), 1.0)], 1.0),
            flat(Color, 0xd8cdcb, 1.0),
        ],
        &[Grayscale(0.5), Contrast(0.95), Brightness(0.9)],
    ),
    insta(
        "Mayfair",
        &[Layer(
            Overlay,
            Paint::Radial {
                center: (0.4, 0.4),
                closest: false,
                stops: &[Stop(0.0, rgb(0xffffff), 0.8), Stop(0.3, rgb(0xffc8c8), 0.6), Stop(0.6, rgb(0x111111), 1.0)],
            },
            0.4,
        )],
        &[Contrast(1.1), Saturate(1.1)],
    ),
    insta(
        "Aden",
        &[Layer(Darken, Paint::Across(&[Stop(0.0, rgb(0x420a0e), 0.2), Stop(1.0, rgb(0x420a0e), 0.0)]), 1.0)],
        &[HueRotate(-20.0), Contrast(0.9), Saturate(0.85), Brightness(1.2)],
    ),
    insta(
        "Perpetua",
        &[Layer(SoftLight, Paint::Down(&[Stop(0.0, rgb(0x005b9a), 1.0), Stop(1.0, rgb(0xe6c13d), 1.0)]), 0.5)],
        &[],
    ),
    insta("Slumber", &[flat(Lighten, 0x45290c, 0.4), flat(SoftLight, 0x7d6918, 0.5)], &[
        Saturate(0.66),
        Brightness(1.05),
    ]),
    insta("Crema", &[flat(Multiply, 0x7d6918, 0.2)], &[
        Sepia(0.5),
        Contrast(1.25),
        Brightness(1.15),
        Saturate(0.9),
        HueRotate(-2.0),
    ]),
    insta("Ludwig", &[flat(Overlay, 0x7d6918, 0.1)], &[Sepia(0.25), Contrast(1.05), Brightness(1.05), Saturate(2.0)]),
];

pub fn preset_index(name: &str) -> Option<usize> {
    PRESETS.iter().position(|p| p.name.eq_ignore_ascii_case(name))
}

// ------------------------------------------------------------- adjustments

fn unit(c: [u8; 3]) -> [f32; 3] {
    c.map(|v| v as f32 / 255.0)
}

fn to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

fn oklab([r, g, b]: [f32; 3]) -> [f32; 3] {
    let l = (0.4122215 * r + 0.5363326 * g + 0.0514460 * b).max(0.0).cbrt();
    let m = (0.2119035 * r + 0.6806995 * g + 0.107397 * b).max(0.0).cbrt();
    let s = (0.0883025 * r + 0.2817189 * g + 0.6299787 * b).max(0.0).cbrt();
    [
        0.2104543 * l + 0.7936178 * m - 0.0040720 * s,
        1.9779985 * l - 2.4285922 * m + 0.4505937 * s,
        0.0259040 * l + 0.7827718 * m - 0.8086758 * s,
    ]
}

fn oklab_linear([l, a, b]: [f32; 3]) -> [f32; 3] {
    let l_ = (l + 0.3963378 * a + 0.2158038 * b).powi(3);
    let m_ = (l - 0.1055613 * a - 0.0638542 * b).powi(3);
    let s_ = (l - 0.0894842 * a - 1.2914855 * b).powi(3);
    [
        4.0767417 * l_ - 3.3077116 * m_ + 0.2309699 * s_,
        -1.268438 * l_ + 2.6097574 * m_ - 0.3413193 * s_,
        -0.0041961 * l_ - 0.7034186 * m_ + 1.7076147 * s_,
    ]
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// An S-curve through (0,0), (.5,.5), (1,1); steeper with `k`.
fn s_curve(x: f32, k: f32) -> f32 {
    if k < 0.01 {
        return x;
    }
    let s = |v: f32| 1.0 / (1.0 + (-k * (v - 0.5)).exp());
    (s(x) - s(0.0)) / (s(1.0) - s(0.0))
}

/// Apply adjustments to an sRGB color (0..1).
fn adjust(c: [f32; 3], a: &Adjust, p: &Place) -> [f32; 3] {
    let f = |v: i32| v as f32 / 100.0;
    let mut lin = c.map(to_linear);
    // Light: exposure (±1.5 stops), white balance, vignette.
    if a.exposure != 0 {
        let k = 2f32.powf(f(a.exposure) * 1.5);
        lin = lin.map(|v| v * k);
    }
    if a.warmth != 0 || a.tint != 0 {
        let (w, t) = (f(a.warmth), f(a.tint));
        lin = [lin[0] * (1.0 + 0.14 * w), lin[1] * (1.0 - 0.1 * t), lin[2] * (1.0 - 0.14 * w)];
    }
    if a.vignette != 0 {
        let k = smoothstep(0.3, 1.05, p.d) * f(a.vignette);
        lin = if k > 0.0 { lin.map(|v| v * (1.0 - 0.8 * k)) } else { lin.map(|v| v + (1.0 - v) * -0.6 * k) };
    }
    let tone = a.brightness != 0 || a.contrast != 0 || a.highlights != 0 || a.shadows != 0 || a.fade != 0;
    let color = a.saturation != 0 || a.vibrance != 0 || a.hue != 0 || a.fade != 0;
    if tone || color {
        let [mut l, mut ca, mut cb] = oklab(lin.map(|v| v.max(0.0)));
        if tone {
            l = l.clamp(0.0, 1.0);
            if a.brightness != 0 {
                l = l.powf(2f32.powf(-f(a.brightness) * 0.7));
            }
            if a.contrast > 0 {
                l = s_curve(l, f(a.contrast) * 9.0);
            } else if a.contrast < 0 {
                l += (0.5 - l) * -f(a.contrast) * 0.5;
            }
            if a.highlights != 0 {
                let (h, w) = (f(a.highlights), smoothstep(0.45, 1.0, l));
                l += if h > 0.0 { (1.0 - l) * h * w * 0.6 } else { l * h * w * 0.35 };
            }
            if a.shadows != 0 {
                let (s, w) = (f(a.shadows), 1.0 - smoothstep(0.0, 0.55, l));
                l += if s > 0.0 { (1.0 - l) * s * w * 0.45 } else { l * s * w * 0.6 };
            }
            if a.fade != 0 {
                let k = f(a.fade);
                l = 0.2 * k + l * (1.0 - 0.3 * k);
            }
        }
        if color {
            let mut chroma = ca.hypot(cb);
            let mut hue = cb.atan2(ca);
            chroma *= 1.0 + f(a.saturation);
            if a.vibrance != 0 {
                chroma *= 1.0 + f(a.vibrance) * (1.0 - (chroma / 0.2).min(1.0));
            }
            chroma *= 1.0 - 0.25 * f(a.fade);
            hue += f(a.hue) * std::f32::consts::PI;
            (ca, cb) = (chroma.max(0.0) * hue.cos(), chroma.max(0.0) * hue.sin());
        }
        lin = oklab_linear([l, ca, cb]);
    }
    lin.map(to_srgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MID: (f32, f32) = (5.0, 5.0);
    const AREA: (f32, f32) = (10.0, 10.0);

    fn run(f: &Filter, c: [u8; 3]) -> [u8; 3] {
        f.apply(c, MID, AREA)
    }

    #[test]
    fn every_preset_from_the_spec_is_there() {
        let names = [
            "Vivid", "Vivid Warm", "Vivid Cool", "Dramatic", "Dramatic Warm", "Dramatic Cool", "Mono",
            "Silvertone", "Noir", "Chrome", "Fade", "Instant", "Process", "Transfer", "Tonal", "Clarendon",
            "Gingham", "Juno", "Lark", "Reyes", "Moon", "Valencia", "Nashville", "X-Pro II", "Lo-Fi",
            "Earlybird", "Hudson", "Amaro", "Rise", "Sierra", "Inkwell", "1977", "Toaster", "Kelvin", "Walden",
            "Willow", "Mayfair", "Aden", "Perpetua", "Slumber", "Crema", "Ludwig",
        ];
        for n in names {
            assert!(preset_index(n).is_some(), "{n} missing");
        }
        assert_eq!(PRESETS.len(), names.len() + 1);
        assert_eq!(PRESETS[0].name, "Original");
    }

    #[test]
    fn original_and_zero_change_nothing() {
        let f = Filter::default();
        assert!(f.is_identity());
        for c in [[0, 0, 0], [255, 255, 255], [12, 200, 90]] {
            assert_eq!(run(&f, c), c);
        }
        // A preset at strength 0 is off too.
        let f = Filter { strength: 0, ..Filter::preset(preset_index("Noir").unwrap()) };
        assert_eq!(run(&f, [200, 30, 30]), [200, 30, 30]);
    }

    #[test]
    fn color_spaces_round_trip_without_drift() {
        for c in [[0, 0, 0], [255, 255, 255], [128, 64, 32], [10, 120, 250]] {
            let lin = unit(c).map(to_linear);
            let back = oklab_linear(oklab(lin)).map(to_srgb).map(|v| (v * 255.0).round() as u8);
            assert_eq!(back, c);
        }
    }

    #[test]
    fn mono_presets_are_gray() {
        for n in ["Mono", "Noir", "Tonal", "Inkwell", "Moon"] {
            let f = Filter::preset(preset_index(n).unwrap());
            let [r, g, b] = run(&f, [200, 40, 90]);
            assert!(r.abs_diff(g) <= 3 && g.abs_diff(b) <= 3, "{n}: {r},{g},{b}");
        }
    }

    #[test]
    fn warm_and_cool_go_opposite_ways() {
        let gray = [128, 128, 128];
        let warm = run(&Filter::preset(preset_index("Vivid Warm").unwrap()), gray);
        let cool = run(&Filter::preset(preset_index("Vivid Cool").unwrap()), gray);
        assert!(warm[0] > warm[2], "{warm:?}");
        assert!(cool[2] > cool[0], "{cool:?}");
    }

    #[test]
    fn sliders_move_the_right_way() {
        let c = [120, 100, 80];
        let luma = |c: [u8; 3]| c[0] as i32 * 3 + c[1] as i32 * 6 + c[2] as i32;
        let chroma = |c: [u8; 3]| c.iter().max().unwrap() - c.iter().min().unwrap();
        let with = |k: Knob, v: i32| {
            let mut f = Filter::default();
            f.set(k, v);
            run(&f, c)
        };
        assert!(luma(with(Knob::Exposure, 50)) > luma(c));
        assert!(luma(with(Knob::Brightness, -50)) < luma(c));
        assert!(luma(with(Knob::Shadows, 80)) > luma(c));
        assert!(chroma(with(Knob::Saturation, -100)) <= 1);
        assert!(chroma(with(Knob::Vibrance, 80)) > chroma(c));
        assert!(with(Knob::Warmth, 60)[0] > c[0]);
        // Fade lifts black.
        let mut f = Filter::default();
        f.set(Knob::Fade, 100);
        assert!(run(&f, [0, 0, 0])[0] > 20);
        // Contrast pushes a dark gray darker and a light one lighter.
        let mut f = Filter::default();
        f.set(Knob::Contrast, 60);
        assert!(run(&f, [60, 60, 60])[0] < 60);
        assert!(run(&f, [200, 200, 200])[0] > 200);
    }

    #[test]
    fn vignette_darkens_edges_not_center() {
        let mut f = Filter::default();
        f.set(Knob::Vignette, 100);
        let c = [200, 200, 200];
        assert_eq!(f.apply(c, (5.0, 5.0), AREA), c);
        assert!(f.apply(c, (0.2, 0.2), AREA)[0] < 120);
        // Radial gradients follow the position too.
        let lofi = Filter::preset(preset_index("Lo-Fi").unwrap());
        let center = lofi.apply(c, (5.0, 5.0), AREA);
        let corner = lofi.apply(c, (0.1, 0.1), AREA);
        assert!(corner[0] < center[0], "{corner:?} vs {center:?}");
    }

    #[test]
    fn css_matches_browser_math() {
        // sepia(1) on white is the classic (255, 255, 239) after clamping.
        let s = Css::Sepia(1.0).apply([1.0, 1.0, 1.0]).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        assert_eq!(s, [255, 255, 239]);
        // hue-rotate(360) is the identity.
        let h = Css::HueRotate(360.0).apply([0.2, 0.5, 0.8]);
        assert!((h[0] - 0.2).abs() < 1e-4 && (h[2] - 0.8).abs() < 1e-4);
        // Screen with black leaves the backdrop alone; multiply with white too.
        assert_eq!(Blend::Screen.mix([0.3, 0.4, 0.5], [0.0; 3]), [0.3, 0.4, 0.5]);
        assert_eq!(Blend::Multiply.mix([0.3, 0.4, 0.5], [1.0; 3]), [0.3, 0.4, 0.5]);
    }

    #[test]
    fn transparent_gradient_stops_do_not_darken() {
        let (c, a) = gradient(&[Stop(0.0, rgb(0xffffff), 1.0), Stop(1.0, rgb(0x000000), 0.0)], 0.5);
        assert!((a - 0.5).abs() < 1e-4);
        assert!(c[0] > 0.99, "{c:?}");
    }

    #[test]
    fn strength_blends_toward_the_original() {
        let noir = preset_index("Noir").unwrap();
        let c = [220, 40, 40];
        let full = run(&Filter::preset(noir), c);
        let half = run(&Filter { strength: 50, ..Filter::preset(noir) }, c);
        assert!(half[0] > full[0] && half[0] < c[0] + 1);
    }

    #[test]
    fn images_keep_alpha() {
        let mut img = RgbaImage::new(4, 2);
        img.put_pixel(1, 1, image::Rgba([100, 150, 200, 77]));
        let out = Filter::preset(preset_index("1977").unwrap()).apply_image(&img);
        assert_eq!(out.get_pixel(1, 1)[3], 77);
        assert_ne!(out.get_pixel(1, 1).0, [100, 150, 200, 77]);
    }

    #[test]
    fn knobs_clamp_to_their_range() {
        let mut f = Filter::default();
        f.set(Knob::Fade, -40);
        assert_eq!(f.get(Knob::Fade), 0);
        f.set(Knob::Hue, 400);
        assert_eq!(f.get(Knob::Hue), 100);
        f.set(Knob::Strength, 55);
        assert_eq!(f.get(Knob::Strength), 55);
    }
}
