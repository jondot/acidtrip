//! PNG and GIF exports through the pixel-exact core renderer.

use std::collections::HashMap;

use acidtrip_core::render::{self, CELL_H, RenderOptions};
use acidtrip_core::{Cell, Document, Grid, LayerKind, Palette};
use anyhow::ensure;
use image::RgbaImage;

use super::ansi::{Replay, encode};
use super::{GifMode, SaveOptions, export_grid, export_rows, frame_delay_cs, frame_grids};

pub(super) fn render_opts(doc: &Document, opts: &SaveOptions) -> RenderOptions {
    RenderOptions { scale: opts.scale.clamp(1, 8), nine_px: doc.meta.letter_spacing_9px }
}

pub fn save_png(doc: &Document, opts: &SaveOptions) -> Vec<u8> {
    let img = render::render_document(doc, Some(export_rows(doc, opts)), render_opts(doc, opts));
    render::png_bytes(&img)
}

/// Render cells `(x0..x0+w, y0..y0+h)` from `cell`.
pub(super) fn render_rect(
    ro: RenderOptions,
    pal: &Palette,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    cell: impl Fn(usize, usize) -> Cell,
) -> RgbaImage {
    render::render_cells(w, h, ro, |x, y| {
        let c = cell(x0 + x, y0 + y);
        (c.ch, c.fg.rgb(pal), c.bg.rgb(pal))
    })
}

pub(super) struct GifOut {
    enc: gif::Encoder<Vec<u8>>,
    /// Global palette lookup when ≤ 256 colors; otherwise frames quantize.
    lut: Option<HashMap<[u8; 3], u8>>,
    colors: Vec<[u8; 3]>,
    pending: Option<gif::Frame<'static>>,
}

impl GifOut {
    pub(super) fn new(w: usize, h: usize, colors: Vec<[u8; 3]>, animated: bool) -> anyhow::Result<Self> {
        ensure!(w <= 65535 && h <= 65535, "image too large for GIF ({w}x{h} px)");
        let fits = colors.len() <= 256;
        let global: Vec<u8> = if fits { colors.iter().flatten().copied().collect() } else { Vec::new() };
        let mut enc = gif::Encoder::new(Vec::new(), w as u16, h as u16, &global)?;
        if animated {
            enc.set_repeat(gif::Repeat::Infinite)?;
        }
        let lut = fits.then(|| colors.iter().enumerate().map(|(i, &c)| (c, i as u8)).collect());
        Ok(GifOut { enc, lut, colors, pending: None })
    }

    fn index(&self, lut: &HashMap<[u8; 3], u8>, c: [u8; 3]) -> u8 {
        lut.get(&c).copied().unwrap_or_else(|| {
            let pal = Palette { name: String::new(), colors: self.colors.clone() };
            pal.nearest(c, 256)
        })
    }

    pub(super) fn frame(&mut self, img: RgbaImage, left: u32, top: u32, delay: u16) -> anyhow::Result<()> {
        let (w, h) = (img.width() as u16, img.height() as u16);
        let mut f = match &self.lut {
            Some(lut) => {
                let buf: Vec<u8> = img.pixels().map(|p| self.index(lut, [p[0], p[1], p[2]])).collect();
                gif::Frame { width: w, height: h, buffer: buf.into(), ..gif::Frame::default() }
            }
            None => gif::Frame::from_rgba_speed(w, h, &mut img.into_raw(), 10),
        };
        f.left = left as u16;
        f.top = top as u16;
        f.delay = delay;
        f.dispose = gif::DisposalMethod::Keep;
        if let Some(p) = self.pending.replace(f) {
            self.enc.write_frame(&p)?;
        }
        Ok(())
    }

    pub(super) fn extend(&mut self, cs: u16) {
        if let Some(p) = &mut self.pending {
            p.delay = p.delay.saturating_add(cs);
        }
    }

    pub(super) fn finish(mut self, hold: Option<u16>) -> anyhow::Result<Vec<u8>> {
        if let Some(mut p) = self.pending.take() {
            if let Some(h) = hold {
                p.delay = h;
            }
            self.enc.write_frame(&p)?;
        }
        Ok(self.enc.into_inner()?)
    }
}

fn distinct(cells: impl Iterator<Item = Cell>, pal: &Palette) -> Vec<[u8; 3]> {
    let mut seen = Vec::new();
    for c in std::iter::once(Cell::BLANK).chain(cells) {
        for col in [c.bg.rgb(pal), c.fg.rgb(pal)] {
            if !seen.contains(&col) {
                seen.push(col);
                if seen.len() > 256 {
                    return seen;
                }
            }
        }
    }
    seen
}

pub fn save_gif(doc: &Document, opts: &SaveOptions) -> anyhow::Result<Vec<u8>> {
    let pal = &doc.meta.palette;
    let ro = render_opts(doc, opts);
    let flat = export_grid(doc, opts);
    let (w, rows) = (flat.width, flat.height);
    let cw = if ro.nine_px { 9 } else { 8 } * ro.scale as usize;
    let ch = CELL_H as usize * ro.scale as usize;
    let (pw, ph) = (w * cw, rows * ch);
    match opts.gif_mode {
        GifMode::Still => {
            let mut out = GifOut::new(pw, ph, distinct(flat.cells.iter().copied(), pal), false)?;
            out.frame(render_rect(ro, pal, 0, 0, w, rows, |x, y| flat.get(x, y)), 0, 0, 0)?;
            out.finish(None)
        }
        GifMode::LayersAsFrames => {
            let layers: Vec<Grid> = doc
                .canvas
                .layers
                .iter()
                .filter(|l| l.visible && l.kind == LayerKind::Normal)
                .map(|l| Grid {
                    width: w,
                    height: rows,
                    cells: l.cells[..w * rows].iter().map(|c| c.unwrap_or(Cell::BLANK)).collect(),
                })
                .collect();
            let layers = if layers.is_empty() { vec![flat] } else { layers };
            let mut out =
                GifOut::new(pw, ph, distinct(layers.iter().flat_map(|g| g.cells.iter().copied()), pal), true)?;
            for g in &layers {
                out.frame(render_rect(ro, pal, 0, 0, w, rows, |x, y| g.get(x, y)), 0, 0, 25)?;
            }
            out.finish(None)
        }
        GifMode::Reveal => {
            let stream = encode(
                doc,
                &SaveOptions { line_length: None, clear_screen: false, ice_hint: false, animate: false, ..opts.clone() },
            );
            let bps = (opts.baud as usize / 10).max(1);
            let chunk = bps.div_ceil(20).max(stream.len().div_ceil(2000)).max(1);
            let delay = ((chunk * 100) / bps).clamp(2, u16::MAX as usize) as u16;
            let mut out = GifOut::new(pw, ph, distinct(flat.cells.iter().copied(), pal), true)?;
            let mut shown = Grid::new(w, rows);
            out.frame(render_rect(ro, pal, 0, 0, w, rows, |x, y| shown.get(x, y)), 0, 0, delay)?;
            let mut replay = Replay::new(w);
            for part in stream.chunks(chunk) {
                replay.feed(part);
                let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
                for y in 0..rows {
                    for x in 0..w {
                        let c = replay.screen.cell(x, y);
                        if c != shown.get(x, y) {
                            shown.set(x, y, c);
                            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                        }
                    }
                }
                if x0 == usize::MAX {
                    out.extend(delay);
                    continue;
                }
                let img = render_rect(ro, pal, x0, y0, x1 - x0, y1 - y0, |x, y| shown.get(x, y));
                out.frame(img, (x0 * cw) as u32, (y0 * ch) as u32, delay)?;
            }
            out.finish(Some(300))
        }
        GifMode::Frames => {
            let frames = frame_grids(doc, opts);
            let rows = frames.first().map_or(rows, |(g, _)| g.height);
            let mut out = GifOut::new(
                pw,
                rows * ch,
                distinct(frames.iter().flat_map(|(g, _)| g.cells.iter().copied()), pal),
                frames.len() > 1,
            )?;
            let mut prev: Option<&Grid> = None;
            for (g, hold) in &frames {
                let delay = frame_delay_cs(doc.fps(), *hold);
                // Only the part that changed since the last frame.
                let changed = |x: usize, y: usize| prev.is_none_or(|p| p.get(x, y) != g.get(x, y));
                let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
                for y in 0..g.height {
                    for x in 0..g.width {
                        if changed(x, y) {
                            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                        }
                    }
                }
                if x0 == usize::MAX {
                    out.extend(delay);
                } else {
                    let img = render_rect(ro, pal, x0, y0, x1 - x0, y1 - y0, |x, y| g.get(x, y));
                    out.frame(img, (x0 * cw) as u32, (y0 * ch) as u32, delay)?;
                }
                prev = Some(g);
            }
            out.finish(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::{Color, DocKind};

    fn frames(bytes: &[u8]) -> Vec<(u16, u16, u16, u16, u16)> {
        let mut o = gif::DecodeOptions::new();
        o.set_color_output(gif::ColorOutput::Indexed);
        let mut d = o.read_info(bytes).unwrap();
        let mut v = Vec::new();
        while let Some(f) = d.read_next_frame().unwrap() {
            v.push((f.left, f.top, f.width, f.height, f.delay));
        }
        v
    }

    #[test]
    fn gif_modes() {
        let mut d = Document::new(DocKind::Classic, 10, 3);
        for (i, c) in d.canvas.layers[0].cells.iter_mut().enumerate().take(25) {
            *c = Some(Cell::new('#', Color::Pal((i % 15 + 1) as u8), Color::BLACK));
        }
        let still = save_gif(&d, &SaveOptions::default()).unwrap();
        assert_eq!(frames(&still), vec![(0, 0, 80, 48, 0)]);
        let opts = SaveOptions { gif_mode: GifMode::Reveal, baud: 2400, scale: 2, ..SaveOptions::default() };
        let rev = frames(&save_gif(&d, &opts).unwrap());
        assert!(rev.len() > 3, "{rev:?}");
        assert_eq!((rev[0].0, rev[0].1, rev[0].2, rev[0].3), (0, 0, 160, 96));
        assert_eq!(rev.last().unwrap().4, 300);
        d.canvas.layers.push(acidtrip_core::Layer::new("b", 10, 3));
        let lay = frames(
            &save_gif(&d, &SaveOptions { gif_mode: GifMode::LayersAsFrames, ..SaveOptions::default() }).unwrap(),
        );
        assert_eq!(lay.len(), 2);
    }
}
