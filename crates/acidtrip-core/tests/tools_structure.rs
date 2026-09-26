mod tools_support;

use acidtrip_core::tools::{self, LayerProps, Rect};
use acidtrip_core::{Cell, Color, DocKind, Document, Layer, LayerKind};
use tools_support::*;

/// 3x3 doc: layer 0 "abc/def/ghi", layer 1 has 'X' at (2,0) and 'Y' at (0,2).
fn scene() -> Document {
    let mut d = doc(3, 3);
    draw(&mut d, 0, 0, &["abc", "def", "ghi"]);
    d.canvas.layers.push(Layer::new("top", 3, 3));
    run(&mut d, |b| {
        b.set(1, 2, 0, Some(cell('X')));
        b.set(1, 0, 2, Some(cell('Y')));
    });
    d
}

fn layer_text(d: &Document, l: usize) -> String {
    (0..d.height())
        .map(|y| (0..d.width()).map(|x| d.canvas.get(l, x, y).map_or('~', |c| c.ch)).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn line_and_column_ops_table() {
    type Op = fn(&mut acidtrip_core::TxBuilder);
    let cases: &[(&str, Op, &str, &str)] = &[
        ("insert_line 1", |b| tools::insert_line(b, 1), "abc\n   \ndef", "~~X\n~~~\n~~~"),
        ("insert_line 0", |b| tools::insert_line(b, 0), "   \nabc\ndef", "~~~\n~~X\n~~~"),
        ("delete_line 0", |b| tools::delete_line(b, 0), "def\nghi\n   ", "~~~\nY~~\n~~~"),
        ("delete_line 2", |b| tools::delete_line(b, 2), "abc\ndef\n   ", "~~X\n~~~\n~~~"),
        ("insert_column 0", |b| tools::insert_column(b, 0), " ab\n de\n gh", "~~~\n~~~\n~Y~"),
        ("insert_column 2", |b| tools::insert_column(b, 2), "ab \nde \ngh ", "~~~\n~~~\nY~~"),
        ("delete_column 0", |b| tools::delete_column(b, 0), "bc \nef \nhi ", "~X~\n~~~\n~~~"),
        ("delete_column 2", |b| tools::delete_column(b, 2), "ab \nde \ngh ", "~~~\n~~~\nY~~"),
    ];
    for &(name, op, bg, top) in cases {
        let mut d = scene();
        run(&mut d, op);
        assert_eq!(layer_text(&d, 0), bg, "{name} background");
        assert_eq!(layer_text(&d, 1), top, "{name} top layer");
        assert_eq!((d.width(), d.height()), (3, 3), "{name} keeps size");
        assert!(d.canvas.layers[0].cells.iter().all(Option::is_some), "{name} background stays opaque");
    }
}

#[test]
fn structure_ops_out_of_range_are_noops() {
    let mut d = scene();
    for op in [
        tools::insert_line as fn(&mut acidtrip_core::TxBuilder, usize),
        tools::delete_line,
        tools::insert_column,
        tools::delete_column,
    ] {
        assert!(run(&mut d, |b| op(b, 3)).is_empty());
    }
}

#[test]
fn crop_all_layers() {
    let mut d = scene();
    run(&mut d, |b| tools::crop(b, Rect::new(1, 0, 5, 2)));
    assert_eq!((d.width(), d.height()), (2, 2));
    assert_eq!(layer_text(&d, 0), "bc\nef");
    assert_eq!(layer_text(&d, 1), "~X\n~~");
    assert!(run(&mut d, |b| tools::crop(b, Rect::new(5, 5, 1, 1))).is_empty());
}

#[test]
fn resize_keeps_top_left() {
    let mut d = scene();
    run(&mut d, |b| tools::resize(b, 4, 1));
    assert_eq!(layer_text(&d, 0), "abc~");
    run(&mut d, |b| tools::resize(b, 0, 0));
    assert_eq!((d.width(), d.height()), (1, 1));
}

#[test]
fn add_remove_move_layers() {
    let mut d = scene();
    let i = {
        let mut out = 0;
        run(&mut d, |b| out = tools::add_layer(b, "new", 99));
        out
    };
    assert_eq!(i, 2);
    assert_eq!(d.canvas.layers[2].name, "new");
    assert!(d.canvas.layers[2].is_empty());
    // Index 0 is clamped so the background stays at the bottom.
    let mut j = 9;
    run(&mut d, |b| j = tools::add_layer(b, "low", 0));
    assert_eq!(j, 1);
    assert_eq!(
        d.canvas.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
        ["Background", "low", "top", "new"]
    );

    run(&mut d, |b| tools::move_layer(b, 1, 3));
    assert_eq!(
        d.canvas.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
        ["Background", "top", "new", "low"]
    );
    run(&mut d, |b| tools::move_layer(b, 3, 99));
    assert_eq!(d.canvas.layers[3].name, "low");

    run(&mut d, |b| tools::remove_layer(b, 2));
    assert_eq!(d.canvas.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Background", "top", "low"]);
    for _ in 0..5 {
        run(&mut d, |b| tools::remove_layer(b, 0));
    }
    assert_eq!(d.canvas.layers.len(), 1, "last layer is never removed");
    assert!(run(&mut d, |b| tools::remove_layer(b, 0)).is_empty());
}

#[test]
fn set_layer_props_all_fields() {
    let mut d = scene();
    let p = LayerProps { name: Some("ref".into()), visible: Some(false), locked: Some(true), reference: Some(true) };
    run(&mut d, |b| tools::set_layer_props(b, 1, &p));
    let l = &d.canvas.layers[1];
    assert_eq!((l.name.as_str(), l.visible, l.locked, l.kind), ("ref", false, true, LayerKind::Reference));
    run(&mut d, |b| tools::set_layer_props(b, 1, &LayerProps { reference: Some(false), ..Default::default() }));
    assert_eq!(d.canvas.layers[1].kind, LayerKind::Normal);
    assert_eq!(d.canvas.layers[1].name, "ref", "unset fields are kept");
    let tx = run(&mut d, |b| tools::set_layer_props(b, 7, &p));
    assert!(tx.is_empty());
}

#[test]
fn locked_layer_blocks_painting_after_props() {
    let mut d = scene();
    run(&mut d, |b| tools::set_layer_props(b, 0, &LayerProps { locked: Some(true), ..Default::default() }));
    assert!(run(&mut d, |b| tools::paint(b, &ctx('#'), 0, 0)).is_empty());
}

#[test]
fn merge_down_overwrites_with_opaque_cells() {
    let mut d = scene();
    run(&mut d, |b| tools::merge_down(b, 1));
    assert_eq!(d.canvas.layers.len(), 1);
    assert_eq!(layer_text(&d, 0), "abX\ndef\nYhi");
    assert!(run(&mut d, |b| tools::merge_down(b, 0)).is_empty());
    assert!(run(&mut d, |b| tools::merge_down(b, 5)).is_empty());
}

#[test]
fn set_kind_modern_to_classic_downsamples_all_layers() {
    let mut d = Document::new(DocKind::Modern, 2, 1);
    d.canvas.layers.push(Layer::new("top", 2, 1));
    run(&mut d, |b| {
        b.set(0, 0, 0, Some(Cell::new('╭', Color::Rgb(250, 250, 90), Color::Rgb(0, 0, 170))));
        b.set(1, 1, 0, Some(Cell::new('😀', Color::Rgb(255, 255, 255), Color::Rgb(255, 85, 85))));
    });
    run(&mut d, |b| tools::set_kind(b, DocKind::Classic));
    assert_eq!(d.meta.kind, DocKind::Classic);
    assert_eq!(d.canvas.get(0, 0, 0), Some(Cell::new('┌', Color::Pal(14), Color::Pal(1))));
    assert_eq!(d.canvas.get(1, 1, 0), Some(Cell::new('?', Color::Pal(15), Color::Pal(12))));
    assert_eq!(d.canvas.get(1, 0, 0), None, "transparency survives");
    // Back to Modern only flips the kind.
    let before = d.canvas.clone();
    run(&mut d, |b| tools::set_kind(b, DocKind::Modern));
    assert_eq!(d.meta.kind, DocKind::Modern);
    assert_eq!(d.canvas, before);
    assert!(run(&mut d, |b| tools::set_kind(b, DocKind::Modern)).is_empty());
}

#[test]
fn set_kind_to_classic_without_ice_dims_backgrounds() {
    let mut d = Document::new(DocKind::Modern, 1, 1);
    d.meta.ice = false;
    run(&mut d, |b| b.set(0, 0, 0, Some(Cell::new('x', Color::Pal(15), Color::Pal(12)))));
    run(&mut d, |b| tools::set_kind(b, DocKind::Classic));
    assert!(matches!(d.canvas.get(0, 0, 0).unwrap().bg, Color::Pal(i) if i < 8));
}

#[test]
fn set_ice_off_maps_bright_backgrounds() {
    let mut d = doc(3, 1);
    d.canvas.layers.push(Layer::new("top", 3, 1));
    run(&mut d, |b| {
        b.set(0, 0, 0, Some(Cell::new('a', Color::Pal(15), Color::Pal(12))));
        b.set(0, 1, 0, Some(Cell::new('b', Color::Pal(9), Color::Pal(3))));
        b.set(1, 2, 0, Some(Cell::new('c', Color::Pal(1), Color::Pal(15))));
    });
    run(&mut d, |b| tools::set_ice(b, false));
    assert!(!d.meta.ice);
    assert_eq!(d.canvas.get(0, 0, 0), Some(Cell::new('a', Color::Pal(15), Color::Pal(4))));
    assert_eq!(d.canvas.get(0, 1, 0), Some(Cell::new('b', Color::Pal(9), Color::Pal(3))));
    assert_eq!(d.canvas.get(1, 2, 0), Some(Cell::new('c', Color::Pal(1), Color::Pal(7))));
    assert!(run(&mut d, |b| tools::set_ice(b, false)).is_empty());
    run(&mut d, |b| tools::set_ice(b, true));
    assert!(d.meta.ice);
}

#[test]
fn set_ice_on_modern_only_records_flag() {
    let mut d = modern(1, 1);
    run(&mut d, |b| b.set(0, 0, 0, Some(Cell::new('a', Color::Pal(15), Color::Pal(12)))));
    run(&mut d, |b| tools::set_ice(b, false));
    assert_eq!(d.canvas.get(0, 0, 0).unwrap().bg, Color::Pal(12));
}

#[test]
fn cell_edits_after_structure_change_in_one_tx() {
    let mut d = scene();
    run(&mut d, |b| {
        tools::insert_line(b, 0);
        tools::paint(b, &ctx('#'), 0, 0);
    });
    assert_eq!(layer_text(&d, 0), "#  \nabc\ndef");
}

#[test]
fn duplicate_layer_copies_cells_above_and_undoes() {
    use acidtrip_core::tools;
    use acidtrip_core::{Cell, Color, DocKind, Document, TxBuilder};
    let mut d = Document::new(DocKind::Classic, 4, 2);
    let mut b = TxBuilder::new(&d, "draw");
    b.set(0, 1, 1, Some(Cell::new('█', Color::Pal(12), Color::BLACK)));
    let t = b.finish();
    d.apply(&t);
    let before = d.clone();
    let mut b = TxBuilder::new(&d, "dup");
    let idx = tools::duplicate_layer(&mut b, 0);
    let tx = b.finish();
    d.apply(&tx);
    assert_eq!(idx, 1);
    assert_eq!(d.canvas.layers.len(), 2);
    assert_eq!(d.canvas.layers[1].name, "Background copy");
    assert_eq!(d.canvas.get(1, 1, 1).unwrap().ch, '█');
    d.revert(&tx);
    assert_eq!(d, before);
}
