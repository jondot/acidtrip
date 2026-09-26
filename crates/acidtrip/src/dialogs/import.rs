//! Import image: pick a picture, then tune the conversion with a live
//! preview. Presets set everything at once; each setting is a row of
//! `‹ value ›` you click (‹ steps back) or change with ←→.
//!
//! Also opens as "Add reference image layer", which always makes a dimmed,
//! locked layer to trace over.

use std::path::{Path, PathBuf};
use std::time::Instant;

use acidtrip_core::filters::{Filter, PRESETS};
use acidtrip_core::{Clip, DocKind, Palette, tools};
use acidtrip_io::import::{self, Analysis, Converted, Dither, Glyphs, ImportOptions, ImportStyle, Preset, Scaling};
use crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use image::RgbaImage;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::dialogs::forms::expand_tilde;
use crate::tab::Tab;
use crate::ui::canvas::rgb;
use crate::ui::widgets::{BtnKind, Buttons, LineInput, ListState, btn, centered, fuzzy, list_line, popup, theme};

const IMAGE_EXT: [&str; 7] = ["png", "gif", "jpg", "jpeg", "webp", "bmp", "PNG"];

/// Where the result goes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Into {
    Layer,
    Document,
    Stamp,
}

/// The color target: the document's 16, 16 fitted to the image (new
/// documents only), or truecolor (switches a Classic document to Modern).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Colors {
    Sixteen,
    Fitted,
    Truecolor,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Opt {
    Into,
    Colors,
    Width,
    Style,
    Glyphs,
    Shades,
    Dither,
    Scaling,
    /// A photo filter (the Filters tool's presets) run on the picture first.
    Filter,
    Brightness,
    Contrast,
    Saturation,
    Sharpen,
    Levels,
    Lines,
    Ink,
    Cleanup,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    Preset(usize),
    Size(u8),
    Import,
    Cancel,
    Change,
    Zoom,
    /// Scroll the settings list (when the terminal is short): -1 up, 1 down.
    More(i8),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    Path,
    Options,
}

struct Loaded {
    path: PathBuf,
    img: RgbaImage,
    info: Analysis,
    /// The preset that suits the picture.
    suggested: Preset,
}

struct Preview {
    /// The options it was made with (the preview's own width).
    opts: ImportOptions,
    /// The photo filter preset it was made with.
    filter: usize,
    fit: bool,
    conv: Converted,
    ms: u128,
}

pub struct ImportDialog {
    reference: bool,
    doc_kind: DocKind,
    doc_width: usize,
    doc_palette: Palette,
    doc_ice: bool,
    path: LineInput,
    focus: Focus,
    files: Vec<PathBuf>,
    filtered: Vec<usize>,
    list: ListState,
    list_area: Rect,
    img: Option<Loaded>,
    preset: Option<Preset>,
    opts: ImportOptions,
    into: Into,
    colors: Colors,
    row: usize,
    /// Show the result cell for cell (scrolling) instead of fitted.
    one_to_one: bool,
    scroll: (usize, usize),
    preview: Option<Preview>,
    preview_area: Rect,
    error: Option<String>,
    btns: Buttons<Act>,
    /// (row y, option, x where the value's ‹ sits).
    opt_hits: Vec<(u16, Opt, u16)>,
    /// First settings row shown, when they don't all fit.
    opt_top: usize,
    /// Settings rows that fit last frame.
    opt_room: usize,
    /// Photo filter preset (index into `filters::PRESETS`; 0 is none).
    filter: usize,
    /// The picture through that filter: (preset, image).
    filtered_img: Option<(usize, RgbaImage)>,
}

impl ImportDialog {
    pub fn new(app: &App, reference: bool) -> Self {
        let doc = &app.tab().doc;
        let mut d = ImportDialog {
            reference,
            doc_kind: doc.meta.kind,
            doc_width: doc.width(),
            doc_palette: doc.meta.palette.clone(),
            doc_ice: doc.meta.ice,
            path: LineInput::default(),
            focus: Focus::Path,
            files: scan_images(),
            filtered: vec![],
            list: ListState::default(),
            list_area: Rect::default(),
            img: None,
            preset: None,
            opts: ImportOptions { width: doc.width(), kind: doc.meta.kind, ..ImportOptions::default() },
            into: if reference { Into::Layer } else { Into::Stamp },
            colors: if doc.meta.kind == DocKind::Modern { Colors::Truecolor } else { Colors::Sixteen },
            row: 0,
            one_to_one: false,
            scroll: (0, 0),
            preview: None,
            preview_area: Rect::default(),
            error: None,
            btns: Buttons::default(),
            opt_hits: vec![],
            opt_top: 0,
            opt_room: usize::MAX,
            filter: 0,
            filtered_img: None,
        };
        d.refilter();
        d
    }

    /// Open straight onto a file (tests, drag and drop).
    fn load(&mut self, path: &Path) {
        self.error = None;
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                self.error = Some(format!("can't read {}: {e}", path.display()));
                return;
            }
        };
        let img = match import::decode(&bytes) {
            Ok(i) => i,
            Err(e) => {
                self.error = Some(format!("not an image I can read: {e}"));
                return;
            }
        };
        let info = import::analyze(&img);
        let suggested = import::suggest(&img);
        self.path = LineInput::new(&path.display().to_string());
        self.img = Some(Loaded { path: path.to_path_buf(), img, info, suggested });
        self.filtered_img = None;
        self.focus = Focus::Options;
        self.row = 0;
        self.scroll = (0, 0);
        // Pixel art with few colors keeps them exactly in a new document.
        if info.pixel_art && info.colors <= 16 && self.into == Into::Document && self.colors == Colors::Sixteen {
            self.colors = Colors::Fitted;
        }
        self.apply_preset(suggested);
        if info.pixel_art && info.native.0 as usize <= 160 {
            self.size(3);
        }
    }

    fn refilter(&mut self) {
        let q = self.path.text.trim().to_string();
        let mut scored: Vec<(i32, usize)> = self
            .files
            .iter()
            .enumerate()
            .filter_map(|(i, p)| fuzzy(&q, &short(p)).map(|s| (s, i)))
            .collect();
        if !q.is_empty() {
            scored.sort_by_key(|x| std::cmp::Reverse(x.0));
        }
        self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        self.list.selected = 0;
    }

    fn pick(&mut self) {
        let typed = expand_tilde(self.path.text.trim());
        if !self.path.text.trim().is_empty() && typed.is_file() {
            self.load(&typed);
        } else if let Some(&i) = self.filtered.get(self.list.selected) {
            let p = self.files[i].clone();
            self.load(&p);
        } else {
            self.error = Some("No image there. Type a path (~ works) or pick one below.".into());
        }
    }

    fn classic(&self) -> bool {
        self.colors != Colors::Truecolor
    }

    /// Options as they will be used, with the document's colors filled in.
    fn effective(&self) -> ImportOptions {
        let classic = self.classic();
        let mut o = self.opts.clone();
        o.kind = if classic { DocKind::Classic } else { DocKind::Modern };
        o.fit_palette = self.colors == Colors::Fitted;
        o.palette = (self.into != Into::Document).then(|| self.doc_palette.clone());
        if classic {
            o.glyphs = Glyphs::Cp437;
        }
        o
    }

    fn ice(&self) -> bool {
        self.into == Into::Document || self.doc_ice
    }

    fn apply_preset(&mut self, p: Preset) {
        self.preset = Some(p);
        self.opts = p.apply(&self.effective());
        if !self.classic() && p == Preset::Photo {
            self.opts.glyphs = Glyphs::Extended;
        }
    }

    /// A setting changed by hand: the preset no longer describes it.
    fn touched(&mut self) {
        self.preset = None;
    }

    fn size(&mut self, which: u8) {
        self.opts.height = None;
        match which {
            0 => self.opts.width = self.doc_width,
            1 => self.opts.width = 80,
            2 => {
                self.opts.width = 80;
                self.opts.height = Some(25);
            }
            _ => {
                if let Some(l) = &self.img {
                    let (w, h) = l.info.native;
                    self.opts.width = match self.opts.style {
                        ImportStyle::HalfBlock => w as usize,
                        ImportStyle::Blocks => (w as usize).div_ceil(8),
                        ImportStyle::Ascii => (w as usize).div_ceil(8).max((h as usize).div_ceil(16)),
                    }
                    .clamp(4, 1000);
                }
            }
        }
    }

    fn rows(&self) -> Vec<Opt> {
        let mut v = vec![];
        if !self.reference {
            v.push(Opt::Into);
        }
        v.extend([Opt::Colors, Opt::Width, Opt::Style]);
        let blocks = self.opts.style == ImportStyle::Blocks;
        if blocks && !self.classic() {
            v.push(Opt::Glyphs);
        }
        if blocks && self.classic() {
            v.push(Opt::Shades);
        }
        if self.classic() || self.opts.style == ImportStyle::Ascii {
            v.push(Opt::Dither);
        }
        v.extend([
            Opt::Scaling,
            Opt::Filter,
            Opt::Brightness,
            Opt::Contrast,
            Opt::Saturation,
            Opt::Sharpen,
            Opt::Levels,
        ]);
        if self.opts.style != ImportStyle::Ascii && self.opts.scaling != Scaling::Pixel {
            v.push(Opt::Lines);
        }
        v.push(Opt::Ink);
        if self.opts.style != ImportStyle::Ascii {
            v.push(Opt::Cleanup);
        }
        v
    }

    fn label(o: Opt) -> &'static str {
        match o {
            Opt::Into => "Into",
            Opt::Colors => "Colors",
            Opt::Width => "Width",
            Opt::Style => "Style",
            Opt::Glyphs => "Glyphs",
            Opt::Shades => "Shades ░▒▓",
            Opt::Dither => "Dither",
            Opt::Scaling => "Scaling",
            Opt::Filter => "Photo filter",
            Opt::Brightness => "Brightness",
            Opt::Contrast => "Contrast",
            Opt::Saturation => "Saturation",
            Opt::Sharpen => "Sharpen",
            Opt::Levels => "Auto levels",
            Opt::Lines => "Keep lines",
            Opt::Ink => "Dark to black",
            Opt::Cleanup => "Remove specks",
        }
    }

    fn value(&self, o: Opt) -> String {
        let a = &self.opts.adjust;
        let onoff = |b: bool| if b { "on" } else { "off" }.to_string();
        match o {
            Opt::Into => match self.into {
                Into::Layer => "new layer",
                Into::Document => "new document",
                Into::Stamp => "stamp (place it)",
            }
            .into(),
            Opt::Colors => match (self.colors, self.into == Into::Document, self.doc_kind) {
                (Colors::Sixteen, false, _) => "document's 16".into(),
                (Colors::Sixteen, true, _) => "16 VGA".into(),
                (Colors::Fitted, ..) => "16 fitted".into(),
                (Colors::Truecolor, false, DocKind::Classic) => "truecolor (→ Modern)".into(),
                (Colors::Truecolor, ..) => "truecolor".into(),
            },
            Opt::Width => match self.opts.height {
                Some(h) => format!("{} (fit {h} rows)", self.opts.width),
                None => self.opts.width.to_string(),
            },
            Opt::Style => match self.opts.style {
                ImportStyle::HalfBlock => "half blocks ▀▄",
                ImportStyle::Blocks => "best-fit blocks",
                ImportStyle::Ascii => "ascii",
            }
            .into(),
            Opt::Glyphs => match self.opts.glyphs {
                Glyphs::Cp437 => "CP437 ▀▄▌▐",
                Glyphs::Extended => "+ ▚▖▂▎🬗 (Unicode)",
            }
            .into(),
            Opt::Shades => onoff(self.opts.shades),
            Opt::Dither => match self.opts.dither {
                Dither::None => "none",
                Dither::Diffuse => "diffuse",
                Dither::Ordered => "ordered",
            }
            .into(),
            Opt::Scaling => {
                let pixel = self.preview.as_ref().is_some_and(|p| p.conv.pixel);
                match self.opts.scaling {
                    Scaling::Auto if pixel => "auto (pixels)".into(),
                    Scaling::Auto => "auto (smooth)".into(),
                    Scaling::Smooth => "smooth".into(),
                    Scaling::Pixel => "pixels".into(),
                }
            }
            Opt::Filter => PRESETS[self.filter].name.into(),
            Opt::Brightness => signed(a.brightness),
            Opt::Contrast => signed(a.contrast),
            Opt::Saturation => format!("{}%", a.saturation),
            Opt::Sharpen => a.sharpen.to_string(),
            Opt::Levels => onoff(a.auto_levels),
            Opt::Lines => if self.opts.lines == 0 { "off".into() } else { format!("{}%", self.opts.lines) },
            Opt::Ink => onoff(self.opts.ink),
            Opt::Cleanup => onoff(self.opts.cleanup),
        }
    }

    fn step(&mut self, o: Opt, dir: i32) {
        fn cycle<T: Copy + PartialEq>(all: &[T], cur: T, dir: i32) -> T {
            let i = all.iter().position(|x| *x == cur).unwrap_or(0) as i32;
            all[(i + dir).rem_euclid(all.len() as i32) as usize]
        }
        let a = &mut self.opts.adjust;
        match o {
            Opt::Into => {
                self.into = cycle(&[Into::Stamp, Into::Layer, Into::Document], self.into, dir);
                if self.into != Into::Document && self.colors == Colors::Fitted {
                    self.colors = Colors::Sixteen;
                }
                return;
            }
            Opt::Colors => {
                let choices: &[Colors] = if self.into == Into::Document {
                    &[Colors::Sixteen, Colors::Fitted, Colors::Truecolor]
                } else {
                    &[Colors::Sixteen, Colors::Truecolor]
                };
                self.colors = cycle(choices, self.colors, dir);
                if let Some(p) = self.preset {
                    self.apply_preset(p);
                }
                return;
            }
            Opt::Width => {
                self.opts.height = None;
                self.opts.width = (self.opts.width as i32 + dir).clamp(4, 1000) as usize;
                return;
            }
            Opt::Style => {
                self.opts.style =
                    cycle(&[ImportStyle::HalfBlock, ImportStyle::Blocks, ImportStyle::Ascii], self.opts.style, dir)
            }
            Opt::Glyphs => self.opts.glyphs = cycle(&[Glyphs::Cp437, Glyphs::Extended], self.opts.glyphs, dir),
            Opt::Shades => self.opts.shades = !self.opts.shades,
            Opt::Dither => {
                self.opts.dither = cycle(&[Dither::None, Dither::Diffuse, Dither::Ordered], self.opts.dither, dir)
            }
            Opt::Scaling => {
                self.opts.scaling = cycle(&[Scaling::Auto, Scaling::Smooth, Scaling::Pixel], self.opts.scaling, dir)
            }
            Opt::Filter => {
                self.filter = (self.filter as i32 + dir).rem_euclid(PRESETS.len() as i32) as usize;
                return;
            }
            Opt::Brightness => a.brightness = (a.brightness + 10 * dir).clamp(-100, 100),
            Opt::Contrast => a.contrast = (a.contrast + 10 * dir).clamp(-100, 100),
            Opt::Saturation => a.saturation = (a.saturation + 10 * dir).clamp(0, 300),
            Opt::Sharpen => a.sharpen = (a.sharpen as i32 + 10 * dir).clamp(0, 100) as u32,
            Opt::Levels => a.auto_levels = !a.auto_levels,
            Opt::Lines => self.opts.lines = (self.opts.lines as i32 + 20 * dir).clamp(0, 100) as u32,
            Opt::Ink => self.opts.ink = !self.opts.ink,
            Opt::Cleanup => self.opts.cleanup = !self.opts.cleanup,
        }
        self.touched();
    }

    /// Final size in cells.
    fn cells(&self) -> Option<(usize, usize)> {
        let l = self.img.as_ref()?;
        Some(import::target_size(l.img.width(), l.img.height(), &self.effective()))
    }

    /// The picture to convert: the loaded one, through the photo filter.
    fn source(&mut self) -> Option<&RgbaImage> {
        let l = self.img.as_ref()?;
        if self.filter == 0 {
            return Some(&l.img);
        }
        if self.filtered_img.as_ref().is_none_or(|(f, _)| *f != self.filter) {
            let img = Filter::preset(self.filter).apply_image(&l.img);
            self.filtered_img = Some((self.filter, img));
        }
        self.filtered_img.as_ref().map(|(_, img)| img)
    }

    /// Convert for the preview pane when the options changed.
    fn refresh(&mut self, pane: Rect) {
        let Some(l) = &self.img else {
            return;
        };
        let full = self.effective();
        let (cw, ch) = import::target_size(l.img.width(), l.img.height(), &full);
        let fit = !self.one_to_one && (cw > pane.width as usize || ch > pane.height as usize);
        let opts = if fit {
            let rows = pane.height as usize;
            ImportOptions { width: cw.min(pane.width as usize), height: Some(rows.min(ch)), ..full }
        } else {
            full
        };
        if self.preview.as_ref().is_some_and(|p| p.opts == opts && p.fit == fit && p.filter == self.filter) {
            return;
        }
        let t = Instant::now();
        let (ice, filter) = (self.ice(), self.filter);
        let Some(src) = self.source() else { return };
        match import::convert(src, &opts, ice) {
            Ok(conv) => {
                self.preview = Some(Preview { opts, filter, fit, conv, ms: t.elapsed().as_millis() });
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    fn submit(&mut self) -> Outcome {
        if self.img.is_none() {
            self.pick();
            return Outcome::Keep;
        }
        let (opts, ice) = (self.effective(), self.ice());
        let Some(src) = self.source() else { return Outcome::Keep };
        let conv = match import::convert(src, &opts, ice) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(format!("import failed: {e:#}"));
                return Outcome::Keep;
            }
        };
        let Some(l) = &self.img else { return Outcome::Keep };
        let name = l.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
        let (into, reference) = (self.into, self.reference);
        let modern = opts.kind == DocKind::Modern;
        Outcome::Then(Box::new(move |app: &mut App| {
            if into == Into::Document {
                let kind = if modern { DocKind::Modern } else { DocKind::Classic };
                let d = import::converted_doc(conv, kind);
                app.replace_doc(Tab::new(d, None), format!("{name} imported as a new document"));
                return;
            }
            if modern && app.tab().doc.meta.kind == DocKind::Classic {
                app.tab_mut().edit("Switch to Modern", |b| tools::set_kind(b, DocKind::Modern));
            }
            if into == Into::Stamp {
                app.float(conv.clip, crate::tools_ctl::FloatSource::Paste);
                return;
            }
            add_layer(app, conv.clip, &name, reference);
        }))
    }

    // ------------------------------------------------------------ drawing

    fn draw_picker(&mut self, f: &mut Frame, area: Rect) {
        let mut y = area.y;
        if let Some(e) = &self.error {
            f.render_widget(Paragraph::new(Span::styled(e.clone(), Style::new().fg(theme::ERR))), Rect::new(area.x, y, area.width, 1));
            y += 1;
        }
        f.render_widget(
            Paragraph::new(Span::styled(
                "Images here and in Downloads, Desktop, Pictures. PNG, GIF, JPEG, WebP, BMP.",
                Style::new().fg(theme::DIM),
            )),
            Rect::new(area.x, y, area.width, 1),
        );
        y += 1;
        let lr = Rect::new(area.x, y, area.width, area.bottom().saturating_sub(y));
        self.list_area = lr;
        if self.filtered.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled("  no images found — type a path, or drop a file here", Style::new().fg(theme::DIM))),
                lr,
            );
            return;
        }
        let range = self.list.visible(self.filtered.len(), lr.height as usize);
        let lines: Vec<_> = range
            .map(|i| {
                let p = &self.files[self.filtered[i]];
                let ext = p.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
                list_line(short(p), ext, i == self.list.selected, lr.width)
            })
            .collect();
        f.render_widget(Paragraph::new(lines), lr);
    }

    fn draw_options(&mut self, f: &mut Frame, area: Rect) {
        let buf = f.buffer_mut();
        let mut y = area.y;
        // Presets and sizes as chips.
        buf.set_string(area.x, y, "PRESET", Style::new().fg(theme::DIM).add_modifier(Modifier::BOLD));
        y += 1;
        let chips: Vec<_> = Preset::ALL
            .iter()
            .enumerate()
            .map(|(i, p)| {
                btn(Act::Preset(i), "", p.name()).kind(if self.preset == Some(*p) { BtnKind::On } else { BtnKind::Normal })
            })
            .collect();
        y += self.btns.row(buf, Rect::new(area.x, y, area.width, 2), &chips);
        y += 1;
        buf.set_string(area.x, y, "SIZE", Style::new().fg(theme::DIM).add_modifier(Modifier::BOLD));
        y += 1;
        let doc_w = format!("doc {}", self.doc_width);
        let mut sizes = vec![btn(Act::Size(1), "", "80"), btn(Act::Size(2), "", "fit 80×25"), btn(Act::Size(3), "", "native")];
        if self.into != Into::Document && self.doc_width != 80 {
            sizes.insert(0, btn(Act::Size(0), "", doc_w.as_str()));
        }
        y += self.btns.row(buf, Rect::new(area.x, y, area.width, 2), &sizes);
        y += 1;

        self.opt_hits.clear();
        let rows = self.rows();
        self.row = self.row.min(rows.len().saturating_sub(1));
        let label_w = 15u16;
        // A short terminal: scroll the settings, keeping the focused one in
        // view, with ▲ ▼ buttons on the last line.
        let room = area.bottom().saturating_sub(y) as usize;
        let fits = rows.len() <= room;
        let shown = if fits { rows.len() } else { room.saturating_sub(1).max(1) };
        self.opt_room = shown;
        if fits {
            self.opt_top = 0;
        } else {
            self.opt_top = self.opt_top.min(rows.len() - shown);
        }
        for (i, o) in rows.iter().enumerate().skip(self.opt_top).take(shown) {
            let focused = self.focus == Focus::Options && i == self.row;
            let st = if focused { Style::new().fg(theme::BG).bg(theme::ACCENT2) } else { Style::new().fg(theme::TEXT) };
            let label = format!("{:>w$} ", Self::label(*o), w = label_w as usize);
            buf.set_string(area.x, y, &label, Style::new().fg(theme::DIM));
            let vx = area.x + label_w + 1;
            let val: String = self.value(*o).chars().take((area.width.saturating_sub(label_w + 5)) as usize).collect();
            buf.set_string(vx, y, format!("‹ {val} ›"), st);
            self.opt_hits.push((y, *o, vx));
            y += 1;
        }
        if !fits && y < area.bottom() {
            let (up, down) = (self.opt_top > 0, self.opt_top + shown < rows.len());
            let mut x = area.x + label_w - 5;
            x += self.btns.draw(buf, x, y, area.right(), &btn(Act::More(-1), "", "▲").enabled(up)) + 1;
            x += self.btns.draw(buf, x, y, area.right(), &btn(Act::More(1), "", "▼").enabled(down)) + 1;
            let n = format!("{}–{} of {}", self.opt_top + 1, self.opt_top + shown, rows.len());
            buf.set_stringn(x + 1, y, n, area.right().saturating_sub(x + 1) as usize, Style::new().fg(theme::DIM));
        }
    }

    /// Scroll the settings so the focused row shows.
    fn follow(&mut self) {
        if self.row < self.opt_top {
            self.opt_top = self.row;
        } else if self.row >= self.opt_top + self.opt_room {
            self.opt_top = self.row + 1 - self.opt_room;
        }
    }

    fn draw_preview(&mut self, f: &mut Frame, area: Rect) {
        self.preview_area = area;
        self.refresh(area);
        let Some(p) = &self.preview else {
            return;
        };
        let clip: &Clip = &p.conv.clip;
        let pal = &p.conv.palette;
        let (w, h) = (clip.width, clip.height);
        let (vw, vh) = (area.width as usize, area.height as usize);
        self.scroll.0 = self.scroll.0.min(w.saturating_sub(vw));
        self.scroll.1 = self.scroll.1.min(h.saturating_sub(vh));
        // Centered when smaller than the pane.
        let ox = area.x + (vw.saturating_sub(w) / 2) as u16;
        let oy = area.y + (vh.saturating_sub(h) / 2) as u16;
        let buf = f.buffer_mut();
        for y in 0..h.min(vh) {
            for x in 0..w.min(vw) {
                let (sx, sy) = (x + self.scroll.0, y + self.scroll.1);
                let Some(cell) = buf.cell_mut((ox + x as u16, oy + y as u16)) else {
                    continue;
                };
                match clip.get(sx, sy) {
                    Some(c) => {
                        cell.set_char(c.ch).set_fg(rgb(c.fg, pal)).set_bg(rgb(c.bg, pal));
                    }
                    None => {
                        // Transparent: a faint checker.
                        let dark = (sx / 2 + sy) % 2 == 0;
                        cell.set_char(' ').set_bg(if dark { theme::OUTSIDE } else { theme::PANEL });
                    }
                }
            }
        }
    }

    fn status(&self) -> String {
        let Some(l) = &self.img else {
            return String::new();
        };
        let i = &l.info;
        let colors = if i.colors > 256 { "many colors".to_string() } else { format!("{} colors", i.colors) };
        let scale = if i.native.0 != i.width { format!(" ({}×{} at {}x)", i.native.0, i.native.1, i.width / i.native.0.max(1)) } else { String::new() };
        let kind = match l.suggested {
            Preset::Photo => String::new(),
            p => format!(" · looks like {}", p.name()),
        };
        format!("{}×{}{scale} · {colors}{kind}", i.width, i.height)
    }
}

impl Dialog for ImportDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        let r = centered(area, area.width.saturating_sub(4).clamp(60, 150), area.height.saturating_sub(2).clamp(20, 50));
        let title = if self.reference { "Reference image layer" } else { "Import image" };
        let hint = if self.img.is_none() {
            "type to filter or a path · Tab complete · Enter open · Esc"
        } else {
            "↑↓ setting · ←→ change · 1-7 preset · Enter import · Esc"
        };
        let inner = popup(f, r, title, hint);
        self.btns.clear();
        // File line.
        let fr = Rect::new(inner.x, inner.y, inner.width, 1);
        if self.img.is_none() || self.focus == Focus::Path {
            self.path.render(f, fr, "Image ", self.focus == Focus::Path);
        } else if let Some(l) = &self.img {
            let name = l.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let buf = f.buffer_mut();
            buf.set_string(fr.x, fr.y, "Image ", Style::new().fg(theme::DIM));
            buf.set_string(fr.x + 6, fr.y, &name, Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD));
            let x = fr.x + 7 + name.chars().count() as u16;
            let w = self.btns.draw(buf, x, fr.y, fr.right(), &btn(Act::Change, "", "change…"));
            let room = fr.right().saturating_sub(x + w + 1) as usize;
            buf.set_stringn(x + w + 1, fr.y, self.status(), room, Style::new().fg(theme::DIM));
        }
        let body = Rect::new(inner.x, inner.y + 2, inner.width, inner.height.saturating_sub(3));
        if self.img.is_none() {
            self.draw_picker(f, body);
            return;
        }
        let left_w = 42.min(body.width / 2);
        let left = Rect::new(body.x, body.y, left_w, body.height);
        let right = Rect::new(body.x + left_w + 1, body.y, body.width.saturating_sub(left_w + 1), body.height.saturating_sub(1));
        for y in body.y..body.bottom() {
            f.buffer_mut().set_string(body.x + left_w, y, "│", Style::new().fg(theme::BORDER));
        }
        self.draw_options(f, left);
        self.draw_preview(f, right);

        // Under the preview: what you see, then the buttons.
        let by = inner.bottom().saturating_sub(1);
        let mut note = match (self.cells(), &self.preview) {
            (Some((w, h)), Some(p)) if p.fit => {
                format!("{w}×{h} cells · preview shrunk to {}×{} · {} ms", p.conv.clip.width, p.conv.clip.height, p.ms)
            }
            (Some((w, h)), Some(p)) => format!("{w}×{h} cells · {} ms", p.ms),
            _ => String::new(),
        };
        if let Some(e) = &self.error {
            note = e.clone();
        }
        let buf = f.buffer_mut();
        let mut x = inner.x;
        let import = if self.reference { "Add layer" } else { "Import" };
        x += self.btns.draw(buf, x, by, inner.right(), &btn(Act::Import, "Enter", import).primary()) + 1;
        x += self.btns.draw(buf, x, by, inner.right(), &btn(Act::Cancel, "Esc", "Cancel")) + 1;
        let zoom = if self.one_to_one { "fit" } else { "1:1" };
        let big = self.preview.as_ref().is_some_and(|p| p.fit) || self.one_to_one;
        if big {
            x += self.btns.draw(buf, x, by, inner.right(), &btn(Act::Zoom, "z", zoom)) + 1;
        }
        let st = if self.error.is_some() { Style::new().fg(theme::ERR) } else { Style::new().fg(theme::DIM) };
        let room = inner.right().saturating_sub(x + 1) as usize;
        buf.set_string(x + 1, by, note.chars().take(room).collect::<String>(), st);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        match k.code {
            KeyCode::Esc if self.img.is_some() && self.focus == Focus::Path => {
                self.focus = Focus::Options;
                return Outcome::Keep;
            }
            KeyCode::Esc => return Outcome::Close,
            _ => {}
        }
        if self.focus == Focus::Path {
            match k.code {
                KeyCode::Enter => self.pick(),
                KeyCode::Tab => self.complete(),
                _ => {
                    if !(self.img.is_none() && self.list.key(&k, self.filtered.len(), 10)) && self.path.key(&k) {
                        self.refilter();
                    }
                }
            }
            return Outcome::Keep;
        }
        let rows = self.rows();
        match k.code {
            KeyCode::Enter => return self.submit(),
            KeyCode::Up => {
                self.row = (self.row + rows.len() - 1) % rows.len();
                self.follow();
            }
            KeyCode::Down | KeyCode::Tab => {
                self.row = (self.row + 1) % rows.len();
                self.follow();
            }
            KeyCode::Left => self.step(rows[self.row.min(rows.len() - 1)], -1),
            KeyCode::Right | KeyCode::Char(' ') => self.step(rows[self.row.min(rows.len() - 1)], 1),
            KeyCode::PageUp if rows.get(self.row) == Some(&Opt::Width) => self.step(Opt::Width, 10),
            KeyCode::PageDown if rows.get(self.row) == Some(&Opt::Width) => self.step(Opt::Width, -10),
            KeyCode::Char(c @ '1'..='7') => self.apply_preset(Preset::ALL[c as usize - '1' as usize]),
            KeyCode::Char('z') => self.one_to_one = !self.one_to_one,
            KeyCode::Char('o') => self.focus = Focus::Path,
            _ => {}
        }
        Outcome::Keep
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        if let Some(a) = self.btns.mouse(&m) {
            match a {
                Act::Preset(i) => self.apply_preset(Preset::ALL[i]),
                Act::Size(s) => self.size(s),
                Act::Import => return self.submit(),
                Act::Cancel => return Outcome::Close,
                Act::Change => {
                    self.focus = Focus::Path;
                    self.img = None;
                    self.preview = None;
                    self.path = LineInput::default();
                    self.refilter();
                }
                Act::Zoom => self.one_to_one = !self.one_to_one,
                Act::More(d) => {
                    let page = self.opt_room.saturating_sub(1).max(1);
                    self.opt_top = if d < 0 { self.opt_top.saturating_sub(page) } else { self.opt_top + page };
                }
            }
            return Outcome::Keep;
        }
        let r = self.list_area;
        let inside = |r: Rect| m.row >= r.y && m.row < r.bottom() && m.column >= r.x && m.column < r.right();
        match m.kind {
            MouseEventKind::Down(_) if self.img.is_none() && inside(r) => {
                let i = self.list.offset + (m.row - r.y) as usize;
                if i < self.filtered.len() {
                    self.list.selected = i;
                    self.path = LineInput::default();
                    self.pick();
                }
            }
            MouseEventKind::Down(_) => {
                if let Some(&(_, o, vx)) = self.opt_hits.iter().find(|(y, ..)| *y == m.row)
                    && m.column >= vx
                {
                    self.focus = Focus::Options;
                    self.row = self.rows().iter().position(|x| *x == o).unwrap_or(0);
                    self.step(o, if m.column < vx + 2 { -1 } else { 1 });
                }
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let d = if m.kind == MouseEventKind::ScrollDown { 1 } else { -1 };
                if self.img.is_none() {
                    let n = self.filtered.len().saturating_sub(1);
                    self.list.selected = (self.list.selected as i32 + d).clamp(0, n as i32) as usize;
                } else if inside(self.preview_area) {
                    self.scroll.1 = (self.scroll.1 as i32 + 3 * d).max(0) as usize;
                } else if let Some(&(_, o, _)) = self.opt_hits.iter().find(|(y, ..)| *y == m.row) {
                    self.step(o, -d);
                }
            }
            _ => {}
        }
        Outcome::Keep
    }

    fn paste(&mut self, s: &str) {
        // A dropped file arrives as its path, sometimes quoted or escaped.
        let p = s.trim().trim_matches(['\'', '"']).replace("\\ ", " ");
        let path = expand_tilde(&p);
        if path.is_file() {
            self.load(&path);
        } else if self.focus == Focus::Path {
            self.path.paste(s.trim());
            self.refilter();
        }
    }
}

impl ImportDialog {
    fn complete(&mut self) {
        let typed = expand_tilde(self.path.text.trim());
        let (dir, prefix) = if self.path.text.ends_with('/') {
            (typed.clone(), String::new())
        } else {
            (
                typed.parent().map(Path::to_path_buf).unwrap_or_default(),
                typed.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            )
        };
        let dir = if dir.as_os_str().is_empty() { PathBuf::from(".") } else { dir };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut matches: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&prefix)))
            .filter(|p| p.is_dir() || is_image(p))
            .collect();
        matches.sort();
        if let Some(m) = matches.first() {
            let mut s = m.display().to_string();
            if m.is_dir() {
                s.push('/');
                self.files = scan_dir(m, 3);
            }
            self.path = LineInput::new(&s);
            self.refilter();
        }
    }
}

/// Add `clip` as a new layer above the others (a locked, dimmed reference
/// layer plus a fresh drawing layer when `reference`).
fn add_layer(app: &mut App, clip: Clip, name: &str, reference: bool) {
    let t = app.tab_mut();
    let at = t.doc.canvas.layers.len();
    let mut idx = at;
    t.edit(if reference { "Reference image" } else { "Import image" }, |b| {
        if b.width() < clip.width || b.height() < clip.height {
            let (w, h) = (b.width().max(clip.width), b.height().max(clip.height));
            tools::resize(b, w, h);
        }
        idx = tools::add_layer(b, name, at);
        tools::stamp(b, idx, &clip, 0, 0, tools::StampMode::Opaque);
        if reference {
            tools::set_layer_props(
                b,
                idx,
                &tools::LayerProps { reference: Some(true), locked: Some(true), ..Default::default() },
            );
        }
    });
    if reference {
        // Draw on a fresh layer above the reference.
        let above = idx + 1;
        t.edit("Add layer", |b| {
            tools::add_layer(b, "Drawing", above);
        });
        t.layer = above;
    } else {
        t.layer = idx;
    }
    app.flash(format!("imported {name} as a layer"), Level::Ok);
}

fn signed(v: i32) -> String {
    if v > 0 { format!("+{v}") } else { v.to_string() }
}

fn is_image(p: &Path) -> bool {
    p.extension().is_some_and(|e| IMAGE_EXT.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn scan_dir(root: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, d)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            if p.is_dir() {
                if d < depth {
                    stack.push((p, d + 1));
                }
            } else if is_image(&p) {
                out.push(p);
                if out.len() >= 2000 {
                    return out;
                }
            }
        }
    }
    out
}

/// Images under the current directory, then the usual download spots.
fn scan_images() -> Vec<PathBuf> {
    let mut v = scan_dir(Path::new("."), 3);
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        // Newest first: the picture you just saved is the one you want.
        let mut more = vec![];
        for d in ["Downloads", "Desktop", "Pictures"] {
            more.extend(scan_dir(&home.join(d), 1));
        }
        let modified = |p: &PathBuf| p.metadata().and_then(|m| m.modified()).ok();
        more.sort_by_key(|p| std::cmp::Reverse(modified(p)));
        v.extend(more);
    }
    v
}

/// A path as the list shows it: relative here, ~ at home.
fn short(p: &Path) -> String {
    let s = p.strip_prefix(".").unwrap_or(p).display().to_string();
    match std::env::var("HOME") {
        Ok(h) if s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_paths() {
        assert_eq!(short(Path::new("./art/x.png")), "art/x.png");
        assert!(is_image(Path::new("a.JPG")));
        assert!(!is_image(Path::new("a.ans")));
    }
}
