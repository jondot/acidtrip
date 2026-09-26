//! Save / Export: one dialog, two modes.
//!
//! * **Save** keeps working on the file: only formats acidtrip reopens
//!   exactly, and the tab adopts the new file + format (Ctrl-S resaves).
//! * **Export** writes a copy (images, web, code arrays, art files); the tab's
//!   file and format stay untouched.

use std::path::{Path, PathBuf};

use acidtrip_io::format::{self, Format, GifMode, SaveOptions};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{Dialog, Outcome};
use crate::app::{App, Level};
use crate::dialogs::forms::expand_tilde;
use crate::dialogs::prompt::ConfirmDialog;
use crate::ui::widgets::{Buttons, LineInput, btn, centered, popup, theme};

/// Formats that load back exactly, in the order the Save list shows them.
const SAVE_FORMATS: [Format; 11] = [
    Format::Acid,
    Format::Ansi,
    Format::XBin,
    Format::Bin,
    Format::Adf,
    Format::Idf,
    Format::Tnd,
    Format::Utf8Ansi,
    Format::Pcb,
    Format::Avt,
    Format::Ascii,
];

/// Output-only formats, listed first in Export mode.
const EXPORT_FORMATS: [Format; 10] = [
    Format::Png,
    Format::Gif,
    Format::Svg,
    Format::Html,
    Format::React,
    Format::CArray,
    Format::PascalArray,
    Format::AsmArray,
    Format::Mirc,
    Format::Asciicast,
];

const ART_HEADING: &str = "also as art file:";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Save,
    Export,
}

/// The buttons under the options, so one mouse button saves.
#[derive(Clone, Copy)]
enum Do {
    Go,
    Cancel,
}

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Mode,
    Formats,
    Path,
    Options,
}

pub struct ExportDialog {
    mode: Mode,
    /// Selected index into [`Self::formats`] per mode.
    save_sel: usize,
    export_sel: usize,
    /// First visible list row.
    offset: usize,
    path: LineInput,
    /// The path text we generated last; if the input still equals it, the
    /// user hasn't edited it and we may regenerate freely.
    generated: String,
    focus: Focus,
    opts: SaveOptions,
    opt_idx: usize,
    /// File name stem (with directory) the default path is built from.
    stem: PathBuf,
    /// Loss warnings per format for this document (computed once).
    losses: Vec<(Format, Vec<String>)>,
    current_file: Option<PathBuf>,
    /// The document has more than one animation frame.
    animated: bool,
    error: Option<String>,
    // Hit areas from the last draw.
    save_btn: Rect,
    export_btn: Rect,
    list_area: Rect,
    path_area: Rect,
    opts_area: Rect,
    btns: Buttons<Do>,
}

impl ExportDialog {
    fn build(app: &App, mode: Mode) -> Self {
        let tab = app.tab();
        let doc = &tab.doc;
        let losses: Vec<_> = Format::ALL.iter().map(|&f| (f, format::loss_warnings(doc, f))).collect();
        let lossless = |f: Format| losses.iter().any(|(g, w)| *g == f && w.is_empty());

        let stem = match &tab.file {
            Some(p) => {
                let p = if p.is_absolute() {
                    p.clone()
                } else {
                    std::env::current_dir().map(|d| d.join(p)).unwrap_or_else(|_| p.clone())
                };
                p.with_extension("")
            }
            None => {
                let name = if doc.meta.sauce.title.trim().is_empty() {
                    "untitled".to_string()
                } else {
                    doc.meta.sauce.title.trim().to_lowercase().replace(' ', "-")
                };
                std::env::current_dir().map(|d| d.join(&name)).unwrap_or_else(|_| PathBuf::from(name))
            }
        };

        // Save: the tab's format if it reloads exactly, else ANSI when nothing
        // would be lost, else the native format that keeps everything.
        let save_fmt = tab.format.filter(|f| SAVE_FORMATS.contains(f)).unwrap_or(if lossless(Format::Ansi) {
            Format::Ansi
        } else {
            Format::Acid
        });
        let export_list = Self::list_for(Mode::Export);
        let save_sel = SAVE_FORMATS.iter().position(|f| *f == save_fmt).unwrap_or(0);
        let export_sel = export_list.iter().position(|f| *f == Format::Png).unwrap_or(0);

        let mut opts = SaveOptions { scale: app.config.share.png_scale, ..SaveOptions::default() };
        opts.sauce = Some(doc.meta.sauce.attach);
        opts.ice_hint = doc.meta.ice;
        if doc.is_animated() {
            opts.gif_mode = GifMode::Frames;
        }
        let mut d = ExportDialog {
            mode,
            save_sel,
            export_sel,
            offset: 0,
            path: LineInput::default(),
            generated: String::new(),
            focus: Focus::Formats,
            opts,
            opt_idx: 0,
            stem,
            losses,
            current_file: tab.file.clone(),
            animated: doc.is_animated(),
            error: None,
            save_btn: Rect::default(),
            export_btn: Rect::default(),
            list_area: Rect::default(),
            path_area: Rect::default(),
            opts_area: Rect::default(),
            btns: Buttons::default(),
        };
        d.sync_path();
        d
    }

    /// Open in Save mode.
    pub fn save_as(app: &App) -> Self {
        Self::build(app, Mode::Save)
    }

    /// Open in Export mode.
    pub fn export(app: &App) -> Self {
        Self::build(app, Mode::Export)
    }

    fn list_for(mode: Mode) -> Vec<Format> {
        match mode {
            Mode::Save => SAVE_FORMATS.to_vec(),
            Mode::Export => EXPORT_FORMATS.iter().chain(SAVE_FORMATS.iter()).copied().collect(),
        }
    }

    fn formats(&self) -> Vec<Format> {
        Self::list_for(self.mode)
    }

    fn sel(&self) -> usize {
        match self.mode {
            Mode::Save => self.save_sel,
            Mode::Export => self.export_sel,
        }
    }

    fn set_sel(&mut self, i: usize) {
        match self.mode {
            Mode::Save => self.save_sel = i,
            Mode::Export => self.export_sel = i,
        }
    }

    fn format(&self) -> Format {
        let list = self.formats();
        list[self.sel().min(list.len() - 1)]
    }

    fn losses(&self, f: Format) -> &[String] {
        self.losses.iter().find(|(g, _)| *g == f).map(|(_, w)| w.as_slice()).unwrap_or(&[])
    }

    /// List rows: `None` is the Export-mode subheading.
    fn rows(&self) -> Vec<Option<Format>> {
        match self.mode {
            Mode::Save => SAVE_FORMATS.iter().map(|f| Some(*f)).collect(),
            Mode::Export => EXPORT_FORMATS
                .iter()
                .map(|f| Some(*f))
                .chain(std::iter::once(None))
                .chain(SAVE_FORMATS.iter().map(|f| Some(*f)))
                .collect(),
        }
    }

    fn select(&mut self, i: usize) {
        self.set_sel(i);
        self.opt_idx = 0;
        self.error = None;
        self.sync_path();
    }

    fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.offset = 0;
        self.opt_idx = 0;
        self.error = None;
        // Keep a typed path's extension if it names a format in the new mode.
        let typed = Format::from_path(&expand_tilde(self.path.text.trim()));
        match typed.and_then(|f| self.formats().iter().position(|g| *g == f)) {
            Some(i) if self.path.text != self.generated => self.set_sel(i),
            _ => self.sync_path(),
        }
    }

    /// Rebuild the path for the selected format, keeping any directory/name
    /// the user typed.
    fn sync_path(&mut self) {
        if self.path.text != self.generated && !self.path.text.trim().is_empty() {
            let typed = PathBuf::from(self.path.text.trim());
            self.stem = if Format::from_path(&typed).is_some() { typed.with_extension("") } else { typed };
        }
        let fmt = self.format();
        let ext = fmt.extensions().first().copied().unwrap_or("txt");
        let name = if fmt == Format::React {
            self.stem.with_file_name(format!("{}.tsx", pascal(&self.stem)))
        } else {
            let file = self.stem.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.stem.with_file_name(format!("{file}.{ext}"))
        };
        self.path = LineInput::new(&name.display().to_string());
        self.generated = self.path.text.clone();
    }

    /// After editing the path: a typed extension selects its format.
    fn follow_extension(&mut self) {
        self.error = None;
        if let Some(f) = Format::from_path(Path::new(self.path.text.trim()))
            && let Some(i) = self.formats().iter().position(|g| *g == f)
            && i != self.sel()
        {
            self.set_sel(i);
            self.opt_idx = 0;
        }
    }

    /// Option rows for the current format: (label, value text).
    fn options(&self) -> Vec<(&'static str, String)> {
        option_rows(&self.opts, self.format(), self.animated)
    }

    fn cycle_option(&mut self, dir: i32) {
        if let Some((label, _)) = self.options().get(self.opt_idx).cloned() {
            cycle_option(&mut self.opts, label, dir);
        }
    }

    fn next_focus(&self, back: bool) -> Focus {
        let has_opts = !self.options().is_empty();
        let order: &[Focus] = if has_opts {
            &[Focus::Mode, Focus::Formats, Focus::Path, Focus::Options]
        } else {
            &[Focus::Mode, Focus::Formats, Focus::Path]
        };
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let n = order.len();
        order[if back { (i + n - 1) % n } else { (i + 1) % n }]
    }

    fn submit(&mut self) -> Outcome {
        let text = self.path.text.trim();
        if text.is_empty() {
            self.error = Some("Type a file name first.".into());
            self.focus = Focus::Path;
            return Outcome::Keep;
        }
        let path = expand_tilde(text);
        let typed = Format::from_path(&path);
        // A typed extension wins over the list selection (art.xb → XBin).
        let fmt = match typed {
            Some(f) if self.formats().contains(&f) => f,
            Some(f) if self.mode == Mode::Save => {
                self.error = Some(format!("{} doesn't reopen as art. Switch to Export (Ctrl-E).", f.name()));
                self.focus = Focus::Path;
                return Outcome::Keep;
            }
            _ => self.format(),
        };
        if path.is_dir() {
            self.error = Some("That's a folder. Add a file name.".into());
            self.focus = Focus::Path;
            return Outcome::Keep;
        }
        // Say so here, with the path still in the box, rather than after closing.
        if let Some(problem) = write_problem(&path) {
            self.error = Some(problem);
            self.focus = Focus::Path;
            return Outcome::Keep;
        }
        let opts = self.opts.clone();
        let mode = self.mode;
        let write = move |app: &mut App| match mode {
            Mode::Save => app.save_to(&path, fmt, &opts),
            Mode::Export => match format::save(&app.tab().doc, &path, fmt, &opts) {
                Ok(()) => app.flash(format!("exported {}", path.display()), Level::Ok),
                Err(e) => app.flash(format!("export failed: {e:#}"), Level::Error),
            },
        };
        let target = expand_tilde(text);
        let is_current = self.current_file.as_deref().is_some_and(|cur| same_file(cur, &target));
        if target.exists() && !is_current {
            let verb = if mode == Mode::Save { "Save" } else { "Export" };
            let msg = format!("{} already exists.\n{verb} over it?", target.display());
            // Stay open under the question: "no" comes back here, path and
            // all, to pick another name; "yes" closes this dialog too.
            let yes = move |app: &mut App| {
                app.dialogs.pop();
                write(app);
            };
            Outcome::KeepThen(Box::new(move |app: &mut App| {
                app.dialogs.push(Box::new(ConfirmDialog::new(&msg, Box::new(yes))));
            }))
        } else {
            Outcome::Then(Box::new(write))
        }
    }

    // ------------------------------------------------------------ drawing

    fn draw_mode(&mut self, f: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Mode;
        let button = |label: &str, on: bool| -> Span<'static> {
            let st = if on {
                Style::new()
                    .fg(theme::BG)
                    .bg(if focused { theme::ACCENT2 } else { theme::ACCENT })
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme::DIM).bg(theme::PANEL_HI)
            };
            Span::styled(format!("  {label}  "), st)
        };
        let save = button("Save", self.mode == Mode::Save);
        let export = button("Export", self.mode == Mode::Export);
        let (sw, ew) = (save.width() as u16, export.width() as u16);
        let x = area.x + 1;
        self.save_btn = Rect::new(x, area.y, sw, 1);
        self.export_btn = Rect::new(x + sw + 2, area.y, ew, 1);
        let mut spans = vec![Span::raw(" "), save, Span::raw("  "), export];
        if focused {
            spans.push(Span::styled("  ←→ switch", Style::new().fg(theme::DIM)));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), Rect::new(area.x, area.y, area.width, 1));
        let about = match self.mode {
            Mode::Save => "Keeps working on this file. Only formats acidtrip reopens exactly.",
            Mode::Export => "Writes a copy for sharing. Your document stays where it is.",
        };
        f.render_widget(
            Paragraph::new(Span::styled(format!(" {about}"), Style::new().fg(theme::DIM))),
            Rect::new(area.x, area.y + 1, area.width, 1),
        );
        f.render_widget(
            Paragraph::new(Span::styled("─".repeat(area.width as usize), Style::new().fg(theme::BORDER))),
            Rect::new(area.x, area.y + 2, area.width, 1),
        );
    }

    fn draw_list(&mut self, f: &mut Frame, area: Rect) {
        self.list_area = area;
        let rows = self.rows();
        let sel = self.format();
        let sel_row = rows.iter().position(|r| *r == Some(sel)).unwrap_or(0);
        let h = area.height as usize;
        if sel_row < self.offset {
            // Show the subheading above the first art format when scrolling up to it.
            self.offset = if sel_row > 0 && rows[sel_row - 1].is_none() { sel_row - 1 } else { sel_row };
        }
        if h > 0 && sel_row >= self.offset + h {
            self.offset = sel_row + 1 - h;
        }
        self.offset = self.offset.min(rows.len().saturating_sub(h));
        let w = area.width as usize;
        let focused = self.focus == Focus::Formats;
        let lines: Vec<Line> = rows
            .iter()
            .skip(self.offset)
            .take(h)
            .map(|row| match row {
                None => Line::from(Span::styled(format!(" {ART_HEADING}"), Style::new().fg(theme::DIM))),
                Some(fm) => self.format_line(*fm, *fm == sel, focused, w),
            })
            .collect();
        f.render_widget(Paragraph::new(lines), area);
        // Scroll hints sit on the pane divider.
        if self.offset > 0 {
            f.render_widget(
                Paragraph::new(Span::styled("▲", Style::new().fg(theme::DIM))),
                Rect::new(area.right(), area.y, 1, 1),
            );
        }
        if self.offset + h < rows.len() {
            f.render_widget(
                Paragraph::new(Span::styled("▼", Style::new().fg(theme::DIM))),
                Rect::new(area.right(), area.bottom().saturating_sub(1), 1, 1),
            );
        }
    }

    fn format_line(&self, fm: Format, selected: bool, focused: bool, w: usize) -> Line<'static> {
        let (badge, badge_color) = match self.mode {
            Mode::Save => {
                let n = self.losses(fm).len();
                if n == 0 {
                    ("✓ lossless".to_string(), theme::OK)
                } else {
                    (format!("⚠ loses {n} {}", if n == 1 { "thing" } else { "things" }), theme::WARN)
                }
            }
            Mode::Export => (format!(".{}", fm.extensions().first().copied().unwrap_or("")), theme::DIM),
        };
        let blen = badge.chars().count();
        let name = if self.mode == Mode::Save && fm == Format::Acid {
            let room = w.saturating_sub(blen + 4);
            ["acidtrip (layers, everything)", "acidtrip (everything)", "acidtrip"]
                .into_iter()
                .find(|n| n.chars().count() <= room)
                .unwrap_or("acidtrip")
        } else {
            fm.name()
        };
        // "▸" + name + gap + badge + 1 trailing space (room for scroll marks).
        let nmax = w.saturating_sub(blen + 4);
        let mut t: String = name.chars().take(nmax).collect();
        if name.chars().count() > nmax && nmax > 1 {
            t = t.chars().take(nmax - 1).collect::<String>() + "…";
        }
        let pad = w.saturating_sub(1 + t.chars().count() + blen + 1);
        let base = if selected {
            let bg = if focused { theme::PANEL_HI } else { Color::Rgb(32, 32, 46) };
            Style::new().bg(bg).fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme::TEXT)
        };
        let marker = if selected { if focused { "▸" } else { "›" } } else { " " };
        Line::from(vec![
            Span::styled(format!("{marker}{t}"), base),
            Span::styled(" ".repeat(pad), base),
            Span::styled(badge, base.fg(badge_color).remove_modifier(Modifier::BOLD)),
            Span::styled(" ", base),
        ])
    }

    fn draw_right(&mut self, f: &mut Frame, area: Rect) {
        let rx = area.x + 1;
        let rw = area.width.saturating_sub(2);
        let fmt = self.format();
        let label = match self.mode {
            Mode::Save => "Save as ",
            Mode::Export => "Export to ",
        };
        self.path_area = Rect::new(rx, area.y, rw, 1);
        if self.focus == Focus::Path {
            self.path.render(f, self.path_area, label, true);
        } else {
            // Unfocused: show the file name end, which matters most.
            let room = (rw as usize).saturating_sub(label.chars().count());
            let n = self.path.text.chars().count();
            let shown = if n > room && room > 1 {
                "…".to_string() + &self.path.text.chars().skip(n + 1 - room).collect::<String>()
            } else {
                self.path.text.clone()
            };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(label, Style::new().fg(theme::DIM)),
                    Span::styled(shown, Style::new().fg(theme::TEXT)),
                ])),
                self.path_area,
            );
        }
        let mut y = area.y + 1;
        let under = if let Some(e) = &self.error {
            Span::styled(e.clone(), Style::new().fg(theme::ERR))
        } else {
            Span::styled(fmt.name().to_string(), Style::new().fg(theme::DIM))
        };
        // Room for the whole error: a long folder name can wrap it to three lines.
        let err_h = self.error.as_deref().map_or(1, |e| wrapped_rows(e, rw).clamp(2, 4));
        f.render_widget(Paragraph::new(Line::from(under)).wrap(Wrap { trim: true }), Rect::new(rx, y, rw, err_h));
        y += err_h + 1;

        let opts = self.options();
        let label_w = (rw as usize / 2).clamp(8, 20);
        self.opts_area = Rect::new(rx, y, rw, opts.len() as u16);
        for (i, (label, val)) in opts.iter().enumerate() {
            if y >= area.bottom() {
                break;
            }
            let focused = self.focus == Focus::Options && i == self.opt_idx;
            let st = if focused { Style::new().fg(theme::BG).bg(theme::ACCENT2) } else { Style::new().fg(theme::TEXT) };
            let label: String = label.chars().take(label_w).collect();
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!("{label:>label_w$} "), Style::new().fg(theme::DIM)),
                    Span::styled(format!("‹ {val} ›"), st),
                ])),
                Rect::new(rx, y, rw, 1),
            );
            y += 1;
        }
        if !opts.is_empty() {
            y += 1;
        }

        let warnings = self.losses(fmt);
        let mut w = vec![];
        if warnings.is_empty() {
            let ok = match self.mode {
                Mode::Save if fmt == Format::Acid => "✓ keeps everything: layers, colors, metadata",
                Mode::Save => "✓ lossless for this document",
                Mode::Export => "✓ nothing lost in this copy",
            };
            w.push(Line::from(Span::styled(ok, Style::new().fg(theme::OK))));
        } else {
            let head = match self.mode {
                Mode::Save => "saving will lose:",
                Mode::Export => "this copy will lose:",
            };
            w.push(Line::from(Span::styled(head, Style::new().fg(theme::WARN).add_modifier(Modifier::BOLD))));
            for x in warnings {
                w.push(Line::from(Span::styled(format!("• {x}"), Style::new().fg(theme::WARN))));
            }
            if self.mode == Mode::Save {
                w.push(Line::from(Span::styled("  .acid keeps everything", Style::new().fg(theme::DIM))));
            }
        }
        w.push(Line::from(""));
        let after = match self.mode {
            Mode::Save => "After saving, Ctrl-S writes to this file.".to_string(),
            Mode::Export => match &self.current_file {
                Some(p) => format!(
                    "Ctrl-S still saves {}.",
                    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                ),
                None => "Your document stays unsaved.".to_string(),
            },
        };
        w.push(Line::from(Span::styled(after, Style::new().fg(theme::DIM))));
        let by = area.bottom().saturating_sub(1);
        f.render_widget(Paragraph::new(w).wrap(Wrap { trim: false }), Rect::new(rx, y, rw, by.saturating_sub(y)));
        self.btns.clear();
        let go = match self.mode {
            Mode::Save => "save",
            Mode::Export => "export",
        };
        let bar = [btn(Do::Go, "⏎", go).primary(), btn(Do::Cancel, "esc", "cancel")];
        self.btns.row(f.buffer_mut(), Rect::new(rx, by, rw, 1), &bar);
    }
}

/// Option rows for `f`: (label, value text). Shared with the EXPORT panel's
/// row editor.
pub fn option_rows(o: &SaveOptions, f: Format, animated: bool) -> Vec<(&'static str, String)> {
    let mut v = vec![];
    if matches!(
        f,
        Format::Ansi
            | Format::Bin
            | Format::XBin
            | Format::Adf
            | Format::Idf
            | Format::Tnd
            | Format::Pcb
            | Format::Avt
            | Format::Ascii
    ) {
        v.push(("SAUCE record", onoff(o.sauce.unwrap_or(true))));
    }
    if animated && matches!(f, Format::Ansi | Format::Asciicast) {
        v.push(("All frames", onoff(o.animate)));
    }
    if f == Format::Ansi {
        v.push(("Line length", o.line_length.map(|n| n.to_string()).unwrap_or_else(|| "unlimited".into())));
        v.push(("Clear screen first", onoff(o.clear_screen)));
        v.push(("iCE hint ESC[?33h", onoff(o.ice_hint)));
        v.push(("EOF char", onoff(o.eof_char)));
    }
    if matches!(f, Format::Png | Format::Gif) {
        v.push(("Scale", format!("{}x", o.scale)));
    }
    if f == Format::Gif {
        v.push(("Animation", format!("{:?}", o.gif_mode)));
        v.push(("Baud", o.baud.to_string()));
    }
    if f == Format::Asciicast {
        v.push(("Baud", o.baud.to_string()));
    }
    if f == Format::Svg {
        v.push(("Pixel-exact glyphs", onoff(o.svg_pixel_exact)));
    }
    if matches!(f, Format::CArray | Format::PascalArray | Format::AsmArray | Format::React) {
        v.push(("Identifier", o.identifier.clone()));
    }
    if !matches!(f, Format::Acid) {
        v.push(("Trim to used rows", onoff(o.trim_height)));
    }
    v
}

/// Step option `label` forward (or back, `dir < 0`).
pub fn cycle_option(o: &mut SaveOptions, label: &str, dir: i32) {
    match label {
        "SAUCE record" => o.sauce = Some(!o.sauce.unwrap_or(true)),
        "Line length" => {
            o.line_length = match (o.line_length, dir > 0) {
                (None, true) => Some(80),
                (Some(80), true) => Some(79),
                (Some(79), true) => Some(70),
                (Some(70), true) => None,
                (None, false) => Some(70),
                (Some(70), false) => Some(79),
                (Some(79), false) => Some(80),
                _ => None,
            }
        }
        "Clear screen first" => o.clear_screen = !o.clear_screen,
        "iCE hint ESC[?33h" => o.ice_hint = !o.ice_hint,
        "EOF char" => o.eof_char = !o.eof_char,
        "Scale" => o.scale = ((o.scale as i32 + dir - 1).rem_euclid(8) + 1) as u32,
        "Animation" => {
            const MODES: [GifMode; 4] = [GifMode::Frames, GifMode::Still, GifMode::Reveal, GifMode::LayersAsFrames];
            let i = MODES.iter().position(|&m| m == o.gif_mode).unwrap_or(1) as i32;
            o.gif_mode = MODES[(i + dir).rem_euclid(MODES.len() as i32) as usize];
        }
        "All frames" => o.animate = !o.animate,
        "Baud" => {
            const RATES: [u32; 7] = [2400, 9600, 14400, 28800, 57600, 115200, 1_000_000];
            let i = RATES.iter().position(|&r| r == o.baud).unwrap_or(2) as i32;
            o.baud = RATES[(i + dir).rem_euclid(RATES.len() as i32) as usize];
        }
        "Pixel-exact glyphs" => o.svg_pixel_exact = !o.svg_pixel_exact,
        "Trim to used rows" => o.trim_height = !o.trim_height,
        _ => {}
    }
}

fn onoff(b: bool) -> String {
    if b { "on".into() } else { "off".into() }
}

/// Why `path` can't be written, when the folder it goes in says so: a file
/// in the way, or a folder we may not write in. Missing folders are fine
/// (saving creates them).
fn write_problem(path: &Path) -> Option<String> {
    let mut dir = path.parent()?;
    while !dir.as_os_str().is_empty() && !dir.exists() {
        dir = dir.parent()?;
    }
    if dir.as_os_str().is_empty() {
        return None;
    }
    if !dir.is_dir() {
        return Some(format!("{} is a file, not a folder.", dir.display()));
    }
    let probe = dir.join(format!(".acidtrip-write-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            None
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
        Err(_) => Some(format!("Can't write in {}. Pick another folder.", dir.display())),
    }
}

/// Rows `text` takes word-wrapped at `width` (how `Wrap { trim: true }` breaks it).
fn wrapped_rows(text: &str, width: u16) -> u16 {
    let width = width.max(1) as usize;
    let mut rows = 1;
    let mut col = 0;
    for word in text.split_whitespace() {
        let n = word.chars().count();
        if col > 0 && col + 1 + n > width {
            rows += 1;
            col = 0;
        }
        if col > 0 {
            col += 1;
        }
        // A word longer than the line breaks mid-word.
        col += n;
        while col > width {
            rows += 1;
            col -= width;
        }
    }
    rows
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn pascal(p: &Path) -> String {
    let s = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = String::new();
    let mut up = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(if up { c.to_uppercase().collect::<Vec<_>>() } else { vec![c] });
            up = false;
        } else {
            up = true;
        }
    }
    if out.is_empty() { "AcidArt".into() } else { out }
}

fn hit(r: Rect, m: &MouseEvent) -> bool {
    m.row >= r.y && m.row < r.bottom() && m.column >= r.x && m.column < r.right()
}

impl Dialog for ExportDialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, _app: &App) {
        let r = centered(area, 96, 28);
        let verb = if self.mode == Mode::Save { "save" } else { "export" };
        let hint = format!("Tab focus · ↑↓ format · ^S/^E mode · Enter {verb} · Esc");
        let inner = popup(f, r, "Save / Export", &hint);
        let [head, body] = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).areas(inner);
        self.draw_mode(f, head);
        let left_w = (inner.width / 2).clamp(24, 44);
        let [left, gap, right] =
            Layout::horizontal([Constraint::Length(left_w), Constraint::Length(1), Constraint::Min(20)]).areas(body);
        f.render_widget(
            Paragraph::new(vec![Line::from(Span::styled("│", Style::new().fg(theme::BORDER))); gap.height as usize]),
            gap,
        );
        self.draw_list(f, left);
        self.draw_right(f, right);
    }

    fn key(&mut self, k: KeyEvent, _app: &App) -> Outcome {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => return Outcome::Close,
            KeyCode::Enter => return self.submit(),
            KeyCode::Char('s') if ctrl => {
                self.set_mode(Mode::Save);
                return Outcome::Keep;
            }
            KeyCode::Char('e') if ctrl => {
                self.set_mode(Mode::Export);
                return Outcome::Keep;
            }
            KeyCode::Tab => {
                self.focus = self.next_focus(false);
                return Outcome::Keep;
            }
            KeyCode::BackTab => {
                self.focus = self.next_focus(true);
                return Outcome::Keep;
            }
            _ => {}
        }
        match self.focus {
            Focus::Mode => match k.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                    self.set_mode(if self.mode == Mode::Save { Mode::Export } else { Mode::Save })
                }
                KeyCode::Down => self.focus = Focus::Formats,
                _ => {}
            },
            Focus::Formats => {
                let n = self.formats().len();
                let i = self.sel();
                let next = match k.code {
                    KeyCode::Up if i == 0 => {
                        self.focus = Focus::Mode;
                        return Outcome::Keep;
                    }
                    KeyCode::Up => i - 1,
                    KeyCode::Down => (i + 1).min(n - 1),
                    KeyCode::PageUp => i.saturating_sub(8),
                    KeyCode::PageDown => (i + 8).min(n - 1),
                    KeyCode::Home => 0,
                    KeyCode::End => n - 1,
                    KeyCode::Left if self.mode == Mode::Export => {
                        self.set_mode(Mode::Save);
                        return Outcome::Keep;
                    }
                    KeyCode::Right if self.mode == Mode::Save => {
                        self.set_mode(Mode::Export);
                        return Outcome::Keep;
                    }
                    _ => return Outcome::Keep,
                };
                if next != i {
                    self.select(next);
                }
            }
            Focus::Path => {
                if self.path.key(&k) {
                    self.follow_extension();
                }
            }
            Focus::Options => {
                let n = self.options().len().max(1);
                match k.code {
                    KeyCode::Up => self.opt_idx = (self.opt_idx + n - 1) % n,
                    KeyCode::Down => self.opt_idx = (self.opt_idx + 1) % n,
                    KeyCode::Left => self.cycle_option(-1),
                    KeyCode::Right | KeyCode::Char(' ') => self.cycle_option(1),
                    _ => {}
                }
            }
        }
        Outcome::Keep
    }

    fn mouse(&mut self, m: MouseEvent, _app: &App) -> Outcome {
        match self.btns.mouse(&m) {
            Some(Do::Go) => return self.submit(),
            Some(Do::Cancel) => return Outcome::Close,
            None => {}
        }
        match m.kind {
            MouseEventKind::Down(button) => {
                if hit(self.save_btn, &m) {
                    self.focus = Focus::Mode;
                    self.set_mode(Mode::Save);
                } else if hit(self.export_btn, &m) {
                    self.focus = Focus::Mode;
                    self.set_mode(Mode::Export);
                } else if hit(self.list_area, &m) {
                    let row = self.offset + (m.row - self.list_area.y) as usize;
                    if let Some(Some(fm)) = self.rows().get(row).copied()
                        && let Some(i) = self.formats().iter().position(|g| *g == fm)
                    {
                        self.focus = Focus::Formats;
                        if i != self.sel() {
                            self.select(i);
                        }
                    }
                } else if hit(self.path_area, &m) {
                    self.focus = Focus::Path;
                } else if hit(self.opts_area, &m) {
                    let i = (m.row - self.opts_area.y) as usize;
                    if i < self.options().len() {
                        self.focus = Focus::Options;
                        self.opt_idx = i;
                        self.cycle_option(if button == MouseButton::Right { -1 } else { 1 });
                    }
                }
            }
            MouseEventKind::ScrollUp if hit(self.list_area, &m) => {
                let i = self.sel();
                if i > 0 {
                    self.select(i - 1);
                }
            }
            MouseEventKind::ScrollDown if hit(self.list_area, &m) => {
                let i = self.sel();
                if i + 1 < self.formats().len() {
                    self.select(i + 1);
                }
            }
            _ => {}
        }
        Outcome::Keep
    }

    fn paste(&mut self, s: &str) {
        if self.focus == Focus::Path {
            self.path.paste(s.trim());
            self.follow_extension();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_problem_names_what_is_in_the_way() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(write_problem(&dir.path().join("a.ans")), None);
        // Missing folders get created on save.
        assert_eq!(write_problem(&dir.path().join("new/deeper/a.ans")), None);
        assert_eq!(write_problem(Path::new("a.ans")), None);
        let file = dir.path().join("plain");
        std::fs::write(&file, b"x").unwrap();
        let p = write_problem(&file.join("a.ans")).unwrap();
        assert!(p.contains("is a file, not a folder"), "{p}");
        // The probe leaves nothing behind.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn wrapped_rows_counts_like_the_paragraph() {
        assert_eq!(wrapped_rows("short", 20), 1);
        assert_eq!(wrapped_rows("Can't write in /tmp/acidtrip-flows/documents-21/ro. Pick another folder.", 47), 3);
        assert_eq!(wrapped_rows("aaaaaaaaaa", 4), 3);
        assert_eq!(wrapped_rows("", 10), 1);
    }

    #[cfg(unix)]
    #[test]
    fn write_problem_sees_a_read_only_folder() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let ro = dir.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        let p = write_problem(&ro.join("a.ans"));
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Root writes anywhere.
        if std::env::var("USER").is_ok_and(|u| u == "root") {
            return;
        }
        assert!(p.is_some_and(|p| p.contains("Can't write in")));
    }
}
