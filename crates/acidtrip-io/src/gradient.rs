//! Gradient fills: a ramp of colors laid over a region along a drag, linear
//! or radial, drawn the way 90s artists shaded by hand.
//!
//! The ramp is a list of stops, interpolated in OKLab. How it becomes cells:
//! - shades: solid "keys" with ░▒▓ mixes of each neighbouring pair between
//!   them, in even bands (the scene look). In Classic the keys are the
//!   palette colors nearest the stops, plus any palette color that lies on
//!   the way (black→white steps through both grays);
//! - dither: the same ladder, with an ordered pattern across band edges;
//! - halves: ▀/▄ with a color per half cell (Classic: the keys, dithered
//!   per half cell; Modern: truecolor);
//! - smooth: a truecolor block per cell (Modern; Classic dithers instead).
//!
//! Color matching reuses the image importer's OKLab code.

use acidtrip_core::{Canvas, Cell, Color, DocKind, DocMeta, Document, TxBuilder};

use crate::import::BAYER;
use crate::import::lab::{self, Lab};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Linear,
    /// Rings around where the drag starts; the drag length is the radius.
    Radial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Style {
    /// ░▒▓█ two-color mixes (CP437-safe).
    #[default]
    Shades,
    /// ▀▄ with two colors per cell: twice the steps vertically.
    Halves,
    /// Truecolor per cell (Modern only; Classic dithers instead).
    Smooth,
    /// The shade ladder with an ordered dither across band edges.
    Dither,
}

impl Style {
    pub const ALL: [Style; 4] = [Style::Shades, Style::Halves, Style::Smooth, Style::Dither];

    pub fn name(self) -> &'static str {
        match self {
            Style::Shades => "shades",
            Style::Halves => "half blocks",
            Style::Smooth => "smooth",
            Style::Dither => "dither",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ramp {
    /// Foreground to background.
    #[default]
    Brush,
    Fire,
    Ice,
    Sunset,
    Gray,
    Rainbow,
}

impl Ramp {
    pub const ALL: [Ramp; 6] = [Ramp::Brush, Ramp::Fire, Ramp::Ice, Ramp::Sunset, Ramp::Gray, Ramp::Rainbow];

    pub fn name(self) -> &'static str {
        match self {
            Ramp::Brush => "fg → bg",
            Ramp::Fire => "fire",
            Ramp::Ice => "ice",
            Ramp::Sunset => "sunset",
            Ramp::Gray => "gray",
            Ramp::Rainbow => "rainbow",
        }
    }

    /// The ramp's colors, first to last. Presets use VGA colors, so Classic
    /// documents step through exactly these.
    pub fn stops(self, fg: Color, bg: Color) -> Vec<Color> {
        let pal = |ix: &[u8]| ix.iter().map(|&i| Color::Pal(i)).collect();
        match self {
            Ramp::Brush => vec![fg, bg],
            Ramp::Fire => pal(&[0, 4, 12, 14, 15]),
            Ramp::Ice => pal(&[0, 1, 9, 11, 15]),
            Ramp::Sunset => pal(&[1, 5, 12, 14]),
            Ramp::Gray => pal(&[0, 8, 7, 15]),
            Ramp::Rainbow => pal(&[12, 14, 10, 11, 9, 13]),
        }
    }
}

/// The gradient tool's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Options {
    pub shape: Shape,
    pub style: Style,
    pub ramp: Ramp,
    /// Run the ramp last to first.
    pub reverse: bool,
}

impl Options {
    pub fn stops(&self, fg: Color, bg: Color) -> Vec<Color> {
        let mut s = self.ramp.stops(fg, bg);
        if self.reverse {
            s.reverse();
        }
        s
    }
}

/// How far (OKLab ΔE, L 0–100) a palette color may sit off the straight
/// path between two stops and still become a step on the way.
const ON_PATH: f32 = 7.0;

/// Shade glyphs by how much of the cell the foreground covers.
const MIXES: [(f32, char); 3] = [(0.25, '░'), (0.5, '▒'), (0.75, '▓')];

/// Everything resolved for one document and ramp.
struct Plan {
    classic: bool,
    style: Style,
    /// Palette in OKLab (Classic: the 16 colors).
    pal: Vec<Lab>,
    /// Background colors allowed (Classic without iCE: 8).
    limit: usize,
    /// The ramp in OKLab, stops evenly spaced over t in 0..1.
    stops: Vec<Lab>,
    /// Solid colors on the ladder with where they sit on t, ascending.
    keys: Vec<(Color, f32)>,
    /// Keys with ░▒▓ mixes between: 4 * (keys - 1) + 1 cells.
    ladder: Vec<Cell>,
}

impl Plan {
    fn new(meta: &DocMeta, style: Style, stops: &[Color]) -> Plan {
        let classic = meta.kind == DocKind::Classic;
        let n = if classic { 16 } else { meta.palette.len() };
        let pal: Vec<Lab> = (0..n.max(16)).map(|i| lab::from_rgb(meta.palette.get(i as u8))).collect();
        let limit = if classic && !meta.ice { 8 } else { 16 };
        let stops: Vec<Color> = if stops.is_empty() { vec![Color::WHITE] } else { stops.to_vec() };
        let labs: Vec<Lab> = stops.iter().map(|c| lab::from_rgb(c.rgb(&meta.palette))).collect();
        let at = |i: usize| if labs.len() > 1 { i as f32 / (labs.len() - 1) as f32 } else { 0.0 };
        let mut keys: Vec<(Color, f32)> = vec![];
        if classic {
            let near = |c: Lab| lab::nearest(&pal[..16], c);
            for (i, &s) in labs.iter().enumerate() {
                keys.push((Color::Pal(near(s) as u8), at(i)));
                let Some(&e) = labs.get(i + 1) else { break };
                let (a, b) = (near(s), near(e));
                // Palette colors on the way, in order along the segment.
                let d = [e[0] - s[0], e[1] - s[1], e[2] - s[2]];
                let len2 = lab::d2(s, e);
                let mut on: Vec<(f32, usize)> = (0..16)
                    .filter(|&c| c != a && c != b && len2 > 0.0)
                    .filter_map(|c| {
                        let p = pal[c];
                        let k = ((p[0] - s[0]) * d[0] + (p[1] - s[1]) * d[1] + (p[2] - s[2]) * d[2]) / len2;
                        let q = [s[0] + d[0] * k, s[1] + d[1] * k, s[2] + d[2] * k];
                        ((0.12..=0.88).contains(&k) && lab::d2(p, q) <= ON_PATH * ON_PATH).then_some((k, c))
                    })
                    .collect();
                on.sort_by(|x, y| x.0.total_cmp(&y.0));
                keys.extend(on.into_iter().map(|(k, c)| (Color::Pal(c as u8), at(i) + k * (at(i + 1) - at(i)))));
            }
        } else {
            keys = stops.iter().enumerate().map(|(i, &c)| (c, at(i))).collect();
        }
        keys.dedup_by(|b, a| a.0 == b.0);
        if keys.len() > 1
            && let Some(last) = keys.last_mut()
        {
            last.1 = 1.0;
        }
        let mut plan = Plan { classic, style, pal, limit, stops: labs, keys, ladder: vec![] };
        plan.ladder = plan.build_ladder();
        plan
    }

    fn lab_of(&self, c: Color) -> Lab {
        match c {
            Color::Pal(i) => self.pal.get(i as usize).copied().unwrap_or([0.0; 3]),
            Color::Rgb(r, g, b) => lab::from_rgb([r, g, b]),
        }
    }

    fn bg_ok(&self, c: Color) -> bool {
        !self.classic || matches!(c, Color::Pal(i) if (i as usize) < self.limit)
    }

    fn build_ladder(&self) -> Vec<Cell> {
        let mut out = vec![];
        for w in self.keys.windows(2) {
            let (a, b) = (w[0].0, w[1].0);
            out.push(solid(a));
            for (f, ch) in MIXES {
                out.push(self.mix(a, b, f, ch));
            }
        }
        out.extend(self.keys.last().map(|k| solid(k.0)));
        out
    }

    /// A cell that looks `f` of the way from `a` to `b`: `b` as the ink
    /// over `a`, or `a` over `b` with the opposite shade, whichever puts the
    /// darker color behind (and an allowed one).
    fn mix(&self, a: Color, b: Color, f: f32, ch: char) -> Cell {
        let flip = MIXES.iter().find(|m| (m.0 - (1.0 - f)).abs() < 0.01).map_or(ch, |m| m.1);
        let over_a = Cell::new(ch, b, a);
        let over_b = Cell::new(flip, a, b);
        let darker_a = self.lab_of(a)[0] <= self.lab_of(b)[0];
        match (self.bg_ok(a), self.bg_ok(b)) {
            (true, true) if darker_a => over_a,
            (true, true) => over_b,
            (true, false) => over_a,
            (false, true) => over_b,
            // Two bright colors without iCE: the nearer one, flat.
            (false, false) => solid(if f <= 0.5 { a } else { b }),
        }
    }

    /// The ramp's color at `t`.
    fn target(&self, t: f32) -> Lab {
        let n = self.stops.len();
        if n == 1 {
            return self.stops[0];
        }
        let s = t.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (s.floor() as usize).min(n - 2);
        let f = s - i as f32;
        let (a, b) = (self.stops[i], self.stops[i + 1]);
        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * f)
    }

    /// Where `t` falls between the keys, 0..=1 over the whole ladder.
    fn ladder_pos(&self, t: f32) -> f32 {
        let k = self.keys.len();
        if k < 2 {
            return 0.0;
        }
        let t = t.clamp(0.0, 1.0);
        let i = self.keys.windows(2).position(|w| t <= w[1].1).unwrap_or(k - 2);
        let (t0, t1) = (self.keys[i].1, self.keys[i + 1].1);
        let f = if t1 > t0 { ((t - t0) / (t1 - t0)).clamp(0.0, 1.0) } else { 1.0 };
        (i as f32 + f) / (k - 1) as f32
    }

    /// Index into `n` even bands at `pos` (0..=1), dithered by `thr` (0..1)
    /// across band edges when given.
    fn band(pos: f32, n: usize, thr: Option<f32>) -> usize {
        let v = match thr {
            Some(th) => pos * n as f32 - 0.5 + th,
            None => pos * n as f32,
        };
        (v.floor().max(0.0) as usize).min(n.saturating_sub(1))
    }

    /// The cell at (x, y), given the ramp position of the cell's upper and
    /// lower halves.
    fn cell(&self, x: usize, y: usize, t_top: f32, t_bot: f32) -> Cell {
        let thr = |px: usize, py: usize| (BAYER[py % 4][px % 4] + 0.5) / 16.0;
        let t = (t_top + t_bot) / 2.0;
        match (self.style, self.classic) {
            (Style::Shades, _) => self.ladder[Plan::band(self.ladder_pos(t), self.ladder.len(), None)],
            (Style::Dither, _) | (Style::Smooth, true) => {
                self.ladder[Plan::band(self.ladder_pos(t), self.ladder.len(), Some(thr(x, y)))]
            }
            (Style::Smooth, false) => solid(rgb(self.target(t))),
            (Style::Halves, false) => halves(rgb(self.target(t_top)), rgb(self.target(t_bot)), |_| true),
            (Style::Halves, true) => {
                let k = self.keys.len();
                let key = |t: f32, py: usize| self.keys[Plan::band(self.ladder_pos(t), k, Some(thr(x, py)))].0;
                halves(key(t_top, 2 * y), key(t_bot, 2 * y + 1), |c| self.bg_ok(c))
            }
        }
    }
}

fn rgb(c: Lab) -> Color {
    let [r, g, b] = lab::to_rgb(c);
    Color::Rgb(r, g, b)
}

fn is_black(c: Color) -> bool {
    matches!(c, Color::Pal(0) | Color::Rgb(0, 0, 0))
}

/// A flat cell: a full block, or plain background for black.
fn solid(c: Color) -> Cell {
    if is_black(c) { Cell::BLANK } else { Cell::new('█', c, Color::BLACK) }
}

/// Upper half `top`, lower half `bot`, with an allowed background.
fn halves(top: Color, bot: Color, bg_ok: impl Fn(Color) -> bool) -> Cell {
    if top == bot {
        solid(top)
    } else if bg_ok(bot) {
        Cell::new('▀', top, bot)
    } else if bg_ok(top) {
        Cell::new('▄', bot, top)
    } else {
        solid(top)
    }
}

/// Cells are twice as tall as wide: geometry runs in square units.
fn point(x: f32, y: f32) -> (f32, f32) {
    (x + 0.5, (y + 0.5) * 2.0)
}

/// Maps a point to its place on the ramp (0..=1).
struct Field {
    shape: Shape,
    from: (f32, f32),
    dir: (f32, f32),
    /// |dir|² (linear) or |dir| (radial).
    len: f32,
}

impl Field {
    /// From the drag. A click without a drag runs top to bottom over the
    /// region (radial: out from the click to the region's farthest cell).
    fn new(shape: Shape, mask: &[bool], w: usize, from: (usize, usize), to: (usize, usize)) -> Field {
        let (mut a, mut b) = (point(from.0 as f32, from.1 as f32), point(to.0 as f32, to.1 as f32));
        let cells = || mask.iter().enumerate().filter(|m| *m.1).map(|(i, _)| (i % w, i / w));
        if from == to {
            match shape {
                Shape::Linear => {
                    let (y0, y1) = cells().fold((usize::MAX, 0), |(lo, hi), (_, y)| (lo.min(y), hi.max(y)));
                    let (x0, x1) = cells().fold((usize::MAX, 0), |(lo, hi), (x, _)| (lo.min(x), hi.max(x)));
                    if y1 > y0 {
                        (a, b) = (point(a.0 - 0.5, y0 as f32), point(a.0 - 0.5, y1 as f32));
                    } else if x1 > x0 {
                        (a, b) = (point(x0 as f32, y0 as f32), point(x1 as f32, y0 as f32));
                    }
                }
                Shape::Radial => {
                    let far = cells()
                        .map(|(x, y)| point(x as f32, y as f32))
                        .max_by(|p, q| dist2(*p, a).total_cmp(&dist2(*q, a)));
                    b = far.unwrap_or(a);
                }
            }
        }
        let dir = (b.0 - a.0, b.1 - a.1);
        let len = match shape {
            Shape::Linear => dir.0 * dir.0 + dir.1 * dir.1,
            Shape::Radial => (dir.0 * dir.0 + dir.1 * dir.1).sqrt(),
        };
        Field { shape, from: a, dir, len }
    }

    fn t(&self, p: (f32, f32)) -> f32 {
        if self.len <= 1e-6 {
            return 0.0;
        }
        let d = (p.0 - self.from.0, p.1 - self.from.1);
        let t = match self.shape {
            Shape::Linear => (d.0 * self.dir.0 + d.1 * self.dir.1) / self.len,
            Shape::Radial => (d.0 * d.0 + d.1 * d.1).sqrt() / self.len,
        };
        t.clamp(0.0, 1.0)
    }
}

fn dist2(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)
}

/// Fill the cells of `mask` (row-major over the canvas) on `layer` with the
/// ramp `stops`, running from the drag's first cell to its last.
pub fn fill(
    b: &mut TxBuilder,
    layer: usize,
    mask: &[bool],
    drag: ((usize, usize), (usize, usize)),
    opts: &Options,
    stops: &[Color],
) {
    let w = b.width();
    let plan = Plan::new(b.meta(), opts.style, stops);
    let field = Field::new(opts.shape, mask, w, drag.0, drag.1);
    for (i, _) in mask.iter().enumerate().filter(|m| *m.1) {
        let (x, y) = (i % w, i / w);
        let (xf, yf) = (x as f32, y as f32);
        let (t_top, t_bot) = (field.t(point(xf, yf - 0.25)), field.t(point(xf, yf + 0.25)));
        b.set(layer, x, y, Some(plan.cell(x, y, t_top, t_bot)));
    }
}

/// The ramp as a `w`x`h` strip, left to right, drawn in `opts.style` for a
/// document like `meta` (the tool panel's preview).
pub fn strip(meta: &DocMeta, w: usize, h: usize, opts: &Options, stops: &[Color]) -> Vec<Cell> {
    let doc = Document::with_canvas(meta.clone(), Canvas::new(w, h));
    let mut b = TxBuilder::new(&doc, "strip");
    let o = Options { shape: Shape::Linear, ..*opts };
    fill(&mut b, 0, &vec![true; w * h], ((0, 0), (w.saturating_sub(1), 0)), &o, stops);
    (0..w * h).map(|i| b.get(0, i % w, i / w).unwrap_or(Cell::BLANK)).collect()
}

/// The ramp's color at `t` (0..=1) as RGB, for swatches.
pub fn sample(meta: &DocMeta, stops: &[Color], t: f32) -> [u8; 3] {
    lab::to_rgb(Plan::new(meta, Style::Smooth, stops).target(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(kind: DocKind, w: usize, h: usize) -> Document {
        Document::new(kind, w, h)
    }

    fn run(d: &Document, from: (usize, usize), to: (usize, usize), opts: Options, stops: &[Color]) -> Document {
        let mask = vec![true; d.width() * d.height()];
        let mut b = TxBuilder::new(d, "g");
        fill(&mut b, 0, &mask, (from, to), &opts, stops);
        let tx = b.finish();
        let mut out = d.clone();
        out.apply(&tx);
        out
    }

    fn row(d: &Document, y: usize) -> Vec<Cell> {
        (0..d.width()).map(|x| d.canvas.composite(x, y)).collect()
    }

    fn lightness(d: &Document, c: Cell) -> f32 {
        // What the cell looks like from afar: the glyph's ink over its background.
        let ink = match c.ch {
            '░' => 0.25,
            '▒' => 0.5,
            '▓' => 0.75,
            ' ' => 0.0,
            _ => 1.0,
        };
        let (f, b) = (c.fg.rgb(&d.meta.palette).map(lab::linear), c.bg.rgb(&d.meta.palette).map(lab::linear));
        lab::from_linear(std::array::from_fn(|i| ink * f[i] + (1.0 - ink) * b[i]))[0]
    }

    #[test]
    fn classic_black_to_white_steps_through_the_grays_with_shades() {
        let d = doc(DocKind::Classic, 13, 1);
        let out = run(&d, (0, 0), (12, 0), Options::default(), &[Color::BLACK, Color::WHITE]);
        let cells = row(&out, 0);
        assert_eq!(cells[0], Cell::BLANK);
        assert_eq!(cells[12], Cell::new('█', Color::WHITE, Color::BLACK));
        let used: Vec<Color> = cells.iter().flat_map(|c| [c.fg, c.bg]).collect();
        assert!(used.contains(&Color::Pal(8)) && used.contains(&Color::Pal(7)), "{cells:?}");
        assert!(cells.iter().any(|c| c.ch == '▒'));
        // Every step is at least as light as the one before.
        let l: Vec<f32> = cells.iter().map(|&c| lightness(&out, c)).collect();
        assert!(l.windows(2).all(|w| w[1] >= w[0] - 0.01), "{l:?}");
    }

    #[test]
    fn two_colors_off_each_others_path_mix_directly() {
        // Blue to red: a 90s artist shades one into the other, no detour.
        let d = doc(DocKind::Classic, 5, 1);
        let out = run(&d, (0, 0), (4, 0), Options::default(), &[Color::Pal(1), Color::Pal(4)]);
        for c in row(&out, 0) {
            assert!([c.fg, c.bg].iter().all(|k| matches!(k, Color::Pal(0 | 1 | 4))), "{c:?}");
        }
    }

    #[test]
    fn without_ice_backgrounds_stay_dark() {
        let mut d = doc(DocKind::Classic, 40, 6);
        d.meta.ice = false;
        for style in Style::ALL {
            let opts = Options { style, ramp: Ramp::Rainbow, ..Options::default() };
            let out = run(&d, (0, 0), (39, 5), opts, &Ramp::Rainbow.stops(Color::WHITE, Color::BLACK));
            for y in 0..6 {
                for c in row(&out, y) {
                    assert!(matches!(c.bg, Color::Pal(0..=7)), "{style:?} {c:?}");
                }
            }
        }
    }

    #[test]
    fn radial_starts_at_the_center() {
        let d = doc(DocKind::Classic, 21, 11);
        let opts = Options { shape: Shape::Radial, ..Options::default() };
        let out = run(&d, (10, 5), (20, 5), opts, &[Color::WHITE, Color::BLACK]);
        assert_eq!(out.canvas.composite(10, 5), Cell::new('█', Color::WHITE, Color::BLACK));
        assert_eq!(out.canvas.composite(20, 5), Cell::BLANK);
        // Round in square units: two rows up is as far as four columns across.
        assert_eq!(out.canvas.composite(10, 3), out.canvas.composite(14, 5));
    }

    #[test]
    fn a_click_runs_top_to_bottom() {
        let d = doc(DocKind::Classic, 4, 9);
        let out = run(&d, (1, 4), (1, 4), Options::default(), &[Color::WHITE, Color::BLACK]);
        assert_eq!(out.canvas.composite(3, 0), Cell::new('█', Color::WHITE, Color::BLACK));
        assert_eq!(out.canvas.composite(0, 8), Cell::BLANK);
        assert_eq!(row(&out, 4)[0], row(&out, 4)[3]);
    }

    #[test]
    fn modern_smooth_is_truecolor_and_monotone() {
        let d = doc(DocKind::Modern, 30, 1);
        let opts = Options { style: Style::Smooth, ..Options::default() };
        let out = run(&d, (0, 0), (29, 0), opts, &[Color::Pal(1), Color::Pal(14)]);
        let cells = row(&out, 0);
        assert!(cells.iter().all(|c| c.ch == '█' && matches!(c.fg, Color::Rgb(..))));
        let l: Vec<f32> = cells.iter().map(|&c| lightness(&out, c)).collect();
        assert!(l.windows(2).all(|w| w[1] >= w[0] - 0.01), "{l:?}");
    }

    #[test]
    fn halves_split_the_cell_vertically() {
        let d = doc(DocKind::Modern, 1, 4);
        let opts = Options { style: Style::Halves, ..Options::default() };
        let out = run(&d, (0, 0), (0, 3), opts, &[Color::WHITE, Color::BLACK]);
        let c = out.canvas.composite(0, 1);
        assert_eq!(c.ch, '▀');
        assert_ne!(c.fg, c.bg);
        // Classic: two palette colors per cell, bright one on top.
        let d = doc(DocKind::Classic, 1, 8);
        let out = run(&d, (0, 0), (0, 7), opts, &[Color::WHITE, Color::BLACK]);
        assert!((0..8).any(|y| matches!(out.canvas.composite(0, y).ch, '▀' | '▄')));
    }

    #[test]
    fn dither_mixes_neighbouring_bands() {
        let d = doc(DocKind::Classic, 32, 4);
        let opts = Options { style: Style::Dither, ..Options::default() };
        let out = run(&d, (0, 0), (31, 0), opts, &[Color::Pal(1), Color::Pal(9)]);
        // Some column holds two different cells stacked.
        assert!((0..32).any(|x| out.canvas.composite(x, 0) != out.canvas.composite(x, 1)));
        let first = out.canvas.composite(0, 0);
        assert_eq!(first, Cell::new('█', Color::Pal(1), Color::BLACK));
    }

    #[test]
    fn reverse_and_strip() {
        let o = Options { ramp: Ramp::Fire, reverse: true, ..Options::default() };
        let s = o.stops(Color::WHITE, Color::BLACK);
        assert_eq!(s.first(), Some(&Color::Pal(15)));
        let meta = DocMeta::default();
        let cells = strip(&meta, 28, 1, &o, &s);
        assert_eq!(cells[0], Cell::new('█', Color::WHITE, Color::BLACK));
        assert_eq!(cells[27], Cell::BLANK);
        assert_eq!(sample(&meta, &s, 1.0), [0, 0, 0]);
    }
}
