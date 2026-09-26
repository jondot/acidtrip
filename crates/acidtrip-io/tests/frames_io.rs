//! Animation frames through the file formats: .acid keeps them, ANSI plays
//! and loads them back, GIF and asciicast time them.

use acidtrip_core::{Cell, Color, DocKind, Document, TxBuilder};
use acidtrip_io::format::{Format, GifMode, SaveOptions, frame_grids, load_bytes, save_bytes};

/// Three 10x3 frames: "one", "two" (held 2 ticks), "three" in different colors.
fn animated() -> Document {
    let mut d = Document::new(DocKind::Classic, 10, 3);
    for (i, word) in ["one", "two", "three"].iter().enumerate() {
        if i > 0 {
            let blank = d.blank_frame_canvas();
            let mut b = TxBuilder::new(&d, "frame");
            b.insert_frame(i, blank, if i == 1 { 2 } else { 1 });
            let t = b.finish();
            d.apply(&t);
            d.show_frame(i);
        }
        let mut b = TxBuilder::new(&d, "draw");
        for (x, ch) in word.chars().enumerate() {
            b.set(0, x + i, i, Some(Cell::new(ch, Color::Pal(9 + i as u8), Color::Pal(1))));
        }
        let t = b.finish();
        d.apply(&t);
    }
    d.show_frame(0);
    d
}

#[test]
fn acid_keeps_frames() {
    let mut d = animated();
    d.show_frame(2);
    let back = load_bytes(&save_bytes(&d, Format::Acid, &SaveOptions::default()).unwrap(), Format::Acid).unwrap();
    assert_eq!(back, d);
    assert_eq!(back.frame_count(), 3);
    assert_eq!(back.current_frame(), 2);
    assert_eq!(back.hold(1), 2);
}

#[test]
fn ansi_animation_loads_back_as_frames() {
    let d = animated();
    let opts = SaveOptions::default();
    let bytes = save_bytes(&d, Format::Ansi, &opts).unwrap();
    let back = load_bytes(&bytes, Format::Ansi).unwrap();
    assert_eq!(back.frame_count(), 3);
    assert_eq!((back.hold(0), back.hold(1), back.hold(2)), (1, 2, 1));
    let want = frame_grids(&d, &opts);
    let got = frame_grids(&back, &opts);
    for (i, ((a, _), (b, _))) in want.iter().zip(&got).enumerate() {
        assert_eq!(a.cells, b.cells, "frame {i}");
    }
    // Without frames it's an ordinary still of the frame shown.
    let still = SaveOptions { animate: false, ..SaveOptions::default() };
    let back = load_bytes(&save_bytes(&d, Format::Ansi, &still).unwrap(), Format::Ansi).unwrap();
    assert_eq!(back.frame_count(), 1);
    assert_eq!(back.canvas.composite(0, 0).ch, 'o');
}

#[test]
fn ordinary_ansi_stays_one_frame() {
    for data in [&b"\x1b[2J\x1b[Hhello\r\nworld"[..], b"abc\x1b[1;1Habc", b"no escapes at all"] {
        assert_eq!(load_bytes(data, Format::Ansi).unwrap().frame_count(), 1);
    }
}

#[test]
fn corpus_art_stays_still() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus");
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ans")) {
            let d = load_bytes(&std::fs::read(&p).unwrap(), Format::Ansi).unwrap();
            assert_eq!(d.frame_count(), 1, "{}", p.display());
        }
    }
}

#[test]
fn gif_frames_follow_fps_and_hold() {
    let d = animated();
    let opts = SaveOptions { gif_mode: GifMode::Frames, ..SaveOptions::default() };
    let bytes = save_bytes(&d, Format::Gif, &opts).unwrap();
    let mut o = gif::DecodeOptions::new();
    o.set_color_output(gif::ColorOutput::Indexed);
    let mut dec = o.read_info(bytes.as_slice()).unwrap();
    let mut delays = Vec::new();
    while let Some(f) = dec.read_next_frame().unwrap() {
        delays.push(f.delay);
    }
    // 8 fps: one tick is 12.5 cs.
    assert_eq!(delays, vec![13, 25, 13]);
}

#[test]
fn cast_plays_every_frame() {
    let d = animated();
    let cast = String::from_utf8(save_bytes(&d, Format::Asciicast, &SaveOptions::default()).unwrap()).unwrap();
    let homes = cast.lines().filter(|l| l.contains("\\u001b[H")).count();
    assert!(homes >= 3 && homes % 3 == 0, "{homes} frames drawn");
    assert!(cast.contains("three"));
}
