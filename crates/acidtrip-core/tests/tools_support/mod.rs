//! Shared helpers for the tools integration tests.
#![allow(dead_code)]

use acidtrip_core::tools::{Brush, Ctx};
use acidtrip_core::{Cell, Color, DocKind, Document, Grid, Transaction, TxBuilder};

pub const RED: Color = Color::Pal(4);
pub const BLUE: Color = Color::Pal(1);
pub const GREEN: Color = Color::Pal(2);
pub const YELLOW: Color = Color::Pal(14);
pub const WHITE: Color = Color::WHITE;

pub fn doc(w: usize, h: usize) -> Document {
    Document::new(DocKind::Classic, w, h)
}

pub fn modern(w: usize, h: usize) -> Document {
    Document::new(DocKind::Modern, w, h)
}

pub fn ctx(ch: char) -> Ctx {
    Ctx { brush: Brush { ch, fg: WHITE, bg: BLUE }, ..Ctx::default() }
}

pub fn cell(ch: char) -> Cell {
    Cell::new(ch, WHITE, Color::BLACK)
}

/// Run a tool against `d`, apply it, and return the transaction.
pub fn run(d: &mut Document, f: impl FnOnce(&mut TxBuilder)) -> Transaction {
    let mut b = TxBuilder::new(d, "test");
    f(&mut b);
    let tx = b.finish();
    d.apply(&tx);
    tx
}

/// Run a tool, check apply + revert restores the document exactly.
pub fn assert_undoable(d: &Document, f: impl FnOnce(&mut TxBuilder)) {
    let mut work = d.clone();
    let tx = run(&mut work, f);
    assert!(!tx.is_empty(), "tool made no change");
    assert_ne!(&work, d, "tool made no visible change");
    work.revert(&tx);
    assert_eq!(&work, d, "revert did not restore the document");
    work.apply(&tx);
    work.revert(&tx);
    assert_eq!(&work, d, "second revert did not restore the document");
}

pub fn grid_text(g: &Grid) -> String {
    (0..g.height).map(|y| g.row(y).iter().map(|c| c.ch).collect::<String>()).collect::<Vec<_>>().join("\n")
}

/// Composite chars as text, one line per row.
pub fn text(d: &Document) -> String {
    grid_text(&d.flatten())
}

/// Set layer-0 chars from rows of text (white on black).
pub fn draw(d: &mut Document, x: usize, y: usize, rows: &[&str]) {
    run(d, |b| {
        for (dy, row) in rows.iter().enumerate() {
            for (dx, ch) in row.chars().enumerate() {
                b.set(0, x + dx, y + dy, Some(cell(ch)));
            }
        }
    });
}

pub fn ch(d: &Document, x: usize, y: usize) -> char {
    d.canvas.composite(x, y).ch
}
