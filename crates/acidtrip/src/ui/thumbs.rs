//! Pixel-exact minimap and layer thumbnails through the terminal's graphics
//! protocol (iTerm2 / kitty / sixel): the real VGA-font render, scaled down.
//! Terminals without graphics keep the half-block previews in `minimap`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use acidtrip_core::render::{CELL_H, CELL_W, RenderOptions, render_cells};
use acidtrip_core::{Canvas, Cell, Palette};
use image::imageops::{self, FilterType};
use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::Frame;
use ratatui::layout::{Rect, Size};
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};

use super::minimap::{MiniGeom, crop_rows};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Minimap,
    Layer(usize),
    /// A dialog's picture of some art (gallery posters), by the caller's key.
    Art(u64),
    /// The sidebar's gallery strip, by position.
    Recent(usize),
}

#[derive(Clone, Debug, PartialEq)]
struct Key {
    doc: uuid::Uuid,
    revision: u64,
    rect: Rect,
    row0: usize,
    /// Minimap only: viewport (x, y, w, h), drawn as an outline.
    view: (usize, usize, usize, usize),
}

/// An encoded image ready to draw.
enum Rendered {
    /// kitty / sixel via ratatui-image.
    Proto(Protocol),
    /// iTerm2 inline image sized in *cells* (ratatui-image sizes iTerm2 images
    /// in px, which iTerm2 treats as points: 2x too big on Retina screens).
    Iterm2 { seq: String, size: Size },
}

struct Entry {
    key: Key,
    img: Rendered,
    geom: MiniGeom,
}

/// The document a preview is drawn from.
pub struct Source<'a> {
    pub doc_id: uuid::Uuid,
    /// History revision: the cache key for "the art changed".
    pub revision: u64,
    pub canvas: &'a Canvas,
    pub palette: &'a Palette,
}

pub struct Thumbs {
    picker: Picker,
    cache: HashMap<Slot, Entry>,
    last_build: Instant,
    /// A rebuild was skipped by the throttle; the app keeps redrawing until done.
    pub stale: bool,
    /// The scaled-down minimap render before the viewport overlay: scrolling
    /// only changes the overlay, so this is reused until the art changes.
    base: Option<(BaseKey, RgbaImage)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct BaseKey {
    doc: uuid::Uuid,
    revision: u64,
    row0: usize,
    rows: usize,
    size: (u32, u32),
}

/// Minimum time between rebuilds while the art changes (brush strokes).
const THROTTLE: Duration = Duration::from_millis(150);

const VIEW_OUTLINE: Rgba<u8> = Rgba([255, 85, 255, 255]);
const EMPTY: [u8; 3] = [26, 26, 36];

impl Thumbs {
    /// Only for terminals with a real graphics protocol. `ACIDTRIP_GRAPHICS`
    /// (iterm2 | kitty | sixel | off) overrides detection.
    pub fn new(picker: Option<&Picker>) -> Option<Thumbs> {
        let forced = std::env::var("ACIDTRIP_GRAPHICS").ok();
        let picker = match forced.as_deref() {
            Some("off") | Some("halfblocks") => return None,
            Some(p) => {
                let mut pk = picker.cloned().unwrap_or_else(Picker::halfblocks);
                pk.set_protocol_type(match p {
                    "kitty" => ProtocolType::Kitty,
                    "sixel" => ProtocolType::Sixel,
                    _ => ProtocolType::Iterm2,
                });
                pk
            }
            None => {
                let mut pk = picker.filter(|p| p.protocol_type() != ProtocolType::Halfblocks)?.clone();
                // iTerm2 also answers kitty queries, but shows its own protocol reliably.
                if is_iterm2() {
                    pk.set_protocol_type(ProtocolType::Iterm2);
                }
                pk
            }
        };
        Some(Thumbs { picker, cache: HashMap::new(), last_build: Instant::now() - THROTTLE, stale: false, base: None })
    }

    fn cell_px(&self) -> (u32, u32) {
        let fs = self.picker.font_size();
        (fs.width.max(1) as u32, fs.height.max(1) as u32)
    }

    /// Draw the minimap into `area`; returns click geometry.
    pub fn minimap(
        &mut self,
        f: &mut Frame,
        area: Rect,
        src: &Source,
        view: (usize, usize, usize, usize),
    ) -> Option<MiniGeom> {
        let (doc_id, revision, canvas, pal) = (src.doc_id, src.revision, src.canvas, src.palette);
        let (cw, ch) = self.cell_px();
        let (w, h) = (canvas.width.max(1), canvas.height.max(1));
        let target_w = area.width as u32 * cw;
        // Scale the art so its width fills the panel.
        let s = target_w as f32 / (w as u32 * CELL_W) as f32;
        let win_rows = ((area.height as u32 * ch) as f32 / (CELL_H as f32 * s)).floor().max(1.0) as usize;
        let win_rows = win_rows.min(h);
        let center = view.1 + view.3 / 2;
        let row0 = center.saturating_sub(win_rows / 2).min(h - win_rows);
        let rows_cells = (((win_rows as u32 * CELL_H) as f32 * s) / ch as f32).ceil().max(1.0) as u16;
        let rect = Rect::new(area.x, area.y, area.width, rows_cells.min(area.height));
        let key = Key { doc: doc_id, revision, rect, row0, view };
        let geom = MiniGeom {
            rect,
            cols_per_cell: w as f32 / area.width.max(1) as f32,
            rows_per_cell: win_rows as f32 / rect.height.max(1) as f32,
            row0,
        };
        let build = |th: &mut Thumbs| {
            let (iw, ih) = (target_w, rect.height as u32 * ch);
            let bk = BaseKey { doc: doc_id, revision, row0, rows: win_rows, size: (iw, ih) };
            let mut img = match &th.base {
                Some((k, img)) if *k == bk => img.clone(),
                _ => {
                    let win = crop_rows(canvas, row0, win_rows);
                    let full = render_canvas(&win, pal, |c| c.composite(0, 0), None);
                    let img = imageops::resize(&full, iw, ih.max(1), FilterType::Triangle);
                    th.base = Some((bk, img.clone()));
                    img
                }
            };
            // Dim outside the viewport and outline it.
            let sx = iw as f32 / w as f32;
            let sy = ih as f32 / win_rows as f32;
            let x0 = (view.0 as f32 * sx) as i64;
            let x1 = ((view.0 + view.2) as f32 * sx) as i64;
            let y0 = ((view.1 as f32 - row0 as f32) * sy) as i64;
            let y1 = (((view.1 + view.3) as f32 - row0 as f32) * sy) as i64;
            for (x, y, p) in img.enumerate_pixels_mut() {
                let (x, y) = (x as i64, y as i64);
                let inside = x >= x0 && x < x1 && y >= y0 && y < y1;
                if !inside {
                    p.0 = [p.0[0] / 3, p.0[1] / 3, p.0[2] / 3, 255];
                }
                let edge = inside && (x - x0 < 2 || x1 - x <= 2 || y - y0 < 2 || y1 - y <= 2);
                if edge {
                    *p = VIEW_OUTLINE;
                }
            }
            if let Ok(dir) = std::env::var("ACIDTRIP_DUMP_THUMBS") {
                let _ = img.save(format!("{dir}/minimap-row0-{row0}-rows-{win_rows}.png"));
            }
            img
        };
        self.show(f, Slot::Minimap, key, geom, build)
    }

    /// Draw one layer's thumbnail (alone; transparency shows dark gray).
    pub fn layer(&mut self, f: &mut Frame, area: Rect, src: &Source, layer: usize, row0: usize) {
        let (doc_id, revision, canvas, pal) = (src.doc_id, src.revision, src.canvas, src.palette);
        let (cw, ch) = self.cell_px();
        let (w, h) = (canvas.width.max(1), canvas.height.max(1));
        let (iw, ih) = (area.width as u32 * cw, area.height as u32 * ch);
        let s = iw as f32 / (w as u32 * CELL_W) as f32;
        let win_rows = ((ih as f32 / (CELL_H as f32 * s)).ceil() as usize).clamp(1, h);
        let row0 = row0.min(h - win_rows);
        let key = Key { doc: doc_id, revision, rect: area, row0, view: (layer, 0, 0, 0) };
        let geom = MiniGeom { rect: area, cols_per_cell: 1.0, rows_per_cell: 1.0, row0 };
        let build = |_: &mut Thumbs| {
            let win = crop_rows(canvas, row0, win_rows);
            let img = render_canvas(&win, pal, |c| c.composite(0, 0), Some(layer));
            imageops::resize(&img, iw, ih.max(1), FilterType::Triangle)
        };
        self.show(f, Slot::Layer(layer), key, geom, build);
    }

    fn show(
        &mut self,
        f: &mut Frame,
        slot: Slot,
        key: Key,
        geom: MiniGeom,
        build: impl FnOnce(&mut Thumbs) -> RgbaImage,
    ) -> Option<MiniGeom> {
        let fresh = self.cache.get(&slot).is_some_and(|e| e.key == key);
        let same_frame = self.cache.get(&slot).is_some_and(|e| e.key.rect == key.rect);
        let throttled = self.last_build.elapsed() < THROTTLE && same_frame;
        if !fresh && !throttled {
            let t0 = Instant::now();
            let img = build(self);
            let size = Size::new(key.rect.width, key.rect.height);
            let rendered = if self.picker.protocol_type() == ProtocolType::Iterm2 {
                Some(Rendered::Iterm2 { seq: iterm2_seq(&img, size), size })
            } else {
                self.picker
                    .new_protocol(DynamicImage::ImageRgba8(img), size, Resize::Fit(None))
                    .ok()
                    .map(Rendered::Proto)
            };
            if let Ok(path) = std::env::var("ACIDTRIP_PERF_LOG") {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = writeln!(f, "{slot:?} rebuilt in {:?}", t0.elapsed());
                }
            }
            if let Some(img) = rendered {
                self.cache.insert(slot, Entry { key: key.clone(), img, geom });
                self.last_build = Instant::now();
            }
        }
        self.stale |= !fresh && throttled;
        let e = self.cache.get(&slot)?;
        match &e.img {
            Rendered::Proto(p) => f.render_widget(Image::new(p), e.key.rect),
            Rendered::Iterm2 { seq, size } => place(f.buffer_mut(), e.key.rect, seq, *size),
        }
        Some(e.geom)
    }

    /// A picture of `canvas`'s top rows filling `area`'s width (gallery
    /// posters). `key` names the art; `revision` changes when it does.
    pub fn art(&mut self, f: &mut Frame, area: Rect, key: u64, canvas: &Canvas, pal: &Palette) {
        self.art_in(f, Slot::Art(key), area, key, canvas, pal, false);
    }

    /// A gallery-strip thumbnail at strip position `pos`; `dim` darkens the
    /// ones not selected.
    #[allow(clippy::too_many_arguments)]
    pub fn recent(&mut self, f: &mut Frame, pos: usize, area: Rect, key: u64, canvas: &Canvas, pal: &Palette, dim: bool) {
        self.art_in(f, Slot::Recent(pos), area, key ^ dim as u64, canvas, pal, dim);
    }

    #[allow(clippy::too_many_arguments)]
    fn art_in(&mut self, f: &mut Frame, slot: Slot, area: Rect, key: u64, canvas: &Canvas, pal: &Palette, dim: bool) {
        let (cw, ch) = self.cell_px();
        let (w, h) = (canvas.width.max(1), canvas.height.max(1));
        let (iw, ih) = (area.width as u32 * cw, area.height as u32 * ch);
        let s = iw as f32 / (w as u32 * CELL_W) as f32;
        let win_rows = ((ih as f32 / (CELL_H as f32 * s)).ceil() as usize).clamp(1, h);
        // Art shorter than the area: a shorter picture, not a stretched one.
        let rows_cells = ((((win_rows as u32 * CELL_H) as f32 * s) / ch as f32).ceil() as u16).clamp(1, area.height);
        let rect = Rect::new(area.x, area.y, area.width, rows_cells);
        let k = Key { doc: uuid::Uuid::nil(), revision: key, rect, row0: 0, view: (0, 0, 0, 0) };
        let geom = MiniGeom { rect, cols_per_cell: 1.0, rows_per_cell: 1.0, row0: 0 };
        let build = |_: &mut Thumbs| {
            let win = crop_rows(canvas, 0, win_rows);
            let img = render_canvas(&win, pal, |c| c.composite(0, 0), None);
            let mut img = imageops::resize(&img, iw, (rows_cells as u32 * ch).max(1), FilterType::Triangle);
            if dim {
                for p in img.pixels_mut() {
                    p.0 = [p.0[0] / 3, p.0[1] / 3, p.0[2] / 3, 255];
                }
            }
            img
        };
        self.show_unthrottled(f, slot, k, geom, build);
    }

    /// Forget art pictures not in `keep` (the ones on screen now).
    pub fn retain_art(&mut self, keep: &[u64]) {
        self.cache.retain(|s, _| !matches!(s, Slot::Art(k) if !keep.contains(k)));
    }

    /// Like `show`, without the throttle: posters don't change while shown.
    fn show_unthrottled(
        &mut self,
        f: &mut Frame,
        slot: Slot,
        key: Key,
        geom: MiniGeom,
        build: impl FnOnce(&mut Thumbs) -> RgbaImage,
    ) {
        if self.cache.get(&slot).is_none_or(|e| e.key != key) {
            let img = build(self);
            let size = Size::new(key.rect.width, key.rect.height);
            let rendered = if self.picker.protocol_type() == ProtocolType::Iterm2 {
                Some(Rendered::Iterm2 { seq: iterm2_seq(&img, size), size })
            } else {
                self.picker
                    .new_protocol(DynamicImage::ImageRgba8(img), size, Resize::Fit(None))
                    .ok()
                    .map(Rendered::Proto)
            };
            match rendered {
                Some(img) => {
                    self.cache.insert(slot, Entry { key, img, geom });
                }
                None => return,
            }
        }
        if let Some(e) = self.cache.get(&slot) {
            match &e.img {
                Rendered::Proto(p) => f.render_widget(Image::new(p), e.key.rect),
                Rendered::Iterm2 { seq, size } => place(f.buffer_mut(), e.key.rect, seq, *size),
            }
        }
    }

    /// Call once per frame before drawing previews.
    pub fn begin_frame(&mut self) {
        self.stale = false;
    }

    /// Forget gallery-strip thumbnails past the first `n` positions.
    pub fn retain_recent(&mut self, n: usize) {
        self.cache.retain(|s, _| !matches!(s, Slot::Recent(i) if *i >= n));
    }

    /// Forget layer thumbnails that no longer exist.
    pub fn retain_layers(&mut self, n: usize) {
        self.cache.retain(|s, _| !matches!(s, Slot::Layer(i) if *i >= n));
    }
}

/// Render a canvas with the VGA font at 1:1 (8x16 per cell). With
/// `only_layer`, just that layer; its transparent cells use a dark fill.
fn render_canvas(
    c: &Canvas,
    pal: &Palette,
    _composite: impl Fn(&Canvas) -> Cell,
    only_layer: Option<usize>,
) -> RgbaImage {
    render_cells(c.width, c.height, RenderOptions::default(), |x, y| {
        let cell = match only_layer {
            Some(l) => match c.get(l, x, y) {
                Some(cell) => cell,
                None => return (' ', EMPTY, EMPTY),
            },
            None => c.composite(x, y),
        };
        (cell.ch, cell.fg.rgb(pal), cell.bg.rgb(pal))
    })
}

fn is_iterm2() -> bool {
    std::env::var("TERM_PROGRAM").is_ok_and(|t| t.contains("iTerm"))
        || std::env::var("LC_TERMINAL").is_ok_and(|t| t.contains("iTerm"))
}

/// iTerm2 inline-image escape, sized in cells so it fills exactly `size`.
fn iterm2_seq(img: &RgbaImage, size: Size) -> String {
    use base64::Engine;
    let mut png = Vec::new();
    let _ = img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png);
    format!(
        "\x1b]1337;File=inline=1;size={};width={};height={};preserveAspectRatio=0;doNotMoveCursor=1:{}\x07",
        png.len(),
        size.width,
        size.height,
        base64::engine::general_purpose::STANDARD.encode(&png)
    )
}

/// Put the escape in the top-left cell and tell ratatui to skip the rest of
/// the area (the terminal draws the image over it). The area is blanked first
/// so nothing stale shows if the terminal can't display the image.
fn place(buf: &mut ratatui::buffer::Buffer, area: Rect, seq: &str, size: Size) {
    use ratatui::buffer::CellDiffOption;
    use std::fmt::Write as _;
    let r = Rect::new(area.x, area.y, size.width.min(area.width), size.height.min(area.height));
    let mut full = String::new();
    for row in 0..r.height {
        let _ = write!(full, "\x1b[{};{}H\x1b[{}X", r.y + row + 1, r.x + 1, r.width);
    }
    let _ = write!(full, "\x1b[{};{}H{seq}", r.y + 1, r.x + 1);
    let seq = full.as_str();
    for y in r.top()..r.bottom() {
        for x in r.left()..r.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                if (x, y) == (r.x, r.y) {
                    c.set_symbol(seq)
                        .set_diff_option(CellDiffOption::ForcedWidth(std::num::NonZeroU16::new(1).unwrap()));
                } else {
                    c.set_diff_option(CellDiffOption::Skip);
                }
            }
        }
    }
}
