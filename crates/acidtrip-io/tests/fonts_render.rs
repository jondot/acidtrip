use std::path::PathBuf;

use acidtrip_core::{Clip, Color};
use acidtrip_io::fonts::{FontKind, FontLibrary, TextRenderOptions};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tdf")
}

fn plain(c: &Clip) -> String {
    (0..c.height)
        .map(|y| {
            (0..c.width).map(|x| c.get(x, y).map_or(' ', |cell| cell.ch)).collect::<String>().trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const FIGLET: [&str; 16] = [
    "standard", "slant", "small", "big", "banner", "block", "bubble", "digital", "lean", "mini", "script", "shadow",
    "smscript", "smshadow", "smslant", "term",
];

#[test]
fn bundled_figlet_fonts_all_load() {
    let lib = FontLibrary::load(None);
    let list = lib.list();
    for id in FIGLET {
        let f = lib.find(id).unwrap_or_else(|| panic!("{id} missing"));
        assert_eq!(f.kind, FontKind::Figlet);
        assert_eq!(f.path, None);
        assert!(f.charset.contains('A') && f.charset.contains('z'), "{id}: {}", f.charset);
        let c = lib.render(id, "Hi", &TextRenderOptions::default()).unwrap();
        assert!(c.cells.iter().any(Option::is_some), "{id} rendered nothing");
    }
    assert_eq!(list.len(), FIGLET.len());
    assert!(lib.find("STANDARD").is_some());
    assert!(lib.find("nope").is_none());
    assert!(lib.render("nope", "x", &TextRenderOptions::default()).is_err());
}

#[test]
fn standard_acid_snapshot() {
    let lib = FontLibrary::load(None);
    let c = lib.render("standard", "ACiD", &TextRenderOptions::default()).unwrap();
    assert_eq!(c.height, 6);
    insta::assert_snapshot!(plain(&c));
    let cell = c.cells.iter().flatten().next().unwrap();
    assert_eq!((cell.fg, cell.bg), (Color::WHITE, Color::BLACK));
}

#[test]
fn figlet_uses_opts_colors_and_multiline() {
    let lib = FontLibrary::load(None);
    let opts = TextRenderOptions { fg: Color::Pal(12), bg: Color::Pal(1), ..Default::default() };
    let c = lib.render("small", "A\nBC", &opts).unwrap();
    let h = lib.render("small", "A", &opts).unwrap().height;
    assert_eq!(c.height, 2 * h);
    assert!(c.cells.iter().flatten().all(|cell| cell.fg == Color::Pal(12) && cell.bg == Color::Pal(1)));
    assert!((h..2 * h).any(|y| (0..c.width).any(|x| c.get(x, y).is_some())), "second line drawn");
}

#[test]
fn missing_chars_leave_one_column_gap() {
    let lib = FontLibrary::load(None);
    let o = TextRenderOptions::default();
    let a = lib.render("standard", "A", &o).unwrap().width;
    assert_eq!(lib.render("standard", "A\u{263a}A", &o).unwrap().width, 2 * a + 1);
    assert_eq!(lib.render("standard", "", &o).unwrap().cells.len(), 0);
}

#[test]
fn tdf_fixtures_render() {
    let lib = FontLibrary::load(Some(&fixtures()));
    let tdf: Vec<_> = lib.list().into_iter().filter(|f| f.path.is_some()).collect();
    assert!(tdf.len() >= 8, "{tdf:?}");
    let mut kinds = Vec::new();
    let mut multicolor = false;
    for f in &tdf {
        kinds.push(f.kind);
        assert!(f.path.as_ref().unwrap().starts_with(fixtures()));
        let opts = TextRenderOptions { fg: Color::Pal(10), bg: Color::Pal(0), spacing: 2, ..Default::default() };
        let c = lib.render(&f.id, "ACiD", &opts).unwrap();
        let drawn: Vec<_> = c.cells.iter().flatten().collect();
        assert!(!drawn.is_empty(), "{} rendered nothing", f.id);
        assert!(c.cells.iter().any(Option::is_none), "{} has no transparency", f.id);
        match f.kind {
            FontKind::TdfColor => {
                let colors: std::collections::HashSet<_> = drawn.iter().map(|c| (c.fg, c.bg)).collect();
                multicolor |= colors.len() > 1;
                assert!(drawn.iter().all(|c| matches!(c.fg, Color::Pal(0..=15)) && matches!(c.bg, Color::Pal(0..=7))));
                let other = TextRenderOptions { fg: Color::Pal(4), bg: Color::Pal(1), ..opts.clone() };
                assert_eq!(lib.render(&f.id, "ACiD", &other).unwrap(), c, "{}: color fonts ignore opts colors", f.id);
            }
            _ => assert!(drawn.iter().all(|c| c.fg == Color::Pal(10)), "{} ignores opts.fg", f.id),
        }
        // A fully transparent gap column between the two letters.
        let wa = lib.render(&f.id, "A", &opts).unwrap().width;
        let two = lib.render(&f.id, "AA", &opts).unwrap();
        assert!(two.width > wa + 1);
        assert!((0..two.height).all(|y| two.get(wa, y).is_none()), "{}: no gap after first letter", f.id);
    }
    assert!(kinds.contains(&FontKind::TdfColor));
    assert!(multicolor, "no fixture color font rendered more than one color");
}

/// Block and Outline TDF fonts built in memory (the fixtures are all Color fonts).
fn synthetic_fonts(dir: &std::path::Path) {
    use retrofont::tdf::{TdfFont, TdfFontType};
    use retrofont::{Glyph, GlyphPart};
    let chars = |s: &str| {
        let mut parts = Vec::new();
        for (i, row) in s.split('\n').enumerate() {
            if i > 0 {
                parts.push(GlyphPart::NewLine);
            }
            parts.extend(row.chars().map(GlyphPart::Char));
        }
        parts
    };
    let mut block = TdfFont::new("Blocky", TdfFontType::Block, 1);
    block.add_glyph('A', Glyph { width: 3, height: 2, parts: chars("█▀█\n█▀█") });
    block.add_glyph('B', Glyph { width: 3, height: 2, parts: chars("█▀▄\n█▄▀") });
    let mut outline = TdfFont::new("Outlined", TdfFontType::Outline, 1);
    let p = GlyphPart::OutlinePlaceholder;
    outline.add_glyph(
        'A',
        Glyph {
            width: 3,
            height: 2,
            parts: vec![p(b'E'), p(b'A'), p(b'F'), GlyphPart::NewLine, p(b'I'), p(b'A'), p(b'J')],
        },
    );
    std::fs::write(dir.join("synth.tdf"), TdfFont::serialize_bundle(&[block, outline]).unwrap()).unwrap();
}

#[test]
fn block_and_outline_fonts_use_opts_colors() {
    let tmp = tempfile::tempdir().unwrap();
    synthetic_fonts(tmp.path());
    let lib = FontLibrary::load(Some(tmp.path()));
    let block = lib.find("Blocky").unwrap();
    let outline = lib.find("Outlined").unwrap();
    assert_eq!((block.kind, block.id.as_str(), block.index), (FontKind::TdfBlock, "synth#0", 0));
    assert_eq!((outline.kind, outline.id.as_str(), outline.index), (FontKind::TdfOutline, "synth#1", 1));
    assert_eq!(block.charset, "AB");

    let opts = TextRenderOptions { fg: Color::Pal(14), bg: Color::Pal(4), ..Default::default() };
    let c = lib.render("synth#0", "AB a", &opts).unwrap();
    // A(3) + gap(1) + B(3) + gap(1) + space + gap(1) + a->A(3)
    assert_eq!(plain(&c).lines().next().unwrap().chars().filter(|&ch| ch != ' ').count(), 9);
    assert!(c.cells.iter().flatten().all(|cell| cell.fg == Color::Pal(14) && cell.bg == Color::Pal(4)));
    assert!((0..2).all(|y| c.get(3, y).is_none()), "gap is transparent");
    assert_eq!(c.get(4, 0).unwrap().ch, '█');

    let tight = lib.render("synth#0", "AB", &TextRenderOptions { spacing: -1, ..Default::default() }).unwrap();
    assert_eq!(tight.width, 6);

    let o0 = lib.render("synth#1", "A", &opts).unwrap();
    assert_eq!(plain(&o0), "┌─┐\n└─┘");
    let o15 = lib.render("synth#1", "A", &TextRenderOptions { outline_style: 15, ..opts.clone() }).unwrap();
    assert_eq!(plain(&o15), "╔═╗\n╚═╝");
    assert_eq!(o0.get(1, 0).unwrap().fg, Color::Pal(14));
}

#[test]
fn install_copies_and_reloads() {
    let tmp = tempfile::tempdir().unwrap();
    let mut lib = FontLibrary::load(Some(tmp.path()));
    let before = lib.list().len();
    let got = lib.install(&fixtures().join("fire.tdf"), tmp.path()).unwrap();
    assert!(!got.is_empty());
    assert!(tmp.path().join("fire.tdf").is_file());
    assert_eq!(lib.list().len(), before + got.len());
    assert!(lib.render(&got[0].id, "A", &TextRenderOptions::default()).is_ok());
    let junk = tmp.path().join("junk.tdf");
    std::fs::write(&junk, "not a font").unwrap();
    assert!(lib.install(&junk, &tmp.path().join("other")).is_err());
}

#[test]
fn zip_archives_are_indexed() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let mut z = zip::ZipWriter::new(std::fs::File::create(tmp.path().join("pack.zip")).unwrap());
    let o = zip::write::SimpleFileOptions::default();
    z.start_file("pack/fonts/neon.tdf", o).unwrap();
    z.write_all(&std::fs::read(fixtures().join("neon.tdf")).unwrap()).unwrap();
    z.start_file("pack/readme.txt", o).unwrap();
    z.write_all(b"hi").unwrap();
    z.finish().unwrap();
    let lib = FontLibrary::load(Some(tmp.path()));
    let from_zip: Vec<_> = lib.list().into_iter().filter(|f| f.path.is_some()).collect();
    assert!(!from_zip.is_empty());
    assert!(from_zip.iter().all(|f| f.id.starts_with("neon") && f.path.as_ref().unwrap().ends_with("pack.zip")));

    // The same zip also works as a download pack.
    let out = tmp.path().join("out");
    let n = acidtrip_io::fonts::extract_pack(&std::fs::read(tmp.path().join("pack.zip")).unwrap(), &out).unwrap();
    assert_eq!(n, 1);
    assert!(out.join("neon.tdf").is_file());
}

#[test]
#[ignore = "network"]
fn download_packs_fetches_tdf_fonts() {
    let tmp = tempfile::tempdir().unwrap();
    let n = acidtrip_io::fonts::download_packs(tmp.path()).unwrap();
    assert!(n > 500, "{n}");
    let lib = FontLibrary::load(Some(tmp.path()));
    assert!(lib.list().len() > n);
}
