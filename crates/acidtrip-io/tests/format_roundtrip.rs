//! Round trips of synthetic documents through every loadable format.

use acidtrip_core::{Cell, Color, DocKind, Document, Grid, Layer, cp437};
use acidtrip_io::format::{Format, SaveOptions, load_bytes, save_bytes};

/// Tiny deterministic PRNG (xorshift) so tests need no extra deps.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Bytes the ANSI-family formats can't print raw (the saver substitutes them).
fn ansi_safe(b: u8) -> bool {
    !matches!(b, 0x00 | 0x08..=0x0A | 0x0D | 0x1A | 0x1B)
}

fn random_doc(seed: u64, w: usize, h: usize, any_byte: bool) -> Document {
    let mut r = Rng(seed);
    let mut d = Document::new(DocKind::Classic, w, h);
    for c in d.canvas.layers[0].cells.iter_mut() {
        let cell = match r.below(4) {
            0 => Cell::BLANK,
            1 => Cell::new(' ', Color::Pal(r.below(16) as u8), Color::Pal(r.below(16) as u8)),
            _ => {
                let b = loop {
                    let b = r.below(256) as u8;
                    if any_byte || ansi_safe(b) {
                        break b;
                    }
                };
                let fg = Color::Pal(r.below(16) as u8);
                // Runs of equal cells exercise RLE paths.
                let bg = Color::Pal(if r.below(3) == 0 { 0 } else { r.below(16) as u8 });
                Cell::new(cp437::to_char(b), fg, bg)
            }
        };
        *c = Some(cell);
    }
    // A few long runs.
    for y in 0..h.min(4) {
        let c = Cell::new('▒', Color::Pal(y as u8 + 9), Color::Pal(1));
        for x in 0..w.min(30) {
            d.canvas.layers[0].cells[y * w + x] = Some(c);
        }
    }
    d.meta.sauce.title = format!("random {seed}");
    d.meta.sauce.attach = true;
    d
}

/// Visual equality: a space/NUL on black shows nothing, whatever its fg.
fn norm(c: Cell) -> Cell {
    let ch = if c.ch == '\0' { ' ' } else { c.ch };
    if ch == ' ' && c.bg == Color::BLACK { Cell::BLANK } else { Cell { ch, ..c } }
}

pub fn assert_same(a: &Grid, b: &Grid, visual: bool, what: &str) {
    for y in 0..a.height.max(b.height) {
        for x in 0..a.width.max(b.width) {
            let (p, q) = (a.get(x, y), b.get(x, y));
            let ok = if visual { norm(p) == norm(q) } else { p == q };
            assert!(ok, "{what}: cell ({x},{y}) differs: {p:?} vs {q:?}");
        }
    }
}

fn roundtrip(doc: &Document, f: Format, opts: &SaveOptions) -> Document {
    let bytes = save_bytes(doc, f, opts).unwrap_or_else(|e| panic!("save {f:?}: {e}"));
    load_bytes(&bytes, f).unwrap_or_else(|e| panic!("load {f:?}: {e}"))
}

#[test]
fn random_docs_binary_formats_are_exact() {
    let full = SaveOptions { trim_height: false, ..SaveOptions::default() };
    for seed in 1..=12u64 {
        let w = [80, 160, 40, 132][seed as usize % 4];
        let d = random_doc(seed * 7919, w, 30, true);
        for f in [Format::Bin, Format::XBin, Format::Idf, Format::Tnd, Format::Avt] {
            let back = roundtrip(&d, f, &full);
            let visual = f == Format::Avt;
            assert_same(&d.flatten(), &back.flatten(), visual, &format!("{f:?} seed {seed} w {w}"));
            assert_eq!(back.meta.sauce.title, format!("random {}", seed * 7919), "{f:?} keeps SAUCE");
            if f != Format::Tnd {
                assert_eq!(back.width(), w, "{f:?} width");
            }
        }
        if w == 80 {
            let back = roundtrip(&d, Format::Adf, &full);
            assert_same(&d.flatten(), &back.flatten(), false, &format!("Adf seed {seed}"));
        }
    }
}

#[test]
fn random_docs_ansi_family() {
    for seed in 1..=12u64 {
        let w = [80, 160, 40, 100][seed as usize % 4];
        let d = random_doc(seed * 104729, w, 40, false);
        for (f, opts) in [
            (Format::Ansi, SaveOptions::default()),
            (
                Format::Ansi,
                SaveOptions { line_length: Some(79), clear_screen: true, ice_hint: true, ..SaveOptions::default() },
            ),
            (
                Format::Ansi,
                SaveOptions { sauce: Some(true), eof_char: false, trim_height: false, ..SaveOptions::default() },
            ),
            (Format::Pcb, SaveOptions::default()),
            (Format::Avt, SaveOptions::default()),
            (Format::Utf8Ansi, SaveOptions::default()),
        ] {
            let back = roundtrip(&d, f, &opts);
            assert_same(&d.flatten(), &back.flatten(), true, &format!("{f:?} {opts:?} seed {seed} w {w}"));
            if f != Format::Utf8Ansi {
                assert_eq!(back.meta.kind, DocKind::Classic);
                assert_eq!(back.width(), w);
            }
        }
    }
}

#[test]
fn ansi_rgb_in_classic_uses_pablo_codes() {
    let mut d = Document::new(DocKind::Classic, 80, 1);
    d.canvas.layers[0].cells[0] = Some(Cell::new('A', Color::Rgb(1, 2, 3), Color::Rgb(200, 100, 50)));
    d.canvas.layers[0].cells[1] = Some(Cell::new('B', Color::Pal(9), Color::Pal(0)));
    let bytes = save_bytes(&d, Format::Ansi, &SaveOptions::default()).unwrap();
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.contains("\x1b[1;1;2;3t") && s.contains("\x1b[0;200;100;50t"), "{s:?}");
    let back = load_bytes(&bytes, Format::Ansi).unwrap();
    assert_eq!(back.meta.kind, DocKind::Modern, "real RGB makes the loaded doc Modern");
    assert_same(&d.flatten(), &back.flatten(), false, "pablo");
}

#[test]
fn modern_doc_to_ansi_is_downsampled() {
    let mut d = Document::new(DocKind::Modern, 80, 1);
    d.canvas.layers[0].cells[0] = Some(Cell::new('╭', Color::Rgb(250, 250, 90), Color::Rgb(0, 0, 170)));
    let back = roundtrip(&d, Format::Ansi, &SaveOptions::default());
    assert_eq!(back.meta.kind, DocKind::Classic);
    assert_eq!(back.flatten().get(0, 0), Cell::new('┌', Color::Pal(14), Color::Pal(1)));
    // TundraDraw keeps the truecolor.
    let back = roundtrip(&d, Format::Tnd, &SaveOptions::default());
    assert_eq!(back.flatten().get(0, 0).fg, Color::Rgb(250, 250, 90));
    // UTF-8 ANSI keeps both.
    let back = roundtrip(&d, Format::Utf8Ansi, &SaveOptions::default());
    let (a, b) = (back.flatten().get(0, 0), d.flatten().get(0, 0));
    let pal = &d.meta.palette;
    assert_eq!((a.ch, a.fg.rgb(pal), a.bg.rgb(pal)), (b.ch, b.fg.rgb(pal), b.bg.rgb(pal)));
}

#[test]
fn acid_roundtrip_keeps_everything() {
    let mut d = random_doc(42, 90, 12, true);
    let mut top = Layer::new("top", 90, 12);
    top.cells[5] = Some(Cell::new('x', Color::Rgb(1, 2, 3), Color::BLACK));
    top.visible = false;
    d.canvas.layers.push(top);
    d.meta.sauce.comments = vec!["a".into(), "b".into()];
    d.meta.letter_spacing_9px = true;
    let back = roundtrip(&d, Format::Acid, &SaveOptions::default());
    assert_eq!(back, d);
}

#[test]
fn save_is_atomic_and_load_detects_by_extension_and_magic() {
    let dir = tempfile::tempdir().unwrap();
    let d = random_doc(3, 80, 10, false);
    for f in [Format::Acid, Format::XBin, Format::Idf, Format::Tnd, Format::Ansi, Format::Bin, Format::Adf] {
        let p = dir.path().join(format!("art.{}", f.extensions()[0]));
        acidtrip_io::format::save(&d, &p, f, &SaveOptions { trim_height: false, ..SaveOptions::default() }).unwrap();
        let back = acidtrip_io::format::load(&p).unwrap();
        assert_same(&d.flatten(), &back.flatten(), f == Format::Ansi, &format!("{f:?} via path"));
        if matches!(f, Format::Acid | Format::XBin | Format::Idf | Format::Tnd) {
            // No usable extension → sniffed from magic bytes.
            let q = dir.path().join(format!("noext{}", f.extensions()[0]));
            std::fs::copy(&p, &q).unwrap();
            let back = acidtrip_io::format::load(&q).unwrap();
            assert_same(&d.flatten(), &back.flatten(), false, &format!("{f:?} sniffed"));
        }
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 11, "no temp files left behind");
}

#[test]
fn ascii_roundtrip() {
    let mut d = Document::new(DocKind::Classic, 80, 3);
    for (i, ch) in "Hello ░▒▓ world".chars().enumerate() {
        d.canvas.layers[0].cells[80 + i] = Some(Cell { ch, ..Cell::BLANK });
    }
    let bytes = save_bytes(&d, Format::Ascii, &SaveOptions::default()).unwrap();
    assert_eq!(bytes, b"\r\nHello \xb0\xb1\xb2 world");
    let back = load_bytes(&bytes, Format::Ascii).unwrap();
    assert_same(&d.flatten(), &back.flatten(), false, "ascii");
}

#[test]
fn images_do_not_reopen_as_the_same_document() {
    // An opened PNG is an image import: Ctrl-S must not resave over it.
    assert!(!Format::Png.reopens());
    assert!(!Format::Gif.reopens());
    assert!(!Format::Svg.reopens());
    for f in [Format::Acid, Format::Ansi, Format::XBin, Format::Bin, Format::Adf, Format::Idf, Format::Tnd] {
        assert!(f.reopens(), "{f:?}");
    }
}
