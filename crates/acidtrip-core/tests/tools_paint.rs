mod tools_support;

use acidtrip_core::tools::{self, Brush, Ctx, PaintMode, Symmetry};
use acidtrip_core::{Cell, Color, Layer};
use tools_support::*;

#[test]
fn paint_modes_table() {
    // Existing cell: 'A' red on green. Brush: '#' white on blue.
    let base = Cell::new('A', RED, GREEN);
    let cases: &[(PaintMode, Option<Cell>)] = &[
        (PaintMode::Char, Some(Cell::new('#', WHITE, BLUE))),
        (PaintMode::Color, Some(Cell::new('A', WHITE, BLUE))),
        (PaintMode::Fg, Some(Cell::new('A', WHITE, GREEN))),
        (PaintMode::Bg, Some(Cell::new('A', RED, BLUE))),
        (PaintMode::Shade { up: true }, Some(Cell::new('░', WHITE, GREEN))),
        (PaintMode::Shade { up: false }, Some(base)),
        (PaintMode::Colorize, Some(Cell::new('A', WHITE, GREEN))),
        (PaintMode::Erase, Some(Cell::BLANK)),
    ];
    for &(mode, want) in cases {
        let mut d = doc(3, 1);
        run(&mut d, |b| b.set(0, 1, 0, Some(base)));
        let c = Ctx { mode, ..ctx('#') };
        run(&mut d, |b| tools::paint(b, &c, 1, 0));
        assert_eq!(d.canvas.get(0, 1, 0), want, "{mode:?}");
        assert_eq!(d.canvas.get(0, 0, 0), Some(Cell::BLANK), "{mode:?} touched a neighbor");
    }
}

#[test]
fn shade_steps_up_and_down() {
    let seq = [' ', '░', '▒', '▓', '█', '█'];
    let mut d = doc(1, 1);
    let up = Ctx { mode: PaintMode::Shade { up: true }, ..ctx('x') };
    for w in seq.windows(2) {
        assert_eq!(ch(&d, 0, 0), w[0]);
        run(&mut d, |b| tools::paint(b, &up, 0, 0));
        assert_eq!(ch(&d, 0, 0), w[1]);
    }
    let down = Ctx { mode: PaintMode::Shade { up: false }, ..ctx('x') };
    for want in ['▓', '▒', '░', ' ', ' '] {
        run(&mut d, |b| tools::paint(b, &down, 0, 0));
        assert_eq!(ch(&d, 0, 0), want);
    }
    assert_eq!(d.canvas.get(0, 0, 0).unwrap().fg, WHITE);
}

#[test]
fn shade_step_helper() {
    assert_eq!(tools::shade_step('A', true), Some('░'));
    assert_eq!(tools::shade_step('A', false), None);
    assert_eq!(tools::shade_step('\u{0}', true), Some('░'));
    assert_eq!(tools::shade_step('░', false), Some(' '));
}

#[test]
fn shade_keeps_existing_background() {
    let mut d = doc(1, 1);
    run(&mut d, |b| b.set(0, 0, 0, Some(Cell::new(' ', RED, GREEN))));
    let up = Ctx { mode: PaintMode::Shade { up: true }, ..ctx('x') };
    run(&mut d, |b| tools::paint(b, &up, 0, 0));
    assert_eq!(d.canvas.get(0, 0, 0), Some(Cell::new('░', WHITE, GREEN)));
}

#[test]
fn colorize_skips_blank_glyphs() {
    let mut d = doc(3, 1);
    draw(&mut d, 0, 0, &["A B"]);
    let c = Ctx { mode: PaintMode::Colorize, brush: Brush { ch: 'x', fg: YELLOW, bg: RED }, ..Ctx::default() };
    let tx = run(&mut d, |b| tools::paint_line(b, &c, 0, 0, 2, 0));
    assert_eq!(tx.cells.len(), 2);
    assert_eq!(d.canvas.get(0, 0, 0), Some(Cell::new('A', YELLOW, Color::BLACK)));
    assert_eq!(d.canvas.get(0, 1, 0).unwrap().fg, WHITE);
    assert_eq!(d.canvas.get(0, 2, 0).unwrap().fg, YELLOW);
}

#[test]
fn modes_fall_back_to_composite_on_transparent_layer() {
    let mut d = doc(2, 1);
    draw(&mut d, 0, 0, &["Q"]);
    d.canvas.layers.push(Layer::new("top", 2, 1));
    let c = Ctx { layer: 1, mode: PaintMode::Fg, ..ctx('#') };
    run(&mut d, |b| tools::paint(b, &c, 0, 0));
    assert_eq!(d.canvas.get(1, 0, 0), Some(Cell::new('Q', WHITE, Color::BLACK)));
}

#[test]
fn erase_on_upper_layer_is_transparent() {
    let mut d = doc(2, 1);
    d.canvas.layers.push(Layer::new("top", 2, 1));
    run(&mut d, |b| b.set(1, 0, 0, Some(cell('Z'))));
    let c = Ctx { layer: 1, mode: PaintMode::Erase, ..ctx('#') };
    run(&mut d, |b| tools::paint(b, &c, 0, 0));
    assert_eq!(d.canvas.get(1, 0, 0), None);
}

#[test]
fn symmetry_positions_and_glyph_mirroring() {
    // (width, height, symmetry, expected positions with glyphs)
    type Case = (usize, usize, Symmetry, &'static [(usize, usize, char)]);
    let cases: &[Case] = &[
        (6, 4, Symmetry::None, &[(1, 0, '┌')]),
        (6, 4, Symmetry::X, &[(1, 0, '┌'), (4, 0, '┐')]),
        (5, 4, Symmetry::X, &[(1, 0, '┌'), (3, 0, '┐')]),
        (6, 4, Symmetry::Y, &[(1, 0, '┌'), (1, 3, '└')]),
        (6, 5, Symmetry::Both, &[(1, 0, '┌'), (4, 0, '┐'), (1, 4, '└'), (4, 4, '┘')]),
    ];
    for &(w, h, sym, want) in cases {
        let mut d = doc(w, h);
        let c = Ctx { symmetry: sym, ..ctx('┌') };
        let tx = run(&mut d, |b| tools::paint(b, &c, 1, 0));
        assert_eq!(tx.cells.len(), want.len(), "{w}x{h} {sym:?}");
        for &(x, y, g) in want {
            assert_eq!(ch(&d, x, y), g, "{w}x{h} {sym:?} at {x},{y}");
        }
    }
}

#[test]
fn symmetry_center_column_on_odd_width_paints_once() {
    let mut d = doc(5, 1);
    let c = Ctx { symmetry: Symmetry::X, mode: PaintMode::Shade { up: true }, ..ctx('x') };
    let tx = run(&mut d, |b| tools::paint(b, &c, 2, 0));
    assert_eq!(tx.cells.len(), 1);
    assert_eq!(ch(&d, 2, 0), '░', "center must shade once, not twice");
}

#[test]
fn symmetric_line_crossing_center_shades_once() {
    let mut d = doc(6, 1);
    let c = Ctx { symmetry: Symmetry::X, mode: PaintMode::Shade { up: true }, ..ctx('x') };
    run(&mut d, |b| tools::paint_line(b, &c, 0, 0, 5, 0));
    assert_eq!(text(&d), "░░░░░░");
}

#[test]
fn symmetry_at_border() {
    let mut d = doc(4, 3);
    let c = Ctx { symmetry: Symmetry::Both, ..ctx('▀') };
    run(&mut d, |b| tools::paint(b, &c, 3, 2));
    assert_eq!(text(&d), "▄  ▄\n    \n▀  ▀");
}

#[test]
fn paint_out_of_bounds_is_ignored() {
    let mut d = doc(3, 3);
    let c = Ctx { symmetry: Symmetry::Both, ..ctx('#') };
    let tx = run(&mut d, |b| tools::paint(b, &c, 3, 1));
    assert!(tx.is_empty());
}

#[test]
fn paint_line_bresenham() {
    let mut d = doc(5, 3);
    run(&mut d, |b| tools::paint_line(b, &ctx('*'), 0, 0, 4, 2));
    assert_eq!(text(&d), "*    \n **  \n   **");
    let mut d = doc(3, 3);
    run(&mut d, |b| tools::paint_line(b, &ctx('*'), 2, 2, 2, 2));
    assert_eq!(text(&d), "   \n   \n  *");
    let mut d = doc(3, 3);
    run(&mut d, |b| tools::paint_line(b, &ctx('*'), 2, 0, 0, 2));
    assert_eq!(text(&d), "  *\n * \n*  ");
}

#[test]
fn paint_line_clips_off_canvas() {
    let mut d = doc(3, 1);
    run(&mut d, |b| tools::paint_line(b, &ctx('-'), 0, 0, 10, 0));
    assert_eq!(text(&d), "---");
}

#[test]
fn type_char_uses_brush_colors() {
    let mut d = doc(2, 1);
    run(&mut d, |b| tools::type_char(b, &ctx('#'), 1, 0, 'k'));
    assert_eq!(d.canvas.get(0, 1, 0), Some(Cell::new('k', WHITE, BLUE)));
}

#[test]
fn put_text_multiline_and_transparent_spaces() {
    let mut d = doc(6, 3);
    draw(&mut d, 0, 0, &["......", "......", "......"]);
    let wh = run_ret(&mut d, |b| tools::put_text(b, &ctx('#'), 1, 0, "ab c\nxyz\n", true));
    assert_eq!(wh, (4, 2));
    assert_eq!(text(&d), ".ab.c.\n.xyz..\n......");
    let wh = run_ret(&mut d, |b| tools::put_text(b, &ctx('#'), 0, 2, "a b", false));
    assert_eq!(wh, (3, 1));
    assert_eq!(text(&d).lines().nth(2), Some("a b..."));
    assert_eq!(run_ret(&mut d, |b| tools::put_text(b, &ctx('#'), 0, 0, "", false)), (0, 0));
}

#[test]
fn put_text_clips_at_edge_but_reports_full_size() {
    let mut d = doc(3, 1);
    let wh = run_ret(&mut d, |b| tools::put_text(b, &ctx('#'), 1, 0, "hello\nworld", false));
    assert_eq!(wh, (5, 2));
    assert_eq!(text(&d), " he");
}

#[test]
fn eyedrop_reads_composite() {
    let mut d = doc(2, 1);
    d.canvas.layers.push(Layer::new("top", 2, 1));
    run(&mut d, |b| b.set(1, 1, 0, Some(cell('T'))));
    let b = acidtrip_core::TxBuilder::new(&d, "x");
    assert_eq!(tools::eyedrop(&b, 1, 0).ch, 'T');
    assert_eq!(tools::eyedrop(&b, 0, 0), Cell::BLANK);
}

#[test]
fn locked_layer_is_untouched() {
    let mut d = doc(2, 1);
    d.canvas.layers[0].locked = true;
    let tx = run(&mut d, |b| tools::paint(b, &ctx('#'), 0, 0));
    assert!(tx.is_empty());
}

#[test]
fn modern_keeps_unicode_and_rgb() {
    let mut d = modern(2, 1);
    let c = Ctx { brush: Brush { ch: '🬂', fg: Color::Rgb(1, 2, 3), bg: Color::Rgb(4, 5, 6) }, ..Ctx::default() };
    run(&mut d, |b| tools::paint(b, &c, 0, 0));
    assert_eq!(d.canvas.get(0, 0, 0), Some(Cell::new('🬂', Color::Rgb(1, 2, 3), Color::Rgb(4, 5, 6))));
}

fn run_ret<T>(d: &mut acidtrip_core::Document, f: impl FnOnce(&mut acidtrip_core::TxBuilder) -> T) -> T {
    let mut out = None;
    run(d, |b| out = Some(f(b)));
    out.unwrap()
}
