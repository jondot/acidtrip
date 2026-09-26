mod tools_support;

use acidtrip_core::tools::pixel;
use acidtrip_core::{Cell, Color, DocKind, Document, Layer, TxBuilder};
use tools_support::*;

const LRED: Color = Color::Pal(12);

/// Pixel map as text: '.' for empty, hex palette index otherwise.
fn pixels(d: &Document) -> String {
    let b = TxBuilder::new(d, "read");
    (0..d.height() * 2)
        .map(|y| {
            (0..d.width())
                .map(|x| match pixel::get(&b, x, y) {
                    None => '.',
                    Some(Color::Pal(i)) => char::from_digit(i as u32, 16).unwrap(),
                    Some(Color::Rgb(..)) => '*',
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn get_decodes_composite() {
    let mut d = doc(4, 1);
    run(&mut d, |b| {
        b.set(0, 0, 0, Some(Cell::new('▀', RED, GREEN)));
        b.set(0, 1, 0, Some(Cell::new('▄', RED, GREEN)));
        b.set(0, 2, 0, Some(Cell::new('█', RED, GREEN)));
        b.set(0, 3, 0, Some(Cell::new(' ', RED, GREEN)));
    });
    assert_eq!(pixels(&d), "4242\n2442");
    let b = TxBuilder::new(&d, "r");
    assert_eq!(pixel::get(&b, 4, 0), None);
    assert_eq!(pixel::get(&b, 0, 2), None);
}

#[test]
fn line_in_pixel_space() {
    let mut d = doc(4, 2);
    run(&mut d, |b| pixel::line(b, 0, 0, 0, 3, 3, LRED));
    assert_eq!(pixels(&d), "c...\n.c..\n..c.\n...c");
    assert_eq!(text(&d), "▀▄  \n  ▀▄");
}

#[test]
fn line_clips_negative_coordinates() {
    let mut d = doc(3, 1);
    run(&mut d, |b| pixel::line(b, 0, -2, 0, 4, 0, LRED));
    assert_eq!(pixels(&d), "ccc\n...");
}

#[test]
fn rect_outline_and_filled() {
    let mut d = doc(4, 2);
    run(&mut d, |b| pixel::rect(b, 0, 0, 0, 4, 4, RED, false));
    assert_eq!(pixels(&d), "4444\n4..4\n4..4\n4444");
    run(&mut d, |b| pixel::rect(b, 0, 1, 1, 2, 2, YELLOW, true));
    assert_eq!(pixels(&d), "4444\n4ee4\n4ee4\n4444");
    let mut e = doc(2, 1);
    assert!(run(&mut e, |b| pixel::rect(b, 0, 0, 0, 0, 5, RED, true)).is_empty());
}

#[test]
fn ellipse_outline_filled_and_clipped() {
    let mut d = doc(9, 5);
    run(&mut d, |b| pixel::ellipse(b, 0, 4, 4, 4, 4, WHITE, false));
    let s = pixels(&d);
    let rows: Vec<&str> = s.lines().collect();
    assert_eq!(rows[0], "...fff...");
    assert_eq!(rows[4], "f.......f");
    assert_eq!(rows[8], "...fff...");
    let mut f = doc(9, 5);
    run(&mut f, |b| pixel::ellipse(b, 0, 4, 4, 4, 4, WHITE, true));
    assert_eq!(pixels(&f).lines().nth(4), Some("fffffffff"));
    // Clipped: centered on the corner.
    let mut c = doc(3, 2);
    run(&mut c, |b| pixel::ellipse(b, 0, 0, 0, 2, 2, WHITE, true));
    assert_eq!(pixels(&c), "fff\nfff\nff.\n...");
}

#[test]
fn dither_mix_extremes_and_half() {
    for (mix, want_c1, want_c2) in [(0.0, 16, 0), (1.0, 0, 16), (0.5, 8, 8)] {
        let mut d = doc(4, 2);
        run(&mut d, |b| pixel::dither_rect(b, 0, 0, 0, 4, 4, RED, GREEN, mix));
        let s = pixels(&d);
        assert_eq!(s.matches('4').count(), want_c1, "mix {mix}");
        assert_eq!(s.matches('2').count(), want_c2, "mix {mix}");
    }
    assert_eq!(pixel::bayer(0, 0), 0.5 / 16.0);
}

#[test]
fn flood_fill_pixels() {
    let mut d = doc(5, 2);
    run(&mut d, |b| pixel::line(b, 0, 2, 0, 2, 3, WHITE));
    run(&mut d, |b| pixel::flood_fill(b, 0, 0, 0, RED));
    assert_eq!(pixels(&d), "44f..\n44f..\n44f..\n44f..");
    // Filling a region with its own color is a no-op.
    assert!(run(&mut d, |b| pixel::flood_fill(b, 0, 1, 1, RED)).is_empty());
    // Black fill clears it again.
    run(&mut d, |b| pixel::flood_fill(b, 0, 0, 0, Color::BLACK));
    assert_eq!(pixels(&d), "..f..\n..f..\n..f..\n..f..");
    assert!(run(&mut d, |b| pixel::flood_fill(b, 0, 9, 9, RED)).is_empty());
}

#[test]
fn flood_fill_splits_cells_between_halves() {
    let mut d = doc(3, 2);
    run(&mut d, |b| pixel::line(b, 0, 0, 1, 2, 1, WHITE));
    run(&mut d, |b| pixel::flood_fill(b, 0, 0, 3, YELLOW));
    assert_eq!(pixels(&d), "...\nfff\neee\neee");
    assert_eq!(text(&d), "▄▄▄\n███");
}

#[test]
fn pixels_on_upper_layer_keep_visible_other_half() {
    let mut d = doc(1, 1);
    run(&mut d, |b| pixel::set(b, 0, 0, 1, RED));
    d.canvas.layers.push(Layer::new("px", 1, 1));
    run(&mut d, |b| pixel::set(b, 1, 0, 0, YELLOW));
    assert_eq!(pixels(&d), "e\n4");
    assert_eq!(d.canvas.get(1, 0, 0), Some(Cell::new('▀', YELLOW, RED)));
}

#[test]
fn classic_without_ice_never_writes_bright_bg() {
    let mut d = doc(6, 3);
    d.meta.ice = false;
    run(&mut d, |b| {
        pixel::dither_rect(b, 0, 0, 0, 6, 6, LRED, WHITE, 0.5);
        pixel::line(b, 0, 0, 5, 5, 0, YELLOW);
    });
    for l in &d.canvas.layers {
        for c in l.cells.iter().flatten() {
            assert!(matches!(c.bg, Color::Pal(i) if i < 8), "bright bg {c:?}");
        }
    }
}

#[test]
fn modern_rgb_circle() {
    let mut d = Document::new(DocKind::Modern, 5, 3);
    let c = Color::Rgb(200, 100, 50);
    run(&mut d, |b| pixel::ellipse(b, 0, 2, 2, 2, 2, c, true));
    let b = TxBuilder::new(&d, "r");
    assert_eq!(pixel::get(&b, 2, 2), Some(c));
    assert_eq!(pixel::get(&b, 0, 0), None);
}
