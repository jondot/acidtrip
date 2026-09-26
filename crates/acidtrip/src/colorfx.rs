//! The Filters and Recolor tools' shared machinery: what they would change
//! (cached, so the live canvas preview and the sidebar stay cheap), the
//! preview overlay, and Apply as one undo step.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use acidtrip_core::filters::{Filter, PRESETS};
use acidtrip_core::render::{RenderOptions, render_cells};
use acidtrip_core::tools::recolor::{self, Change, Replace, Scope};
use acidtrip_core::{Cell, Color, DocKind, Document, TxBuilder};
use acidtrip_io::import::{self, Dither, Glyphs, ImportOptions, ImportStyle, Scaling};

use crate::tab::Tab;

/// Which colors Recolor replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Fg,
    Bg,
    Both,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::Fg => "fg",
            Target::Bg => "bg",
            Target::Both => "both",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct FilterOpts {
    pub filter: Filter,
    /// Every editable layer instead of the current one.
    pub all_layers: bool,
    /// Classic: re-fit glyphs to the filtered picture (the image importer's
    /// engine) instead of mapping each color to the nearest of 16.
    pub rerender: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RecolorOpts {
    /// The color to replace (picked on the canvas or from the used colors).
    pub from: Option<Color>,
    pub target: Target,
    pub all_layers: bool,
    /// 0..=100: near colors count too, keeping their shading.
    pub tolerance: u8,
    /// First used-color chip shown.
    pub page: usize,
}

impl Default for RecolorOpts {
    fn default() -> Self {
        RecolorOpts { from: None, target: Target::Both, all_layers: false, tolerance: 0, page: 0 }
    }
}

type Cached<T> = RefCell<Option<(u64, Rc<T>)>>;

#[derive(Default)]
pub struct ColorFx {
    pub filter: FilterOpts,
    pub recolor: RecolorOpts,
    /// Held down on the canvas: show the original.
    pub comparing: bool,
    changes: Cached<Vec<Change>>,
    used: Cached<Vec<(Color, usize)>>,
}

/// Which of the two tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Filter,
    Recolor,
}

fn doc_key(tab: &Tab, h: &mut DefaultHasher) {
    tab.doc.meta.id.hash(h);
    tab.history.revision().hash(h);
    (tab.doc.width(), tab.doc.height(), tab.doc.canvas.layers.len(), tab.layer).hash(h);
    tab.selection.map(|r| (r.x, r.y, r.w, r.h)).hash(h);
}

fn cached<T>(cell: &Cached<T>, key: u64, make: impl FnOnce() -> T) -> Rc<T> {
    if let Some((k, v)) = &*cell.borrow()
        && *k == key
    {
        return v.clone();
    }
    let v = Rc::new(make());
    *cell.borrow_mut() = Some((key, v.clone()));
    v
}

impl ColorFx {
    pub fn scope(&self, tab: &Tab, kind: Kind) -> Scope {
        let all = match kind {
            Kind::Filter => self.filter.all_layers,
            Kind::Recolor => self.recolor.all_layers,
        };
        Scope::new(&tab.doc, tab.layer, tab.selection, all)
    }

    fn replace(&self, to: Color) -> Option<Replace> {
        let r = &self.recolor;
        Some(Replace {
            from: r.from?,
            to,
            fg: r.target != Target::Bg,
            bg: r.target != Target::Fg,
            tolerance: r.tolerance,
        })
    }

    /// What the tool would change (`to` is the brush FG, Recolor's target).
    pub fn changes(&self, tab: &Tab, kind: Kind, to: Color) -> Rc<Vec<Change>> {
        let mut h = DefaultHasher::new();
        doc_key(tab, &mut h);
        kind.hash(&mut h);
        match kind {
            Kind::Filter => self.filter.hash(&mut h),
            Kind::Recolor => {
                (&self.recolor.from, self.recolor.target, self.recolor.all_layers, self.recolor.tolerance, to)
                    .hash(&mut h)
            }
        }
        cached(&self.changes, h.finish(), || {
            let scope = self.scope(tab, kind);
            match kind {
                Kind::Filter => {
                    let o = &self.filter;
                    if o.rerender && tab.doc.meta.kind == DocKind::Classic && !o.filter.is_identity() {
                        rerender(&tab.doc, &scope, &o.filter)
                    } else {
                        recolor::filter_cells(&tab.doc, &scope, &o.filter)
                    }
                }
                Kind::Recolor => match self.replace(to) {
                    Some(r) => recolor::replace_cells(&tab.doc, &scope, &r),
                    None => vec![],
                },
            }
        })
    }

    /// The colors in Recolor's scope, most used first.
    pub fn used_colors(&self, tab: &Tab) -> Rc<Vec<(Color, usize)>> {
        let mut h = DefaultHasher::new();
        doc_key(tab, &mut h);
        self.recolor.all_layers.hash(&mut h);
        cached(&self.used, h.finish(), || recolor::used_colors(&tab.doc, &self.scope(tab, Kind::Recolor)))
    }

    /// Canvas overlay: the composite as it would look after Apply.
    pub fn preview(&self, tab: &Tab, kind: Kind, to: Color) -> Vec<(usize, usize, Cell)> {
        if self.comparing {
            return vec![];
        }
        let changes = self.changes(tab, kind, to);
        if changes.is_empty() {
            return vec![];
        }
        let mut b = TxBuilder::new(&tab.doc, "preview");
        recolor::write(&mut b, &changes);
        let mut seen = std::collections::HashSet::new();
        changes
            .iter()
            .filter(|&&(_, x, y, _)| seen.insert((x, y)))
            .map(|&(_, x, y, _)| (x, y, b.composite(x, y)))
            .collect()
    }

    /// Commit the changes as one undo step.
    pub fn apply(&mut self, tab: &mut Tab, kind: Kind, to: Color) -> String {
        if kind == Kind::Recolor && self.recolor.from.is_none() {
            return "click a color on the canvas (or a chip) to replace first".into();
        }
        let changes = self.changes(tab, kind, to);
        if changes.is_empty() {
            return "nothing to change".into();
        }
        let label = match kind {
            Kind::Filter => format!("Filter {}", self.filter.filter.current().name),
            Kind::Recolor => "Recolor".into(),
        };
        tab.edit(&label, |b| recolor::write(b, &changes));
        let cells = changes.iter().map(|&(_, x, y, _)| (x, y)).collect::<std::collections::HashSet<_>>().len();
        let msg = format!("{label}: {cells} cells changed (Ctrl-Z undoes)");
        if kind == Kind::Recolor {
            // What was replaced is gone; the new color is what's there now.
            self.recolor.from = Some(to);
        }
        msg
    }

    pub fn step_preset(&mut self, dir: i32) -> String {
        let n = PRESETS.len() as i32;
        let f = &mut self.filter.filter;
        f.preset = (f.preset as i32 + dir).rem_euclid(n) as usize;
        format!("{} ({}/{})", f.current().name, f.preset + 1, PRESETS.len())
    }
}

/// Classic re-render: draw the filtered cells as a picture and convert it
/// back with the image importer, so glyphs are re-fitted (half blocks and
/// shades mix the 16 colors) instead of each color snapping to its nearest.
fn rerender(doc: &Document, scope: &Scope, filter: &Filter) -> Vec<Change> {
    let filtered = recolor::filter_cells(doc, scope, filter);
    let r = scope.rect;
    let pal = &doc.meta.palette;
    let opts = ImportOptions {
        width: r.w,
        style: ImportStyle::Blocks,
        kind: DocKind::Classic,
        glyphs: Glyphs::Cp437,
        shades: true,
        dither: Dither::Diffuse,
        scaling: Scaling::Smooth,
        palette: Some(pal.clone()),
        ..ImportOptions::default()
    };
    let mut out = vec![];
    for &l in &scope.layers {
        let mine: std::collections::HashMap<(usize, usize), Cell> =
            filtered.iter().filter(|c| c.0 == l).map(|&(_, x, y, c)| ((x, y), c)).collect();
        let img = render_cells(r.w, r.h, RenderOptions::default(), |x, y| {
            let (cx, cy) = (r.x + x, r.y + y);
            let c = mine
                .get(&(cx, cy))
                .copied()
                .or_else(|| doc.canvas.get(l, cx, cy))
                .unwrap_or_else(|| doc.canvas.composite(cx, cy));
            (c.ch, c.fg.rgb(pal), c.bg.rgb(pal))
        });
        let Ok(conv) = import::convert(&img, &opts, doc.meta.ice) else { continue };
        for y in 0..r.h.min(conv.clip.height) {
            for x in 0..r.w.min(conv.clip.width) {
                let (cx, cy) = (r.x + x, r.y + y);
                let (Some(old), Some(new)) = (doc.canvas.get(l, cx, cy), conv.clip.get(x, y)) else { continue };
                // Palette indices are the document's (its palette was passed in).
                if new != old {
                    out.push((l, cx, cy, new));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::filters::preset_index;
    use acidtrip_core::{Document, Layer};

    fn tab(kind: DocKind) -> Tab {
        let mut d = Document::new(kind, 6, 3);
        d.canvas.layers.push(Layer::new("top", 6, 3));
        for i in 0..6 {
            d.canvas.layers[1].cells[i] = Some(Cell::new('█', Color::Pal(4), Color::Pal(1)));
        }
        Tab::new(d, None)
    }

    #[test]
    fn recolor_previews_then_applies_as_one_undo_step() {
        let mut t = tab(DocKind::Classic);
        let mut fx = ColorFx::default();
        assert!(fx.preview(&t, Kind::Recolor, Color::Pal(2)).is_empty(), "nothing picked yet");
        fx.recolor.from = Some(Color::Pal(4));
        fx.recolor.target = Target::Fg;
        let pv = fx.preview(&t, Kind::Recolor, Color::Pal(2));
        assert_eq!(pv.len(), 6);
        assert!(pv.iter().all(|c| c.2.fg == Color::Pal(2)));
        fx.apply(&mut t, Kind::Recolor, Color::Pal(2));
        assert_eq!(t.doc.canvas.get(1, 3, 0).unwrap().fg, Color::Pal(2));
        assert!(fx.preview(&t, Kind::Recolor, Color::Pal(2)).is_empty(), "applied: nothing left to do");
        t.undo();
        assert_eq!(t.doc.canvas.get(1, 3, 0).unwrap().fg, Color::Pal(4));
    }

    #[test]
    fn filters_follow_the_selection_and_rerender_keeps_classic_colors() {
        let mut t = tab(DocKind::Classic);
        t.selection = Some(acidtrip_core::tools::Rect::new(0, 0, 2, 1));
        let mut fx = ColorFx::default();
        fx.filter.filter = Filter::preset(preset_index("Mono").unwrap());
        let pv = fx.preview(&t, Kind::Filter, Color::WHITE);
        assert!(!pv.is_empty() && pv.iter().all(|c| c.0 < 2 && c.1 == 0));
        fx.filter.rerender = true;
        fx.filter.all_layers = true;
        t.selection = None;
        let ch = fx.changes(&t, Kind::Filter, Color::WHITE);
        assert!(!ch.is_empty());
        assert!(ch.iter().all(|c| matches!(c.3.fg, Color::Pal(i) if i < 16)));
        let n = t.history.revision();
        fx.apply(&mut t, Kind::Filter, Color::WHITE);
        assert_eq!(t.history.revision(), n + 1);
    }

    #[test]
    fn used_colors_are_cached_per_revision() {
        let t = tab(DocKind::Classic);
        let fx = ColorFx::default();
        let a = fx.used_colors(&t);
        let b = fx.used_colors(&t);
        assert!(Rc::ptr_eq(&a, &b));
        assert_eq!(a[0].1, 6);
    }
}
