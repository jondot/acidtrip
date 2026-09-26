//! Animation frames: frame operations as transactions, edits that find
//! their frame, undo, shared undo, replay and file compatibility.

use acidtrip_core::replay::{Timeline, TimelineOptions};
use acidtrip_core::tools;
use acidtrip_core::{Cell, Color, DocKind, Document, History, Transaction, TxBuilder};

fn cell(ch: char) -> Cell {
    Cell::new(ch, Color::WHITE, Color::BLACK)
}

fn tx(d: &Document, f: impl FnOnce(&mut TxBuilder)) -> Transaction {
    let mut b = TxBuilder::new(d, "t");
    f(&mut b);
    b.finish()
}

fn put(d: &Document, x: usize, ch: char) -> Transaction {
    tx(d, |b| b.set(0, x, 0, Some(cell(ch))))
}

fn row(d: &Document, i: usize) -> String {
    let c = d.frame_canvas(i);
    (0..c.width).map(|x| c.composite(x, 0).ch).collect()
}

/// Two frames: "A   " and "B   ", showing the first.
fn two_frames(h: &mut History) -> Document {
    let mut d = Document::new(DocKind::Classic, 4, 1);
    let t = put(&d, 0, 'A');
    h.commit(&mut d, t);
    let blank = d.blank_frame_canvas();
    let t = tx(&d, |b| {
        b.insert_frame(1, blank, 1);
    });
    h.commit(&mut d, t);
    d.show_frame(1);
    let t = put(&d, 0, 'B');
    h.commit(&mut d, t);
    d.show_frame(0);
    d
}

#[test]
fn one_frame_documents_serialize_as_before() {
    let d = Document::new(DocKind::Classic, 3, 2);
    let json = serde_json::to_value(&d).unwrap();
    assert!(json.get("frames").is_none());
    let t = put(&d, 1, 'x');
    let tj = serde_json::to_string(&t).unwrap();
    assert!(!tj.contains("frame"), "{tj}");
    // A file written before frames existed loads as one frame.
    let old = serde_json::json!({ "meta": json["meta"], "canvas": json["canvas"] });
    let back: Document = serde_json::from_value(old).unwrap();
    assert_eq!(back, d);
    assert_eq!(back.frame_count(), 1);
    // And so does an old transaction.
    let back: Transaction = serde_json::from_str(&tj).unwrap();
    assert_eq!(back, t);
}

#[test]
fn frames_survive_serialization() {
    let mut h = History::new();
    let mut d = two_frames(&mut h);
    d.show_frame(1);
    let json = serde_json::to_string(&d).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back, d);
    assert_eq!(back.current_frame(), 1);
    assert_eq!(row(&back, 0), "A   ");
    assert_eq!(row(&back, 1), "B   ");
}

#[test]
fn switching_frames_is_not_an_edit() {
    let mut h = History::new();
    let mut d = two_frames(&mut h);
    let before = d.clone();
    let rev = h.revision();
    assert!(d.show_frame(1));
    assert_eq!(h.revision(), rev);
    assert_eq!(d, before, "which frame is shown isn't part of the document");
    assert_eq!(d.canvas.composite(0, 0).ch, 'B');
}

#[test]
fn undo_after_a_switch_hits_the_right_frame() {
    let mut h = History::new();
    let mut d = two_frames(&mut h);
    // Showing frame 0; the last edit was on frame 1.
    let id1 = d.frames.list[1].id;
    assert_eq!(h.undo_frame(), Some(id1));
    h.undo(&mut d);
    assert_eq!(row(&d, 0), "A   ");
    assert_eq!(row(&d, 1), "    ");
    h.redo(&mut d);
    assert_eq!(row(&d, 1), "B   ");
    // Undo the insert, then the first stroke.
    h.undo(&mut d);
    h.undo(&mut d);
    assert_eq!(d.frame_count(), 1);
    h.undo(&mut d);
    assert_eq!(row(&d, 0), "    ");
    for _ in 0..3 {
        h.redo(&mut d);
    }
    assert_eq!(d.frame_count(), 2);
    assert_eq!(row(&d, 1), "B   ");
}

#[test]
fn frame_ops_undo_and_redo() {
    let mut h = History::new();
    let mut d = two_frames(&mut h);
    let orig = d.clone();
    // Duplicate frame 0 to the end, move it first, hold it, change fps.
    let copy = d.frame_canvas(0).clone();
    let t = tx(&d, |b| {
        b.insert_frame(2, copy, 1);
    });
    h.commit(&mut d, t);
    let t = tx(&d, |b| b.move_frame(2, 0));
    h.commit(&mut d, t);
    let t = tx(&d, |b| b.set_hold(0, 3));
    h.commit(&mut d, t);
    let t = tx(&d, |b| b.set_fps(12));
    h.commit(&mut d, t);
    assert_eq!(d.frame_count(), 3);
    assert_eq!(d.hold(0), 3);
    assert_eq!(d.fps(), 12);
    assert_eq!((row(&d, 0), row(&d, 1), row(&d, 2)), ("A   ".into(), "A   ".into(), "B   ".into()));
    let done = d.clone();
    for _ in 0..4 {
        h.undo(&mut d);
    }
    assert_eq!(d, orig);
    for _ in 0..4 {
        h.redo(&mut d);
    }
    assert_eq!(d, done);
}

#[test]
fn deleting_the_shown_frame_moves_to_a_neighbour() {
    let mut h = History::new();
    let mut d = two_frames(&mut h);
    let orig = d.clone();
    let t = tx(&d, |b| b.remove_frame(0));
    h.commit(&mut d, t);
    assert_eq!(d.frame_count(), 1);
    assert_eq!(d.canvas.composite(0, 0).ch, 'B');
    let t = tx(&d, |b| b.remove_frame(0));
    assert!(t.is_empty(), "the last frame stays");
    h.undo(&mut d);
    assert_eq!(d, orig);
}

#[test]
fn resize_and_color_conversion_reach_every_frame() {
    let mut h = History::new();
    let mut d = two_frames(&mut h);
    let orig = d.clone();
    let t = tx(&d, |b| tools::resize(b, 6, 2));
    h.commit(&mut d, t);
    assert!((0..2).all(|i| d.frame_canvas(i).width == 6 && d.frame_canvas(i).height == 2));
    assert_eq!(row(&d, 1), "B     ");
    let t = tx(&d, |b| tools::set_kind(b, DocKind::Modern));
    h.commit(&mut d, t);
    h.undo(&mut d);
    h.undo(&mut d);
    assert_eq!(d, orig);
}

#[test]
fn replay_rebuilds_frames() {
    let mut h = History::new();
    let mut d = two_frames(&mut h);
    let t = tx(&d, |b| b.remove_frame(0));
    h.commit(&mut d, t);
    h.undo(&mut d);
    let mut tl = Timeline::new(h.log(), TimelineOptions::default()).unwrap();
    tl.seek(tl.len());
    assert_eq!(tl.doc(), &d);
    tl.seek(3);
    assert_eq!(tl.doc().frame_count(), 2);
    assert_eq!(tl.doc().current_frame(), 1, "replay shows the frame being drawn on");
    tl.seek(0);
    assert_eq!(tl.doc().frame_count(), 1);
    assert_eq!(row(tl.doc(), 0), "    ");
    // Hiding undone work drops the removal too.
    let mut tl = Timeline::new(h.log(), TimelineOptions { skip_idle: true, hide_undone: true }).unwrap();
    tl.seek(tl.len());
    assert_eq!(tl.doc(), &d);
}

#[test]
fn shared_edits_land_on_their_frame_whatever_is_shown() {
    let mut mine = History::new();
    let mut d = two_frames(&mut mine);
    let mut theirs = d.clone();
    theirs.show_frame(1);
    // They draw on frame 1 while I show frame 0.
    let t = put(&theirs, 2, 'Z');
    theirs.apply(&t);
    mine.apply_remote(&mut d, &t);
    assert_eq!(row(&d, 1), "B Z ");
    assert_eq!(d.current_frame(), 0);
    assert_eq!(d, theirs);
}

#[test]
fn shared_undo_keeps_a_frame_someone_drew_on() {
    let mut d = Document::new(DocKind::Classic, 4, 1);
    let mut h = History::new();
    h.set_shared(true);
    let blank = d.blank_frame_canvas();
    let t = tx(&d, |b| {
        b.insert_frame(1, blank, 1);
    });
    h.commit(&mut d, t);
    let mut other = d.clone();
    other.show_frame(1);
    let theirs = put(&other, 0, 'Z');
    h.apply_remote(&mut d, &theirs);
    h.take_changes();
    h.undo(&mut d);
    assert_eq!(d.frame_count(), 2, "their drawing survives my undo");
    assert!(h.take_changes().is_empty());
    // An untouched frame is removed, and what goes out replays the same.
    let mut d = Document::new(DocKind::Classic, 4, 1);
    let start = d.clone();
    let mut h = History::new();
    h.set_shared(true);
    let blank = d.blank_frame_canvas();
    let t = tx(&d, |b| {
        b.insert_frame(1, blank, 1);
    });
    h.commit(&mut d, t);
    h.undo(&mut d);
    assert_eq!(d, start);
    let mut copy = start.clone();
    for t in h.take_changes() {
        copy.apply(&t);
    }
    assert_eq!(copy, d);
}
