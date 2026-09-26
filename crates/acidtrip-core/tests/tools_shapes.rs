mod tools_support;

use std::time::Instant;

use acidtrip_core::tools::{self, BoxStyle, Brush, Ctx, FillMatch, PaintMode, Rect, ShapeFill, Symmetry};
use acidtrip_core::{Cell, Color, DocKind, Document, Layer};
use tools_support::*;

fn boxed(w: usize, h: usize, r: Rect, fill: ShapeFill, style: BoxStyle, kind: DocKind) -> String {
    let mut d = Document::new(kind, w, h);
    run(&mut d, |b| tools::rect(b, &ctx('▒'), r, fill, style));
    text(&d)
}

#[test]
fn box_styles_table() {
    let r = Rect::new(0, 0, 4, 3);
    let cases = [
        (BoxStyle::Single, "┌──┐\n│  │\n└──┘"),
        (BoxStyle::Double, "╔══╗\n║  ║\n╚══╝"),
        (BoxStyle::DoubleH, "╒══╕\n│  │\n╘══╛"),
        (BoxStyle::DoubleV, "╓──╖\n║  ║\n╙──╜"),
        (BoxStyle::Block, "████\n█  █\n████"),
        (BoxStyle::Brush, "▒▒▒▒\n▒  ▒\n▒▒▒▒"),
        (BoxStyle::Rounded, "┌──┐\n│  │\n└──┘"),
    ];
    for (style, want) in cases {
        assert_eq!(boxed(4, 3, r, ShapeFill::Outline, style, DocKind::Classic), want, "{style:?}");
    }
    assert_eq!(boxed(4, 3, r, ShapeFill::Outline, BoxStyle::Rounded, DocKind::Modern), "╭──╮\n│  │\n╰──╯");
}

#[test]
fn degenerate_boxes() {
    let cases = [
        (Rect::new(0, 0, 1, 1), BoxStyle::Single, "─  \n   \n   "),
        (Rect::new(0, 1, 3, 1), BoxStyle::Double, "   \n═══\n   "),
        (Rect::new(1, 0, 1, 3), BoxStyle::Double, " ║ \n ║ \n ║ "),
        (Rect::new(0, 0, 2, 2), BoxStyle::Single, "┌┐ \n└┘ \n   "),
        (Rect::new(0, 0, 3, 3), BoxStyle::Block, "███\n█ █\n███"),
    ];
    for (r, style, want) in cases {
        assert_eq!(boxed(3, 3, r, ShapeFill::Outline, style, DocKind::Classic), want, "{r:?}");
    }
    let mut d = doc(3, 3);
    let tx = run(&mut d, |b| tools::rect(b, &ctx('#'), Rect::new(0, 0, 0, 3), ShapeFill::Filled, BoxStyle::Single));
    assert!(tx.is_empty());
}

#[test]
fn filled_boxes() {
    let r = Rect::new(0, 0, 4, 3);
    assert_eq!(boxed(4, 3, r, ShapeFill::Filled, BoxStyle::Brush, DocKind::Classic), "▒▒▒▒\n▒▒▒▒\n▒▒▒▒");
    assert_eq!(boxed(4, 3, r, ShapeFill::Filled, BoxStyle::Block, DocKind::Classic), "████\n█▒▒█\n████");
    let mut d = doc(4, 3);
    draw(&mut d, 0, 0, &["xxxx", "xxxx", "xxxx"]);
    run(&mut d, |b| tools::rect(b, &ctx('#'), r, ShapeFill::Filled, BoxStyle::Single));
    assert_eq!(text(&d), "┌──┐\n│  │\n└──┘");
    assert_eq!(d.canvas.get(0, 1, 1), Some(Cell::new(' ', WHITE, BLUE)));
    assert_eq!(d.canvas.get(0, 0, 0), Some(Cell::new('┌', WHITE, BLUE)));
}

#[test]
fn box_clips_at_canvas_edge() {
    let s = boxed(3, 2, Rect::new(1, 0, 5, 5), ShapeFill::Outline, BoxStyle::Single, DocKind::Classic);
    assert_eq!(s, " ┌─\n │ ");
}

#[test]
fn box_in_mirror_mode_mirrors_glyphs() {
    let mut d = doc(6, 2);
    let c = Ctx { symmetry: Symmetry::X, ..ctx('#') };
    run(&mut d, |b| tools::rect(b, &c, Rect::new(0, 0, 2, 2), ShapeFill::Outline, BoxStyle::Single));
    assert_eq!(text(&d), "┌┐  ┌┐\n└┘  └┘");
}

#[test]
fn outline_is_rect_outline() {
    let mut d = doc(3, 3);
    draw(&mut d, 0, 0, &["xxx", "xxx", "xxx"]);
    run(&mut d, |b| tools::outline(b, &ctx('#'), Rect::new(0, 0, 3, 3), BoxStyle::Double));
    assert_eq!(text(&d), "╔═╗\n║x║\n╚═╝");
}

#[test]
fn ellipse_outline_and_filled() {
    let mut d = doc(9, 5);
    run(&mut d, |b| tools::ellipse(b, &ctx('o'), Rect::new(0, 0, 9, 5), ShapeFill::Outline));
    let s = text(&d);
    let rows: Vec<&str> = s.lines().collect();
    for row in &rows {
        let r: String = row.chars().rev().collect();
        assert_eq!(*row, r, "outline must be left-right symmetric:\n{s}");
    }
    assert_eq!(rows[0], rows[4]);
    assert!(rows[2].starts_with('o') && rows[2].ends_with('o'));
    assert_eq!(rows[2].trim_matches('o'), "       ");

    let mut f = doc(9, 5);
    run(&mut f, |b| tools::ellipse(b, &ctx('o'), Rect::new(0, 0, 9, 5), ShapeFill::Filled));
    let fs = text(&f);
    // Filled = outline hull; every row is a single contiguous run.
    for (orow, frow) in s.lines().zip(fs.lines()) {
        let first = orow.find('o').unwrap();
        let last = orow.rfind('o').unwrap();
        assert_eq!(frow.find('o'), Some(first));
        assert_eq!(frow.rfind('o'), Some(last));
        assert!(frow[first..=last].chars().all(|c| c == 'o'));
    }
}

#[test]
fn ellipse_small_sizes() {
    let cases = [
        (Rect::new(0, 0, 1, 1), "o  \n   \n   "),
        (Rect::new(0, 0, 3, 1), "ooo\n   \n   "),
        (Rect::new(1, 0, 1, 3), " o \n o \n o "),
        (Rect::new(0, 0, 2, 2), "oo \noo \n   "),
    ];
    for (r, want) in cases {
        let mut d = doc(3, 3);
        run(&mut d, |b| tools::ellipse(b, &ctx('o'), r, ShapeFill::Outline));
        assert_eq!(text(&d), want, "{r:?}");
    }
}

#[test]
fn ellipse_clips_at_edge() {
    let mut d = doc(8, 8);
    run(&mut d, |b| tools::ellipse(b, &ctx('o'), Rect::new(2, 2, 10, 10), ShapeFill::Filled));
    assert_eq!(ch(&d, 2, 2), ' ');
    assert_eq!(ch(&d, 2, 6), 'o');
    assert_eq!(ch(&d, 7, 7), 'o');
    assert_eq!(ch(&d, 1, 6), ' ');
}

fn fill_scene() -> Document {
    // Two regions of '.' separated by a wall; one '.' has a red fg, one a green bg.
    let mut d = doc(7, 3);
    draw(&mut d, 0, 0, &["..|....", "..|....", "..|...."]);
    run(&mut d, |b| {
        for y in 0..3 {
            b.set(0, 2, y, Some(Cell::new('|', YELLOW, RED)));
        }
        b.set(0, 4, 1, Some(Cell::new('.', RED, Color::BLACK)));
        b.set(0, 5, 1, Some(Cell::new(':', WHITE, Color::BLACK)));
        b.set(0, 6, 2, Some(Cell::new('.', WHITE, GREEN)));
    });
    d
}

#[test]
fn flood_fill_match_table() {
    let t = |ch: bool, fg: bool, bg: bool| FillMatch { ch, fg, bg };
    let cases = [
        (t(true, true, true), "..|####\n..|#.:#\n..|###."),
        (t(true, false, false), "..|####\n..|##:#\n..|####"),
        (t(false, true, false), "..|####\n..|#.##\n..|####"),
        (t(false, false, true), "..|####\n..|####\n..|###."),
        (t(false, false, false), "#######\n#######\n#######"),
    ];
    for (m, want) in cases {
        let mut d = fill_scene();
        run(&mut d, |b| tools::flood_fill(b, &ctx('#'), 3, 0, m));
        assert_eq!(text(&d), want, "{m:?}");
    }
}

#[test]
fn flood_fill_respects_mode() {
    let mut d = fill_scene();
    let c = Ctx { mode: PaintMode::Bg, ..ctx('#') };
    run(&mut d, |b| tools::flood_fill(b, &c, 0, 0, FillMatch::default()));
    assert_eq!(text(&d), "..|....\n..|..:.\n..|....");
    assert_eq!(d.canvas.get(0, 1, 1).unwrap().bg, BLUE);
    assert_eq!(d.canvas.get(0, 3, 0).unwrap().bg, Color::BLACK);
}

#[test]
fn flood_fill_same_color_terminates() {
    let mut d = doc(4, 4);
    let c = Ctx { brush: Brush { ch: ' ', fg: Color::LIGHT_GRAY, bg: Color::BLACK }, ..Ctx::default() };
    let tx = run(&mut d, |b| tools::flood_fill(b, &c, 1, 1, FillMatch::default()));
    assert!(tx.is_empty());
}

#[test]
fn flood_fill_snakes_through_maze() {
    let mut d = doc(7, 5);
    draw(&mut d, 0, 0, &["  X    ", "X X XX ", "  X  X ", " XXX X ", "     X "]);
    run(&mut d, |b| tools::flood_fill(b, &ctx('o'), 0, 0, FillMatch::default()));
    assert_eq!(text(&d), "ooXoooo\nXoXoXXo\nooXooXo\noXXXoXo\noooooXo");
}

#[test]
fn flood_fill_on_upper_layer_reads_composite() {
    let mut d = doc(5, 1);
    draw(&mut d, 0, 0, &["  |  "]);
    d.canvas.layers.push(Layer::new("top", 5, 1));
    let c = Ctx { layer: 1, ..ctx('#') };
    run(&mut d, |b| tools::flood_fill(b, &c, 0, 0, FillMatch::default()));
    assert_eq!(text(&d), "##|  ");
    assert_eq!(d.canvas.get(0, 0, 0).unwrap().ch, ' ');
    assert_eq!(d.canvas.get(1, 3, 0), None);
}

#[test]
fn flood_fill_out_of_bounds_is_noop() {
    let mut d = doc(3, 3);
    assert!(run(&mut d, |b| tools::flood_fill(b, &ctx('#'), 3, 0, FillMatch::default())).is_empty());
}

#[test]
fn flood_fill_ignores_symmetry() {
    let mut d = doc(5, 1);
    draw(&mut d, 0, 0, &["  |  "]);
    let c = Ctx { symmetry: Symmetry::X, ..ctx('#') };
    run(&mut d, |b| tools::flood_fill(b, &c, 0, 0, FillMatch::default()));
    assert_eq!(text(&d), "##|  ");
}

#[test]
fn flood_fill_large_canvas_is_fast() {
    let mut d = Document::new(DocKind::Modern, 1000, 1000);
    // A wall with a gap forces the fill around it.
    run(&mut d, |b| {
        for y in 0..999 {
            b.set(0, 500, y, Some(cell('|')));
        }
    });
    let t = Instant::now();
    let tx = run(&mut d, |b| tools::flood_fill(b, &ctx('#'), 0, 0, FillMatch::default()));
    let el = t.elapsed();
    assert_eq!(tx.cells.len(), 1000 * 1000 - 999);
    assert_eq!(ch(&d, 999, 0), '#');
    assert!(el.as_secs() < 20, "fill took {el:?}");
}
