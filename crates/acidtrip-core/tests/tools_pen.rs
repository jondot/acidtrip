//! Smart pen: glyph fitting on real strokes, apply semantics, undo.

mod tools_support;

use acidtrip_core::charsets;
use acidtrip_core::tools::pen::{self, PenStroke};
use acidtrip_core::tools::{self, BoxStyle, Ctx, PaintMode, Symmetry};
use acidtrip_core::{Cell, Color, Document, TxBuilder};
use tools_support::*;

fn blocks() -> Vec<char> {
    pen::default_candidates(&charsets::builtin()[5].chars)
}

fn boxes() -> Vec<char> {
    pen::default_candidates(&charsets::builtin()[0].chars)
}

/// Draw a polyline (glyph-pixel space) with the pen, applying per point as
/// the UI does while dragging, and commit it as one transaction.
fn stroke(d: &mut Document, c: &Ctx, radius: f32, pts: &[(f32, f32)], cands: &[char]) -> acidtrip_core::Transaction {
    let mut s = PenStroke::new(radius);
    run(d, |b| {
        for &(x, y) in pts {
            let changed = s.add_point(x, y);
            pen::apply(b, c, &s, changed, cands);
        }
    })
}

/// Densely sampled segment, like mouse reports.
fn seg(a: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)> {
    let n = ((b.0 - a.0).hypot(b.1 - a.1) / 2.0).ceil().max(1.0) as usize;
    (0..=n).map(|i| i as f32 / n as f32).map(|t| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)).collect()
}

fn row(d: &Document, y: usize) -> String {
    text(d).lines().nth(y).unwrap().to_string()
}

#[test]
fn horizontal_line_in_upper_half_is_upper_blocks() {
    let mut d = doc(12, 4);
    stroke(&mut d, &ctx('█'), pen::RADIUS_BLOCKS, &seg((4.0, 36.0), (84.0, 36.0)), &blocks());
    assert_eq!(row(&d, 2).trim_end(), "▀▀▀▀▀▀▀▀▀▀▀");
    assert_eq!(text(&d).chars().filter(|c| !c.is_whitespace()).count(), 11);
}

#[test]
fn vertical_line_at_left_edge_is_left_blocks() {
    let mut d = doc(6, 8);
    stroke(&mut d, &ctx('█'), 2.0, &seg((26.0, 8.0), (26.0, 120.0)), &blocks());
    for y in 0..8 {
        assert_eq!(ch(&d, 3, y), '▌', "row {y}:\n{}", text(&d));
    }
    assert_eq!(text(&d).chars().filter(|c| !c.is_whitespace()).count(), 8);
}

#[test]
fn diagonal_thick_stroke_is_connected_blocks() {
    let mut d = doc(24, 12);
    stroke(&mut d, &ctx('█'), 5.0, &seg((8.0, 8.0), (180.0, 180.0)), &blocks());
    let t = text(&d);
    let used: std::collections::BTreeSet<char> = t.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(used.iter().any(|c| "▀▄▌▐█".contains(*c)), "{t}");
    // Connected: every row between the ends has ink.
    for (y, line) in t.lines().enumerate().take(11) {
        assert!(line.trim() != "", "gap at row {y}:\n{t}");
    }
    insta::assert_snapshot!(t);
}

#[test]
fn shallow_line_has_no_dashes() {
    let mut d = doc(34, 4);
    stroke(&mut d, &ctx('█'), pen::RADIUS_BLOCKS, &seg((4.0, 14.0), (260.0, 50.0)), &blocks());
    let t = text(&d);
    assert!(!t.contains('■') && !t.contains('·'), "{t}");
    insta::assert_snapshot!(t);
}

#[test]
fn a_tap_is_a_dot() {
    let mut d = doc(3, 3);
    stroke(&mut d, &ctx('█'), 3.0, &[(12.0, 24.0)], &blocks());
    assert_eq!(ch(&d, 1, 1), '■');
}

#[test]
fn box_set_thin_line_through_centers() {
    let mut d = doc(10, 3);
    let pts: Vec<_> = (1..9).map(|x| pen::cell_center(x, 1)).collect();
    stroke(&mut d, &ctx('─'), pen::RADIUS_BOX, &pts, &boxes());
    assert_eq!(row(&d, 1), " ──────── ");
    let mut d = doc(3, 6);
    let pts: Vec<_> = (0..6).map(|y| pen::cell_center(1, y)).collect();
    stroke(&mut d, &ctx('─'), pen::RADIUS_BOX, &pts, &boxes());
    assert_eq!(text(&d), " │ \n │ \n │ \n │ \n │ \n │ ");
}

#[test]
fn box_set_diagonal_is_a_connected_staircase() {
    let mut d = doc(16, 6);
    stroke(&mut d, &ctx('─'), pen::RADIUS_BOX, &seg((4.0, 8.0), (124.0, 88.0)), &boxes());
    insta::assert_snapshot!(text(&d));
}

#[test]
fn box_set_jitter_along_a_border_stays_straight() {
    let mut d = doc(12, 4);
    // Wobble ±1.5px around the border between rows 1 and 2.
    let pts: Vec<_> = (0..=45).map(|i| (4.0 + i as f32 * 2.0, 32.0 + if i % 2 == 0 { 1.5 } else { -1.0 })).collect();
    stroke(&mut d, &ctx('─'), pen::RADIUS_BOX, &pts, &boxes());
    let t = text(&d);
    assert!(!t.contains(['┬', '┴', '├', '┤', '┼']), "{t}");
}

#[test]
fn box_crossing_merges_into_existing_lines() {
    let mut d = doc(7, 5);
    let hz: Vec<_> = (0..7).map(|x| pen::cell_center(x, 2)).collect();
    let vt: Vec<_> = (0..5).map(|y| pen::cell_center(3, y)).collect();
    let cands = tools::box_candidates(BoxStyle::Single);
    stroke(&mut d, &ctx('─'), pen::RADIUS_BOX, &hz, &cands);
    stroke(&mut d, &ctx('─'), pen::RADIUS_BOX, &vt, &cands);
    assert_eq!(text(&d), "   │   \n   │   \n───┼───\n   │   \n   │   ");
}

#[test]
fn blocks_merge_with_same_color_ink() {
    let mut d = doc(3, 1);
    // Upper half first, then a stroke along the lower half: together a full block.
    stroke(&mut d, &ctx('█'), 3.0, &seg((4.0, 4.0), (20.0, 4.0)), &blocks());
    assert_eq!(text(&d), "▀▀▀");
    stroke(&mut d, &ctx('█'), 3.5, &seg((4.0, 12.0), (20.0, 12.0)), &blocks());
    assert_eq!(text(&d), "███");
}

#[test]
fn keeps_background_and_uses_brush_fg() {
    let mut d = doc(4, 1);
    draw(&mut d, 0, 0, &["    "]);
    run(&mut d, |b| b.set(0, 1, 0, Some(Cell::new(' ', WHITE, RED))));
    let c = Ctx { brush: acidtrip_core::tools::Brush { ch: '█', fg: YELLOW, bg: GREEN }, ..Ctx::default() };
    stroke(&mut d, &c, 3.0, &seg((4.0, 4.0), (28.0, 4.0)), &blocks());
    assert_eq!(d.canvas.get(0, 1, 0), Some(Cell::new('▀', YELLOW, RED)));
    assert_eq!(d.canvas.get(0, 2, 0), Some(Cell::new('▀', YELLOW, Color::BLACK)));
}

#[test]
fn erase_mode_clears_fitted_cells_only() {
    let mut d = doc(6, 2);
    draw(&mut d, 0, 0, &["xxxxxx", "xxxxxx"]);
    let c = Ctx { mode: PaintMode::Erase, ..ctx('█') };
    stroke(&mut d, &c, 3.0, &seg((12.0, 4.0), (36.0, 4.0)), &blocks());
    assert_eq!(text(&d), "x    x\nxxxxxx");
}

#[test]
fn symmetry_mirrors_positions_and_glyphs() {
    let mut d = doc(6, 2);
    let c = Ctx { symmetry: Symmetry::Both, ..ctx('█') };
    stroke(&mut d, &c, 2.0, &seg((2.0, 4.0), (10.0, 4.0)), &blocks());
    // Top-left quarter of the cells 0-1 on row 0 is inked; mirrors follow.
    let t = text(&d);
    let rows: Vec<&str> = t.lines().collect();
    assert_eq!(rows[0].chars().rev().collect::<String>(), rows[0], "{t}");
    assert!(rows[0].starts_with('▀') && rows[1].starts_with('▄'), "{t}");
}

#[test]
fn undo_restores_exactly() {
    let mut d = doc(20, 8);
    draw(&mut d, 2, 2, &["┌──┐", "│▀▄│", "└──┘"]);
    let orig = d.clone();
    let tx = stroke(&mut d, &ctx('█'), pen::RADIUS_BLOCKS, &seg((0.0, 0.0), (150.0, 120.0)), &blocks());
    assert!(!tx.is_empty());
    let tx2 = stroke(&mut d, &ctx('─'), pen::RADIUS_BOX, &seg((0.0, 100.0), (150.0, 20.0)), &boxes());
    d.revert(&tx2);
    d.revert(&tx);
    assert_eq!(d, orig);
}

#[test]
fn per_event_builders_match_one_builder() {
    // The UI may commit one transaction per drag event; the result must not
    // depend on that.
    let pts = seg((3.0, 5.0), (140.0, 90.0));
    let cands = blocks();
    let mut one = doc(20, 8);
    stroke(&mut one, &ctx('█'), pen::RADIUS_BLOCKS, &pts, &cands);
    let mut many = doc(20, 8);
    let mut s = PenStroke::new(pen::RADIUS_BLOCKS);
    for &(x, y) in &pts {
        let changed = s.add_point(x, y);
        let mut b = TxBuilder::new(&many, "pen");
        pen::apply(&mut b, &ctx('█'), &s, changed, &cands);
        let tx = b.finish();
        many.apply(&tx);
    }
    assert_eq!(text(&one), text(&many));
    let bpts = seg((3.0, 100.0), (150.0, 10.0));
    let bc = boxes();
    let mut one = doc(20, 8);
    stroke(&mut one, &ctx('─'), pen::RADIUS_BOX, &bpts, &bc);
    let mut many = doc(20, 8);
    let mut s = PenStroke::new(pen::RADIUS_BOX);
    for &(x, y) in &bpts {
        let changed = s.add_point(x, y);
        let mut b = TxBuilder::new(&many, "pen");
        pen::apply(&mut b, &ctx('─'), &s, changed, &bc);
        let tx = b.finish();
        many.apply(&tx);
    }
    assert_eq!(text(&one), text(&many));
}

#[test]
fn long_drag_is_fast_enough() {
    // A fast full-canvas scribble in debug builds.
    let mut d = doc(160, 60);
    let pts: Vec<_> = (0..2000)
        .map(|i| {
            let t = i as f32 / 2000.0;
            (640.0 + 600.0 * (t * 37.0).sin(), 480.0 + 440.0 * (t * 23.0).cos())
        })
        .collect();
    let start = std::time::Instant::now();
    stroke(&mut d, &ctx('█'), pen::RADIUS_BLOCKS, &pts, &blocks());
    let per_event = start.elapsed() / pts.len() as u32;
    assert!(per_event.as_millis() < 16, "{per_event:?} per drag event");
}
