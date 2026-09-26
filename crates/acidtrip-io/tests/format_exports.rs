//! Exports: dimensions, well-formedness and small snapshots.

use acidtrip_core::{Cell, Color, DocKind, Document};
use acidtrip_io::format::{Format, GifMode, SaveOptions, save_bytes};

fn tiny() -> Document {
    let mut d = Document::new(DocKind::Classic, 6, 2);
    let cells = [
        Cell::new('A', Color::Pal(12), Color::Pal(1)),
        Cell::new('█', Color::Pal(14), Color::BLACK),
        Cell::new('▀', Color::Pal(10), Color::Pal(4)),
        Cell::new('<', Color::Pal(15), Color::BLACK),
        Cell::new('&', Color::Pal(15), Color::BLACK),
    ];
    for (i, c) in cells.into_iter().enumerate() {
        d.canvas.layers[0].cells[i] = Some(c);
    }
    d.canvas.layers[0].cells[7] = Some(Cell::new('░', Color::Pal(3), Color::Pal(9)));
    d.meta.sauce.title = "Tiny <test>".into();
    d
}

fn text(f: Format, opts: &SaveOptions) -> String {
    String::from_utf8(save_bytes(&tiny(), f, opts).unwrap()).unwrap()
}

#[test]
fn png_dimensions() {
    let mut d = tiny();
    for (scale, nine) in [(1, false), (3, false), (2, true)] {
        d.meta.letter_spacing_9px = nine;
        let opts = SaveOptions { scale, ..SaveOptions::default() };
        let img = image::load_from_memory(&save_bytes(&d, Format::Png, &opts).unwrap()).unwrap();
        let cw = if nine { 9 } else { 8 };
        assert_eq!((img.width(), img.height()), (6 * cw * scale, 2 * 16 * scale));
    }
    let full = SaveOptions { trim_height: false, ..SaveOptions::default() };
    let mut tall = Document::new(DocKind::Classic, 80, 30);
    tall.canvas.layers[0].cells[0] = Some(Cell::new('x', Color::WHITE, Color::BLACK));
    let img = image::load_from_memory(&save_bytes(&tall, Format::Png, &full).unwrap()).unwrap();
    assert_eq!(img.height(), 30 * 16);
    let img = image::load_from_memory(&save_bytes(&tall, Format::Png, &SaveOptions::default()).unwrap()).unwrap();
    assert_eq!(img.height(), 16, "trimmed to used rows");
}

#[test]
fn gif_still_and_reveal() {
    let g = save_bytes(&tiny(), Format::Gif, &SaveOptions::default()).unwrap();
    assert_eq!(&g[..6], b"GIF89a");
    let r = save_bytes(
        &tiny(),
        Format::Gif,
        &SaveOptions { gif_mode: GifMode::Reveal, baud: 300, ..SaveOptions::default() },
    )
    .unwrap();
    assert!(r.len() > g.len());
}

#[test]
fn svg_is_well_formed_and_scales() {
    for exact in [false, true] {
        let s = text(Format::Svg, &SaveOptions { svg_pixel_exact: exact, scale: 2, ..SaveOptions::default() });
        let doc = roxmltree::Document::parse(&s).unwrap_or_else(|e| panic!("exact={exact}: {e}\n{s}"));
        let root = doc.root_element();
        assert_eq!(root.attribute("viewBox"), Some("0 0 48 32"));
        assert_eq!(root.attribute("width"), Some("96"));
        assert!(s.contains("Tiny &lt;test&gt;"));
        if exact {
            assert!(doc.descendants().any(|n| n.has_tag_name("symbol")));
            assert!(!doc.descendants().any(|n| n.has_tag_name("text")));
        } else {
            assert!(s.contains("Perfect DOS VGA 437") && s.contains(r#"xml:space="preserve""#));
            assert!(doc.descendants().any(|n| n.has_tag_name("text") && n.text() == Some("A")));
        }
    }
}

#[test]
fn html_is_self_contained() {
    let h = text(Format::Html, &SaveOptions::default());
    assert!(h.starts_with("<!DOCTYPE html>"));
    assert!(h.contains("<title>Tiny &lt;test&gt;</title>"));
    assert!(h.contains(r#"<pre class="acid""#));
    assert_eq!(h.matches("<span").count(), h.matches("</span>").count());
    assert!(!h.contains("<script") && !h.contains("<link"), "no external resources");
    assert!(h.contains("&lt;&amp;"));
}

#[test]
fn react_component_shape() {
    let t = text(Format::React, &SaveOptions { identifier: "logo-art".into(), ..SaveOptions::default() });
    assert!(t.contains(
        "export function Logo_art(props: { className?: string; scale?: number; style?: React.CSSProperties })"
    ));
    assert!(t.contains("import * as React from \"react\";"));
    assert_eq!(t.matches('{').count(), t.matches('}').count());
}

/// Optional: `ACIDTRIP_CHECK_TSX=1` parses the component with esbuild via npx.
#[test]
fn react_component_parses_with_esbuild() {
    if std::env::var("ACIDTRIP_CHECK_TSX").is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("Art.tsx");
    std::fs::write(&p, text(Format::React, &SaveOptions::default())).unwrap();
    let st = std::process::Command::new("npx").args(["-y", "esbuild", "--loader:.tsx=tsx"]).arg(&p).output().unwrap();
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
}

#[test]
fn asciicast_header_and_events() {
    let c = text(Format::Asciicast, &SaveOptions { baud: 2400, ..SaveOptions::default() });
    let mut lines = c.lines();
    let h: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    assert_eq!((h["version"].as_u64(), h["width"].as_u64(), h["height"].as_u64()), (Some(2), Some(6), Some(2)));
    let events: Vec<serde_json::Value> = lines.map(|l| serde_json::from_str(l).unwrap()).collect();
    assert!(events.len() > 2);
    let out: String = events.iter().map(|e| e[2].as_str().unwrap()).collect();
    assert!(out.contains("\x1b[0m\r\n"));
    let times: Vec<f64> = events.iter().map(|e| e[0].as_f64().unwrap()).collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn snapshots() {
    let o = SaveOptions::default();
    let esc = |f: Format| {
        format!(
            "{:?}",
            String::from_utf8_lossy(&save_bytes(&tiny(), f, &SaveOptions { sauce: Some(false), ..o.clone() }).unwrap())
        )
    };
    insta::assert_snapshot!("ansi", esc(Format::Ansi));
    insta::assert_snapshot!("utf8ansi", esc(Format::Utf8Ansi));
    insta::assert_snapshot!("pcb", esc(Format::Pcb));
    insta::assert_snapshot!("avt", esc(Format::Avt));
    insta::assert_snapshot!("mirc", esc(Format::Mirc));
    insta::assert_snapshot!("c_array", text(Format::CArray, &o));
    insta::assert_snapshot!("pascal_array", text(Format::PascalArray, &o));
    insta::assert_snapshot!("asm_array", text(Format::AsmArray, &o));
    insta::assert_snapshot!("svg_text", text(Format::Svg, &o));
    insta::assert_snapshot!("svg_exact", text(Format::Svg, &SaveOptions { svg_pixel_exact: true, ..o.clone() }));
    insta::assert_snapshot!("html", text(Format::Html, &o));
    insta::assert_snapshot!("react", text(Format::React, &o));
}
