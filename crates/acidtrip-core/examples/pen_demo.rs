//! Draws a spiral, a sine wave, a diagonal and a circle with the smart pen,
//! once with the blocks set and once with the single-line box set, plus the
//! smart shapes, and renders them to PNGs in /tmp/acidtrip-pen/.
//!
//! cargo run -p acidtrip-core --example pen_demo

use std::f32::consts::TAU;

use acidtrip_core::render::{self, RenderOptions};
use acidtrip_core::tools::{self, Brush, Ctx, Rect, ShapeFill, pen};
use acidtrip_core::{Color, DocKind, Document, TxBuilder};

const W: usize = 100;
const H: usize = 30;

/// Polylines in glyph-pixel space.
fn strokes() -> Vec<Vec<(f32, f32)>> {
    let spiral = (0..=600)
        .map(|i| {
            let t = i as f32 / 600.0;
            let a = t * 3.0 * TAU;
            let r = 10.0 + 100.0 * t;
            (130.0 + r * a.cos(), 240.0 + r * a.sin())
        })
        .collect();
    let sine = (0..=250).map(|i| (280.0 + i as f32 * 2.0, 80.0 + 45.0 * (i as f32 / 20.0).sin())).collect();
    let diagonal = vec![(290.0, 200.0), (530.0, 450.0)];
    let circle = (0..=360)
        .map(|i| {
            let a = i as f32 / 360.0 * TAU;
            (670.0 + 110.0 * a.cos(), 320.0 + 110.0 * a.sin())
        })
        .collect();
    let shallow = vec![(300.0, 150.0), (560.0, 190.0)];
    vec![spiral, sine, diagonal, circle, shallow]
}

fn draw(name: &str, radius: f32, candidates: &[char]) {
    let mut d = Document::new(DocKind::Classic, W, H);
    let ctx = Ctx { brush: Brush { ch: '█', fg: Color::Pal(14), bg: Color::BLACK }, ..Ctx::default() };
    for pts in strokes() {
        let mut s = pen::PenStroke::new(radius);
        let mut b = TxBuilder::new(&d, "pen");
        // Apply incrementally, as the UI does while dragging.
        for (x, y) in pts {
            let changed = s.add_point(x, y);
            pen::apply(&mut b, &ctx, &s, changed, candidates);
        }
        let tx = b.finish();
        d.apply(&tx);
    }
    save(&d, name);
}

fn shapes() {
    let mut d = Document::new(DocKind::Classic, W, H);
    let mut c = Ctx { brush: Brush { ch: '█', fg: Color::Pal(12), bg: Color::BLACK }, ..Ctx::default() };
    let mut b = TxBuilder::new(&d, "shapes");
    tools::smart_ellipse(&mut b, &c, Rect::new(1, 1, 24, 12), ShapeFill::Outline);
    tools::smart_ellipse(&mut b, &c, Rect::new(27, 1, 16, 8), ShapeFill::Filled);
    tools::smart_rect(&mut b, &c, Rect::new(45, 1, 12, 6), ShapeFill::Outline);
    tools::smart_line(&mut b, &c, 60, 1, 98, 12);
    tools::smart_line(&mut b, &c, 27, 11, 57, 13);
    c.brush = Brush { ch: '─', fg: Color::Pal(11), ..c.brush };
    tools::smart_ellipse(&mut b, &c, Rect::new(1, 15, 24, 12), ShapeFill::Outline);
    tools::smart_rect(&mut b, &c, Rect::new(27, 15, 12, 6), ShapeFill::Outline);
    tools::smart_line(&mut b, &c, 41, 15, 70, 28);
    tools::smart_line(&mut b, &c, 41, 28, 70, 23);
    c.brush = Brush { ch: '═', fg: Color::Pal(10), ..c.brush };
    tools::smart_ellipse(&mut b, &c, Rect::new(72, 15, 26, 13), ShapeFill::Outline);
    tools::smart_line(&mut b, &c, 27, 22, 39, 28);
    let tx = b.finish();
    d.apply(&tx);
    save(&d, "shapes");
}

fn save(d: &Document, name: &str) {
    let dir = std::path::Path::new("/tmp/acidtrip-pen");
    std::fs::create_dir_all(dir).expect("mkdir");
    let img = render::render_document(d, None, RenderOptions { scale: 1, nine_px: false });
    let path = dir.join(format!("{name}.png"));
    img.save(&path).expect("save png");
    println!("{}", path.display());
    if std::env::var_os("PEN_DEMO_TEXT").is_some() {
        let g = d.flatten();
        for y in 0..g.height {
            println!("{}", g.row(y).iter().map(|c| c.ch).collect::<String>().trim_end());
        }
    }
}

fn main() {
    let sets = acidtrip_core::charsets::builtin();
    let blocks = pen::default_candidates(&sets[5].chars);
    let boxes = pen::default_candidates(&sets[0].chars);
    draw("pen-blocks", pen::RADIUS_BLOCKS, &blocks);
    draw("pen-box", pen::RADIUS_BOX, &boxes);
    shapes();
}
