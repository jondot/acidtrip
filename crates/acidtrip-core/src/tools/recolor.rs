//! Whole-piece color transforms, shared by the Filters and Recolor tools: a
//! scope (the selection or the canvas, on the current layer or all of them)
//! and a per-cell map over it that yields only the cells that change.

use crate::color::{Color, color_distance};
use crate::filters::Filter;
use crate::model::{Cell, Document, LayerKind, conform_cell};
use crate::tools::{Rect, has_ink};
use crate::tx::TxBuilder;

/// Where a transform applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub layers: Vec<usize>,
    pub rect: Rect,
}

impl Scope {
    /// The selection (or the whole canvas) on `layer`, or on every editable
    /// layer. Locked, hidden and reference layers are left alone.
    pub fn new(doc: &Document, layer: usize, selection: Option<Rect>, all_layers: bool) -> Scope {
        let (w, h) = (doc.width(), doc.height());
        let rect = selection
            .filter(|r| r.x < w && r.y < h && r.w > 0 && r.h > 0)
            .map(|r| Rect::new(r.x, r.y, r.w.min(w - r.x), r.h.min(h - r.y)))
            .unwrap_or(Rect::new(0, 0, w, h));
        let editable =
            |i: usize| doc.canvas.layers.get(i).is_some_and(|l| !l.locked && l.visible && l.kind == LayerKind::Normal);
        let layers = if all_layers {
            (0..doc.canvas.layers.len()).filter(|&i| editable(i)).collect()
        } else {
            [layer].into_iter().filter(|&i| editable(i)).collect()
        };
        Scope { layers, rect }
    }

    fn cells<'a>(&'a self, doc: &'a Document) -> impl Iterator<Item = (usize, usize, usize, Cell)> + 'a {
        let r = self.rect;
        self.layers.iter().flat_map(move |&l| {
            (r.y..r.y + r.h)
                .flat_map(move |y| (r.x..r.x + r.w).filter_map(move |x| doc.canvas.get(l, x, y).map(|c| (l, x, y, c))))
        })
    }
}

/// A changed cell: (layer, x, y, new cell).
pub type Change = (usize, usize, usize, Cell);

/// Run `f` over every (non-transparent) cell in scope; keep the changes
/// (conformed to the document, so a Classic color that snaps back to what
/// was there is no change).
pub fn map_cells(doc: &Document, scope: &Scope, mut f: impl FnMut(usize, usize, Cell) -> Cell) -> Vec<Change> {
    scope
        .cells(doc)
        .filter_map(|(l, x, y, c)| {
            let n = conform_cell(&doc.meta, f(x, y, c));
            (n != c).then_some((l, x, y, n))
        })
        .collect()
}

/// Stage changes (conformed to the document, so Classic art maps back to
/// its 16 colors).
pub fn write(b: &mut TxBuilder, changes: &[Change]) {
    for &(l, x, y, c) in changes {
        b.set(l, x, y, Some(c));
    }
}

/// Run a photo filter over fg and bg. Positions count from the scope's
/// corner, so a vignette frames the selection.
pub fn filter_cells(doc: &Document, scope: &Scope, filter: &Filter) -> Vec<Change> {
    if filter.is_identity() {
        return Vec::new();
    }
    let pal = &doc.meta.palette;
    let r = scope.rect;
    let size = (r.w as f32, r.h as f32 * 2.0);
    let run = |c: Color, at| {
        let rgb = c.rgb(pal);
        let out = filter.apply(rgb, at, size);
        if out == rgb { c } else { Color::Rgb(out[0], out[1], out[2]) }
    };
    map_cells(doc, scope, |x, y, c| {
        let at = ((x - r.x) as f32 + 0.5, ((y - r.y) as f32 + 0.5) * 2.0);
        Cell { fg: run(c.fg, at), bg: run(c.bg, at), ..c }
    })
}

/// Replace one color with another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Replace {
    pub from: Color,
    pub to: Color,
    pub fg: bool,
    pub bg: bool,
    /// 0..=100: how far from `from` still counts. Near misses keep their
    /// offset from `from`, so shading survives.
    pub tolerance: u8,
}

/// The largest [`color_distance`] (black to white).
const MAX_DISTANCE: f32 = 584_970.0;

impl Replace {
    fn map(&self, pal: &crate::Palette, c: Color) -> Color {
        let (from, rgb) = (self.from.rgb(pal), c.rgb(pal));
        if rgb == from {
            return self.to;
        }
        if self.tolerance == 0 {
            return c;
        }
        let d = (color_distance(rgb, from) as f32 / MAX_DISTANCE).sqrt() * 100.0;
        if d > self.tolerance as f32 {
            return c;
        }
        let to = self.to.rgb(pal);
        let v = |i: usize| (to[i] as i32 + rgb[i] as i32 - from[i] as i32).clamp(0, 255) as u8;
        Color::Rgb(v(0), v(1), v(2))
    }
}

/// Replace `r.from` in scope.
pub fn replace_cells(doc: &Document, scope: &Scope, r: &Replace) -> Vec<Change> {
    let pal = &doc.meta.palette;
    map_cells(doc, scope, |_, _, c| Cell {
        fg: if r.fg && has_ink(c.ch) { r.map(pal, c.fg) } else { c.fg },
        bg: if r.bg { r.map(pal, c.bg) } else { c.bg },
        ..c
    })
}

/// The colors a scope shows (fg of inked glyphs, every bg), most used first.
/// Colors that look the same are counted together under the first seen.
pub fn used_colors(doc: &Document, scope: &Scope) -> Vec<(Color, usize)> {
    let pal = &doc.meta.palette;
    let mut seen: Vec<([u8; 3], Color, usize)> = Vec::new();
    let mut count = |c: Color| {
        let rgb = c.rgb(pal);
        match seen.iter_mut().find(|s| s.0 == rgb) {
            Some(s) => s.2 += 1,
            None => seen.push((rgb, c, 1)),
        }
    };
    for (_, _, _, c) in scope.cells(doc) {
        if has_ink(c.ch) {
            count(c.fg);
        }
        count(c.bg);
    }
    seen.sort_by_key(|s| std::cmp::Reverse(s.2));
    seen.into_iter().map(|(_, c, n)| (c, n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::preset_index;
    use crate::model::{DocKind, Layer};

    fn doc(kind: DocKind) -> Document {
        let mut d = Document::new(kind, 4, 2);
        d.canvas.layers.push(Layer::new("top", 4, 2));
        let red = Cell::new('#', Color::Pal(4), Color::Pal(1));
        d.canvas.layers[1].cells[0] = Some(red);
        d.canvas.layers[1].cells[5] = Some(Cell::new('#', Color::Pal(4), Color::Pal(0)));
        d
    }

    #[test]
    fn scope_skips_locked_hidden_and_clips_selection() {
        let mut d = doc(DocKind::Classic);
        let s = Scope::new(&d, 1, Some(Rect::new(2, 1, 10, 10)), false);
        assert_eq!(s.rect, Rect::new(2, 1, 2, 1));
        assert_eq!(s.layers, vec![1]);
        assert_eq!(Scope::new(&d, 0, None, true).layers, vec![0, 1]);
        d.canvas.layers[0].locked = true;
        d.canvas.layers[1].visible = false;
        assert!(Scope::new(&d, 1, None, true).layers.is_empty());
    }

    #[test]
    fn replace_exact_on_one_layer() {
        let d = doc(DocKind::Classic);
        let s = Scope::new(&d, 1, None, false);
        let r = Replace { from: Color::Pal(4), to: Color::Pal(2), fg: true, bg: false, tolerance: 0 };
        let ch = replace_cells(&d, &s, &r);
        assert_eq!(ch.len(), 2);
        assert!(ch.iter().all(|c| c.3.fg == Color::Pal(2) && c.0 == 1));
        // bg only: nothing is red there.
        let r = Replace { fg: false, bg: true, ..r };
        assert!(replace_cells(&d, &s, &r).is_empty());
    }

    #[test]
    fn tolerance_keeps_shading() {
        let mut d = Document::new(DocKind::Modern, 2, 1);
        d.canvas.layers[0].cells[0] = Some(Cell::new('#', Color::Rgb(200, 0, 0), Color::BLACK));
        d.canvas.layers[0].cells[1] = Some(Cell::new('#', Color::Rgb(180, 10, 0), Color::BLACK));
        let s = Scope::new(&d, 0, None, false);
        let r = Replace { from: Color::Rgb(200, 0, 0), to: Color::Rgb(0, 0, 200), fg: true, bg: false, tolerance: 0 };
        assert_eq!(replace_cells(&d, &s, &r).len(), 1);
        let ch = replace_cells(&d, &s, &Replace { tolerance: 15, ..r });
        assert_eq!(ch.len(), 2);
        assert_eq!(ch[1].3.fg, Color::Rgb(0, 10, 200));
    }

    #[test]
    fn used_colors_counts_visible_ink_and_backgrounds() {
        let d = doc(DocKind::Classic);
        let used = used_colors(&d, &Scope::new(&d, 1, None, false));
        assert_eq!(used[0], (Color::Pal(4), 2));
        assert!(used.contains(&(Color::Pal(1), 1)));
        assert!(used.contains(&(Color::Pal(0), 1)));
    }

    #[test]
    fn filters_write_back_to_the_palette_in_classic() {
        let d = doc(DocKind::Classic);
        let s = Scope::new(&d, 1, None, false);
        let ch = filter_cells(&d, &s, &Filter::preset(preset_index("Mono").unwrap()));
        assert!(!ch.is_empty());
        let mut b = TxBuilder::new(&d, "Filter");
        write(&mut b, &ch);
        let tx = b.finish();
        for c in &tx.cells {
            let cell = c.after.unwrap();
            assert!(matches!(cell.fg, Color::Pal(i) if i < 16));
            assert!(matches!(cell.fg, Color::Pal(0 | 7 | 8 | 15)), "{cell:?}");
        }
        assert!(filter_cells(&d, &s, &Filter::default()).is_empty());
    }
}
