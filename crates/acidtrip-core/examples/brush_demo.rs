//! Print every brush preset on a wave: `cargo run -p acidtrip-core --example brush_demo [name]`.

use acidtrip_core::tools::brush::presets;
use acidtrip_core::tools::pen::{self, PenStroke};
use acidtrip_core::tools::{Brush, Ctx};
use acidtrip_core::{Color, DocKind, Document, TxBuilder, charsets};

fn main() {
    let only = std::env::args().nth(1);
    let pts: Vec<(f32, f32)> = (0..=100)
        .map(|i| i as f32 / 100.0)
        .map(|t| (12.0 + t * 296.0, 64.0 + (t * std::f32::consts::TAU).sin() * 36.0))
        .collect();
    let ctx = Ctx { brush: Brush { ch: '█', fg: Color::WHITE, bg: Color::BLACK }, ..Ctx::default() };
    for spec in presets() {
        if only.as_ref().is_some_and(|n| !spec.name.eq_ignore_ascii_case(n)) {
            continue;
        }
        let cands = spec.glyphs.candidates(&charsets::builtin()[5].chars);
        let mut d = Document::new(DocKind::Classic, 40, 8);
        let mut b = TxBuilder::new(&d, "demo");
        let mut s = PenStroke::with_brush(&spec);
        for &(x, y) in &pts {
            let c = s.add_point(x, y);
            pen::apply(&mut b, &ctx, &s, c, &cands);
        }
        let c = s.finish();
        pen::apply(&mut b, &ctx, &s, c, &cands);
        let tx = b.finish();
        d.apply(&tx);
        let g = d.flatten();
        println!("── {}", spec.name);
        for y in 0..g.height {
            println!("{}", g.row(y).iter().map(|c| c.ch).collect::<String>().trim_end());
        }
    }
}
