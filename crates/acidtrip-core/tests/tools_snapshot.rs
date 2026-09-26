mod tools_support;

use acidtrip_core::Color;
use acidtrip_core::tools::{self, BoxStyle, Brush, Ctx, FillMatch, Rect, ShapeFill};
use tools_support::*;

#[test]
fn scene_snapshot() {
    let mut d = doc(40, 12);
    let c = ctx('▓');
    run(&mut d, |b| {
        tools::rect(b, &c, Rect::new(0, 0, 40, 12), ShapeFill::Outline, BoxStyle::Double);
        tools::rect(b, &c, Rect::new(2, 1, 14, 5), ShapeFill::Outline, BoxStyle::Single);
        tools::put_text(b, &c, 4, 3, "acidtrip", false);
        tools::ellipse(
            b,
            &Ctx { brush: Brush { ch: 'o', ..c.brush }, ..c },
            Rect::new(18, 1, 20, 7),
            ShapeFill::Outline,
        );
        tools::flood_fill(b, &Ctx { brush: Brush { ch: '░', ..c.brush }, ..c }, 27, 4, FillMatch::default());
        tools::pixel::ellipse(b, 0, 8, 16, 5, 4, Color::Pal(14), true);
        tools::paint_line(b, &Ctx { brush: Brush { ch: '*', ..c.brush }, ..c }, 18, 9, 37, 10);
    });
    insta::assert_snapshot!(text(&d));
}
