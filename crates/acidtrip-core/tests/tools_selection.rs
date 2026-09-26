mod tools_support;

use acidtrip_core::tools::{self, FillWhat, Justify, Rect, StampMode};
use acidtrip_core::{Cell, Clip, Document, Layer};
use tools_support::*;

fn clip_text(c: &Clip) -> String {
    (0..c.height)
        .map(|y| (0..c.width).map(|x| c.get(x, y).map_or('~', |c| c.ch)).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn clip_from(rows: &[&str]) -> Clip {
    let mut c = Clip::new(rows[0].chars().count(), rows.len());
    for (y, r) in rows.iter().enumerate() {
        for (x, ch) in r.chars().enumerate() {
            c.set(x, y, (ch != '~').then(|| cell(ch)));
        }
    }
    c
}

fn two_layers() -> Document {
    let mut d = doc(4, 2);
    draw(&mut d, 0, 0, &["abcd", "efgh"]);
    d.canvas.layers.push(Layer::new("top", 4, 2));
    run(&mut d, |b| b.set(1, 1, 0, Some(cell('X'))));
    d
}

#[test]
fn copy_composite_and_layer() {
    let d = two_layers();
    let b = acidtrip_core::TxBuilder::new(&d, "r");
    assert_eq!(clip_text(&tools::copy(&b, None, Rect::new(0, 0, 3, 2))), "aXc\nefg");
    assert_eq!(clip_text(&tools::copy(&b, Some(1), Rect::new(0, 0, 3, 2))), "~X~\n~~~");
    assert_eq!(clip_text(&tools::copy(&b, Some(0), Rect::new(1, 0, 2, 1))), "bc");
    // Clipped to the canvas.
    let c = tools::copy(&b, None, Rect::new(2, 1, 10, 10));
    assert_eq!((c.width, c.height), (2, 1));
    assert_eq!(tools::copy(&b, None, Rect::new(9, 9, 2, 2)).width, 0);
}

#[test]
fn stamp_modes_table() {
    // Target: layer 1 has 'X' at (1,0); composite has "aXcd/efgh".
    let clip = clip_from(&["~ Q", "RS~"]);
    type Case = (StampMode, usize, &'static str, &'static str);
    let cases: &[Case] = &[
        // mode, layer, composite after, layer 1 after
        (StampMode::Opaque, 1, "a Qd\nRSgh", "~ Q~\nRS~~"),
        (StampMode::Transparent, 1, "aXQd\nRSgh", "~XQ~\nRS~~"),
        (StampMode::Opaque, 0, " XQd\nRS h", "~X~~\n~~~~"),
    ];
    for &(mode, layer, comp, top) in cases {
        let mut d = two_layers();
        run(&mut d, |b| tools::stamp(b, layer, &clip, 0, 0, mode));
        assert_eq!(text(&d), comp, "{mode:?} on {layer}");
        let b = acidtrip_core::TxBuilder::new(&d, "r");
        assert_eq!(clip_text(&tools::copy(&b, Some(1), Rect::new(0, 0, 4, 2))), top, "{mode:?} on {layer}");
    }
}

#[test]
fn stamp_under_only_fills_blank_targets() {
    let mut d = doc(4, 1);
    draw(&mut d, 0, 0, &["a  d"]);
    d.canvas.layers.push(Layer::new("top", 4, 1));
    run(&mut d, |b| b.set(1, 1, 0, Some(cell('X'))));
    let clip = clip_from(&["1234"]);
    run(&mut d, |b| tools::stamp(b, 1, &clip, 0, 0, StampMode::Under));
    assert_eq!(text(&d), "aX3d");
    assert_eq!(d.canvas.get(1, 0, 0), None);
}

#[test]
fn stamp_transparent_skips_blank_cells() {
    let mut d = doc(3, 1);
    draw(&mut d, 0, 0, &["abc"]);
    let mut clip = clip_from(&["1 3"]);
    clip.set(1, 0, Some(Cell::BLANK));
    run(&mut d, |b| tools::stamp(b, 0, &clip, 0, 0, StampMode::Transparent));
    assert_eq!(text(&d), "1b3");
    // A space with a colored background is not blank and is stamped.
    clip.set(1, 0, Some(Cell::new(' ', WHITE, RED)));
    run(&mut d, |b| tools::stamp(b, 0, &clip, 0, 0, StampMode::Transparent));
    assert_eq!(text(&d), "1 3");
}

#[test]
fn stamp_clips_at_edges() {
    let mut d = doc(3, 2);
    run(&mut d, |b| tools::stamp(b, 0, &clip_from(&["abcd", "efgh"]), 1, 1, StampMode::Opaque));
    assert_eq!(text(&d), "   \n ab");
}

#[test]
fn erase_rect_per_layer() {
    let mut d = two_layers();
    run(&mut d, |b| tools::erase(b, 1, Rect::new(0, 0, 4, 2)));
    assert!(d.canvas.layers[1].is_empty());
    run(&mut d, |b| tools::erase(b, 0, Rect::new(1, 1, 5, 5)));
    assert_eq!(text(&d), "abcd\ne   ");
    assert_eq!(d.canvas.get(0, 2, 1), Some(Cell::BLANK));
}

#[test]
fn fill_rect_what_table() {
    let base = Cell::new('a', RED, GREEN);
    let cases = [
        (FillWhat::All, Cell::new('#', WHITE, BLUE)),
        (FillWhat::Char, Cell::new('#', RED, GREEN)),
        (FillWhat::Fg, Cell::new('a', WHITE, GREEN)),
        (FillWhat::Bg, Cell::new('a', RED, BLUE)),
        (FillWhat::Colors, Cell::new('a', WHITE, BLUE)),
    ];
    for (what, want) in cases {
        let mut d = doc(3, 3);
        run(&mut d, |b| {
            for y in 0..3 {
                for x in 0..3 {
                    b.set(0, x, y, Some(base));
                }
            }
        });
        run(&mut d, |b| tools::fill_rect(b, &ctx('#'), Rect::new(1, 1, 5, 5), what));
        assert_eq!(d.canvas.get(0, 2, 2), Some(want), "{what:?}");
        assert_eq!(d.canvas.get(0, 0, 0), Some(base), "{what:?}");
    }
}

#[test]
fn flips_with_and_without_glyph_mirroring() {
    let c = clip_from(&["┌▀a", "▌~z"]);
    assert_eq!(clip_text(&tools::flip_x(&c, true)), "a▀┐\nz~▐");
    assert_eq!(clip_text(&tools::flip_x(&c, false)), "a▀┌\nz~▌");
    assert_eq!(clip_text(&tools::flip_y(&c, true)), "▌~z\n└▄a");
    assert_eq!(clip_text(&tools::flip_y(&c, false)), "▌~z\n┌▀a");
    assert_eq!(clip_text(&tools::rotate_180(&c)), "z~▐\na▄┘");
    assert_eq!(tools::flip_x(&tools::flip_x(&c, true), true), c);
    assert_eq!(tools::flip_y(&tools::flip_y(&c, true), true), c);
    let empty = Clip::new(0, 0);
    assert_eq!(tools::flip_x(&empty, true), empty);
}

#[test]
fn flip_keeps_colors() {
    let mut c = Clip::new(2, 1);
    c.set(0, 0, Some(Cell::new('▌', RED, GREEN)));
    let f = tools::flip_x(&c, true);
    assert_eq!(f.get(1, 0), Some(Cell::new('▐', RED, GREEN)));
    assert_eq!(f.get(0, 0), None);
}

#[test]
fn justify_table() {
    let rows = ["  ab c    ", "xy        ", "          ", "      1234"];
    let cases = [
        (Justify::Left, ["ab c      ", "xy        ", "          ", "1234      "]),
        (Justify::Center, ["   ab c   ", "    xy    ", "          ", "   1234   "]),
        (Justify::Right, ["      ab c", "        xy", "          ", "      1234"]),
    ];
    for (j, want) in cases {
        let mut d = doc(12, 4);
        draw(&mut d, 1, 0, &rows);
        run(&mut d, |b| tools::justify(b, 0, Rect::new(1, 0, 10, 4), j));
        let got = text(&d);
        let want: Vec<String> = want.iter().map(|r| format!(" {r} ")).collect();
        assert_eq!(got, want.join("\n"), "{j:?}");
    }
}

#[test]
fn justify_on_transparent_layer() {
    let mut d = doc(5, 1);
    d.canvas.layers.push(Layer::new("top", 5, 1));
    run(&mut d, |b| b.set(1, 0, 0, Some(cell('A'))));
    run(&mut d, |b| tools::justify(b, 1, Rect::new(0, 0, 5, 1), Justify::Right));
    assert_eq!(text(&d), "    A");
    assert_eq!(d.canvas.get(1, 0, 0), None);
}

#[test]
fn delete_block_shifts_left() {
    let mut d = doc(8, 3);
    draw(&mut d, 0, 0, &["abcdefgh", "ijklmnop", "qrstuvwx"]);
    run(&mut d, |b| tools::delete_block(b, 0, Rect::new(2, 0, 3, 2)));
    assert_eq!(text(&d), "abfgh   \nijnop   \nqrstuvwx");
    // Block reaching the right edge just erases.
    run(&mut d, |b| tools::delete_block(b, 0, Rect::new(6, 2, 10, 1)));
    assert_eq!(text(&d).lines().nth(2), Some("qrstuv  "));
}

#[test]
fn delete_block_on_layer_leaves_transparency() {
    let mut d = doc(4, 1);
    draw(&mut d, 0, 0, &["bbbb"]);
    d.canvas.layers.push(Layer::new("top", 4, 1));
    run(&mut d, |b| {
        b.set(1, 2, 0, Some(cell('T')));
        b.set(1, 3, 0, Some(cell('U')));
    });
    run(&mut d, |b| tools::delete_block(b, 1, Rect::new(0, 0, 1, 1)));
    assert_eq!(text(&d), "bTUb");
    assert_eq!(d.canvas.get(1, 3, 0), None);
}

#[test]
fn eyedrop_matches_copy() {
    let d = two_layers();
    let b = acidtrip_core::TxBuilder::new(&d, "r");
    assert_eq!(Some(tools::eyedrop(&b, 1, 0)), tools::copy(&b, None, Rect::new(1, 0, 1, 1)).get(0, 0));
}
