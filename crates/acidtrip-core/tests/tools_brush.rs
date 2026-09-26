//! Brush engine: every preset on the same stroke, and what each parameter does.

mod tools_support;

use acidtrip_core::charsets;
use acidtrip_core::tools::brush::{BrushSpec, GlyphSet, presets};
use acidtrip_core::tools::pen::{self, PenStroke};
use acidtrip_core::{Document, TxBuilder};
use tools_support::*;

fn preset(name: &str) -> BrushSpec {
    presets().into_iter().find(|b| b.name == name).unwrap()
}

/// Draw like the UI: apply per mouse event, then finish (end taper) and
/// apply once more, all in one transaction.
fn paint(d: &mut Document, spec: &BrushSpec, pts: &[(f32, f32)]) {
    let cands = spec.glyphs.candidates(&charsets::builtin()[5].chars);
    let mut s = PenStroke::with_brush(spec);
    run(d, |b: &mut TxBuilder| {
        for &(x, y) in pts {
            let changed = s.add_point(x, y);
            pen::apply(b, &ctx('█'), &s, changed, &cands);
        }
        let changed = s.finish();
        pen::apply(b, &ctx('█'), &s, changed, &cands);
    });
}

/// A wave across a 40x8 canvas, sampled like mouse reports (every ~3px).
fn wave() -> Vec<(f32, f32)> {
    (0..=100)
        .map(|i| i as f32 / 100.0)
        .map(|t| (12.0 + t * 296.0, 64.0 + (t * std::f32::consts::TAU).sin() * 36.0))
        .collect()
}

fn line(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)> {
    let n = ((b.0 - a.0).hypot(b.1 - a.1) / 3.0).ceil().max(1.0) as usize;
    (0..=n).map(|i| i as f32 / n as f32).map(|t| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)).collect()
}

fn inked(t: &str) -> usize {
    t.chars().filter(|c| !c.is_whitespace()).count()
}

fn column_ink(d: &Document, x: usize) -> usize {
    text(d).lines().filter(|l| l.chars().nth(x).is_some_and(|c| c != ' ')).count()
}

#[test]
fn every_preset_on_the_same_wave() {
    let mut all = String::new();
    for spec in presets() {
        let mut d = doc(40, 8);
        paint(&mut d, &spec, &wave());
        let t = text(&d);
        assert!(inked(&t) > 10, "{} drew almost nothing:\n{t}", spec.name);
        all.push_str(&format!("── {}\n{}\n", spec.name, t.lines().map(str::trim_end).collect::<Vec<_>>().join("\n")));
    }
    insta::assert_snapshot!(all);
}

#[test]
fn glyph_sets_limit_the_glyphs() {
    let only = |name: &str, allowed: &str| {
        let mut d = doc(40, 8);
        paint(&mut d, &preset(name), &wave());
        let t = text(&d);
        for c in t.chars().filter(|c| !c.is_whitespace()) {
            assert!(allowed.contains(c), "{name} used {c:?}:\n{t}");
        }
    };
    only("Airbrush", "░▒▓█");
    only("Soft shade", "░▒▓█");
    only("Spray", "·∙•°○■");
    only("Line art", "─│┌┐└┘├┤┬┴┼");
    only("ASCII", ".,'`:;-_~=+*#%@/\\|()<>^oO8");
}

#[test]
fn soft_edges_fade_into_shades() {
    let mut d = doc(20, 8);
    paint(&mut d, &preset("Airbrush"), &line((12.0, 64.0), (150.0, 64.0)));
    let t = text(&d);
    // Dense core, lighter rows above and below.
    let col: Vec<char> = t.lines().map(|l| l.chars().nth(10).unwrap()).collect();
    let density = |c: char| " ░▒▓█".find(c).map(|i| i / 3).unwrap_or(0);
    let core = density(col[4]);
    assert!(core >= density(col[2]) && core > density(col[1]), "{t}");
    assert!(t.contains('░') || t.contains('▒'), "no soft edge:\n{t}");
}

#[test]
fn opacity_below_full_never_reaches_solid() {
    let mut d = doc(20, 8);
    let mut spec = preset("Soft shade");
    spec.opacity = 0.5;
    // Scribble over the same spot: opacity caps the stroke, however dense.
    let mut pts = line((20.0, 40.0), (140.0, 80.0));
    pts.extend(line((140.0, 80.0), (20.0, 40.0)));
    paint(&mut d, &spec, &pts);
    let t = text(&d);
    assert!(!t.contains('█') && !t.contains('▓'), "{t}");
    assert!(t.contains('▒') || t.contains('░'), "{t}");
}

#[test]
fn taper_thins_both_ends() {
    let mut d = doc(40, 8);
    let spec = BrushSpec { taper: 90.0, velocity: 0.0, size: 7.0, ..preset("Brush pen") };
    paint(&mut d, &spec, &line((8.0, 64.0), (312.0, 64.0)));
    let (start, mid, end) = (column_ink(&d, 1), column_ink(&d, 20), column_ink(&d, 38));
    assert!(mid > start && mid > end, "start {start} mid {mid} end {end}:\n{}", text(&d));
}

#[test]
fn end_taper_restores_cells_the_stroke_no_longer_covers() {
    let mut d = doc(40, 8);
    // Put something under the end of the stroke; the redraw must bring it back.
    run(&mut d, |b| b.set(0, 38, 2, Some(cell('x'))));
    let spec = BrushSpec { taper: 120.0, velocity: 0.0, size: 14.0, glyphs: GlyphSet::Blocks, ..preset("Brush pen") };
    paint(&mut d, &spec, &line((8.0, 56.0), (312.0, 56.0)));
    assert_eq!(ch(&d, 38, 2), 'x', "\n{}", text(&d));
}

#[test]
fn flat_nib_is_thick_one_way_and_thin_the_other() {
    let spec = preset("Calligraphy"); // 45°
    let mut down = doc(40, 12);
    paint(&mut down, &spec, &line((40.0, 20.0), (200.0, 180.0))); // along the nib: thin
    let mut up = doc(40, 12);
    paint(&mut up, &spec, &line((40.0, 180.0), (200.0, 20.0))); // across the nib: thick
    assert!(inked(&text(&up)) > inked(&text(&down)) + 4, "\n{}\n--\n{}", text(&up), text(&down));
}

#[test]
fn grain_breaks_ink_up() {
    let solid = BrushSpec { grain: 0.0, ..preset("Chalk") };
    let chalk = preset("Chalk");
    let tone = |spec: &BrushSpec| {
        let mut d = doc(40, 8);
        paint(&mut d, spec, &line((8.0, 64.0), (312.0, 64.0)));
        text(&d).chars().filter(|c| "░▒▓".contains(*c)).count()
    };
    assert!(tone(&chalk) > tone(&solid) + 5);
}

#[test]
fn scatter_is_deterministic() {
    let draw = || {
        let mut d = doc(40, 8);
        paint(&mut d, &preset("Spray"), &wave());
        text(&d)
    };
    assert_eq!(draw(), draw());
}

#[test]
fn brush_strokes_undo_cleanly() {
    let d = doc(40, 8);
    for spec in presets() {
        let cands = spec.glyphs.candidates(&charsets::builtin()[5].chars);
        assert_undoable(&d, |b| {
            let mut s = PenStroke::with_brush(&spec);
            for (x, y) in wave() {
                let changed = s.add_point(x, y);
                pen::apply(b, &ctx('█'), &s, changed, &cands);
            }
            let changed = s.finish();
            pen::apply(b, &ctx('█'), &s, changed, &cands);
        });
    }
}
