//! Replay exports: a piece's edit log played back as an animated GIF or an
//! asciinema `.cast`, speed-paint style.

use std::collections::HashSet;
use std::fmt::Write as _;

use acidtrip_core::replay::{EditLog, Speed, Timeline, TimelineOptions};
use acidtrip_core::{Cell, Grid, cp437};
use anyhow::Context;

use super::raster::{GifOut, render_opts, render_rect};
use super::{SaveOptions, export_grid};
use acidtrip_core::render::CELL_H;

/// How a replay is exported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReplayExport {
    pub timeline: TimelineOptions,
    pub speed: Speed,
    /// GIF pixel scale.
    pub scale: u32,
}

impl Default for ReplayExport {
    fn default() -> Self {
        ReplayExport { timeline: TimelineOptions::default(), speed: Speed::Fit(30), scale: 1 }
    }
}

/// Longest frame list written; longer replays get fewer frames per second.
const MAX_FRAMES: u64 = 3000;
/// Frame length at most speeds (10 fps).
const FRAME_MS: u64 = 100;
/// The finished piece stays up this long (centiseconds / ms).
const HOLD_CS: u16 = 300;

struct Frames {
    tl: Timeline,
    /// Virtual time of each frame.
    times: Vec<u64>,
    /// Output length of one frame, ms.
    frame_ms: u64,
    opts: SaveOptions,
}

impl Frames {
    fn new(log: &EditLog, o: &ReplayExport) -> anyhow::Result<Frames> {
        let tl = Timeline::new(log, o.timeline).context("this piece has no edit history to replay")?;
        let dur = tl.duration();
        let f = o.speed.factor(dur);
        let out = (dur as f64 / f) as u64;
        let frame_ms = (out / MAX_FRAMES).max(FRAME_MS).div_ceil(10) * 10;
        let n = out / frame_ms;
        let mut times: Vec<u64> = (0..=n).map(|k| ((k * frame_ms) as f64 * f) as u64).collect();
        if times.last().is_some_and(|&t| t < dur) {
            times.push(dur);
        }
        let opts = SaveOptions { scale: o.scale.clamp(1, 8), ..SaveOptions::default() };
        Ok(Frames { tl, times, frame_ms, opts })
    }

    /// Visit every frame's flattened grid.
    fn each(&mut self, mut f: impl FnMut(&Timeline, &Grid) -> anyhow::Result<()>) -> anyhow::Result<()> {
        self.tl.seek(0);
        for i in 0..self.times.len() {
            self.tl.seek_time(self.times[i]);
            let g = export_grid(self.tl.doc(), &self.opts);
            f(&self.tl, &g)?;
        }
        Ok(())
    }

    /// Largest frame (cells) and the colors used, for sizing the output.
    fn survey(&mut self) -> anyhow::Result<(usize, usize, Vec<[u8; 3]>)> {
        let (mut w, mut h) = (1, 1);
        let mut seen = HashSet::new();
        let mut colors = Vec::new();
        self.each(|tl, g| {
            (w, h) = (w.max(g.width), h.max(g.height));
            let pal = &tl.doc().meta.palette;
            if colors.len() <= 256 {
                for c in std::iter::once(&Cell::BLANK).chain(&g.cells) {
                    for col in [c.bg.rgb(pal), c.fg.rgb(pal)] {
                        if seen.insert(col) {
                            colors.push(col);
                        }
                    }
                }
            }
            Ok(())
        })?;
        Ok((w, h, colors))
    }
}

/// Changed cells between `shown` and `g` (updating `shown`), as a bounding box.
fn diff(shown: &mut Grid, g: &Grid) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..shown.height {
        for x in 0..shown.width {
            let c = g.get(x, y);
            if c != shown.get(x, y) {
                shown.set(x, y, c);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x0 != usize::MAX).then_some((x0, y0, x1, y1))
}

pub fn replay_gif(log: &EditLog, o: &ReplayExport) -> anyhow::Result<Vec<u8>> {
    let mut fr = Frames::new(log, o)?;
    let (w, h, colors) = fr.survey()?;
    let final_doc = fr.tl.doc().clone();
    let ro = render_opts(&final_doc, &fr.opts);
    let cw = if ro.nine_px { 9 } else { 8 } * ro.scale as usize;
    let ch = CELL_H as usize * ro.scale as usize;
    let delay = (fr.frame_ms / 10).min(u16::MAX as u64) as u16;
    let mut out = GifOut::new(w * cw, h * ch, colors, true)?;
    let mut shown = Grid::new(w, h);
    let mut first = true;
    fr.each(|tl, g| {
        let pal = &tl.doc().meta.palette;
        let changed = diff(&mut shown, g);
        match (first, changed) {
            (true, _) => out.frame(render_rect(ro, pal, 0, 0, w, h, |x, y| shown.get(x, y)), 0, 0, delay)?,
            (false, None) => out.extend(delay),
            (false, Some((x0, y0, x1, y1))) => {
                let img = render_rect(ro, pal, x0, y0, x1 - x0, y1 - y0, |x, y| shown.get(x, y));
                out.frame(img, (x0 * cw) as u32, (y0 * ch) as u32, delay)?;
            }
        }
        first = false;
        Ok(())
    })?;
    out.finish(Some(HOLD_CS))
}

/// Char as a terminal should show it.
fn shown(ch: char) -> char {
    match ch {
        '\0' => ' ',
        c if c.is_control() => cp437::to_char(cp437::from_char_lossy(c)),
        c => c,
    }
}

pub fn replay_cast(log: &EditLog, o: &ReplayExport) -> anyhow::Result<Vec<u8>> {
    let mut fr = Frames::new(log, o)?;
    let (w, h, _) = fr.survey()?;
    let title = fr.tl.doc().meta.sauce.title.trim().to_string();
    let header = serde_json::json!({
        "version": 2,
        "width": w,
        "height": h,
        "title": if title.is_empty() { "acidtrip replay".to_string() } else { format!("{title} (replay)") },
        "env": { "TERM": "xterm-256color", "SHELL": "/bin/sh" },
    });
    let mut out = format!("{header}\n");
    let emit = |out: &mut String, ms: u64, data: &str| {
        let t = ms as f64 / 1000.0;
        let _ = writeln!(out, "[{t:.3}, \"o\", {}]", serde_json::to_string(data).unwrap_or_default());
    };
    emit(&mut out, 0, "\x1b[?25l\x1b[0m\x1b[2J\x1b[H");
    let mut shown_grid = Grid::new(w, h);
    let mut k = 0u64;
    let mut first = true;
    let frame_ms = fr.frame_ms;
    fr.each(|tl, g| {
        let pal = &tl.doc().meta.palette;
        let mut s = String::new();
        let mut sgr = None;
        for y in 0..h {
            let mut at = None;
            for x in 0..w {
                let c = g.get(x, y);
                if !first && c == shown_grid.get(x, y) {
                    continue;
                }
                shown_grid.set(x, y, c);
                if at != Some(x) {
                    let _ = write!(s, "\x1b[{};{}H", y + 1, x + 1);
                }
                let (f, b) = (c.fg.rgb(pal), c.bg.rgb(pal));
                if sgr != Some((f, b)) {
                    let _ = write!(s, "\x1b[38;2;{};{};{};48;2;{};{};{}m", f[0], f[1], f[2], b[0], b[1], b[2]);
                    sgr = Some((f, b));
                }
                s.push(shown(c.ch));
                at = Some(x + 1);
            }
        }
        if !s.is_empty() {
            emit(&mut out, k * frame_ms, &s);
        }
        first = false;
        k += 1;
        Ok(())
    })?;
    let end = k.saturating_sub(1) * frame_ms + HOLD_CS as u64 * 10;
    emit(&mut out, end, &format!("\x1b[0m\x1b[{};1H\x1b[?25h\r\n", h));
    Ok(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use acidtrip_core::replay::LogOp;
    use acidtrip_core::{Color, DocKind, Document, TxBuilder};

    fn log() -> EditLog {
        let mut d = Document::new(DocKind::Classic, 10, 3);
        let mut log = EditLog::new();
        for i in 0..10 {
            let mut b = TxBuilder::new(&d, "put");
            b.set(0, i, i % 3, Some(Cell::new('#', Color::Pal((i % 15 + 1) as u8), Color::BLACK)));
            let tx = b.finish();
            log.record_at(i as u64 * 500, &d, LogOp::Commit, tx.clone());
            d.apply(&tx);
        }
        log
    }

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
    fn gif_plays_the_strokes() {
        let o = ReplayExport { speed: Speed::Times(1.0), ..ReplayExport::default() };
        let f = frames(&replay_gif(&log(), &o).unwrap());
        // The first frame is the whole (blank) canvas, then one small patch per stroke.
        assert_eq!((f[0].0, f[0].1, f[0].2, f[0].3), (0, 0, 80, 48));
        assert_eq!(f.len(), 11, "{f:?}");
        assert!(f[1..].iter().all(|p| p.2 == 8 && p.3 == 16));
        assert_eq!(f.last().unwrap().4, HOLD_CS);
        // 4x faster: same strokes, shorter delays.
        let fast = frames(&replay_gif(&log(), &ReplayExport { speed: Speed::Times(4.0), ..o }).unwrap());
        assert!(fast.len() <= 11);
        assert!(fast[1].4 < f[1].4, "{fast:?}");
    }

    #[test]
    fn cast_draws_cells_over_time() {
        let o = ReplayExport { speed: Speed::Times(1.0), ..ReplayExport::default() };
        let cast = String::from_utf8(replay_cast(&log(), &o).unwrap()).unwrap();
        let lines: Vec<&str> = cast.lines().collect();
        let head: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(head["width"], 10);
        assert_eq!(head["height"], 3);
        let events: Vec<serde_json::Value> = lines[1..].iter().map(|l| serde_json::from_str(l).unwrap()).collect();
        let hashes: usize = events.iter().map(|e| e[2].as_str().unwrap().matches('#').count()).sum();
        assert_eq!(hashes, 10);
        let t: Vec<f64> = events.iter().map(|e| e[0].as_f64().unwrap()).collect();
        assert!(t.windows(2).all(|w| w[0] <= w[1]));
        assert!(*t.last().unwrap() > 4.5);
    }

    #[test]
    fn no_log_is_an_error() {
        assert!(replay_gif(&EditLog::new(), &ReplayExport::default()).is_err());
    }
}
