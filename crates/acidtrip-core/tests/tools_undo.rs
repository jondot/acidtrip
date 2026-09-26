//! Every tool's transaction must revert to the exact original document.

mod tools_support;

use acidtrip_core::tools::{
    self, BoxStyle, Brush, Ctx, FillMatch, FillWhat, Justify, LayerProps, PaintMode, Rect, ShapeFill, StampMode,
    Symmetry,
};
use acidtrip_core::{Cell, Clip, Color, DocKind, Document, Layer, TxBuilder};
use tools_support::*;

fn scene() -> Document {
    let mut d = doc(10, 6);
    draw(&mut d, 0, 0, &["  hello   ", " ┌──┐     ", " │▀▄│  ░▒ ", " └──┘     ", "  ab  cd  ", "          "]);
    d.canvas.layers.push(Layer::new("top", 10, 6));
    run(&mut d, |b| {
        b.set(1, 5, 1, Some(Cell::new('X', RED, Color::Pal(12))));
        b.set(1, 6, 4, Some(Cell::new('Y', GREEN, Color::BLACK)));
    });
    d
}

type Tool = Box<dyn Fn(&mut TxBuilder)>;

fn tools_list() -> Vec<(&'static str, Tool)> {
    let c = ctx('#');
    let up = Ctx { layer: 1, symmetry: Symmetry::Both, mode: PaintMode::Shade { up: true }, ..ctx('#') };
    let mut clip = Clip::new(2, 2);
    clip.set(0, 0, Some(cell('Q')));
    clip.set(1, 1, Some(Cell::BLANK));
    let clip2 = clip.clone();
    let clip3 = clip.clone();
    let modes = [
        PaintMode::Char,
        PaintMode::Color,
        PaintMode::Fg,
        PaintMode::Bg,
        PaintMode::Shade { up: false },
        PaintMode::Colorize,
        PaintMode::Erase,
    ];
    let mut v: Vec<(&'static str, Tool)> = vec![
        ("paint shade sym", Box::new(move |b| tools::paint(b, &up, 1, 2))),
        ("paint_line", Box::new(move |b| tools::paint_line(b, &c, 0, 5, 9, 0))),
        ("rect", Box::new(move |b| tools::rect(b, &c, Rect::new(4, 1, 5, 4), ShapeFill::Filled, BoxStyle::Double))),
        ("outline", Box::new(move |b| tools::outline(b, &c, Rect::new(0, 0, 10, 6), BoxStyle::Rounded))),
        ("ellipse", Box::new(move |b| tools::ellipse(b, &c, Rect::new(0, 0, 10, 6), ShapeFill::Outline))),
        ("ellipse filled", Box::new(move |b| tools::ellipse(b, &c, Rect::new(2, 1, 6, 4), ShapeFill::Filled))),
        ("flood_fill", Box::new(move |b| tools::flood_fill(b, &c, 9, 5, FillMatch::default()))),
        ("type_char", Box::new(move |b| tools::type_char(b, &c, 3, 3, 'Z'))),
        (
            "put_text",
            Box::new(move |b| {
                tools::put_text(b, &c, 1, 4, "hi there\nyo", true);
            }),
        ),
        ("pixel set", Box::new(|b| tools::pixel::set(b, 1, 3, 5, Color::Pal(10)))),
        ("pixel line", Box::new(|b| tools::pixel::line(b, 0, 0, 0, 9, 11, Color::Pal(13)))),
        ("pixel rect", Box::new(|b| tools::pixel::rect(b, 0, 1, 1, 6, 6, Color::Pal(3), false))),
        ("pixel ellipse", Box::new(|b| tools::pixel::ellipse(b, 1, 5, 6, 4, 5, Color::Pal(14), true))),
        ("pixel dither", Box::new(|b| tools::pixel::dither_rect(b, 0, 0, 0, 10, 12, RED, BLUE, 0.3))),
        ("pixel fill", Box::new(|b| tools::pixel::flood_fill(b, 0, 9, 11, Color::Pal(9)))),
        ("stamp opaque", Box::new(move |b| tools::stamp(b, 1, &clip, 4, 0, StampMode::Opaque))),
        ("stamp transparent", Box::new(move |b| tools::stamp(b, 0, &clip2, 8, 4, StampMode::Transparent))),
        ("stamp under", Box::new(move |b| tools::stamp(b, 1, &clip3, 0, 0, StampMode::Under))),
        ("erase", Box::new(|b| tools::erase(b, 0, Rect::new(0, 0, 4, 4)))),
        ("fill_rect", Box::new(move |b| tools::fill_rect(b, &c, Rect::new(2, 2, 3, 3), FillWhat::Colors))),
        ("justify", Box::new(|b| tools::justify(b, 0, Rect::new(0, 0, 10, 6), Justify::Right))),
        ("delete_block", Box::new(|b| tools::delete_block(b, 0, Rect::new(1, 0, 2, 5)))),
        ("insert_line", Box::new(|b| tools::insert_line(b, 2))),
        ("delete_line", Box::new(|b| tools::delete_line(b, 0))),
        ("insert_column", Box::new(|b| tools::insert_column(b, 0))),
        ("delete_column", Box::new(|b| tools::delete_column(b, 5))),
        ("resize", Box::new(|b| tools::resize(b, 20, 3))),
        ("crop", Box::new(|b| tools::crop(b, Rect::new(1, 1, 4, 3)))),
        (
            "add_layer",
            Box::new(|b| {
                tools::add_layer(b, "L", 1);
            }),
        ),
        ("remove_layer", Box::new(|b| tools::remove_layer(b, 1))),
        ("move_layer", Box::new(|b| tools::move_layer(b, 1, 0))),
        (
            "set_layer_props",
            Box::new(|b| {
                tools::set_layer_props(
                    b,
                    1,
                    &LayerProps { name: Some("n".into()), reference: Some(true), ..Default::default() },
                )
            }),
        ),
        ("merge_down", Box::new(|b| tools::merge_down(b, 1))),
        ("set_ice", Box::new(|b| tools::set_ice(b, false))),
        ("set_kind", Box::new(|b| tools::set_kind(b, DocKind::Modern))),
        (
            "combo",
            Box::new(move |b| {
                tools::paint(b, &c, 0, 0);
                tools::insert_line(b, 0);
                tools::paint(b, &c, 9, 5);
                tools::set_ice(b, false);
            }),
        ),
    ];
    for mode in modes {
        let m = Ctx { mode, brush: Brush { ch: '▌', fg: YELLOW, bg: RED }, ..Ctx::default() };
        let name: &'static str = Box::leak(format!("paint {mode:?}").into_boxed_str());
        v.push((name, Box::new(move |b| tools::paint_line(b, &m, 0, 2, 9, 2))));
    }
    v
}

#[test]
fn every_tool_reverts_exactly() {
    let d = scene();
    for (name, tool) in tools_list() {
        let mut work = d.clone();
        let tx = run(&mut work, &*tool);
        assert!(!tx.is_empty(), "{name}: no change");
        assert_ne!(work, d, "{name}: no visible change");
        work.revert(&tx);
        assert_eq!(work, d, "{name}: revert mismatch");
        work.apply(&tx);
        let redo = work.clone();
        work.revert(&tx);
        work.apply(&tx);
        assert_eq!(work, redo, "{name}: redo mismatch");
    }
}

#[test]
fn modern_to_classic_reverts_exactly() {
    let mut d = modern(3, 1);
    run(&mut d, |b| b.set(0, 1, 0, Some(Cell::new('╭', Color::Rgb(1, 2, 3), Color::Rgb(200, 10, 10)))));
    assert_undoable(&d, |b| tools::set_kind(b, DocKind::Classic));
}
