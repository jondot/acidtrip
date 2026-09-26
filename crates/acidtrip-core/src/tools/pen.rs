//! Smart pen: strokes are drawn in *glyph-pixel space* and every touched
//! cell is rendered with the candidate glyph that best reproduces the
//! stroke there.
//!
//! Each cell is the 8x16 VGA glyph grid, so pixel (px, py) lies in cell
//! (px / 8, py / 16). A stroke is a polyline of round-capped thick segments.
//! Its coverage is antialiased (1px ramp at the edge) and kept per cell as
//! 128 floats in 0..=1.
//!
//! # Pixel fitting (blocks, shades, anything else)
//!
//! The cost of a candidate is a weighted sum of three terms:
//!
//! * **blurred** squared error (separable [1 2 2 2 1] x [1 2 1] kernels).
//!   The kernels exactly average the 4x2 dither period of `░ ▒ ▓`, so a
//!   shade matches its mean density and wins on softly covered cells;
//! * **raw** per-pixel squared error ([`RAW_WEIGHT`], border ring boosted by
//!   [`EDGE_WEIGHT`]), so shapes still decide: `▀` beats `▒` on a cell whose
//!   top half is inked;
//! * **side contact** ([`SIDE_WEIGHT`]): which of the four cell sides the
//!   stroke touches versus which the glyph touches, so lines stay connected
//!   from cell to cell. On top of that, when the stroke passes *through* a
//!   cell (touches two sides or more) glyphs that touch no side (`■ ·`) are
//!   not considered: they would turn a line into dashes. They still win for
//!   dots.
//!
//! Blank is always the baseline. A candidate is drawn only when the cell
//! has at least [`MIN_COVERAGE`] ink and its cost beats blank's by
//! [`MIN_GAIN`]; otherwise the cell is left alone. Candidate bitmaps and
//! their blurred versions are computed once per candidate set and cached
//! per thread.
//!
//! # Line drawing (box sets)
//!
//! Box-drawing glyphs are topology, not texture: a thin freehand stroke
//! rarely sits exactly on the glyph's rails, and pixel fitting then picks
//! `┬ ┴` stubs or corners facing the wrong way. So when the candidates can
//! draw both a horizontal and a vertical line (`─ │`, `═ ║`, ...), the pen
//! follows the stroke's centerline instead: it walks a 4-connected chain of
//! cells (with a little hysteresis at cell borders against hand jitter),
//! records which neighbors each cell connects to, and picks the candidate
//! whose arms match (`─ ┐ └ ┼` ...). The result is the staircase a skilled
//! artist draws by hand: `──┐` / `└──`.
//!
//! # Brushes
//!
//! A stroke made with a [`BrushSpec`] that isn't plain (soft, textured,
//! tapered, ...) is laid down as dabs of the brush tip every `spacing`
//! along the path instead of one continuous shape; each dab adds ink up to
//! the brush opacity at its flow. The fit above then turns that ink into
//! glyphs: a soft edge becomes `░ ▒ ▓`, a scattered spray becomes dots.
//! With a taper the stroke is redrawn on release so its end thins out too.
//!
//! # Merging
//!
//! The pen remembers each cell as it was before the stroke. Box lines add
//! the arms of line work already there (crossing a line makes `┼`), and
//! solid blocks of the brush color add their pixels (`▀` under a `▄`
//! stroke becomes `█`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::brush::BrushSpec;
use super::{Ctx, PaintMode, Symmetry, empty_cell};
use crate::mirror;
use crate::model::Cell;
use crate::render;
use crate::tx::TxBuilder;

/// Glyph cell width in pixels.
pub const GW: usize = 8;
/// Glyph cell height in pixels.
pub const GH: usize = 16;
const N: usize = GW * GH;

/// Suggested stroke radius (px) for block/shade sets: an 8px-thick line, the
/// height of one half block, so strokes render as `▀ ▄ ▌ ▐` staircases.
pub const RADIUS_BLOCKS: f32 = 3.5;
/// Suggested stroke radius (px) for box-drawing sets: thin, like `─ │`.
pub const RADIUS_BOX: f32 = 1.0;
/// Weight of the per-pixel term relative to the blurred term.
pub const RAW_WEIGHT: f32 = 0.2;
/// Extra per-pixel weight on the cell's border ring. A stroke that leaves
/// the cell through an edge must be continued by a glyph with ink on that
/// edge, so glyphs floating in the middle (`■ ·`) do not break lines into
/// dashes; they still win for isolated dots.
pub const EDGE_WEIGHT: f32 = 3.0;
/// Weight of the side-contact term: a glyph should touch the same cell sides
/// as the stroke, so lines stay connected across cells (`■` or `·` in the
/// middle of a line would read as dashes) and do not sprout dangling ends.
pub const SIDE_WEIGHT: f32 = 16.0;
/// Minimum ink in a cell (fraction of its 128 pixels) before anything is drawn.
pub const MIN_COVERAGE: f32 = 0.06;
/// A candidate must cost at most `(1 - MIN_GAIN)` times blank's cost.
pub const MIN_GAIN: f32 = 0.15;

/// Per-cell coverage (row-major, 8 columns x 16 rows).
pub type Coverage = [f32; N];

/// Pixel center of cell (x, y) in glyph-pixel space, for callers without
/// pixel-precise mouse reports.
pub fn cell_center(x: usize, y: usize) -> (f32, f32) {
    ((x * GW) as f32 + GW as f32 / 2.0, (y * GH) as f32 + GH as f32 / 2.0)
}

/// Arm bits: which neighbors a line-drawing cell connects to.
pub const ARM_UP: u8 = 1;
pub const ARM_DOWN: u8 = 2;
pub const ARM_LEFT: u8 = 4;
pub const ARM_RIGHT: u8 = 8;

/// How far (px) the pen must move past a cell's edge before the line-drawing
/// path steps into the neighbor. Keeps hand jitter along a cell border from
/// sprouting `┬ ┴` stubs.
const HYST_X: f32 = 1.5;
const HYST_Y: f32 = 3.0;

/// A stroke being drawn: accumulated antialiased coverage per cell, plus the
/// 4-connected chain of cells its centerline walks through (the "arms" each
/// cell connects to), used for line-drawing candidate sets.
#[derive(Clone, Debug, Default)]
pub struct PenStroke {
    coverage: HashMap<(usize, usize), Coverage>,
    arms: HashMap<(usize, usize), u8>,
    radius: f32,
    last: Option<(f32, f32)>,
    cur: Option<(i64, i64)>,
    /// Composite cells as they were before this stroke first wrote them, so
    /// the stroke can merge with existing line work however often `apply`
    /// runs (one builder per drag event or one for the whole stroke).
    base: RefCell<HashMap<(usize, usize), Cell>>,
    /// Layer cells as they were before this stroke first wrote them, so a
    /// cell the stroke no longer covers (after the end taper) is restored.
    written: RefCell<HashMap<(usize, usize), Option<Cell>>>,
    dabs: Option<Box<Dabs>>,
}

/// Dab state of a brush stroke.
#[derive(Clone, Debug)]
struct Dabs {
    spec: BrushSpec,
    /// Distance travelled along the stroke.
    dist: f32,
    /// Distance at which the next dab goes down.
    next: f32,
    /// Smoothed movement per event, a stand-in for pen speed.
    speed: f32,
    rng: u64,
    /// Points so far, to redraw the stroke with its end taper on release.
    points: Vec<(f32, f32)>,
    /// Total length once the stroke is finished (the end taper applies).
    total: Option<f32>,
}

impl Dabs {
    fn new(spec: BrushSpec, total: Option<f32>) -> Self {
        Dabs { spec, dist: 0.0, next: 0.0, speed: 0.0, rng: 0x9E37_79B9_7F4A_7C15, points: vec![], total }
    }

    /// Radius multiplier at distance `s` along the stroke.
    fn size_at(&self, s: f32) -> f32 {
        let sp = &self.spec;
        let mut k = 1.0;
        if sp.taper > 0.0 {
            let start = s / sp.taper;
            let end = self.total.map_or(1.0, |t| (t - s) / sp.taper);
            let t = start.min(end).clamp(0.0, 1.0);
            k *= 0.15 + 0.85 * t * (2.0 - t);
        }
        if sp.velocity > 0.0 {
            k *= 1.0 - sp.velocity * 0.75 * (self.speed / 24.0).min(1.0);
        }
        k
    }

    fn random(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A random offset inside the scatter disc.
    fn jitter(&mut self) -> (f32, f32) {
        let r = self.spec.scatter;
        if r <= 0.0 {
            return (0.0, 0.0);
        }
        loop {
            let (x, y) = (self.random() * 2.0 - 1.0, self.random() * 2.0 - 1.0);
            if x * x + y * y <= 1.0 {
                return (x * r, y * r);
            }
        }
    }
}

fn hash2(x: u64, y: u64) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ y.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Paper texture at a glyph pixel, 0..1: coarse blotches (bilinear, 6px)
/// over fine tooth (2px), fixed to the canvas like real paper.
fn paper(px: usize, py: usize) -> f32 {
    let (fx, fy) = (px as f32 / 6.0, py as f32 / 6.0);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let (x0, y0) = (x0 as u64, y0 as u64);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let coarse =
        lerp(lerp(hash2(x0, y0), hash2(x0 + 1, y0), tx), lerp(hash2(x0, y0 + 1), hash2(x0 + 1, y0 + 1), tx), ty);
    0.6 * coarse + 0.4 * hash2(px as u64 / 2 + 7919, py as u64 / 2)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl PenStroke {
    pub fn new(radius_px: f32) -> Self {
        PenStroke { radius: radius_px.max(0.0), ..Default::default() }
    }

    /// A stroke with a brush: plain brushes draw continuously (as
    /// [`PenStroke::new`] with the brush size), others in dabs.
    pub fn with_brush(spec: &BrushSpec) -> Self {
        let mut s = PenStroke::new(spec.size);
        if !spec.is_plain() {
            s.dabs = Some(Box::new(Dabs::new(spec.clone(), None)));
        }
        s
    }

    /// The stroke ended. With an end taper the whole stroke is redrawn so its
    /// end thins out; returns the cells to refit (none otherwise).
    pub fn finish(&mut self) -> Vec<(usize, usize)> {
        let Some(d) = self.dabs.as_mut() else { return vec![] };
        if d.spec.taper <= 0.0 || d.total.is_some() {
            return vec![];
        }
        let points = std::mem::take(&mut d.points);
        let (spec, total) = (d.spec.clone(), d.dist);
        let mut all: Vec<(usize, usize)> = self.cells().collect();
        self.coverage.clear();
        self.arms.clear();
        self.last = None;
        self.cur = None;
        self.dabs = Some(Box::new(Dabs::new(spec, Some(total))));
        for (x, y) in points {
            self.add_point(x, y);
        }
        all.extend(self.cells());
        all.sort_unstable_by_key(|&(x, y)| (y, x));
        all.dedup();
        all
    }

    /// Lay down dabs from the previous point to (x, y).
    fn add_dabs(&mut self, x: f32, y: f32) -> Vec<(usize, usize)> {
        let mut changed = Vec::new();
        self.walk(x, y, &mut changed);
        let Some(mut d) = self.dabs.take() else { return changed };
        d.points.push((x, y));
        let first = self.last.is_none();
        let (ax, ay) = self.last.unwrap_or((x, y));
        self.last = Some((x, y));
        let len = ((x - ax).powi(2) + (y - ay).powi(2)).sqrt();
        d.speed = if first { 0.0 } else { d.speed * 0.6 + len * 0.4 };
        let step = (d.spec.spacing * d.spec.size).max(0.5);
        let end = d.dist + len;
        while d.next <= end + 1e-4 {
            let t = if len > 0.0 { (d.next - d.dist) / len } else { 0.0 };
            let (px, py) = (ax + (x - ax) * t, ay + (y - ay) * t);
            let r = d.spec.size * d.size_at(d.next);
            for _ in 0..d.spec.count.max(1) {
                let (jx, jy) = d.jitter();
                self.stamp(&d.spec, px + jx, py + jy, r, &mut changed);
            }
            d.next += step;
        }
        d.dist = end;
        self.dabs = Some(d);
        changed.sort_unstable_by_key(|&(x, y)| (y, x));
        changed.dedup();
        changed
    }

    /// One dab of the brush tip centered at (cx, cy) with radius `r`.
    fn stamp(&mut self, sp: &BrushSpec, cx: f32, cy: f32, r: f32, changed: &mut Vec<(usize, usize)>) {
        let reach = r + 0.5;
        let feather = 1.0 + (1.0 - sp.hardness) * r;
        let ext = if sp.square { reach * std::f32::consts::SQRT_2 } else { reach };
        if cx + ext < 0.0 || cy + ext < 0.0 {
            return;
        }
        let (sin, cos) = sp.angle.to_radians().sin_cos();
        let round = sp.roundness.max(0.05);
        let cx0 = ((cx - ext).max(0.0) as usize) / GW;
        let cx1 = ((cx + ext).max(0.0) as usize) / GW;
        let cy0 = ((cy - ext).max(0.0) as usize) / GH;
        let cy1 = ((cy + ext).max(0.0) as usize) / GH;
        for ccy in cy0..=cy1 {
            for ccx in cx0..=cx1 {
                let mut cov = self.coverage.get(&(ccx, ccy)).copied().unwrap_or([0.0; N]);
                let mut hit = false;
                for py in 0..GH {
                    for px in 0..GW {
                        let (ax, ay) = (ccx * GW + px, ccy * GH + py);
                        let (dx, dy) = (ax as f32 + 0.5 - cx, ay as f32 + 0.5 - cy);
                        let u = dx * cos + dy * sin;
                        let v = (dy * cos - dx * sin) / round;
                        let dist = if sp.square { u.abs().max(v.abs()) } else { (u * u + v * v).sqrt() };
                        let mut c = ((reach - dist) / feather).clamp(0.0, 1.0);
                        if c <= 0.0 {
                            continue;
                        }
                        if sp.hardness < 1.0 {
                            c = c * c * (3.0 - 2.0 * c);
                        }
                        if sp.grain > 0.0 {
                            let g = sp.grain * 0.8;
                            c *= smoothstep(g - 0.12, g + 0.12, paper(ax, ay));
                        }
                        let cur = &mut cov[py * GW + px];
                        let nv = if sp.flow >= 1.0 {
                            cur.max(c * sp.opacity)
                        } else {
                            *cur + (sp.opacity - *cur).max(0.0) * c * sp.flow
                        };
                        if nv > *cur + 1e-4 {
                            *cur = nv;
                            hit = true;
                        }
                    }
                }
                if hit {
                    self.coverage.insert((ccx, ccy), cov);
                    changed.push((ccx, ccy));
                }
            }
        }
    }

    /// Arms of a cell on the stroke's centerline chain (see [`ARM_UP`] ...).
    pub fn arms(&self, x: usize, y: usize) -> Option<u8> {
        self.arms.get(&(x, y)).copied()
    }

    fn arm(&mut self, c: (i64, i64), bit: u8, changed: &mut Vec<(usize, usize)>) {
        if c.0 < 0 || c.1 < 0 {
            return;
        }
        let k = (c.0 as usize, c.1 as usize);
        let e = self.arms.entry(k).or_insert(0);
        if *e & bit != bit || bit == 0 {
            *e |= bit;
            changed.push(k);
        }
    }

    /// Walk the centerline chain to (x, y): leave the current cell only when
    /// the point is clearly outside it, then step cell by cell (4-connected)
    /// along the segment from the current cell's center.
    fn walk(&mut self, x: f32, y: f32, changed: &mut Vec<(usize, usize)>) {
        let cell_of = |x: f32, y: f32| ((x / GW as f32).floor() as i64, (y / GH as f32).floor() as i64);
        let Some(mut c) = self.cur else {
            let c = cell_of(x, y);
            self.cur = Some(c);
            self.arm(c, 0, changed);
            return;
        };
        let (l, t) = ((c.0 * GW as i64) as f32, (c.1 * GH as i64) as f32);
        if x >= l - HYST_X && x < l + GW as f32 + HYST_X && y >= t - HYST_Y && y < t + GH as f32 + HYST_Y {
            return;
        }
        let target = cell_of(x, y);
        // Grid traversal in cell units from the current cell's center.
        let (sx, sy) = (c.0 as f32 + 0.5, c.1 as f32 + 0.5);
        let (ex, ey) = (x / GW as f32, y / GH as f32);
        let (dx, dy) = (ex - sx, ey - sy);
        let (stx, sty) = (dx.signum() as i64, dy.signum() as i64);
        let t_delta = |d: f32| if d == 0.0 { f32::INFINITY } else { 1.0 / d.abs() };
        let (tdx, tdy) = (t_delta(dx), t_delta(dy));
        let (mut tmx, mut tmy) = (tdx * 0.5, tdy * 0.5);
        let steps = (target.0 - c.0).abs() + (target.1 - c.1).abs();
        for _ in 0..steps {
            let (next, out, back) = if tmx < tmy || (tmx == tmy && dx.abs() >= dy.abs()) {
                tmx += tdx;
                let (o, b) = if stx > 0 { (ARM_RIGHT, ARM_LEFT) } else { (ARM_LEFT, ARM_RIGHT) };
                ((c.0 + stx, c.1), o, b)
            } else {
                tmy += tdy;
                let (o, b) = if sty > 0 { (ARM_DOWN, ARM_UP) } else { (ARM_UP, ARM_DOWN) };
                ((c.0, c.1 + sty), o, b)
            };
            self.arm(c, out, changed);
            self.arm(next, back, changed);
            c = next;
        }
        self.cur = Some(c);
    }

    pub fn radius(&self) -> f32 {
        self.radius
    }

    /// Add a point (glyph-pixel space) joined to the previous one by a
    /// round-capped segment. Returns the cells whose coverage changed, sorted.
    pub fn add_point(&mut self, x: f32, y: f32) -> Vec<(usize, usize)> {
        if self.dabs.is_some() {
            return self.add_dabs(x, y);
        }
        let (ax, ay) = self.last.unwrap_or((x, y));
        self.last = Some((x, y));
        let mut changed = Vec::new();
        self.walk(x, y, &mut changed);
        let reach = self.radius + 0.5;
        let (minx, maxx) = (ax.min(x) - reach, ax.max(x) + reach);
        let (miny, maxy) = (ay.min(y) - reach, ay.max(y) + reach);
        if maxx < 0.0 || maxy < 0.0 {
            return changed;
        }
        let cx0 = (minx.max(0.0) as usize) / GW;
        let cx1 = (maxx as usize) / GW;
        let cy0 = (miny.max(0.0) as usize) / GH;
        let cy1 = (maxy as usize) / GH;
        // Half-diagonal of a cell: a cell can only be touched when its center
        // is within reach + this of the segment.
        let half_diag = ((GW * GW + GH * GH) as f32).sqrt() / 2.0;
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let (ccx, ccy) = cell_center(cx, cy);
                if seg_dist(ccx, ccy, ax, ay, x, y) > reach + half_diag {
                    continue;
                }
                let mut cov = self.coverage.get(&(cx, cy)).copied().unwrap_or([0.0; N]);
                let mut hit = false;
                for py in 0..GH {
                    for px in 0..GW {
                        let (fx, fy) = ((cx * GW + px) as f32 + 0.5, (cy * GH + py) as f32 + 0.5);
                        let c = (reach - seg_dist(fx, fy, ax, ay, x, y)).clamp(0.0, 1.0);
                        let v = &mut cov[py * GW + px];
                        if c > *v {
                            *v = c;
                            hit = true;
                        }
                    }
                }
                if hit {
                    self.coverage.insert((cx, cy), cov);
                    changed.push((cx, cy));
                }
            }
        }
        changed.sort_unstable_by_key(|&(x, y)| (y, x));
        changed.dedup();
        changed
    }

    /// Every cell the stroke touches (inked or on its centerline chain).
    pub fn cells(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.coverage.keys().copied().chain(self.arms.keys().copied().filter(|k| !self.coverage.contains_key(k)))
    }

    /// Coverage of one cell, if the stroke touches it.
    pub fn coverage(&self, x: usize, y: usize) -> Option<&Coverage> {
        self.coverage.get(&(x, y))
    }
}

/// Distance from (px, py) to the segment (ax, ay)-(bx, by).
fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qy) = (ax + t * dx - px, ay + t * dy - py);
    (qx * qx + qy * qy).sqrt()
}

// ------------------------------------------------------------------ fitting

const KX: [f32; 5] = [1.0, 2.0, 2.0, 2.0, 1.0];
const KY: [f32; 3] = [1.0, 2.0, 1.0];

/// Separable blur inside one cell; the kernel is renormalized at the edges.
fn blur(src: &Coverage) -> Coverage {
    let mut tmp = [0.0; N];
    for y in 0..GH {
        for x in 0..GW {
            let (mut s, mut w) = (0.0, 0.0);
            for (k, kw) in KX.iter().enumerate() {
                let sx = x as isize + k as isize - 2;
                if (0..GW as isize).contains(&sx) {
                    s += kw * src[y * GW + sx as usize];
                    w += kw;
                }
            }
            tmp[y * GW + x] = s / w;
        }
    }
    let mut out = [0.0; N];
    for y in 0..GH {
        for x in 0..GW {
            let (mut s, mut w) = (0.0, 0.0);
            for (k, kw) in KY.iter().enumerate() {
                let sy = y as isize + k as isize - 1;
                if (0..GH as isize).contains(&sy) {
                    s += kw * tmp[sy as usize * GW + x];
                    w += kw;
                }
            }
            out[y * GW + x] = s / w;
        }
    }
    out
}

/// Per-pixel weights of the raw term: 1 inside, `1 + EDGE_WEIGHT` on the border.
fn weights() -> &'static Coverage {
    static W: std::sync::OnceLock<Coverage> = std::sync::OnceLock::new();
    W.get_or_init(|| {
        let mut w = [1.0; N];
        for y in 0..GH {
            for x in 0..GW {
                if x == 0 || y == 0 || x == GW - 1 || y == GH - 1 {
                    w[y * GW + x] += EDGE_WEIGHT;
                }
            }
        }
        w
    })
}

struct Feat {
    ch: char,
    raw: Coverage,
    blur: Coverage,
    sides: [f32; 4],
    /// Touches no side at all (raw bitmap): `■ ·`.
    floating: bool,
}

/// How much a bitmap touches each cell side (top, bottom, left, right): ink
/// on the outermost pixel line plus `inner` times the ink on the line inside
/// it, in pixels, saturating at 2. Strokes use `inner = 0.5`, so one ending
/// a pixel short of the border still touches it; glyphs use 0.
fn sides(c: &Coverage, inner: f32) -> [f32; 4] {
    let row = |y: usize| (0..GW).map(|x| c[y * GW + x]).sum::<f32>();
    let col = |x: usize| (0..GH).map(|y| c[y * GW + x]).sum::<f32>();
    let band = |o: f32, i: f32| ((o + inner * i) / 2.0).min(1.0);
    [band(row(0), row(1)), band(row(GH - 1), row(GH - 2)), band(col(0), col(1)), band(col(GW - 1), col(GW - 2))]
}

/// True when a bitmap has no ink on any border pixel.
fn floating(c: &Coverage) -> bool {
    (0..GH).all(|y| (0..GW).all(|x| (x > 0 && y > 0 && x < GW - 1 && y < GH - 1) || c[y * GW + x] == 0.0))
}

fn feat(ch: char) -> Feat {
    let rows = render::glyph_rows(ch);
    let mut raw = [0.0; N];
    for (y, bits) in rows.iter().enumerate() {
        for x in 0..GW {
            if bits & (0x80 >> x) != 0 {
                raw[y * GW + x] = 1.0;
            }
        }
    }
    Feat { ch, blur: blur(&raw), sides: sides(&raw, 0.0), floating: floating(&raw), raw }
}

/// Candidate glyphs with their precomputed bitmaps.
struct Fitter {
    feats: Vec<Feat>,
    /// Box-drawing candidates with their arms; non-empty only when the set
    /// can draw both horizontal and vertical lines (line-drawing mode).
    lines: Vec<(char, u8)>,
    /// Some candidate reaches the cell border (else dots may draw lines).
    edge_glyphs: bool,
}

/// Arms a glyph bitmap has: ink at the middle of each side.
fn glyph_arms(raw: &Coverage) -> u8 {
    let ink = |x: usize, y: usize| raw[y * GW + x] > 0.0;
    let mut a = 0;
    if (2..6).any(|x| ink(x, 0)) {
        a |= ARM_UP;
    }
    if (2..6).any(|x| ink(x, GH - 1)) {
        a |= ARM_DOWN;
    }
    if (5..10).any(|y| ink(0, y)) {
        a |= ARM_LEFT;
    }
    if (5..10).any(|y| ink(GW - 1, y)) {
        a |= ARM_RIGHT;
    }
    a
}

impl Fitter {
    fn new(candidates: &[char]) -> Self {
        let mut seen = HashSet::new();
        let feats: Vec<Feat> = candidates
            .iter()
            .copied()
            .filter(|&c| super::has_ink(c) && seen.insert(c))
            .map(feat)
            .filter(|f| f.raw.iter().any(|&v| v > 0.0))
            .collect();
        let lines: Vec<(char, u8)> =
            feats.iter().filter(|f| is_box(f.ch)).map(|f| (f.ch, glyph_arms(&f.raw))).collect();
        let has = |m: u8| lines.iter().any(|&(_, a)| a == m);
        let lines = if has(ARM_LEFT | ARM_RIGHT) && has(ARM_UP | ARM_DOWN) { lines } else { vec![] };
        let edge_glyphs = feats.iter().any(|f| !f.floating);
        Fitter { feats, lines, edge_glyphs }
    }

    /// Sets of floating glyphs only (dots) can't follow a shape, so they
    /// render tone: the biggest glyph with less ink than the cell (dots read
    /// darker than their pixel count), else the smallest.
    fn fit_tone(&self, ink: f32) -> Option<char> {
        let ink_of = |f: &Feat| f.raw.iter().sum::<f32>() / N as f32;
        let fits = self.feats.iter().filter(|f| ink_of(f) <= ink * 0.6);
        fits.max_by(|a, b| ink_of(a).total_cmp(&ink_of(b)))
            .or_else(|| self.feats.iter().min_by(|a, b| ink_of(a).total_cmp(&ink_of(b))))
            .map(|f| f.ch)
    }

    /// Line-drawing glyph for a cell whose chain arms are `arms`: the
    /// candidate with the closest arm set (fewest arms on ties). A path end
    /// (one arm) continues straight through the cell.
    fn fit_arms(&self, arms: u8) -> Option<char> {
        let want = match arms {
            ARM_UP | ARM_DOWN => ARM_UP | ARM_DOWN,
            ARM_LEFT | ARM_RIGHT => ARM_LEFT | ARM_RIGHT,
            a => a,
        };
        self.lines.iter().min_by_key(|&&(_, a)| ((a ^ want).count_ones(), a.count_ones())).map(|&(c, _)| c)
    }

    /// Glyph for one cell of `stroke`. `under` is the cell before the stroke
    /// and `merge` whether its ink joins the stroke: box lines add their arms
    /// (a crossing becomes `┼`), solid blocks of the brush color add their
    /// pixels (`▀` under a `▄` stroke becomes `█`).
    fn fit_cell(&self, stroke: &PenStroke, x: usize, y: usize, under: Cell, merge: bool) -> Option<char> {
        let under = merge.then_some(under.ch);
        if !self.lines.is_empty() {
            let extra = under.filter(|&c| is_box(c)).map_or(0, |c| glyph_arms(&feat(c).raw));
            // Line-drawing sets follow the chain; a lone tap falls back to pixels.
            return match stroke.arms(x, y) {
                Some(0) => stroke.coverage(x, y).and_then(|c| self.fit(c)),
                Some(a) => self.fit_arms(a | extra),
                None => None,
            };
        }
        let cov = stroke.coverage(x, y)?;
        match under.filter(|c| matches!(c, '█' | '▀' | '▄' | '▌' | '▐')) {
            Some(c) => {
                let g = feat(c).raw;
                let mut m = *cov;
                m.iter_mut().zip(g).for_each(|(v, g)| *v = v.max(g));
                self.fit(&m)
            }
            None => self.fit(cov),
        }
    }

    fn fit(&self, cov: &Coverage) -> Option<char> {
        let total: f32 = cov.iter().sum();
        if total < MIN_COVERAGE * N as f32 {
            return None;
        }
        if !self.edge_glyphs {
            return self.fit_tone(total / N as f32);
        }
        let bc = blur(cov);
        let wt = weights();
        let sc = sides(cov, 0.5);
        let sp = SIDE_WEIGHT;
        let blank = sp * sc.iter().map(|v| v * v).sum::<f32>()
            + RAW_WEIGHT * (0..N).map(|i| wt[i] * cov[i] * cov[i]).sum::<f32>()
            + bc.iter().map(|v| v * v).sum::<f32>();
        let mut best: Option<(f32, char)> = None;
        // A stroke passing through the cell (touching two or more sides) must
        // be drawn with a glyph that reaches the border, or the line breaks.
        let through = self.edge_glyphs && sides(cov, 0.0).iter().filter(|&&v| v >= 0.5).count() >= 2;
        for f in &self.feats {
            if through && f.floating {
                continue;
            }
            let mut raw = 0.0;
            let mut bl = 0.0;
            for i in 0..N {
                let d = cov[i] - f.raw[i];
                raw += wt[i] * d * d;
                let e = bc[i] - f.blur[i];
                bl += e * e;
            }
            let side: f32 = (0..4).map(|k| (sc[k] - f.sides[k]).powi(2)).sum();
            let cost = RAW_WEIGHT * raw + bl + sp * side;
            if best.is_none_or(|(b, _)| cost < b) {
                best = Some((cost, f.ch));
            }
        }
        let (cost, ch) = best?;
        (cost <= blank * (1.0 - MIN_GAIN)).then_some(ch)
    }
}

thread_local! {
    static FITTERS: RefCell<HashMap<Vec<char>, Rc<Fitter>>> = RefCell::new(HashMap::new());
}

fn fitter(candidates: &[char]) -> Rc<Fitter> {
    FITTERS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(f) = m.get(candidates) {
            return f.clone();
        }
        // Candidate sets are few (one per charset); keep the cache bounded anyway.
        if m.len() > 64 {
            m.clear();
        }
        let f = Rc::new(Fitter::new(candidates));
        m.insert(candidates.to_vec(), f.clone());
        f
    })
}

/// The candidate glyph that best reproduces `coverage`, or `None` when the
/// cell should stay as it is (too little ink, or nothing beats blank).
pub fn fit_glyph(coverage: &Coverage, candidates: &[char]) -> Option<char> {
    fitter(candidates).fit(coverage)
}

fn is_block(c: char) -> bool {
    matches!(c, '\u{2580}'..='\u{259F}' | '■')
}

fn is_box(c: char) -> bool {
    matches!(c, '\u{2500}'..='\u{257F}')
}

/// Blocks and shades, the fallback palette for sets that cannot draw lines.
pub const BLOCKS: [char; 8] = ['░', '▒', '▓', '█', '▀', '▄', '▌', '▐'];

/// Pen candidates for an active character set: its glyphs plus `█` and blank.
/// Sets that contain neither block elements nor box-drawing glyphs (letters,
/// symbols, arrows, ...) cannot draw a stroke on their own, so the blocks
/// and shades are added in front; the set's glyphs stay available so the
/// fitter can still pick one where its shape matches the stroke.
pub fn default_candidates(active_set: &[char]) -> Vec<char> {
    let mut out = Vec::new();
    if !active_set.iter().any(|&c| is_block(c) || is_box(c)) {
        out.extend(BLOCKS);
    }
    out.extend(active_set.iter().copied());
    out.extend(['█', ' ']);
    let mut seen = HashSet::new();
    out.retain(|&c| seen.insert(c));
    out
}

// ------------------------------------------------------------------- apply

/// Refit `cells` of `stroke` and write them on `ctx.layer`: the fitted glyph
/// in the brush fg over the cell's existing composite bg, so strokes over a
/// colored background keep it. Cells that fit nothing are left alone. In
/// Erase mode every cell with a fitted glyph is erased instead. Symmetry
/// mirrors positions and glyphs (`mirror_h` / `mirror_v`).
pub fn apply(
    b: &mut TxBuilder,
    ctx: &Ctx,
    stroke: &PenStroke,
    cells: impl IntoIterator<Item = (usize, usize)>,
    candidates: &[char],
) {
    let f = fitter(candidates);
    let (w, h) = (b.width(), b.height());
    let mut out: Vec<(usize, usize, char)> = Vec::new();
    let mut gone: Vec<(usize, usize)> = Vec::new();
    let mut seen = HashSet::new();
    for (x, y) in cells {
        if x >= w || y >= h || !seen.insert((x, y)) {
            continue;
        }
        let under = *stroke.base.borrow_mut().entry((x, y)).or_insert_with(|| b.composite(x, y));
        // Line work merges whatever its color; solid blocks only with their own color.
        let merge = ctx.mode != PaintMode::Erase && (!f.lines.is_empty() || under.fg == ctx.brush.fg);
        match f.fit_cell(stroke, x, y, under, merge) {
            Some(ch) => out.push((x, y, ch)),
            None => gone.push((x, y)),
        }
    }
    let own: HashSet<(usize, usize)> = out.iter().map(|&(x, y, _)| (x, y)).collect();
    let mirror_of = |x: usize, y: usize| -> Vec<(usize, usize)> {
        let (mx, my) = (w - 1 - x, h - 1 - y);
        match ctx.symmetry {
            Symmetry::None => vec![],
            Symmetry::X => vec![(mx, y)],
            Symmetry::Y => vec![(x, my)],
            Symmetry::Both => vec![(mx, y), (x, my), (mx, my)],
        }
    };
    let gone: Vec<(usize, usize)> =
        gone.iter().flat_map(|&(x, y)| std::iter::once((x, y)).chain(mirror_of(x, y))).collect();
    let mut mirrored = Vec::new();
    for &(x, y, ch) in &out {
        let (mx, my) = (w - 1 - x, h - 1 - y);
        match ctx.symmetry {
            Symmetry::None => {}
            Symmetry::X => mirrored.push((mx, y, mirror::mirror_h(ch))),
            Symmetry::Y => mirrored.push((x, my, mirror::mirror_v(ch))),
            Symmetry::Both => mirrored.extend([
                (mx, y, mirror::mirror_h(ch)),
                (x, my, mirror::mirror_v(ch)),
                (mx, my, mirror::mirror_v(mirror::mirror_h(ch))),
            ]),
        }
    }
    // The stroke's own cells win over mirrored ones.
    let mut done = HashSet::new();
    for (x, y, ch) in out.into_iter().chain(mirrored.into_iter().filter(|&(x, y, _)| !own.contains(&(x, y)))) {
        if !done.insert((x, y)) {
            continue;
        }
        let cell = match ctx.mode {
            PaintMode::Erase => empty_cell(ctx.layer),
            _ => Some(Cell::new(ch, ctx.brush.fg, b.composite(x, y).bg)),
        };
        stroke.written.borrow_mut().entry((x, y)).or_insert_with(|| b.get(ctx.layer, x, y));
        b.set(ctx.layer, x, y, cell);
    }
    // Cells this stroke drew earlier but no longer covers get their old look back.
    for (x, y) in gone {
        if done.contains(&(x, y)) {
            continue;
        }
        if let Some(orig) = stroke.written.borrow_mut().remove(&(x, y)) {
            b.set(ctx.layer, x, y, orig);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cov(f: impl Fn(usize, usize) -> f32) -> Coverage {
        let mut c = [0.0; N];
        for y in 0..GH {
            for x in 0..GW {
                c[y * GW + x] = f(x, y);
            }
        }
        c
    }

    fn blocks() -> Vec<char> {
        default_candidates(&['░', '▒', '▓', '█', '▀', '▄', '▌', '▐', '■', '·'])
    }

    #[test]
    fn synthetic_shapes() {
        let c = blocks();
        assert_eq!(fit_glyph(&cov(|_, y| (y < 7) as u8 as f32), &c), Some('▀'));
        assert_eq!(fit_glyph(&cov(|_, y| (y >= 8) as u8 as f32), &c), Some('▄'));
        assert_eq!(fit_glyph(&cov(|x, _| (x < 4) as u8 as f32), &c), Some('▌'));
        assert_eq!(fit_glyph(&cov(|x, _| (x >= 4) as u8 as f32), &c), Some('▐'));
        assert_eq!(fit_glyph(&cov(|_, _| 1.0), &c), Some('█'));
        assert_eq!(fit_glyph(&cov(|_, _| 0.0), &c), None);
        assert_eq!(fit_glyph(&cov(|_, _| 0.03), &c), None);
    }

    #[test]
    fn uniform_density_picks_matching_shade() {
        let c = blocks();
        assert_eq!(fit_glyph(&cov(|_, _| 0.18), &c), Some('░'));
        assert_eq!(fit_glyph(&cov(|_, _| 0.5), &c), Some('▒'));
        assert_eq!(fit_glyph(&cov(|_, _| 0.75), &c), Some('▓'));
        assert_eq!(fit_glyph(&cov(|_, _| 0.95), &c), Some('█'));
    }

    #[test]
    fn small_blobs() {
        let c = blocks();
        let sq = cov(|x, y| ((1..7).contains(&x) && (4..11).contains(&y)) as u8 as f32);
        assert_eq!(fit_glyph(&sq, &c), Some('■'));
    }

    #[test]
    fn box_candidates() {
        let c = default_candidates(&['┌', '┐', '└', '┘', '─', '│', '├', '┤', '┴', '┬']);
        assert_eq!(fit_glyph(&cov(|_, y| (7..9).contains(&y) as u8 as f32), &c), Some('─'));
        assert_eq!(fit_glyph(&cov(|x, _| (3..5).contains(&x) as u8 as f32), &c), Some('│'));
        let corner = cov(|x, y| ((y == 7 && x >= 3) || ((3..5).contains(&x) && y >= 7)) as u8 as f32);
        assert_eq!(fit_glyph(&corner, &c), Some('┌'));
    }

    #[test]
    fn default_candidates_fallback() {
        let greek = default_candidates(&['α', 'ß']);
        assert_eq!(&greek[..8], &BLOCKS);
        assert!(greek.contains(&'α') && greek.contains(&' '));
        let bx = default_candidates(&['─', '│', '█']);
        assert_eq!(bx, vec!['─', '│', '█', ' ']);
    }

    #[test]
    fn stroke_reports_changed_cells() {
        let mut s = PenStroke::new(2.0);
        let first = s.add_point(4.0, 8.0);
        assert_eq!(first, vec![(0, 0)]);
        let more = s.add_point(20.0, 8.0);
        assert!(more.contains(&(1, 0)) && more.contains(&(2, 0)));
        // Retracing adds nothing new.
        assert!(s.add_point(4.0, 8.0).is_empty());
        assert_eq!(s.cells().count(), 3);
        // Off-canvas points are ignored.
        assert!(s.add_point(-50.0, -50.0).is_empty() || s.cells().all(|(x, y)| x < 10 && y < 10));
    }
}
