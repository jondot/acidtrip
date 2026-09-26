//! Brushes for the smart pen, in the spirit of modern paint apps: a brush is
//! a tip (size, hardness, roundness, angle), how it lays down ink (opacity,
//! flow, spacing, scatter, grain), how it reacts to the hand (taper,
//! velocity, streamline) and the glyphs the ink is rendered with.
//!
//! The pen draws every brush the same way: dabs of ink in glyph-pixel space
//! (8x16 per cell), then each touched cell becomes the glyph of the brush's
//! set that best reproduces the ink there (see [`super::pen`]). Soft edges
//! and low opacity come out as `░ ▒ ▓`, texture as broken shading, a spray
//! as scattered dots.

use serde::{Deserialize, Serialize};

use super::pen;

/// Which glyphs a brush renders with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GlyphSet {
    /// The active tile set (plus blocks when it can't draw lines).
    #[default]
    Tiles,
    /// Half and full blocks with shades.
    Blocks,
    /// `░ ▒ ▓ █` only: pure tone.
    Shades,
    /// ASCII characters, for text-art line work.
    Ascii,
    /// Dots and circles.
    Dots,
    /// Single-line box drawing.
    Lines,
}

impl GlyphSet {
    pub const ALL: [GlyphSet; 6] =
        [GlyphSet::Tiles, GlyphSet::Blocks, GlyphSet::Shades, GlyphSet::Ascii, GlyphSet::Dots, GlyphSet::Lines];

    pub fn name(self) -> &'static str {
        match self {
            GlyphSet::Tiles => "tile set",
            GlyphSet::Blocks => "blocks",
            GlyphSet::Shades => "shades",
            GlyphSet::Ascii => "ascii",
            GlyphSet::Dots => "dots",
            GlyphSet::Lines => "lines",
        }
    }

    /// Candidate glyphs, given the active tile set.
    pub fn candidates(self, active_set: &[char]) -> Vec<char> {
        let fixed: &[char] = match self {
            GlyphSet::Tiles => return pen::default_candidates(active_set),
            GlyphSet::Blocks => &['░', '▒', '▓', '█', '▀', '▄', '▌', '▐', ' '],
            GlyphSet::Shades => &['░', '▒', '▓', '█', ' '],
            GlyphSet::Ascii => &[
                '.', ',', '\'', '`', ':', ';', '-', '_', '~', '=', '+', '*', '#', '%', '@', '/', '\\', '|', '(', ')',
                '<', '>', '^', 'o', 'O', '8', ' ',
            ],
            GlyphSet::Dots => &['·', '∙', '•', '°', '○', '■', ' '],
            GlyphSet::Lines => &['─', '│', '┌', '┐', '└', '┘', '├', '┤', '┬', '┴', '┼', ' '],
        };
        fixed.to_vec()
    }
}

/// A brush preset. Sizes are in glyph pixels (a cell is 8x16).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrushSpec {
    pub name: String,
    pub glyphs: GlyphSet,
    /// Tip radius.
    pub size: f32,
    /// 1 = crisp edge, 0 = the whole radius fades out.
    pub hardness: f32,
    /// Most ink a stroke can lay down (0..1); below 1 never reaches `█`.
    pub opacity: f32,
    /// Ink per dab (0..1); low flow builds up where the pen lingers.
    pub flow: f32,
    /// Distance between dabs, as a fraction of the size.
    pub spacing: f32,
    /// Tip aspect: 1 round, small values a flat nib.
    pub roundness: f32,
    /// Nib angle in degrees (for roundness below 1).
    pub angle: f32,
    /// Square tip instead of round.
    pub square: bool,
    /// Random dab offset, in pixels.
    pub scatter: f32,
    /// Dabs per step (sprays).
    pub count: u32,
    /// Paper texture, 0..1: breaks the ink up like chalk on rough paper.
    pub grain: f32,
    /// Length of the thin start and end of a stroke, in pixels.
    pub taper: f32,
    /// How much a fast stroke thins out, 0..1.
    pub velocity: f32,
    /// Stabilizer, 0..1: the pen trails the mouse for smoother curves.
    pub streamline: f32,
}

impl Default for BrushSpec {
    fn default() -> Self {
        BrushSpec {
            name: "Ink".into(),
            glyphs: GlyphSet::Tiles,
            size: pen::RADIUS_BLOCKS,
            hardness: 1.0,
            opacity: 1.0,
            flow: 1.0,
            spacing: 0.1,
            roundness: 1.0,
            angle: 0.0,
            square: false,
            scatter: 0.0,
            count: 1,
            grain: 0.0,
            taper: 0.0,
            velocity: 0.0,
            streamline: 0.0,
        }
    }
}

impl BrushSpec {
    /// A hard round brush with no dynamics draws as a continuous stroke
    /// (the classic smart pen); anything else is laid down in dabs.
    pub fn is_plain(&self) -> bool {
        self.hardness >= 1.0
            && self.opacity >= 1.0
            && self.flow >= 1.0
            && self.roundness >= 1.0
            && !self.square
            && self.scatter <= 0.0
            && self.grain <= 0.0
            && self.taper <= 0.0
            && self.velocity <= 0.0
    }

    /// Clamp every parameter to its range (presets loaded from disk).
    pub fn sanitized(mut self) -> Self {
        self.size = self.size.clamp(0.5, 64.0);
        self.hardness = self.hardness.clamp(0.0, 1.0);
        self.opacity = self.opacity.clamp(0.05, 1.0);
        self.flow = self.flow.clamp(0.01, 1.0);
        self.spacing = self.spacing.clamp(0.02, 4.0);
        self.roundness = self.roundness.clamp(0.05, 1.0);
        self.angle = self.angle.rem_euclid(180.0);
        self.scatter = self.scatter.clamp(0.0, 64.0);
        self.count = self.count.clamp(1, 16);
        self.grain = self.grain.clamp(0.0, 1.0);
        self.taper = self.taper.clamp(0.0, 256.0);
        self.velocity = self.velocity.clamp(0.0, 1.0);
        self.streamline = self.streamline.clamp(0.0, 0.95);
        if self.name.trim().is_empty() {
            self.name = "Brush".into();
        }
        self
    }
}

/// The built-in brushes.
pub fn presets() -> Vec<BrushSpec> {
    let b = BrushSpec::default;
    vec![
        BrushSpec { name: "Ink".into(), ..b() },
        BrushSpec { name: "Brush pen".into(), size: 7.0, taper: 48.0, velocity: 0.5, streamline: 0.3, ..b() },
        BrushSpec { name: "Marker".into(), glyphs: GlyphSet::Blocks, size: 6.0, square: true, ..b() },
        BrushSpec {
            name: "Calligraphy".into(),
            glyphs: GlyphSet::Blocks,
            size: 9.0,
            roundness: 0.22,
            angle: 45.0,
            streamline: 0.4,
            ..b()
        },
        BrushSpec {
            name: "Airbrush".into(),
            glyphs: GlyphSet::Shades,
            size: 14.0,
            hardness: 0.0,
            flow: 0.1,
            spacing: 0.08,
            ..b()
        },
        BrushSpec {
            name: "Soft shade".into(),
            glyphs: GlyphSet::Shades,
            size: 11.0,
            hardness: 0.25,
            opacity: 0.6,
            ..b()
        },
        BrushSpec { name: "Chalk".into(), glyphs: GlyphSet::Blocks, size: 10.0, hardness: 0.5, grain: 0.6, ..b() },
        BrushSpec {
            name: "Spray".into(),
            glyphs: GlyphSet::Dots,
            size: 1.6,
            spacing: 2.0,
            scatter: 14.0,
            count: 3,
            ..b()
        },
        BrushSpec { name: "ASCII".into(), glyphs: GlyphSet::Ascii, size: 2.5, streamline: 0.3, ..b() },
        BrushSpec { name: "Line art".into(), glyphs: GlyphSet::Lines, size: 1.0, ..b() },
    ]
}

/// One editable parameter of a brush, for UIs: label, range, step, format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Param {
    Glyphs,
    Size,
    Hardness,
    Opacity,
    Flow,
    Spacing,
    Roundness,
    Angle,
    Square,
    Scatter,
    Count,
    Grain,
    Taper,
    Velocity,
    Streamline,
}

impl Param {
    pub const ALL: [Param; 15] = [
        Param::Glyphs,
        Param::Size,
        Param::Hardness,
        Param::Opacity,
        Param::Flow,
        Param::Spacing,
        Param::Roundness,
        Param::Angle,
        Param::Square,
        Param::Scatter,
        Param::Count,
        Param::Grain,
        Param::Taper,
        Param::Velocity,
        Param::Streamline,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Param::Glyphs => "Glyphs",
            Param::Size => "Size",
            Param::Hardness => "Hardness",
            Param::Opacity => "Opacity",
            Param::Flow => "Flow",
            Param::Spacing => "Spacing",
            Param::Roundness => "Roundness",
            Param::Angle => "Angle",
            Param::Square => "Square tip",
            Param::Scatter => "Scatter",
            Param::Count => "Count",
            Param::Grain => "Grain",
            Param::Taper => "Taper",
            Param::Velocity => "Speed thins",
            Param::Streamline => "Streamline",
        }
    }

    /// (min, max) for numeric parameters.
    fn range(self) -> (f32, f32) {
        match self {
            Param::Size => (0.5, 48.0),
            Param::Spacing => (0.02, 4.0),
            Param::Angle => (0.0, 180.0),
            Param::Scatter => (0.0, 48.0),
            Param::Count => (1.0, 16.0),
            Param::Taper => (0.0, 200.0),
            Param::Streamline => (0.0, 0.95),
            Param::Opacity => (0.05, 1.0),
            Param::Flow => (0.01, 1.0),
            Param::Roundness => (0.05, 1.0),
            _ => (0.0, 1.0),
        }
    }

    /// Wide-range sizes: the slider is quadratic, so the small values most
    /// brushes use get most of its length.
    fn curved(self) -> bool {
        matches!(self, Param::Size | Param::Spacing | Param::Scatter | Param::Taper)
    }

    fn step(self) -> f32 {
        match self {
            Param::Size => 0.5,
            Param::Spacing => 0.02,
            Param::Angle => 15.0,
            Param::Scatter => 1.0,
            Param::Count | Param::Glyphs | Param::Square => 1.0,
            Param::Taper => 8.0,
            _ => 0.05,
        }
    }

    fn get(self, b: &BrushSpec) -> f32 {
        match self {
            Param::Glyphs => GlyphSet::ALL.iter().position(|g| *g == b.glyphs).unwrap_or(0) as f32,
            Param::Size => b.size,
            Param::Hardness => b.hardness,
            Param::Opacity => b.opacity,
            Param::Flow => b.flow,
            Param::Spacing => b.spacing,
            Param::Roundness => b.roundness,
            Param::Angle => b.angle,
            Param::Square => b.square as u8 as f32,
            Param::Scatter => b.scatter,
            Param::Count => b.count as f32,
            Param::Grain => b.grain,
            Param::Taper => b.taper,
            Param::Velocity => b.velocity,
            Param::Streamline => b.streamline,
        }
    }

    fn set(self, b: &mut BrushSpec, v: f32) {
        match self {
            Param::Glyphs => {
                let n = GlyphSet::ALL.len() as i32;
                b.glyphs = GlyphSet::ALL[(v.round() as i32).rem_euclid(n) as usize];
            }
            Param::Square => b.square = v.round() as i32 % 2 != 0,
            Param::Angle => b.angle = v.rem_euclid(180.0),
            _ => {
                let (lo, hi) = self.range();
                let v = v.clamp(lo, hi);
                match self {
                    Param::Size => b.size = v,
                    Param::Hardness => b.hardness = v,
                    Param::Opacity => b.opacity = v,
                    Param::Flow => b.flow = v,
                    Param::Spacing => b.spacing = v,
                    Param::Roundness => b.roundness = v,
                    Param::Scatter => b.scatter = v,
                    Param::Count => b.count = v.round() as u32,
                    Param::Grain => b.grain = v,
                    Param::Taper => b.taper = v,
                    Param::Velocity => b.velocity = v,
                    Param::Streamline => b.streamline = v,
                    _ => {}
                }
            }
        }
    }

    /// Step the value by `dir` steps (glyph sets and toggles wrap around).
    pub fn nudge(self, b: &mut BrushSpec, dir: i32) {
        let v = self.get(b) + dir as f32 * self.step();
        // Snap away float drift so values read cleanly (0.35, not 0.3500001).
        let s = self.step();
        self.set(b, (v / s).round() * s);
    }

    /// Set from a 0..1 slider position.
    pub fn set_fraction(self, b: &mut BrushSpec, t: f32) {
        let t = t.clamp(0.0, 1.0);
        match self {
            Param::Glyphs => {
                self.set(b, (t * (GlyphSet::ALL.len() - 1) as f32).round());
            }
            Param::Square => self.set(b, if t >= 0.5 { 1.0 } else { 0.0 }),
            _ => {
                let (lo, hi) = self.range();
                let s = self.step();
                let t = if self.curved() { t * t } else { t };
                self.set(b, ((lo + t * (hi - lo)) / s).round() * s);
            }
        }
    }

    /// Slider position (0..1); None for choices and toggles.
    pub fn fraction(self, b: &BrushSpec) -> Option<f32> {
        match self {
            Param::Glyphs | Param::Square => None,
            _ => {
                let (lo, hi) = self.range();
                let t = ((self.get(b) - lo) / (hi - lo)).clamp(0.0, 1.0);
                Some(if self.curved() { t.sqrt() } else { t })
            }
        }
    }

    pub fn display(self, b: &BrushSpec) -> String {
        match self {
            Param::Glyphs => b.glyphs.name().into(),
            Param::Square => if b.square { "on" } else { "off" }.into(),
            Param::Size => format!("{:.1} px", b.size),
            Param::Angle => format!("{:.0}°", b.angle),
            Param::Scatter | Param::Taper => format!("{:.0} px", self.get(b)),
            Param::Count => b.count.to_string(),
            Param::Spacing => format!("{:.0}%", b.spacing * 100.0),
            _ => format!("{:.0}%", self.get(b) * 100.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_slider_favors_small_brushes() {
        let mut b = BrushSpec::default();
        Param::Size.set_fraction(&mut b, 0.5);
        assert!(b.size < 14.0, "half the slider is {}", b.size);
        let t = Param::Size.fraction(&b).unwrap();
        Param::Size.set_fraction(&mut b, t);
        assert!((Param::Size.fraction(&b).unwrap() - t).abs() < 1e-3);
    }

    #[test]
    fn presets_are_sane_and_distinct() {
        let p = presets();
        let mut names: Vec<_> = p.iter().map(|b| b.name.clone()).collect();
        names.dedup();
        assert_eq!(names.len(), p.len());
        for b in &p {
            assert_eq!(b.clone().sanitized(), *b, "{} out of range", b.name);
        }
        assert!(p[0].is_plain(), "Ink stays the classic smart pen");
        assert!(p.iter().skip(1).filter(|b| !b.is_plain()).count() >= 7);
    }

    #[test]
    fn params_nudge_and_wrap() {
        let mut b = BrushSpec::default();
        Param::Size.nudge(&mut b, 2);
        assert_eq!(b.size, 4.5);
        Param::Hardness.nudge(&mut b, 1);
        assert_eq!(b.hardness, 1.0, "clamped");
        Param::Glyphs.nudge(&mut b, -1);
        assert_eq!(b.glyphs, GlyphSet::Lines, "wraps");
        Param::Square.nudge(&mut b, 1);
        assert!(b.square);
        Param::Opacity.set_fraction(&mut b, 0.0);
        assert_eq!(b.opacity, 0.05);
        assert_eq!(Param::Flow.display(&b), "100%");
    }

    #[test]
    fn toml_round_trip_with_defaults() {
        let b: BrushSpec = toml::from_str("name = \"Mine\"\nglyphs = \"shades\"\nsize = 9\n").unwrap();
        assert_eq!(b.glyphs, GlyphSet::Shades);
        assert_eq!(b.hardness, 1.0);
        let s = toml::to_string(&b).unwrap();
        assert_eq!(toml::from_str::<BrushSpec>(&s).unwrap(), b);
    }
}
