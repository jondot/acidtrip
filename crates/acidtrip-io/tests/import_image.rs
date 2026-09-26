//! Image import end to end.

use acidtrip_core::{Color, DocKind};
use acidtrip_io::format::{Format, load_bytes};
use acidtrip_io::import::{Dither, ImportOptions, ImportStyle, image_to_clip, image_to_doc};

/// A 64x48 gradient with a solid red square.
fn picture() -> Vec<u8> {
    let img = image::RgbaImage::from_fn(64, 48, |x, y| {
        if (8..24).contains(&x) && (8..24).contains(&y) {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([(x * 4) as u8, (y * 5) as u8, 128, 255])
        }
    });
    acidtrip_core::render::png_bytes(&img)
}

#[test]
fn every_style_and_kind_converts() {
    let png = picture();
    for style in [ImportStyle::HalfBlock, ImportStyle::Blocks, ImportStyle::Ascii] {
        for kind in [DocKind::Classic, DocKind::Modern] {
            for dither in [Dither::None, Dither::Diffuse, Dither::Ordered] {
                let opts = ImportOptions { width: 32, style, kind, dither, ink: true, ..ImportOptions::default() };
                let d = image_to_doc(&png, &opts).unwrap();
                assert_eq!(d.width(), 32);
                // 64x48 at 32 cols: 24 half-block pixel rows = 12 cells; 8x16 cells also 12 rows.
                assert_eq!(d.height(), 12, "{style:?}");
                let g = d.flatten();
                let classic_ok =
                    g.cells.iter().all(|c| matches!(c.fg, Color::Pal(0..=15)) && matches!(c.bg, Color::Pal(0..=15)));
                if kind == DocKind::Classic {
                    assert!(classic_ok, "{style:?} classic stays in the palette");
                }
                assert!(g.used_height() > 0);
            }
        }
    }
}

#[test]
fn crop_selects_source_region() {
    let png = picture();
    let opts = ImportOptions { width: 4, crop: Some((8, 8, 16, 16)), ..ImportOptions::default() };
    let c = image_to_clip(&png, &opts).unwrap();
    assert_eq!((c.width, c.height), (4, 2));
    let first = c.cells[0];
    assert!(c.cells.iter().all(|&x| x == first), "{:?}", c.cells);
    assert!(matches!(c.cells[0].unwrap().fg, Color::Pal(4 | 12)), "cropped to the red square");
}

#[test]
fn png_opens_as_document() {
    let d = load_bytes(&picture(), Format::Png).unwrap();
    assert_eq!(d.width(), 80);
    assert!(d.is_classic());
}
