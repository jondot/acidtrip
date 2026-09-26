//! The sidebar's gallery strip: pieces recently viewed in the Gallery, read
//! from the Gallery's shelf file, with their art loaded from the download
//! cache (never the network) for the thumbnails.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use acidtrip_ai::gallery::{self, Piece, Shelf};
use acidtrip_core::Document;

/// Thumbnails shown side by side.
pub const STRIP: usize = 4;

#[derive(Default)]
pub struct Recent {
    pub pieces: Vec<Piece>,
    /// Art by piece key; None when its file isn't on disk.
    docs: HashMap<String, Option<Document>>,
    /// Selected piece (index into `pieces`).
    pub sel: usize,
    /// First piece in the strip.
    pub first: usize,
    shelf: PathBuf,
    cache: PathBuf,
    stamp: Option<SystemTime>,
}

impl Recent {
    pub fn new(state_dir: &Path) -> Recent {
        let mut r = Recent {
            shelf: state_dir.join("gallery.json"),
            cache: state_dir.join("harvest-cache"),
            ..Recent::default()
        };
        r.refresh();
        r
    }

    /// Re-read the shelf if the Gallery changed it, and pick up art that
    /// has been downloaded since.
    pub fn refresh(&mut self) {
        let stamp = std::fs::metadata(&self.shelf).and_then(|m| m.modified()).ok();
        let complete = self.docs.len() == self.pieces.len() && self.docs.values().all(Option::is_some);
        if stamp == self.stamp && stamp.is_some() && complete {
            return;
        }
        self.stamp = stamp;
        let pieces = Shelf::load(&self.shelf).recent;
        if pieces != self.pieces {
            self.sel = 0;
            self.first = 0;
        }
        self.pieces = pieces;
        let cache = &self.cache;
        let keys: Vec<String> = self.pieces.iter().map(Piece::key).collect();
        self.docs.retain(|k, d| keys.contains(k) && d.is_some());
        for p in &self.pieces {
            self.docs.entry(p.key()).or_insert_with(|| gallery::cached_doc(p, cache));
        }
    }

    pub fn doc(&self, i: usize) -> Option<&Document> {
        self.docs.get(&self.pieces.get(i)?.key())?.as_ref()
    }

    pub fn selected(&self) -> Option<&Piece> {
        self.pieces.get(self.sel)
    }

    /// Select piece `i`, scrolling the strip to show it.
    pub fn select(&mut self, i: usize) {
        if i < self.pieces.len() {
            self.sel = i;
            if i < self.first {
                self.first = i;
            } else if i >= self.first + STRIP {
                self.first = i + 1 - STRIP;
            }
        }
    }

    /// Step the selection by `by`, wrapping.
    pub fn step(&mut self, by: i32) {
        let n = self.pieces.len() as i32;
        if n > 0 {
            self.select((self.sel as i32 + by).rem_euclid(n) as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_ai::gallery::Origin;

    fn piece(n: usize) -> Piece {
        Piece {
            origin: Origin::Pack { pack: "p".into(), file: format!("f{n}.ans") },
            title: format!("f{n}"),
            artists: vec![],
            group: String::new(),
            year: 0,
        }
    }

    #[test]
    fn reads_the_shelf_and_scrolls_to_the_selection() {
        let dir = std::env::temp_dir().join(format!("acidtrip-recent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut shelf = Shelf::default();
        for n in (0..6).rev() {
            shelf.opened(&piece(n));
        }
        shelf.save(&dir.join("gallery.json")).unwrap();
        let mut r = Recent::new(&dir);
        assert_eq!(r.pieces.len(), 6);
        assert_eq!(r.pieces[0].title, "f0");
        assert!(r.doc(0).is_none(), "not downloaded: no art, no network");
        r.select(5);
        assert_eq!((r.sel, r.first), (5, 2));
        r.step(1);
        assert_eq!((r.sel, r.first), (0, 0));
        r.step(-1);
        assert_eq!(r.sel, 5);
        // The file lands in the cache: the next refresh picks it up.
        let f = dir.join("harvest-cache/gallery/p/f0.ans");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"\x1b[31mHI").unwrap();
        r.refresh();
        assert_eq!(r.doc(0).map(|d| d.canvas.composite(0, 0).ch), Some('H'));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
