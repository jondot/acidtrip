//! Real art: every corpus file loads, and survives ANSI and the classic
//! binary/stream formats cell-for-cell.

use std::path::{Path, PathBuf};

use acidtrip_core::{Cell, Color, DocKind, Document, Grid};
use acidtrip_io::format::{self, Format, SaveOptions, load_bytes, save_bytes};

fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ans")))
        .collect();
    v.sort();
    assert!(v.len() >= 10, "corpus missing");
    v
}

fn norm(c: Cell) -> Cell {
    let ch = if c.ch == '\0' { ' ' } else { c.ch };
    if ch == ' ' && c.bg == Color::BLACK { Cell::BLANK } else { Cell { ch, ..c } }
}

fn assert_same(a: &Grid, b: &Grid, visual: bool, what: &str) {
    let mut diffs = 0;
    for y in 0..a.height.max(b.height) {
        for x in 0..a.width.max(b.width) {
            let (p, q) = (a.get(x, y), b.get(x, y));
            if (visual && norm(p) != norm(q)) || (!visual && p != q) {
                if diffs < 5 {
                    eprintln!("{what}: ({x},{y}) {p:?} vs {q:?}");
                }
                diffs += 1;
            }
        }
    }
    assert_eq!(diffs, 0, "{what}: {diffs} cells differ");
}

fn name(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().into()
}

#[test]
fn corpus_ans_loads_and_roundtrips() {
    for p in corpus() {
        let d = format::load(&p).unwrap_or_else(|e| panic!("{}: {e:#}", name(&p)));
        assert!(d.width() >= 80 && d.height() >= 25, "{}", name(&p));
        assert!(d.canvas.used_height() > 5, "{} looks empty", name(&p));
        for opts in [
            SaveOptions::default(),
            SaveOptions { line_length: Some(250), sauce: Some(true), ..SaveOptions::default() },
        ] {
            let bytes = save_bytes(&d, Format::Ansi, &opts).unwrap();
            let back = load_bytes(&bytes, Format::Ansi).unwrap();
            assert_same(&d.flatten(), &back.flatten(), true, &format!("{} ANS→ANS {:?}", name(&p), opts.line_length));
            assert_eq!(back.meta.sauce.title, d.meta.sauce.title);
            assert_eq!(back.meta.sauce.author, d.meta.sauce.author);
            assert_eq!(back.meta.kind, d.meta.kind);
        }
    }
}

#[test]
fn corpus_ans_through_every_classic_format() {
    let full = SaveOptions { trim_height: false, ..SaveOptions::default() };
    let mut tested = 0;
    for p in corpus() {
        let d = format::load(&p).unwrap();
        if d.meta.kind != DocKind::Classic {
            continue;
        }
        tested += 1;
        for f in [Format::XBin, Format::Bin, Format::Adf, Format::Idf, Format::Tnd, Format::Avt, Format::Pcb] {
            if f == Format::Adf && d.width() != 80 {
                continue;
            }
            let bytes = save_bytes(&d, f, &full).unwrap();
            let back = load_bytes(&bytes, f).unwrap_or_else(|e| panic!("{} {f:?}: {e:#}", name(&p)));
            // Stream formats trim trailing blanks per line (their fg is invisible).
            let visual = matches!(f, Format::Avt | Format::Pcb);
            assert_same(&d.flatten(), &back.flatten(), visual, &format!("{} ANS→{f:?}", name(&p)));
            assert_eq!(back.meta.kind, DocKind::Classic, "{f:?}");
        }
    }
    assert!(tested >= 10);
}

#[test]
fn corpus_ascii_loads() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/DR-HEART.ASC");
    assert_eq!(Format::from_path(&p), Some(Format::Ascii));
    let d = format::load(&p).unwrap();
    assert!(d.canvas.used_height() > 10);
    let back = load_bytes(&save_bytes(&d, Format::Ascii, &SaveOptions::default()).unwrap(), Format::Ascii).unwrap();
    assert_same(&d.flatten(), &back.flatten(), true, "ASC→ASC");
}

#[test]
fn exports_of_corpus_are_sane() {
    let p = corpus().into_iter().find(|p| name(p) == "LO-LES4.ANS").unwrap();
    let d: Document = format::load(&p).unwrap();
    let rows = d.canvas.used_height();
    for f in [Format::Png, Format::Svg, Format::Html, Format::React, Format::Asciicast, Format::Utf8Ansi, Format::Mirc]
    {
        let b = save_bytes(&d, f, &SaveOptions::default()).unwrap();
        assert!(!b.is_empty(), "{f:?}");
    }
    let png = save_bytes(&d, Format::Png, &SaveOptions { scale: 2, ..SaveOptions::default() }).unwrap();
    let img = image::load_from_memory(&png).unwrap();
    assert_eq!((img.width(), img.height()), (d.width() as u32 * 16, rows as u32 * 32));
}

/// Loads the ansilove textmode corpus from a local clone (it carries no
/// license, so it isn't vendored). `ACIDTRIP_TEXTMODE_CORPUS=/path cargo test -- --ignored`.
#[test]
#[ignore]
fn textmode_corpus() {
    let root = std::env::var("ACIDTRIP_TEXTMODE_CORPUS")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/textmode-corpus").into());
    let mut loaded = 0;
    for (dir, f) in [
        ("xb", Format::XBin),
        ("adf", Format::Adf),
        ("idf", Format::Idf),
        ("tnd", Format::Tnd),
        ("pcb", Format::Pcb),
        ("ans", Format::Ansi),
    ] {
        let Ok(rd) = std::fs::read_dir(Path::new(&root).join(dir)) else {
            continue;
        };
        for e in rd {
            let p = e.unwrap().path();
            let bytes = std::fs::read(&p).unwrap();
            let d = load_bytes(&bytes, f).unwrap_or_else(|e| panic!("{}: {e:#}", p.display()));
            assert!(d.width() > 0 && d.height() > 0);
            assert!(d.canvas.used_height() > 0, "{} is empty", p.display());
            // Same-format round trip (24-bit ANSI loads as Modern, which .ans
            // downsamples by design, so those go through UTF-8 ANSI instead).
            let opts = SaveOptions { trim_height: false, ..SaveOptions::default() };
            let f = if f == Format::Ansi && d.meta.kind == DocKind::Modern { Format::Utf8Ansi } else { f };
            let back = load_bytes(&save_bytes(&d, f, &opts).unwrap(), f).unwrap();
            let visual = matches!(f, Format::Ansi | Format::Pcb | Format::Utf8Ansi);
            assert_same(&d.flatten(), &back.flatten(), visual, &format!("{} {f:?} round trip", p.display()));
            loaded += 1;
        }
    }
    eprintln!("loaded {loaded} textmode corpus files");
    assert!(loaded > 0, "corpus not found at {root}");
}
