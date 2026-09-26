//! Right-hand sidebar: one panel per topic, Photoshop-style. Tools, the
//! active tool's options (every choice visible as a chip, nothing hidden
//! behind a key), colors (Paint-style FG/BG slots), characters, layers with
//! thumbnails, and the minimap. Every element is clickable and has a hover
//! tip ([`tip`]); hit regions are recorded for the mouse handler.

use acidtrip_core::tools::brush::Param;
use acidtrip_core::tools::pattern::Pattern;
use acidtrip_core::tools::{PaintMode, ShapeFill, StampMode, Symmetry};
use acidtrip_core::{Cell as DocCell, Color as DocColor, DocKind};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::canvas::rgb;
use super::minimap;
use super::widgets::theme;
use crate::actions::Action;
use crate::keymap::Keymap;
use crate::colorfx::{Kind, Target};
use crate::tab::Tab;
use crate::tools_ctl::{self, FillMode, LOOKS, PATTERN_SIZE_MAX, PatternAnchor, PatternMode, Tool, ToolState};
use acidtrip_core::filters::{Knob, PRESETS};
use crate::recent::{self, Recent};
use crate::replay::{self, ReplayHit, ReplayView, SPEEDS};
use acidtrip_core::replay::{LogOp, Speed};
use acidtrip_io::artboard::{self, ArtBoard};
use acidtrip_io::gradient::{self, Ramp, Shape as GradShape, Style as GradStyle};

pub const WIDTH: u16 = 30;
/// Usable columns inside the border and gutter.
const INNER: u16 = WIDTH - 2;
/// Tool buttons are 3 columns wide.
const TOOLS_PER_ROW: usize = 9;
/// Rows of the tool options panel under its header. Fixed, so switching
/// tools never moves the panels below (and what you meant to click).
pub const OPTION_ROWS: u16 = 5;
/// The pen's brush preview, in cells.
pub const PREVIEW_W: u16 = INNER;
pub const PREVIEW_H: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Fg,
    Bg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerOp {
    Add,
    Duplicate,
    Remove,
    Up,
    Down,
    Merge,
    Rename,
    Panel,
}

/// A tool option chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opt {
    Paint(PaintMode),
    Match(FillMode),
    Shape(ShapeFill),
    /// Shape look: sets the brush glyph.
    Look(char),
    Stamp(StampMode),
    PixelFill(bool),
    Lighter(bool),
    Insert(bool),
    Mirror(Symmetry),
    /// Previous / next brush preset.
    Brush(i32),
    /// The pen size slider (click or drag along it).
    Size,
    /// Filters: previous / next preset.
    Preset(i32),
    /// Filters: an adjustment slider (index into `Knob::ALL`).
    Knob(usize),
    /// Filters / Recolor: every layer (true) or the current one.
    AllLayers(bool),
    /// Filters on Classic art: re-fit glyphs (true) or nearest colors.
    Rerender(bool),
    /// Recolor: which colors to replace.
    Target(Target),
    /// Recolor: the tolerance slider.
    Tolerance,
    /// Recolor: page through the used colors.
    UsedPage(i32),
    PatternMode(PatternMode),
    /// Pattern brush size, one step smaller / bigger.
    PatternSize(i32),
    PatternAnchor(PatternAnchor),
    /// Paint the pattern in the brush colors.
    PatternRecolor(bool),
    GradShape(GradShape),
    GradStyle(GradStyle),
    GradRamp(Ramp),
    /// Run the gradient's ramp the other way.
    GradReverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Tool(Tool),
    Opt(Opt),
    /// Any command: panel buttons (brush studio, more colors, block ops...).
    Act(Action),
    Color(u8),
    Slot(Slot),
    Swap,
    Glyph(usize),
    CharsetPrev,
    CharsetNext,
    Layer(usize),
    LayerEye(usize),
    LayerOp(LayerOp),
    /// A frame in the FRAMES filmstrip.
    Frame(usize),
    Minimap,
    /// A key on the art tool's keyboard.
    ArtKey(char),
    /// Rotate the art keys' glyph set.
    ArtSet(i32),
    /// A piece in the gallery strip (index into the recent pieces).
    Recent(usize),
    /// Step the gallery strip's selection.
    RecentStep(i32),
    /// Take a part of the selected recent piece.
    TakePart,
    /// The sourcing studio, on the selected recent piece.
    RecentStudio,
    /// Recolor: a color the piece uses (sets the color to replace).
    UsedColor(DocColor),
    /// Recolor: the color being replaced.
    FxFrom,
    /// A control in the replay panel.
    Replay(ReplayHit),
    /// Previous / next pattern.
    PatternStep(i32),
    /// Something in the EXPORT panel.
    Export(ExportHit),
}

/// The EXPORT panel's controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportHit {
    /// Export the whole piece (true) or the selection.
    Whole(bool),
    /// A row's scale chip.
    Scale(usize),
    /// A row's file name.
    Name(usize),
    /// A row's format chip.
    Format(usize),
    Remove(usize),
    Add,
    Folder,
    /// The big Export button.
    Run,
}

impl Hit {
    /// Hits that follow the mouse while the button is held.
    pub fn draggable(self) -> bool {
        matches!(
            self,
            Hit::Minimap | Hit::Opt(Opt::Size | Opt::Knob(_) | Opt::Tolerance) | Hit::Replay(ReplayHit::Scrub)
        )
    }
}

pub struct SidebarView<'a> {
    pub tab: &'a Tab,
    pub tools: &'a ToolState,
    pub keymap: &'a Keymap,
    pub slot: Slot,
    pub minimap: bool,
    /// Visible canvas region (x, y, w, h) for the minimap highlight.
    pub view: (usize, usize, usize, usize),
    /// The pen's brush drawn on a sample stroke, PREVIEW_W x PREVIEW_H.
    pub brush_preview: &'a [DocCell],
    /// Under the mouse: drawn highlighted.
    pub hover: Option<Hit>,
    /// The art-mode keyboard, when art mode is on.
    pub art: Option<&'a ArtBoard>,
    /// Pieces recently viewed in the Gallery.
    pub recent: &'a Recent,
    /// Draw together, when its panel is open (it takes the options slot).
    pub together: Option<&'a crate::together::Together>,
    /// The replay, while one is on screen.
    pub replay: Option<&'a ReplayView>,
}

/// Where previews landed, so a graphics-capable terminal can paint real
/// pixel images over the half-block fallback.
#[derive(Default)]
pub struct Previews {
    /// Full area reserved for the minimap, and the half-block geometry drawn.
    pub minimap: Option<(Rect, minimap::MiniGeom)>,
    /// (layer index, thumbnail rect).
    pub layers: Vec<(usize, Rect)>,
    /// Gallery strip: (thumbnail rect, recent piece index, dimmed).
    pub recent: Vec<(Rect, usize, bool)>,
}

/// Draws into the sidebar, top to bottom, recording hits.
struct Painter<'f, 'a, 'h> {
    f: &'f mut Frame<'a>,
    hits: &'h mut Vec<(Rect, Hit)>,
    hover: Option<Hit>,
    x0: u16,
    y: u16,
    bottom: u16,
}

fn chip_style(active: bool, hovered: bool) -> Style {
    match (active, hovered) {
        (true, _) => Style::new().fg(theme::BG).bg(theme::ACCENT2).add_modifier(Modifier::BOLD),
        (false, true) => Style::new().fg(Color::White).bg(theme::BORDER),
        (false, false) => Style::new().fg(theme::TEXT).bg(theme::PANEL_HI),
    }
}

impl Painter<'_, '_, '_> {
    fn room(&self, rows: u16) -> bool {
        self.y + rows <= self.bottom
    }

    fn put(&mut self, x: u16, y: u16, w: u16, span: Span) {
        if y < self.bottom && w > 0 {
            self.f.render_widget(Paragraph::new(Line::from(span)), Rect::new(x, y, w, 1));
        }
    }

    fn line(&mut self, l: Line) {
        if self.y < self.bottom {
            self.f.render_widget(Paragraph::new(l), Rect::new(self.x0, self.y, INNER, 1));
        }
        self.y += 1;
    }

    fn hint(&mut self, s: &str) {
        self.line(Line::from(Span::styled(s.to_string(), Style::new().fg(theme::DIM))));
    }

    /// A panel title bar, with optional buttons on the right.
    fn header(&mut self, title: Line, buttons: &[(&str, Hit)]) {
        let bar = Style::new().bg(theme::PANEL_HI);
        self.f.render_widget(Paragraph::new("").style(bar), Rect::new(self.x0 - 1, self.y, INNER + 1, 1));
        self.put(self.x0, self.y, INNER, Span::raw(""));
        self.f.render_widget(Paragraph::new(title).style(bar), Rect::new(self.x0, self.y, INNER, 1));
        let mut x = self.x0 + INNER;
        for (label, hit) in buttons.iter().rev() {
            let w = label.chars().count() as u16;
            x -= w;
            let st = if self.hover == Some(*hit) {
                Style::new().fg(Color::White).bg(theme::BORDER)
            } else {
                Style::new().fg(theme::ACCENT2).bg(theme::PANEL_HI)
            };
            self.button_at(Rect::new(x, self.y, w, 1), label, st, *hit);
        }
        self.y += 1;
    }

    fn button_at(&mut self, r: Rect, label: &str, st: Style, hit: Hit) {
        self.put(r.x, r.y, r.width, Span::styled(label.to_string(), st));
        self.hits.push((r, hit));
    }

    /// A labelled row of chips (a segmented control: chips touch, the active
    /// one lit).
    fn chips(&mut self, label: &str, items: &[(String, Opt, bool)]) {
        let lw = label.chars().count() as u16;
        self.put(self.x0, self.y, lw, Span::styled(label.to_string(), Style::new().fg(theme::DIM)));
        let mut x = self.x0 + lw;
        for (text, opt, active) in items {
            let w = text.chars().count() as u16;
            if x + w > self.x0 + INNER {
                break;
            }
            let hit = Hit::Opt(*opt);
            self.button_at(Rect::new(x, self.y, w, 1), text, chip_style(*active, self.hover == Some(hit)), hit);
            x += w;
        }
        self.y += 1;
    }

    fn gap(&mut self) {
        self.y += 1;
    }
}

/// Draw the sidebar; returns where the previews went.
pub fn draw(f: &mut Frame, area: Rect, v: &SidebarView, hits: &mut Vec<(Rect, Hit)>) -> Previews {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::new().fg(theme::BORDER))
        .style(Style::new().bg(theme::PANEL));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let mut p = Painter { f, hits, hover: v.hover, x0: inner.x + 1, y: inner.y, bottom: inner.bottom() };

    tools_panel(&mut p, v);
    match (v.replay, v.together, v.art) {
        (Some(r), _, _) => replay_panel(&mut p, r),
        (None, Some(t), _) => {
            let area = Rect::new(p.x0, p.y, INNER, OPTION_ROWS + 1);
            super::together::panel(p.f, p.hits, p.hover, area, t);
            p.y += OPTION_ROWS + 1;
        }
        (None, None, Some(board)) => art_panel(&mut p, v, board),
        (None, None, None) if v.tools.tool == Tool::Pattern => pattern_panel(&mut p, v),
        (None, None, None) if v.tools.tool == Tool::Gradient => gradient_panel(&mut p, v),
        (None, None, None) if v.tools.tool == Tool::Filters => filters_panel(&mut p, v),
        (None, None, None) if v.tools.tool == Tool::Recolor => recolor_panel(&mut p, v),
        (None, None, None) => options_panel(&mut p, v),
    }
    // An open EXPORT panel keeps its rows: colors and characters give way.
    let export_h = if v.tab.export_panel { 1 + export_rows(v) } else { 0 };
    if p.room(8 + export_h) {
        p.gap();
        colors_panel(&mut p, v);
    }
    if p.room(5 + export_h) {
        p.gap();
        chars_panel(&mut p, v);
    }
    // Frames sit above the layers: each frame has its own.
    if v.tab.frames_panel && p.room(FRAMES_ROWS + 5 + export_h) {
        p.gap();
        frames_panel(&mut p, v);
    }
    // Export sits low, like Figma's: what leaves the piece, last.
    if export_h > 0 && p.room(export_h) {
        p.gap();
        export_panel(&mut p, v);
    }
    let mut out = Previews::default();
    let gallery_h = gallery_rows(v.recent);
    // Layers at their smallest (gap, header, one layer, buttons), then the
    // gallery, and the minimap keeps at least 4 rows under it.
    let gallery = p.room(4 + gallery_h + 1 + if v.minimap { 5 } else { 0 });
    if p.room(4) {
        p.gap();
        // The map fills what is left, so only its smallest size is held back.
        let reserve = if v.minimap { 6 } else { 0 } + if gallery { gallery_h + 1 } else { 0 };
        layers_panel(&mut p, v, reserve, &mut out);
    }
    if gallery && p.room(gallery_h + 1) {
        p.gap();
        gallery_panel(&mut p, v.recent, &mut out);
    }
    if v.minimap && p.room(4) {
        p.gap();
        p.header(
            Line::from(vec![
                Span::styled(" MAP ", Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD)),
                Span::styled(" click or drag to jump", Style::new().fg(theme::DIM)),
            ]),
            &[],
        );
        // The minimap takes all remaining height.
        let area = Rect::new(p.x0, p.y, INNER - 1, p.bottom.saturating_sub(p.y));
        let pal = &v.tab.doc.meta.palette;
        let g = minimap::draw_canvas(p.f.buffer_mut(), area, &v.tab.doc.canvas, pal, v.view);
        p.hits.push((area, Hit::Minimap));
        out.minimap = Some((area, g));
    }
    out
}

fn title(s: &str) -> Line<'static> {
    Line::from(Span::styled(format!(" {s}"), Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD)))
}

fn tools_panel(p: &mut Painter, v: &SidebarView) {
    p.header(title("TOOLS"), &[(" ► replay ", Hit::Act(Action::Replay))]);
    // Centered grid.
    let left = p.x0 + (INNER - TOOLS_PER_ROW as u16 * 3) / 2;
    for (i, t) in Tool::ALL.iter().enumerate() {
        let r = Rect::new(left + (i % TOOLS_PER_ROW) as u16 * 3, p.y + (i / TOOLS_PER_ROW) as u16, 3, 1);
        if r.bottom() > p.bottom {
            break;
        }
        let hit = Hit::Tool(*t);
        let style = if *t == v.tools.tool {
            Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD)
        } else {
            chip_style(false, p.hover == Some(hit))
        };
        p.button_at(r, &format!(" {} ", t.icon()), style, hit);
    }
    p.y += Tool::ALL.len().div_ceil(TOOLS_PER_ROW) as u16;
}

/// The art tool: the keyboard as a glyph board, laid out like the keys,
/// with the set's name (‹ › rotate it) and the right hand's keys beside it.
/// Each key shows its glyph in the brush colors; click one to change it.
fn art_panel(p: &mut Painter, v: &SidebarView, board: &ArtBoard) {
    let ts = v.tools;
    let classic = v.tab.doc.meta.kind == DocKind::Classic;
    let bar = Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD);
    let lit = Style::new().fg(Color::White).bg(theme::BORDER);
    p.f.render_widget(Paragraph::new("").style(bar), Rect::new(p.x0 - 1, p.y, INNER + 1, 1));
    p.put(p.x0, p.y, 6, Span::styled(format!(" {} ART", Tool::Art.icon()), bar));
    // ‹ set name ›
    let y = p.y;
    let name_w = 15u16;
    for (x, label, step) in [(p.x0 + 6, " ‹", -1), (p.x0 + 8 + name_w, "› ", 1)] {
        let hit = Hit::ArtSet(step);
        p.button_at(Rect::new(x, y, 2, 1), label, if p.hover == Some(hit) { lit } else { bar }, hit);
    }
    let name = board.current(classic).name;
    p.put(p.x0 + 8, y, name_w, Span::styled(format!("{name:^w$}", w = name_w as usize), bar));
    p.hits.push((Rect::new(p.x0 + 8, y, name_w, 1), Hit::ArtSet(1)));
    let off = Hit::Act(Action::ArtMode);
    p.button_at(Rect::new(p.x0 + INNER - 3, y, 3, 1), " ✕ ", if p.hover == Some(off) { lit } else { bar }, off);
    p.y += 1;
    let end = p.y + OPTION_ROWS;
    let pal = &v.tab.doc.meta.palette;
    let (fg, bg) = (rgb(ts.brush.fg, pal), rgb(ts.brush.bg, pal));
    // Rows step right like a keyboard's.
    for (row, keys) in artboard::ROWS.iter().enumerate() {
        for (i, k) in keys.chars().enumerate() {
            let r = Rect::new(p.x0 + row as u16 + i as u16 * 3, p.y, 2, 1);
            let hit = Hit::ArtKey(k);
            let hot = p.hover == Some(hit);
            let key_st = match (hot, board.changed(k, classic)) {
                (true, _) => lit,
                (false, true) => Style::new().fg(theme::ACCENT2).bg(theme::PANEL_HI),
                (false, false) => Style::new().fg(theme::DIM).bg(theme::PANEL_HI),
            };
            p.put(r.x, r.y, 1, Span::styled(k.to_ascii_uppercase().to_string(), key_st));
            let cap = match board.glyph(k, classic) {
                // What the key actually types in this document (Classic conforms to CP437).
                Some(g) => {
                    let g = v.tab.doc.conform(DocCell::new(g, ts.brush.fg, ts.brush.bg)).ch;
                    Span::styled(g.to_string(), Style::new().fg(fg).bg(bg))
                }
                None => Span::styled("✕", Style::new().fg(theme::WARN).bg(theme::PANEL_HI)),
            };
            p.put(r.x + 1, r.y, 1, cap);
            p.hits.push((r, hit));
        }
        // The right hand, beside the board.
        let lx = p.x0 + 19;
        let key = Style::new().fg(theme::TEXT);
        let dim = Style::new().fg(theme::DIM);
        match row {
            0 => {
                for (dx, label, step) in [(0, "[", -1), (2, "]", 1)] {
                    let hit = Hit::ArtSet(step);
                    p.button_at(Rect::new(lx + dx, p.y, 1, 1), label, if p.hover == Some(hit) { lit } else { key }, hit);
                }
                p.put(lx + 4, p.y, 5, Span::styled("sets", dim));
            }
            1 => {
                for (dx, label, a) in [(0, "Y↶", Action::Undo), (4, "H↷", Action::Redo)] {
                    let hit = Hit::Act(a);
                    p.button_at(Rect::new(lx + dx, p.y, 2, 1), label, if p.hover == Some(hit) { lit } else { key }, hit);
                }
            }
            2 => {
                p.put(lx, p.y, 4, Span::styled("IJKL", key));
                p.put(lx + 5, p.y, 4, Span::styled("move", dim));
            }
            _ => {
                p.put(lx, p.y, 3, Span::styled("U O", key));
                p.put(lx + 4, p.y, 2, Span::styled("fg", dim));
            }
        }
        p.y += 1;
    }
    p.hint("M , bg · ⇧IJKL draws");
    p.y = p.y.max(end);
}

/// Replay: play / pause and the scrubber, where it is, the speed, what to
/// show, and exports. Takes the options panel's place while replaying.
fn replay_panel(p: &mut Painter, r: &ReplayView) {
    let bar = Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD);
    let lit = Style::new().fg(Color::White).bg(theme::BORDER);
    let dim = Style::new().fg(theme::DIM);
    let hot = |p: &Painter, h: ReplayHit| p.hover == Some(Hit::Replay(h));
    p.f.render_widget(Paragraph::new("").style(bar), Rect::new(p.x0 - 1, p.y, INNER + 1, 1));
    let (pos, len) = (r.tl.pos(), r.tl.len());
    p.put(p.x0, p.y, INNER - 3, Span::styled(format!(" ► REPLAY  step {pos}/{len}"), bar));
    let off = Hit::Act(Action::Replay);
    p.button_at(Rect::new(p.x0 + INNER - 3, p.y, 3, 1), " ✕ ", if p.hover == Some(off) { lit } else { bar }, off);
    p.y += 1;
    let end = p.y + OPTION_ROWS;

    // ► / ‖ and the scrubber.
    let (label, h) = (if r.playing { " ‖ " } else { " ► " }, ReplayHit::Play);
    let st = if hot(p, h) { lit } else { Style::new().fg(theme::BG).bg(theme::ACCENT2).add_modifier(Modifier::BOLD) };
    p.button_at(Rect::new(p.x0, p.y, 3, 1), label, st, Hit::Replay(h));
    let track = Rect::new(p.x0 + 4, p.y, INNER - 4, 1);
    let w = track.width as usize;
    let head = ((r.progress() * (w - 1) as f64).round() as usize).min(w - 1);
    let done = Style::new().fg(theme::ACCENT2);
    let rest = if hot(p, ReplayHit::Scrub) { Style::new().fg(theme::TEXT) } else { dim };
    let spans = vec![
        Span::styled("━".repeat(head), done),
        Span::styled("●", Style::new().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled("─".repeat(w - 1 - head), rest),
    ];
    p.f.render_widget(Paragraph::new(Line::from(spans)), track);
    p.hits.push((track, Hit::Replay(ReplayHit::Scrub)));
    p.y += 1;

    // Where we are: played / total (at this speed), and the step's name.
    let f = r.factor().max(f64::MIN_POSITIVE);
    let clock = format!("{} / {} ", replay::clock(r.t / f), replay::clock(r.tl.duration() as f64 / f));
    let cw = clock.chars().count() as u16;
    p.put(p.x0, p.y, cw, Span::styled(clock, Style::new().fg(theme::TEXT)));
    let what = match r.tl.op() {
        Some(LogOp::Undo) => format!("undo {}", r.tl.label().unwrap_or("").to_lowercase()),
        Some(LogOp::Redo) => format!("redo {}", r.tl.label().unwrap_or("").to_lowercase()),
        Some(_) => r.tl.label().unwrap_or("").to_lowercase(),
        None => "blank canvas".into(),
    };
    p.put(p.x0 + cw, p.y, INNER - cw, Span::styled(what, dim));
    p.y += 1;

    // Speed.
    p.put(p.x0, p.y, 6, Span::styled("speed ", dim));
    let mut x = p.x0 + 6;
    for (i, (label, _)) in SPEEDS.iter().enumerate() {
        let w = label.chars().count() as u16;
        let h = ReplayHit::Speed(i);
        p.button_at(Rect::new(x, p.y, w, 1), label, chip_style(r.speed == i, hot(p, h)), Hit::Replay(h));
        x += w;
    }
    p.y += 1;

    // What to show.
    let o = r.tl.options();
    let mut x = p.x0;
    for (label, h, on) in
        [(" skip idle ", ReplayHit::SkipIdle, o.skip_idle), (" hide undone ", ReplayHit::HideUndone, o.hide_undone)]
    {
        let w = label.chars().count() as u16;
        p.button_at(Rect::new(x, p.y, w, 1), label, chip_style(on, hot(p, h)), Hit::Replay(h));
        x += w + 1;
    }
    p.y += 1;

    // Export.
    p.put(p.x0, p.y, 7, Span::styled("export ", dim));
    let mut x = p.x0 + 7;
    for (label, gif) in [(" ↓ GIF ", true), (" ↓ .cast ", false)] {
        let w = label.chars().count() as u16;
        let h = ReplayHit::Export(gif);
        p.button_at(Rect::new(x, p.y, w, 1), label, chip_style(false, hot(p, h)), Hit::Replay(h));
        x += w + 1;
    }
    p.y += 1;
    p.y = p.y.max(end);
}

/// The active tool's panel: its name and key, then every option as chips.
fn options_panel(p: &mut Painter, v: &SidebarView) {
    let ts = v.tools;
    let tool = ts.tool;
    let key = v.keymap.key_for(tool.action()).unwrap_or_default();
    let bar = Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD);
    p.f.render_widget(Paragraph::new("").style(bar), Rect::new(p.x0 - 1, p.y, INNER + 1, 1));
    p.put(p.x0, p.y, INNER, Span::styled(format!(" {}  {}", tool.icon(), tool.name().to_uppercase()), bar));
    let k = format!("key {key} ");
    let kw = k.chars().count() as u16;
    p.put(p.x0 + INNER - kw, p.y, kw, Span::styled(k, Style::new().fg(theme::BG).bg(theme::ACCENT)));
    p.y += 1;
    let end = p.y + OPTION_ROWS;
    if tool != Tool::Pen {
        p.hint(tool.blurb());
    }
    let o = &ts.opts;
    let paint = |p: &mut Painter| {
        let m = o.brush_mode;
        p.chips(
            "paint ",
            &[
                (" all ".into(), Opt::Paint(PaintMode::Char), m == PaintMode::Char),
                (" color ".into(), Opt::Paint(PaintMode::Color), m == PaintMode::Color),
                (" fg ".into(), Opt::Paint(PaintMode::Fg), m == PaintMode::Fg),
                (" bg ".into(), Opt::Paint(PaintMode::Bg), m == PaintMode::Bg),
            ],
        );
    };
    let shape = |p: &mut Painter, f: ShapeFill| {
        p.chips(
            "shape ",
            &[
                (" outline ".into(), Opt::Shape(ShapeFill::Outline), f == ShapeFill::Outline),
                (" filled ".into(), Opt::Shape(ShapeFill::Filled), f == ShapeFill::Filled),
            ],
        );
    };
    let look = |p: &mut Painter| {
        let classic = v.tab.doc.meta.kind == DocKind::Classic;
        let items: Vec<(String, Opt, bool)> = LOOKS
            .iter()
            .filter(|l| !(classic && l.modern_only))
            .map(|l| (format!(" {} ", l.glyph), Opt::Look(l.glyph), l.active(ts.brush.ch)))
            .collect();
        p.chips("look  ", &items);
    };
    let stamp = |p: &mut Painter| {
        let s = o.stamp_mode;
        p.chips(
            "stamp ",
            &[
                (" clear ".into(), Opt::Stamp(StampMode::Transparent), s == StampMode::Transparent),
                (" solid ".into(), Opt::Stamp(StampMode::Opaque), s == StampMode::Opaque),
                (" under ".into(), Opt::Stamp(StampMode::Under), s == StampMode::Under),
            ],
        );
    };
    match tool {
        Tool::Select => {
            stamp(p);
            let ops = [
                (" ⇆ ", Action::FlipX),
                (" ⇅ ", Action::FlipY),
                (" ↻ ", Action::Rotate180),
                (" ▦ ", Action::FillSelection),
                (" □ ", Action::OutlineSelection),
                (" ⌗ ", Action::CropToSelection),
                (" ♣ ", Action::SaveStencil),
                (" ⋯ ", Action::BlockMenu),
            ];
            let active = v.tab.selection.is_some();
            p.put(p.x0, p.y, 4, Span::styled("sel ", Style::new().fg(theme::DIM)));
            for (i, (label, a)) in ops.iter().enumerate() {
                let hit = Hit::Act(*a);
                let st = if active {
                    chip_style(false, p.hover == Some(hit))
                } else {
                    Style::new().fg(theme::DIM).bg(theme::PANEL_HI)
                };
                let r = Rect::new(p.x0 + 4 + i as u16 * 3, p.y, 3, 1);
                if r.right() <= p.x0 + INNER {
                    p.button_at(r, label, st, hit);
                }
            }
            p.y += 1;
            if !active {
                p.hint("drag on the canvas to select");
            }
        }
        Tool::Text => {
            let ins = v.tab.insert_mode;
            p.chips(
                "keys  ",
                &[(" overwrite ".into(), Opt::Insert(false), !ins), (" insert ".into(), Opt::Insert(true), ins)],
            );
        }
        Tool::Brush => paint(p),
        Tool::Fill => {
            let m = o.fill_mode;
            p.chips(
                "match ",
                &[
                    (" all ".into(), Opt::Match(FillMode::All), m == FillMode::All),
                    (" char ".into(), Opt::Match(FillMode::Char), m == FillMode::Char),
                    (" color ".into(), Opt::Match(FillMode::Colors), m == FillMode::Colors),
                    (" bg ".into(), Opt::Match(FillMode::Bg), m == FillMode::Bg),
                ],
            );
            paint(p);
        }
        Tool::Pen => pen_rows(p, v),
        Tool::Pixel => {
            let fill = o.pixel_fill;
            p.chips(
                "mode  ",
                &[(" pen ".into(), Opt::PixelFill(false), !fill), (" fill ".into(), Opt::PixelFill(true), fill)],
            );
        }
        Tool::Line => look(p),
        Tool::Rect => {
            shape(p, o.rect_fill);
            look(p);
        }
        Tool::Ellipse => {
            shape(p, o.ellipse_fill);
            look(p);
        }
        Tool::Font | Tool::Stencil => {
            let (label, a) = if tool == Tool::Font {
                (" Å  choose a font…          ", Action::ToolFont)
            } else {
                (" ♣  choose a stencil…       ", Action::ToolStencil)
            };
            let hit = Hit::Act(a);
            let st = chip_style(false, p.hover == Some(hit));
            p.button_at(Rect::new(p.x0, p.y, INNER, 1), label, st, hit);
            p.y += 1;
            stamp(p);
        }
        Tool::Shade => {
            let l = o.shade_lighter;
            p.chips(
                "click ",
                &[(" ▲ denser ".into(), Opt::Lighter(false), !l), (" ▼ lighter ".into(), Opt::Lighter(true), l)],
            );
        }
        Tool::Picker
        | Tool::Colorize
        | Tool::Erase
        | Tool::Art
        | Tool::Pattern
        | Tool::Gradient
        | Tool::Filters
        | Tool::Recolor => {}
    }
    if matches!(
        tool,
        Tool::Brush | Tool::Pen | Tool::Line | Tool::Rect | Tool::Ellipse | Tool::Shade | Tool::Colorize | Tool::Erase
    ) && p.y < end
    {
        let s = o.symmetry;
        p.chips(
            "mirror ",
            &[
                (" off ".into(), Opt::Mirror(Symmetry::None), s == Symmetry::None),
                (" ↔ ".into(), Opt::Mirror(Symmetry::X), s == Symmetry::X),
                (" ↕ ".into(), Opt::Mirror(Symmetry::Y), s == Symmetry::Y),
                (" ✚ ".into(), Opt::Mirror(Symmetry::Both), s == Symmetry::Both),
            ],
        );
    }
    p.y = end;
}

/// The pattern tool: ‹ name › in the title bar, a live preview of the tile
/// beside what it paints (brush strokes, rectangles, fills), its size and
/// alignment, and the buttons that make new patterns.
fn pattern_panel(p: &mut Painter, v: &SidebarView) {
    let ts = v.tools;
    let o = &ts.opts;
    let bar = Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD);
    let lit = Style::new().fg(Color::White).bg(theme::BORDER);
    p.f.render_widget(Paragraph::new("").style(bar), Rect::new(p.x0 - 1, p.y, INNER + 1, 1));
    p.put(p.x0, p.y, 3, Span::styled(format!(" {}", Tool::Pattern.icon()), bar));
    let y = p.y;
    let name_w = 18u16;
    for (x, label, step) in [(p.x0 + 3, " ‹", -1), (p.x0 + 5 + name_w, "› ", 1)] {
        let hit = Hit::PatternStep(step);
        p.button_at(Rect::new(x, y, 2, 1), label, if p.hover == Some(hit) { lit } else { bar }, hit);
    }
    // ★ marks a saved pattern, • one not in the list (a fresh selection).
    let mark = if ts.pattern_saved() {
        "★ "
    } else if ts.pattern_idx.is_none() {
        "• "
    } else {
        ""
    };
    let name: String = format!("{mark}{}", ts.pattern.name).chars().take(name_w as usize).collect();
    let browse = Hit::Act(Action::PatternBrowse);
    let st = if p.hover == Some(browse) { lit } else { bar };
    p.button_at(Rect::new(p.x0 + 5, y, name_w, 1), &format!("{name:^w$}", w = name_w as usize), st, browse);
    p.button_at(Rect::new(p.x0 + INNER - 3, y, 3, 1), " ⋯ ", if p.hover == Some(browse) { lit } else { bar }, browse);
    p.y += 1;
    let end = p.y + OPTION_ROWS;

    // The tile, repeated as it lands on the canvas; click to browse.
    let pr = Rect::new(p.x0, p.y, 8, 3.min(p.bottom.saturating_sub(p.y)));
    draw_pattern(p.f.buffer_mut(), pr, &ts.pattern, v.tab, ts.brush.fg, ts.brush.bg, o.pattern_recolor);
    p.hits.push((pr, browse));
    let x0 = p.x0;
    p.x0 += 9;
    let m = o.pattern_mode;
    p.chips(
        "",
        &[
            (" brush ".into(), Opt::PatternMode(PatternMode::Brush), m == PatternMode::Brush),
            (" rect ".into(), Opt::PatternMode(PatternMode::Rect), m == PatternMode::Rect),
            (" fill ".into(), Opt::PatternMode(PatternMode::Fill), m == PatternMode::Fill),
        ],
    );
    // Size only matters to brush strokes.
    let brush = m == PatternMode::Brush;
    let label = Style::new().fg(theme::DIM);
    p.put(p.x0, p.y, 5, Span::styled("size ", label));
    for (dx, text, step) in [(5, " - ", -1), (10, " + ", 1)] {
        let hit = Hit::Opt(Opt::PatternSize(step));
        let at_end = if step < 0 { o.pattern_size <= 1 } else { o.pattern_size >= PATTERN_SIZE_MAX };
        let st = if !brush || at_end {
            Style::new().fg(theme::DIM).bg(theme::PANEL_HI)
        } else {
            chip_style(false, p.hover == Some(hit))
        };
        p.button_at(Rect::new(p.x0 + dx, p.y, 3, 1), text, st, hit);
    }
    let size_st = Style::new().fg(if brush { theme::TEXT } else { theme::DIM }).add_modifier(Modifier::BOLD);
    p.put(p.x0 + 8, p.y, 2, Span::styled(format!("{:^2}", o.pattern_size), size_st));
    p.y += 1;
    let a = o.pattern_anchor;
    p.chips(
        "align ",
        &[
            (" grid ".into(), Opt::PatternAnchor(PatternAnchor::Canvas), a == PatternAnchor::Canvas),
            (" start ".into(), Opt::PatternAnchor(PatternAnchor::Start), a == PatternAnchor::Start),
        ],
    );
    p.x0 = x0;
    if ts.pattern.has_colors() {
        let r = o.pattern_recolor;
        p.chips(
            "colors ",
            &[
                (" its own ".into(), Opt::PatternRecolor(false), !r),
                (" brush ".into(), Opt::PatternRecolor(true), r),
            ],
        );
    } else {
        p.hint("brush colors · right erases");
    }
    let buttons: [(&str, Action); 2] =
        [(" ▦ use selection ", Action::PatternFromSelection), (" ★ save ", Action::PatternSave)];
    let mut x = p.x0;
    for (text, a) in buttons {
        let hit = Hit::Act(a);
        let w = text.chars().count() as u16;
        p.button_at(Rect::new(x, p.y, w, 1), text, chip_style(false, p.hover == Some(hit)), hit);
        x += w + 1;
    }
    p.y += 1;
    p.y = p.y.max(end);
}

/// A pattern repeated over `r`, as the tool would paint it with tile (0, 0)
/// at the top left: in its own colors or the brush's, holes left as the
/// panel shows through.
pub fn draw_pattern(
    buf: &mut ratatui::buffer::Buffer,
    r: Rect,
    pat: &Pattern,
    tab: &Tab,
    fg: DocColor,
    bg: DocColor,
    recolor: bool,
) {
    let pal = &tab.doc.meta.palette;
    for yy in 0..r.height {
        for xx in 0..r.width {
            let Some(bc) = buf.cell_mut((r.x + xx, r.y + yy)) else { continue };
            match pat.at(xx as usize, yy as usize, (0, 0)) {
                Some(t) => {
                    let (f, b) = if recolor { (fg, bg) } else { (t.fg.unwrap_or(fg), t.bg.unwrap_or(bg)) };
                    let c = tab.doc.conform(DocCell::new(t.ch, f, b));
                    bc.set_char(c.ch).set_style(Style::new().fg(rgb(c.fg, pal)).bg(rgb(c.bg, pal)));
                }
                None => {
                    bc.set_char(' ').set_style(Style::new().bg(theme::PANEL_HI));
                }
            }
        }
    }
}

/// The gradient tool: shape, style, the ramps as swatches, and the ramp as
/// it will come out in this document (click it to run it the other way).
fn gradient_panel(p: &mut Painter, v: &SidebarView) {
    let ts = v.tools;
    let g = ts.opts.gradient;
    let meta = &v.tab.doc.meta;
    let classic = meta.kind == DocKind::Classic;
    let key = v.keymap.key_for(Action::ToolGradient).unwrap_or_default();
    let bar = Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD);
    p.f.render_widget(Paragraph::new("").style(bar), Rect::new(p.x0 - 1, p.y, INNER + 1, 1));
    p.put(p.x0, p.y, INNER, Span::styled(format!(" {}  GRADIENT", Tool::Gradient.icon()), bar));
    let k = format!("key {key} ");
    let kw = k.chars().count() as u16;
    p.put(p.x0 + INNER - kw, p.y, kw, Span::styled(k, Style::new().fg(theme::BG).bg(theme::ACCENT)));
    p.y += 1;
    let end = p.y + OPTION_ROWS;
    p.chips(
        "shape ",
        &[
            (" linear ".into(), Opt::GradShape(GradShape::Linear), g.shape == GradShape::Linear),
            (" radial ".into(), Opt::GradShape(GradShape::Radial), g.shape == GradShape::Radial),
        ],
    );
    let rev = Hit::Opt(Opt::GradReverse);
    p.button_at(Rect::new(p.x0 + INNER - 4, p.y - 1, 4, 1), " ⇄  ", chip_style(g.reverse, p.hover == Some(rev)), rev);
    // Style chips fill the row; smooth is Modern only (Classic dithers).
    let mut x = p.x0;
    for (label, s) in [
        (" ░▒▓ ", GradStyle::Shades),
        (" ▀▄ ", GradStyle::Halves),
        (" smooth ", GradStyle::Smooth),
        (" dither ", GradStyle::Dither),
    ] {
        let hit = Hit::Opt(Opt::GradStyle(s));
        let w = label.chars().count() as u16;
        let st = if classic && s == GradStyle::Smooth && p.hover != Some(hit) {
            Style::new().fg(theme::DIM).bg(theme::PANEL_HI)
        } else {
            chip_style(g.style == s, p.hover == Some(hit))
        };
        p.button_at(Rect::new(x, p.y, w, 1), label, st, hit);
        x += w;
    }
    p.y += 1;
    // Ramp swatches: ▸ marks the one in use.
    for (i, r) in Ramp::ALL.into_iter().enumerate() {
        let hit = Hit::Opt(Opt::GradRamp(r));
        let x = p.x0 + i as u16 * 4 + 2;
        let mark = match (g.ramp == r, p.hover == Some(hit)) {
            (true, _) => "▸",
            (false, true) => "›",
            (false, false) => " ",
        };
        p.put(x, p.y, 1, Span::styled(mark, Style::new().fg(theme::ACCENT2)));
        let stops = gradient::Options { ramp: r, reverse: false, ..g }.stops(ts.brush.fg, ts.brush.bg);
        for j in 0..3 {
            let [cr, cg, cb] = gradient::sample(meta, &stops, j as f32 / 2.0);
            p.put(x + 1 + j, p.y, 1, Span::styled(" ", Style::new().bg(Color::Rgb(cr, cg, cb))));
        }
        p.hits.push((Rect::new(x, p.y, 4, 1), hit));
    }
    p.y += 1;
    // The ramp as this document will get it.
    let stops = g.stops(ts.brush.fg, ts.brush.bg);
    let cells = gradient::strip(meta, INNER as usize, 1, &g, &stops);
    let spans: Vec<Span> = cells
        .iter()
        .map(|c| Span::styled(c.ch.to_string(), Style::new().fg(rgb(c.fg, &meta.palette)).bg(rgb(c.bg, &meta.palette))))
        .collect();
    if p.y < p.bottom {
        p.f.render_widget(Paragraph::new(Line::from(spans)), Rect::new(p.x0, p.y, INNER, 1));
    }
    p.hits.push((Rect::new(p.x0, p.y, INNER, 1), rev));
    p.y += 1;
    let where_ = if v.tab.selection.is_some() { "in the selection" } else { "drag over an area" };
    p.hint(&format!("{} · {where_}", g.ramp.name()));
    p.y = end;
}

/// Pen: brush preset (‹ name › + studio), a live preview stroke, size slider.
fn pen_rows(p: &mut Painter, v: &SidebarView) {
    let ts = v.tools;
    let b = &ts.pen_brush;
    let (y, x0) = (p.y, p.x0);
    let arrow = |p: &mut Painter, x: u16, s: &str, hit: Hit| {
        let st = if p.hover == Some(hit) {
            Style::new().fg(Color::White).bg(theme::BORDER)
        } else {
            Style::new().fg(theme::ACCENT2).bg(theme::PANEL_HI)
        };
        p.button_at(Rect::new(x, y, 3, 1), s, st, hit);
    };
    arrow(p, x0, " ‹ ", Hit::Opt(Opt::Brush(-1)));
    let yours = ts.user_brushes.contains(&b.name);
    let modified = ts.brushes.get(ts.brush_idx) != Some(b);
    let name = format!(" {}{}{}", if yours { "★ " } else { "" }, b.name, if modified { " •" } else { "" });
    let studio = Hit::Act(Action::BrushStudio);
    let nw = INNER - 3 - 3 - 4;
    let st = if p.hover == Some(studio) {
        Style::new().fg(Color::White).bg(theme::BORDER).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme::TEXT).bg(theme::PANEL_HI).add_modifier(Modifier::BOLD)
    };
    p.button_at(Rect::new(x0 + 3, y, nw, 1), &format!("{name:<w$}", w = nw as usize), st, studio);
    arrow(p, x0 + 3 + nw, " › ", Hit::Opt(Opt::Brush(1)));
    let gear = if p.hover == Some(studio) {
        Style::new().fg(Color::White).bg(theme::BORDER)
    } else {
        Style::new().fg(theme::BG).bg(theme::ACCENT2)
    };
    p.button_at(Rect::new(x0 + INNER - 3, y, 3, 1), " ⚙ ", gear, studio);
    p.y += 1;

    // Preview stroke, in the brush colors; clicking it opens the studio.
    let pal = &v.tab.doc.meta.palette;
    let pr = Rect::new(x0, p.y, PREVIEW_W, PREVIEW_H.min(p.bottom.saturating_sub(p.y)));
    if v.brush_preview.len() == (PREVIEW_W * PREVIEW_H) as usize {
        let buf = p.f.buffer_mut();
        for yy in 0..pr.height {
            for xx in 0..pr.width {
                let c = v.brush_preview[(yy * PREVIEW_W + xx) as usize];
                let bg = if c.is_blank() { ts.brush.bg } else { c.bg };
                if let Some(bc) = buf.cell_mut((pr.x + xx, pr.y + yy)) {
                    bc.set_char(if c.ch == '\0' { ' ' } else { c.ch })
                        .set_style(Style::new().fg(rgb(c.fg, pal)).bg(rgb(bg, pal)));
                }
            }
        }
    }
    p.hits.push((pr, studio));
    p.y += pr.height;

    // Size slider.
    p.put(x0, p.y, 5, Span::styled("size ", Style::new().fg(theme::DIM)));
    let sw = INNER - 5 - 7;
    let frac = Param::Size.fraction(b).unwrap_or(0.0).clamp(0.0, 1.0);
    let knob = (frac * (sw - 1) as f32).round() as u16;
    let hit = Hit::Opt(Opt::Size);
    let lit = if p.hover == Some(hit) { Color::White } else { theme::ACCENT2 };
    let bar = Line::from(vec![
        Span::styled("━".repeat(knob as usize), Style::new().fg(lit)),
        Span::styled("●", Style::new().fg(lit).add_modifier(Modifier::BOLD)),
        Span::styled("─".repeat((sw - 1 - knob) as usize), Style::new().fg(theme::BORDER)),
    ]);
    let r = Rect::new(x0 + 5, p.y, sw, 1);
    if p.y < p.bottom {
        p.f.render_widget(Paragraph::new(bar), r);
    }
    p.hits.push((r, hit));
    p.put(x0 + 5 + sw, p.y, 7, Span::styled(format!(" {:>4.1}px", b.size), Style::new().fg(theme::TEXT)));
    p.y += 1;
}

/// A tool panel's title bar: icon, name and key on the accent color.
fn tool_bar(p: &mut Painter, v: &SidebarView, tool: Tool) {
    let key = v.keymap.key_for(tool.action()).unwrap_or_default();
    let bar = Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD);
    p.f.render_widget(Paragraph::new("").style(bar), Rect::new(p.x0 - 1, p.y, INNER + 1, 1));
    p.put(p.x0, p.y, INNER, Span::styled(format!(" {}  {}", tool.icon(), tool.name().to_uppercase()), bar));
    let k = format!("key {key} ");
    let kw = k.chars().count() as u16;
    p.put(p.x0 + INNER - kw, p.y, kw, Span::styled(k, Style::new().fg(theme::BG).bg(theme::ACCENT)));
    p.y += 1;
}

/// A slider row: label, track (bipolar ones fill from the middle), value.
fn slider(p: &mut Painter, label: &str, hit: Hit, frac: f32, bipolar: bool, value: String) {
    let (lw, vw) = (11u16, 5u16);
    let sw = INNER - lw - vw;
    p.put(p.x0, p.y, lw, Span::styled(label.to_string(), Style::new().fg(theme::DIM)));
    let knob = (frac.clamp(0.0, 1.0) * (sw - 1) as f32).round() as u16;
    let from = if bipolar { (sw - 1) / 2 } else { 0 };
    let (lo, hi) = (from.min(knob), from.max(knob));
    let lit = if p.hover == Some(hit) { Color::White } else { theme::ACCENT2 };
    let spans: Vec<Span> = (0..sw)
        .map(|i| {
            if i == knob {
                Span::styled("●", Style::new().fg(lit).add_modifier(Modifier::BOLD))
            } else if i >= lo && i <= hi {
                Span::styled("━", Style::new().fg(lit))
            } else if bipolar && i == from {
                Span::styled("┼", Style::new().fg(theme::BORDER))
            } else {
                Span::styled("─", Style::new().fg(theme::BORDER))
            }
        })
        .collect();
    let r = Rect::new(p.x0 + lw, p.y, sw, 1);
    if p.y < p.bottom {
        p.f.render_widget(Paragraph::new(Line::from(spans)), r);
    }
    p.hits.push((r, hit));
    p.put(p.x0 + lw + sw, p.y, vw, Span::styled(format!("{value:>5}"), Style::new().fg(theme::TEXT)));
    p.y += 1;
}

/// A row of buttons, left to right.
fn buttons(p: &mut Painter, items: &[(&str, Hit)]) {
    let mut x = p.x0;
    for (label, hit) in items {
        let w = label.chars().count() as u16;
        let st = chip_style(false, p.hover == Some(*hit));
        p.button_at(Rect::new(x, p.y, w, 1), label, st, *hit);
        x += w + 1;
    }
    p.y += 1;
}

/// Where a color transform applies: this layer or all of them (inside the
/// selection when there is one).
fn scope_chips(p: &mut Painter, v: &SidebarView, all: bool) {
    let label = if v.tab.selection.is_some() { "in sel " } else { "in     " };
    p.chips(label, &[(" this layer ".into(), Opt::AllLayers(false), !all), (" all ".into(), Opt::AllLayers(true), all)]);
}

/// Filters: ‹ preset › browser, a strip showing the look on sample colors,
/// the adjustment sliders, scope, and apply / reset. The canvas shows the
/// result live; holding the mouse on it shows the original.
fn filters_panel(p: &mut Painter, v: &SidebarView) {
    tool_bar(p, v, Tool::Filters);
    let o = &v.tools.fx.filter;
    let f = &o.filter;
    let y = p.y;
    for (x, label, step) in [(p.x0, " ‹ ", -1), (p.x0 + INNER - 3, " › ", 1)] {
        let hit = Hit::Opt(Opt::Preset(step));
        let st = if p.hover == Some(hit) {
            Style::new().fg(Color::White).bg(theme::BORDER)
        } else {
            Style::new().fg(theme::ACCENT2).bg(theme::PANEL_HI)
        };
        p.button_at(Rect::new(x, y, 3, 1), label, st, hit);
    }
    let nw = (INNER - 6) as usize;
    let name = format!("{} {}/{}", f.current().name, f.preset + 1, PRESETS.len());
    let hit = Hit::Opt(Opt::Preset(1));
    let st = if p.hover == Some(hit) { Style::new().fg(Color::White).bg(theme::BORDER) } else { Style::new().fg(theme::TEXT) };
    p.button_at(Rect::new(p.x0 + 3, y, nw as u16, 1), &format!("{name:^nw$}"), st.add_modifier(Modifier::BOLD), hit);
    p.y += 1;
    // Sample strip: a hue ramp over a gray ramp, through the filter.
    if p.y < p.bottom {
        let n = INNER as usize;
        let buf = p.f.buffer_mut();
        for i in 0..n {
            let t = i as f32 / (n - 1) as f32;
            let g = (t * 255.0) as u8;
            let at = (i as f32 + 0.5, 1.0);
            let top = f.apply(hsv(t * 300.0), at, (n as f32, 2.0));
            let bot = f.apply([g, g, g], at, (n as f32, 2.0));
            if let Some(c) = buf.cell_mut((p.x0 + i as u16, p.y)) {
                c.set_char('▀').set_style(
                    Style::new().fg(Color::Rgb(top[0], top[1], top[2])).bg(Color::Rgb(bot[0], bot[1], bot[2])),
                );
            }
        }
    }
    p.y += 1;
    for (i, k) in Knob::ALL.iter().enumerate() {
        let (lo, hi) = k.range();
        let val = f.get(*k);
        let frac = (val - lo) as f32 / (hi - lo) as f32;
        let shown = if lo < 0 && val > 0 { format!("+{val}") } else { val.to_string() };
        slider(p, k.name(), Hit::Opt(Opt::Knob(i)), frac, lo < 0, shown);
    }
    scope_chips(p, v, o.all_layers);
    if v.tab.doc.meta.kind == DocKind::Classic {
        p.chips(
            "fit    ",
            &[(" nearest ".into(), Opt::Rerender(false), !o.rerender), (" re-render ".into(), Opt::Rerender(true), o.rerender)],
        );
    }
    buttons(p, &[(" ✓ apply ", Hit::Act(Action::ApplyFx)), (" ↺ reset ", Hit::Act(Action::ResetFilter))]);
    p.hint("hold on the canvas: original");
}

/// A color from a hue (degrees), full saturation and value.
fn hsv(h: f32) -> [u8; 3] {
    let x = |n: f32| {
        let k = (n + h / 60.0) % 6.0;
        ((1.0 - k.min(4.0 - k).clamp(0.0, 1.0)) * 255.0) as u8
    };
    [x(5.0), x(3.0), x(1.0)]
}

/// Used colors shown per page (two rows of four).
pub const USED_PER_PAGE: usize = 8;

/// Recolor: from (picked on the canvas or from the used colors) → to (the
/// brush FG), which colors, where, how near counts, apply.
fn recolor_panel(p: &mut Painter, v: &SidebarView) {
    tool_bar(p, v, Tool::Recolor);
    let o = &v.tools.fx.recolor;
    let pal = &v.tab.doc.meta.palette;
    let swatch = |p: &mut Painter, label: &str, col: Option<DocColor>, hit: Hit, note: &str| {
        let y = p.y;
        p.put(p.x0, y, 5, Span::styled(label.to_string(), Style::new().fg(theme::DIM)));
        let (sw, name) = match col {
            Some(c) => (Span::styled(" ██ ", Style::new().fg(rgb(c, pal)).bg(theme::PANEL_HI)), color_label(c)),
            None => (Span::styled(" ?? ", Style::new().fg(theme::WARN).bg(theme::PANEL_HI)), String::new()),
        };
        p.put(p.x0 + 5, y, 4, sw);
        let st = if p.hover == Some(hit) {
            Style::new().fg(Color::White).bg(theme::BORDER)
        } else {
            Style::new().fg(theme::TEXT)
        };
        let text = if name.is_empty() { note.to_string() } else { format!("{name} {note}") };
        p.put(p.x0 + 10, y, INNER - 10, Span::styled(text, st));
        p.hits.push((Rect::new(p.x0 + 5, y, INNER - 5, 1), hit));
        p.y += 1;
    };
    swatch(p, "from", o.from, Hit::FxFrom, if o.from.is_none() { "click the canvas" } else { "" });
    swatch(p, "to", Some(v.tools.brush.fg), Hit::Act(Action::ColorDialog), "(FG)");
    // The colors in use, most used first, with counts.
    let used = v.tools.fx.used_colors(v.tab);
    let pages = used.len().div_ceil(USED_PER_PAGE).max(1);
    let first = (o.page % pages) * USED_PER_PAGE;
    for row in 0..if used.len() > 4 { 2 } else { 1 } {
        let y = p.y;
        for col in 0..4 {
            let Some(&(c, n)) = used.get(first + row * 4 + col) else { break };
            let hit = Hit::UsedColor(c);
            let x = p.x0 + col as u16 * 6;
            let picked = o.from.is_some_and(|f| f.rgb(pal) == c.rgb(pal));
            let count = if n >= 1000 { format!("{}k", n / 1000) } else { n.to_string() };
            p.put(x, y, 2, Span::styled("██", Style::new().fg(rgb(c, pal)).bg(theme::PANEL_HI)));
            p.put(x + 2, y, 3, Span::styled(format!("{count:<3}"), chip_style(picked, p.hover == Some(hit))));
            p.hits.push((Rect::new(x, y, 5, 1), hit));
        }
        if pages > 1 {
            let (label, step) = if row == 0 { (" ‹ ", -1) } else { (" › ", 1) };
            let hit = Hit::Opt(Opt::UsedPage(step));
            p.button_at(Rect::new(p.x0 + INNER - 3, y, 3, 1), label, chip_style(false, p.hover == Some(hit)), hit);
        }
        p.y += 1;
    }
    let t = o.target;
    p.chips(
        "swap   ",
        &[
            (" fg ".into(), Opt::Target(Target::Fg), t == Target::Fg),
            (" bg ".into(), Opt::Target(Target::Bg), t == Target::Bg),
            (" both ".into(), Opt::Target(Target::Both), t == Target::Both),
        ],
    );
    scope_chips(p, v, o.all_layers);
    slider(p, "tolerance", Hit::Opt(Opt::Tolerance), o.tolerance as f32 / 100.0, false, o.tolerance.to_string());
    buttons(p, &[(" ✓ apply ", Hit::Act(Action::ApplyFx))]);
    if o.from.is_some() {
        let n = v.tools.fx.changes(v.tab, Kind::Recolor, v.tools.brush.fg).len();
        p.put(p.x0 + 10, p.y - 1, INNER - 10, Span::styled(format!("{n} cells"), Style::new().fg(theme::DIM)));
    }
}

fn colors_panel(p: &mut Painter, v: &SidebarView) {
    let (tab, ts) = (v.tab, v.tools);
    let pal = &tab.doc.meta.palette;
    p.header(title("COLORS"), &[(" ⋯ more ", Hit::Act(Action::ColorDialog))]);
    let b = ts.brush;
    let y = p.y;
    for (i, (slot, col, label)) in [(Slot::Fg, b.fg, "FG"), (Slot::Bg, b.bg, "BG")].into_iter().enumerate() {
        let r = Rect::new(p.x0 + i as u16 * 10, y, 9, 2);
        let [cr, cg, cb] = col.rgb(pal);
        let lum = (cr as u32 * 3 + cg as u32 * 6 + cb as u32) / 10;
        let txt = if lum > 120 { Color::Black } else { Color::White };
        let active = v.slot == slot;
        let mark = if active { format!("▸{label}◂") } else { format!(" {label} ") };
        let st = Style::new().bg(Color::Rgb(cr, cg, cb)).fg(txt).add_modifier(if active {
            Modifier::BOLD
        } else {
            Modifier::empty()
        });
        p.f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(format!(" {mark:<7} "), st)),
                Line::from(Span::styled(format!(" {:<7} ", short_color(col)), st)),
            ]),
            r,
        );
        p.hits.push((r, Hit::Slot(slot)));
    }
    let swap = Rect::new(p.x0 + 21, y, 6, 2);
    let hs = p.hover == Some(Hit::Swap);
    let st = chip_style(false, hs);
    p.f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("  ⇄   ", st)),
            Line::from(Span::styled(" swap ", st.fg(theme::DIM))),
        ]),
        swap,
    );
    p.hits.push((swap, Hit::Swap));
    p.y += 2;
    let bg_limit = if tab.doc.meta.kind == DocKind::Modern || tab.doc.meta.ice { 16 } else { 8 };
    for row in 0..2u8 {
        for col in 0..8u8 {
            let i = row * 8 + col;
            let r = Rect::new(p.x0 + col as u16 * 3 + 2, p.y, 3, 1);
            let [cr, cg, cb] = pal.get(i);
            let lum = (cr as u32 * 3 + cg as u32 * 6 + cb as u32) / 10;
            let txt = if lum > 120 { Color::Black } else { Color::White };
            let mark = match (b.fg == DocColor::Pal(i), b.bg == DocColor::Pal(i)) {
                (true, true) => "FB",
                (true, false) => "F ",
                (false, true) => "B ",
                _ if i >= bg_limit && v.slot == Slot::Bg => " ·",
                _ => "  ",
            };
            let mut st = Style::new().bg(Color::Rgb(cr, cg, cb)).fg(txt);
            if p.hover == Some(Hit::Color(i)) {
                st = st.add_modifier(Modifier::REVERSED);
            }
            p.put(r.x, r.y, 3, Span::styled(format!("{mark} "), st));
            p.hits.push((r, Hit::Color(i)));
        }
        p.y += 1;
    }
    let tip = match v.slot {
        Slot::Fg => "click a color: foreground",
        Slot::Bg => "click a color: background",
    };
    p.hint(&format!("  {tip}"));
}

fn chars_panel(p: &mut Painter, v: &SidebarView) {
    let ts = v.tools;
    let pal = &v.tab.doc.meta.palette;
    let set = &ts.charsets[ts.charset % ts.charsets.len()];
    p.header(title("CHARACTERS"), &[(" ⋯ all ", Hit::Act(Action::CharPicker))]);
    // ‹ name n/N › picks the set.
    let y = p.y;
    let arrow = |p: &mut Painter, x: u16, s: &str, hit: Hit| {
        let st = if p.hover == Some(hit) {
            Style::new().fg(Color::White).bg(theme::BORDER)
        } else {
            Style::new().fg(theme::ACCENT2).bg(theme::PANEL_HI)
        };
        p.button_at(Rect::new(x, y, 3, 1), s, st, hit);
    };
    arrow(p, p.x0, " ‹ ", Hit::CharsetPrev);
    let nw = (INNER - 6) as usize;
    let name = format!("{} {}/{}", set.name, ts.charset + 1, ts.charsets.len());
    p.put(p.x0 + 3, y, nw as u16, Span::styled(format!("{name:^nw$}"), Style::new().fg(theme::TEXT)));
    arrow(p, p.x0 + INNER - 3, " › ", Hit::CharsetNext);
    p.y += 1;
    let (fg, bg) = (rgb(ts.brush.fg, pal), rgb(ts.brush.bg, pal));
    let left = p.x0 + (INNER - 20) / 2;
    for (i, ch) in set.chars.iter().enumerate() {
        let r = Rect::new(left + i as u16 * 2, p.y, 2, 1);
        let st = if ts.brush.ch == *ch {
            Style::new().fg(theme::BG).bg(theme::ACCENT2)
        } else if p.hover == Some(Hit::Glyph(i)) {
            Style::new().fg(fg).bg(theme::BORDER)
        } else {
            Style::new().fg(fg).bg(bg)
        };
        p.button_at(r, &format!("{ch} "), st, Hit::Glyph(i));
    }
    p.y += 1;
    p.put(left, p.y, 20, Span::styled("1 2 3 4 5 6 7 8 9 0 ", Style::new().fg(theme::DIM)));
    p.y += 1;
}

/// Rows the gallery panel takes (header included).
fn gallery_rows(r: &Recent) -> u16 {
    if r.pieces.is_empty() { 4 } else { 6 }
}

/// Recently viewed pieces as a strip of thumbnails: click one to select it,
/// click it again to view it; take a part of it into your art; open the
/// Gallery or the sourcing studio.
fn gallery_panel(p: &mut Painter, r: &Recent, out: &mut Previews) {
    let n = r.pieces.len();
    let mut t = vec![Span::styled(" GALLERY", Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD))];
    if n > 0 {
        t.push(Span::styled(format!("  {}/{n}", r.sel + 1), Style::new().fg(theme::DIM)));
    }
    let steps: &[(&str, Hit)] =
        if n > 1 { &[(" ‹ ", Hit::RecentStep(-1)), (" › ", Hit::RecentStep(1))] } else { &[] };
    p.header(Line::from(t), steps);
    if n == 0 {
        p.hint("art you view in the Gallery");
        p.hint("shows up here, to reuse");
    } else {
        let y = p.y;
        for k in 0..recent::STRIP {
            let i = r.first + k;
            if i >= n {
                break;
            }
            let rect = Rect::new(p.x0 + k as u16 * 7, y, 6, 3);
            let dim = i != r.sel;
            match r.doc(i) {
                Some(d) => minimap::draw_fit(p.f.buffer_mut(), rect, &d.canvas, &d.meta.palette, dim),
                None => {
                    let st = Style::new().fg(theme::DIM).bg(theme::PANEL_HI);
                    for row in 0..rect.height {
                        p.put(rect.x, rect.y + row, rect.width, Span::styled(if row == 1 { "  ⋯   " } else { "      " }, st));
                    }
                }
            }
            p.hits.push((rect, Hit::Recent(i)));
            if r.doc(i).is_some() {
                out.recent.push((rect, i, dim));
            }
        }
        p.y += 3;
        if let Some(piece) = r.selected() {
            let name: String = piece.title.chars().take(INNER as usize).collect();
            let by = piece.byline();
            let room = (INNER as usize).saturating_sub(name.chars().count() + 3);
            let mut spans = vec![Span::styled(name, Style::new().fg(theme::ACCENT2).add_modifier(Modifier::BOLD))];
            if room > 1 && !by.is_empty() {
                let by = if by.chars().count() > room {
                    format!("{}…", by.chars().take(room - 1).collect::<String>())
                } else {
                    by
                };
                spans.push(Span::styled(format!(" · {by}"), Style::new().fg(theme::DIM)));
            }
            p.line(Line::from(spans));
        }
    }
    let mut x = p.x0;
    let mut buttons = vec![(" gallery ", Hit::Act(Action::Gallery)), (" studio ", Hit::Act(Action::Harvest))];
    if n > 0 {
        buttons.insert(0, (" ✂ take ", Hit::TakePart));
        buttons[2].1 = Hit::RecentStudio;
    }
    for (label, hit) in buttons {
        let w = label.chars().count() as u16;
        let st = chip_style(false, p.hover == Some(hit));
        p.button_at(Rect::new(x, p.y, w, 1), label, st, hit);
        x += w + 1;
    }
    p.y += 1;
}

fn layers_panel(p: &mut Painter, v: &SidebarView, reserve: u16, out: &mut Previews) {
    let tab = v.tab;
    let pal = &tab.doc.meta.palette;
    let frames = (" ▸ frames ", Hit::Act(Action::FramesPanel));
    let panel = (" ≡ panel ", Hit::LayerOp(LayerOp::Panel));
    let both = [frames, panel];
    p.header(title("LAYERS"), if tab.frames_panel { &both[1..] } else { &both });
    let room = p.bottom.saturating_sub(p.y + 1 + reserve).max(1) as usize;
    let n = tab.doc.canvas.layers.len();
    // Keep the active layer in view when there are many.
    let first_top = if n > room { (n - 1).min(tab.layer + room / 2).max(room - 1) } else { n - 1 };
    for (shown, i) in (0..=first_top).rev().enumerate() {
        if shown >= room || p.y >= p.bottom {
            break;
        }
        let l = &tab.doc.canvas.layers[i];
        let eye = Rect::new(p.x0, p.y, 2, 1);
        p.put(
            eye.x,
            eye.y,
            2,
            Span::styled(
                if l.visible { "◉ " } else { "○ " },
                Style::new().fg(if l.visible { theme::OK } else { theme::DIM }),
            ),
        );
        p.hits.push((eye, Hit::LayerEye(i)));
        let thumb = Rect::new(p.x0 + 2, p.y, 4, 1);
        minimap::draw_layer(p.f.buffer_mut(), thumb, &tab.doc.canvas, i, pal, tab.scroll.1);
        out.layers.push((i, thumb));
        let mut name = l.name.clone();
        if l.kind == acidtrip_core::LayerKind::Reference {
            name.push_str(" ·ref");
        }
        if l.locked {
            name.push_str(" ·lock");
        }
        let w = INNER - 7;
        let st = if i == tab.layer {
            Style::new().fg(theme::BG).bg(theme::ACCENT2)
        } else if p.hover == Some(Hit::Layer(i)) {
            Style::new().fg(Color::White).bg(theme::BORDER)
        } else {
            Style::new().fg(theme::TEXT)
        };
        p.put(p.x0 + 7, p.y, w, Span::styled(format!("{name:<w$}", w = w as usize), st));
        p.hits.push((Rect::new(p.x0 + 2, p.y, INNER - 2, 1), Hit::Layer(i)));
        p.y += 1;
    }
    if p.y < p.bottom {
        let ops = [
            (" + ", LayerOp::Add),
            (" ⧉ ", LayerOp::Duplicate),
            (" ✎ ", LayerOp::Rename),
            (" ▲ ", LayerOp::Up),
            (" ▼ ", LayerOp::Down),
            (" ⤓ ", LayerOp::Merge),
            (" ✕ ", LayerOp::Remove),
        ];
        for (k, (label, op)) in ops.iter().enumerate() {
            let hit = Hit::LayerOp(*op);
            let r = Rect::new(p.x0 + k as u16 * 4, p.y, 3, 1);
            let st = chip_style(false, p.hover == Some(hit));
            p.button_at(r, label, st, hit);
        }
        p.y += 1;
    }
}

/// Frames shown at once in the filmstrip.
const FILM: usize = 4;
/// Rows the FRAMES panel takes.
pub const FRAMES_ROWS: u16 = 7;

/// The FRAMES panel: a filmstrip of thumbnails (click one to draw on it),
/// frame ops, onion skin, and timing.
fn frames_panel(p: &mut Painter, v: &SidebarView) {
    let tab = v.tab;
    let doc = &tab.doc;
    let pal = &doc.meta.palette;
    let (n, cur) = (doc.frame_count(), doc.current_frame());
    let play = if tab.playing.is_some() { " ■ stop " } else { " ▶ play " };
    p.header(
        Line::from(vec![
            Span::styled(" FRAMES ", Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}/{n}", cur + 1), Style::new().fg(theme::DIM)),
        ]),
        &[(play, Hit::Act(Action::FramePlay)), (" ✕ ", Hit::Act(Action::FramesPanel))],
    );
    // The filmstrip: ‹ four thumbnails ›, the current frame lit below its thumb.
    let first = cur.saturating_sub(1).min(n.saturating_sub(FILM));
    let y = p.y;
    for (x, label, a) in [(p.x0, "‹", Action::FramePrev), (p.x0 + INNER - 2, "›", Action::FrameNext)] {
        let hit = Hit::Act(a);
        let st = if p.hover == Some(hit) { chip_style(false, true) } else { Style::new().fg(theme::ACCENT2) };
        p.button_at(Rect::new(x, y, 2, 1), &format!("{label} "), st, hit);
        p.hits.push((Rect::new(x, y + 1, 2, 2), hit));
    }
    for (k, i) in (first..n).take(FILM).enumerate() {
        let x = p.x0 + 2 + k as u16 * 6;
        let thumb = Rect::new(x, y, 5, 2);
        minimap::draw_fit(p.f.buffer_mut(), thumb, doc.frame_canvas(i), pal, i != cur);
        let hold = doc.hold(i);
        let label = if hold > 1 { format!("{}×{hold}", i + 1) } else { format!("{}", i + 1) };
        let hit = Hit::Frame(i);
        let st = if i == cur {
            Style::new().fg(theme::BG).bg(theme::ACCENT2).add_modifier(Modifier::BOLD)
        } else if p.hover == Some(hit) {
            Style::new().fg(Color::White).bg(theme::BORDER)
        } else {
            Style::new().fg(theme::DIM)
        };
        p.put(x, y + 2, 5, Span::styled(format!("{label:^5}"), st));
        p.hits.push((Rect::new(x, y, 5, 3), hit));
    }
    p.y += 3;
    // Frame ops.
    let ops = [
        (" + ", Action::FrameAdd),
        (" ⧉ ", Action::FrameDuplicate),
        (" ◀ ", Action::FrameMoveLeft),
        (" ▶ ", Action::FrameMoveRight),
        (" ✕ ", Action::FrameRemove),
    ];
    for (k, (label, a)) in ops.iter().enumerate() {
        let hit = Hit::Act(*a);
        p.button_at(Rect::new(p.x0 + k as u16 * 4, p.y, 3, 1), label, chip_style(false, p.hover == Some(hit)), hit);
    }
    p.y += 1;
    // Onion skin.
    p.put(p.x0, p.y, 6, Span::styled("onion ", Style::new().fg(theme::DIM)));
    let onion = [(" ◂ previous ", Action::OnionPrev, tab.onion.0, 6), (" next ▸ ", Action::OnionNext, tab.onion.1, 19)];
    for (label, a, on, dx) in onion {
        let hit = Hit::Act(a);
        let w = label.chars().count() as u16;
        p.button_at(Rect::new(p.x0 + dx, p.y, w, 1), label, chip_style(on, p.hover == Some(hit)), hit);
    }
    p.y += 1;
    // Timing: frames per second, and how many ticks this frame holds.
    let dim = Style::new().fg(theme::DIM);
    let val = Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD);
    p.put(p.x0, p.y, 4, Span::styled("fps", dim));
    p.put(p.x0 + 7, p.y, 3, Span::styled(format!("{:>2}", doc.fps()), val));
    p.put(p.x0 + 14, p.y, 5, Span::styled("hold", dim));
    p.put(p.x0 + 22, p.y, 3, Span::styled(format!("×{}", doc.hold(cur)), val));
    let steps = [(4, " − ", Action::FpsDown), (10, " + ", Action::FpsUp), (19, " − ", Action::HoldLess), (25, " + ", Action::HoldMore)];
    for (dx, label, a) in steps {
        let hit = Hit::Act(a);
        p.button_at(Rect::new(p.x0 + dx, p.y, 3, 1), label, chip_style(false, p.hover == Some(hit)), hit);
    }
    p.y += 1;
}

/// Rows the EXPORT panel takes (header included).
fn export_rows(v: &SidebarView) -> u16 {
    1 + 3 + v.tab.doc.meta.exports.presets.len().max(1) as u16 + 3
}

/// The EXPORT panel, Figma-style: what gets exported (a thumbnail, the
/// whole piece or the selection), one row per file (scale, name, format,
/// remove), + to add one, the folder, and the big Export button.
fn export_panel(p: &mut Painter, v: &SidebarView) {
    use crate::exporter;
    use acidtrip_io::exports;
    let tab = v.tab;
    let rows = exporter::rows(tab);
    let plan = exporter::plan(tab);
    let dim = Style::new().fg(theme::DIM);
    p.header(
        Line::from(vec![
            Span::styled(" EXPORT ", Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{}", rows.len()), dim),
        ]),
        &[(" as… ", Hit::Act(Action::ExportAs)), (" ✕ ", Hit::Act(Action::Export))],
    );
    // What: thumbnail, name and size, whole / selection.
    let scope = exporter::scope(tab);
    let (w, h) = exporter::size(tab);
    let y = p.y;
    let thumb = Rect::new(p.x0, y, 8, 3);
    let pal = &tab.doc.meta.palette;
    match scope {
        Some(r) => minimap::draw_fit(p.f.buffer_mut(), thumb, &exports::crop(&tab.doc.canvas, r), pal, false),
        None => minimap::draw_fit(p.f.buffer_mut(), thumb, &tab.doc.canvas, pal, false),
    }
    let tx = p.x0 + 9;
    let what = if scope.is_some() { "Selection" } else { "Whole piece" };
    p.put(tx, y, INNER - 9, Span::styled(what, Style::new().fg(theme::TEXT).add_modifier(Modifier::BOLD)));
    let frames = if tab.doc.is_animated() { format!(" · {} frames", tab.doc.frame_count()) } else { String::new() };
    p.put(tx, y + 1, INNER - 9, Span::styled(format!("{w}×{h}{frames}"), dim));
    let mut cx = tx;
    for (label, whole) in [(" whole ", true), (" selection ", false)] {
        let hit = Hit::Export(ExportHit::Whole(whole));
        let on = scope.is_none() == whole;
        let st = if !whole && tab.selection.is_none() {
            if p.hover == Some(hit) { chip_style(false, true) } else { Style::new().fg(theme::DIM).bg(theme::PANEL_HI) }
        } else {
            chip_style(on, p.hover == Some(hit))
        };
        let lw = label.chars().count() as u16;
        p.button_at(Rect::new(cx, y + 2, lw, 1), label, st, hit);
        cx += lw;
    }
    p.y += 3;
    // One line per row: [2x] name  [PNG▾] −
    let clash = plan.as_ref().err();
    for (i, row) in rows.iter().enumerate() {
        if p.y >= p.bottom {
            break;
        }
        let y = p.y;
        let fmt = exports::format_of(row);
        let scaled = fmt.is_some_and(exports::has_scale);
        let hit = Hit::Export(ExportHit::Scale(i));
        let (label, st) = if scaled {
            (format!("{:^4}", format!("{}x", row.scale)), chip_style(false, p.hover == Some(hit)))
        } else {
            (" -- ".to_string(), Style::new().fg(theme::DIM).bg(theme::PANEL))
        };
        p.button_at(Rect::new(p.x0, y, 4, 1), &label, st, hit);
        // The name as it will be written (the format chip says the extension).
        let hit = Hit::Export(ExportHit::Name(i));
        let files = exporter::row_files(tab, i);
        let stem = |f: &str| f.rsplit_once('.').map_or(f, |(s, _)| s).to_string();
        let shown = match (files.first(), files.len()) {
            (Some(f), 1) => stem(f),
            (Some(f), n) => format!("{} ×{n}", stem(f)),
            (None, _) => row.name.clone(),
        };
        let nw = 13usize;
        let text = if shown.chars().count() > nw {
            shown.chars().take(nw - 1).collect::<String>() + "…"
        } else {
            format!("{shown:<nw$}")
        };
        let bad = clash.is_some_and(|e| files.iter().any(|f| e.contains(f.as_str())));
        let st = match (p.hover == Some(hit), bad) {
            (true, _) => Style::new().fg(Color::White).bg(theme::BORDER),
            (false, true) => Style::new().fg(theme::WARN).bg(theme::BG),
            (false, false) => Style::new().fg(theme::TEXT).bg(theme::BG),
        };
        p.button_at(Rect::new(p.x0 + 5, y, nw as u16, 1), &text, st, hit);
        let hit = Hit::Export(ExportHit::Format(i));
        let flabel = fmt.map_or("?".to_string(), exports::short_label);
        let st = if p.hover == Some(hit) {
            Style::new().fg(Color::White).bg(theme::BORDER).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme::ACCENT2).bg(theme::PANEL_HI).add_modifier(Modifier::BOLD)
        };
        p.button_at(Rect::new(p.x0 + 19, y, 6, 1), &format!("{flabel:^5}▾"), st, hit);
        if rows.len() > 1 {
            let hit = Hit::Export(ExportHit::Remove(i));
            let st = if p.hover == Some(hit) { chip_style(false, true) } else { dim };
            p.button_at(Rect::new(p.x0 + 25, y, 3, 1), " − ", st, hit);
        }
        p.y += 1;
    }
    // + add, and where the files go.
    let hit = Hit::Export(ExportHit::Add);
    let st = if p.hover == Some(hit) { chip_style(false, true) } else { Style::new().fg(theme::ACCENT2) };
    p.button_at(Rect::new(p.x0, p.y, 14, 1), " + add export ", st, hit);
    p.y += 1;
    let hit = Hit::Export(ExportHit::Folder);
    let dir = exporter::pretty(&exporter::folder(tab));
    let room = INNER as usize - 3;
    let n = dir.chars().count();
    let dir = if n > room { format!("…{}", dir.chars().skip(n + 1 - room).collect::<String>()) } else { dir };
    let st = if p.hover == Some(hit) {
        Style::new().fg(Color::White).bg(theme::BORDER)
    } else {
        Style::new().fg(theme::TEXT).add_modifier(Modifier::UNDERLINED)
    };
    p.put(p.x0, p.y, 3, Span::styled("to ", dim));
    p.button_at(Rect::new(p.x0 + 3, p.y, dir.chars().count() as u16, 1), &dir, st, hit);
    p.hits.push((Rect::new(p.x0, p.y, 3, 1), hit));
    p.y += 1;
    // The button.
    let hit = Hit::Export(ExportHit::Run);
    let (label, st) = match &plan {
        Ok(jobs) => {
            let n = jobs.len();
            let label = format!("↓ Export {n} file{}", if n == 1 { "" } else { "s" });
            let bg = if p.hover == Some(hit) { theme::ACCENT2 } else { theme::ACCENT };
            (label, Style::new().fg(theme::BG).bg(bg).add_modifier(Modifier::BOLD))
        }
        Err(_) => ("two rows, one file name".to_string(), Style::new().fg(theme::WARN).bg(theme::PANEL_HI)),
    };
    p.button_at(Rect::new(p.x0, p.y, INNER, 1), &format!("{label:^w$}", w = INNER as usize), st, hit);
    p.y += 1;
}

fn short_color(c: DocColor) -> String {
    match c {
        DocColor::Pal(i) if (i as usize) < 16 => acidtrip_core::color::VGA_NAMES[i as usize].to_string(),
        DocColor::Pal(i) => format!("#{i}"),
        DocColor::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
    .chars()
    .take(7)
    .collect()
}

pub fn color_label(c: DocColor) -> String {
    match c {
        DocColor::Pal(i) if (i as usize) < 16 => format!("{i:>2} {}", acidtrip_core::color::VGA_NAMES[i as usize]),
        DocColor::Pal(i) => format!("#{i}"),
        DocColor::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

/// What a sidebar element does, for the status bar while hovering it.
pub fn tip(hit: Hit, ts: &ToolState, keymap: &Keymap, slot: Slot) -> String {
    let key = |a: Action| keymap.key_for(a).map(|k| format!("  [{k}]")).unwrap_or_default();
    let opt_key = key(Action::ToolOption);
    match hit {
        Hit::Tool(t) => format!("{} — {}{}", t.name(), t.blurb(), key(t.action())),
        Hit::ArtKey(k) if k == artboard::ERASE_KEY => {
            format!("{} erases the cell at the cursor (typing a glyph onto itself erases too)", k.to_ascii_uppercase())
        }
        Hit::ArtKey(k) => format!("{} types this glyph — click to change it", k.to_ascii_uppercase()),
        Hit::ArtSet(step) => {
            format!("{} glyph set for the letter keys  [{}]", if step < 0 { "previous" } else { "next" }, if step < 0 { '[' } else { ']' })
        }
        Hit::Opt(o) => match o {
            Opt::Paint(m) => format!(
                "paint {}{opt_key}",
                match m {
                    PaintMode::Char => "the glyph and both colors",
                    PaintMode::Color => "colors only, keep the glyphs",
                    PaintMode::Fg => "the foreground only",
                    PaintMode::Bg => "the background only",
                    _ => "",
                }
            ),
            Opt::Match(m) => format!(
                "fill spreads over cells with {}{opt_key}",
                match m {
                    FillMode::All => "the same glyph and colors",
                    FillMode::Char => "the same glyph",
                    FillMode::Colors => "the same colors",
                    FillMode::Bg => "the same background",
                }
            ),
            Opt::Shape(f) => format!("{} shapes{opt_key}", tools_ctl::fill_name(f)),
            Opt::Look(ch) => {
                format!(
                    "draw shapes as {} (sets the brush to '{ch}'){}",
                    tools_ctl::look_of(ch).name,
                    key(Action::ToolStyle)
                )
            }
            Opt::Stamp(s) => format!(
                "stamp {}{opt_key}",
                match s {
                    StampMode::Transparent => "with blank cells see-through",
                    StampMode::Opaque => "as a solid block",
                    StampMode::Under => "only into empty cells (behind the art)",
                }
            ),
            Opt::PixelFill(f) => {
                format!("{}{opt_key}", if f { "flood-fill half-block pixels" } else { "draw half-block pixels" })
            }
            Opt::Lighter(l) => {
                format!(
                    "a click makes cells {} · Alt-drag does the opposite{opt_key}",
                    if l { "lighter" } else { "denser" }
                )
            }
            Opt::Insert(i) => format!(
                "{} · Insert key toggles",
                if i { "typing pushes text right" } else { "typing replaces what's there" }
            ),
            Opt::Mirror(s) => format!("mirror strokes: {}{}", tools_ctl::symmetry_name(s), key(Action::Mirror)),
            Opt::Brush(d) => format!(
                "{} brush{}",
                if d < 0 { "previous" } else { "next" },
                key(if d < 0 { Action::ToolStyle } else { Action::ToolOption })
            ),
            Opt::PatternMode(m) => format!(
                "{}{opt_key}",
                match m {
                    PatternMode::Brush => "paint brush strokes with the pattern · right-drag erases",
                    PatternMode::Rect => "drag a rectangle to fill with the pattern",
                    PatternMode::Fill => "click an area to fill it with the pattern (like the bucket)",
                }
            ),
            Opt::PatternSize(d) => format!(
                "{} brush{}",
                if d < 0 { "smaller" } else { "bigger" },
                key(if d < 0 { Action::BrushSmaller } else { Action::BrushBigger })
            ),
            Opt::PatternAnchor(a) => match a {
                PatternAnchor::Canvas => "tiles line up on the canvas grid, so separate strokes join seamlessly".into(),
                PatternAnchor::Start => "each stroke, rectangle or fill starts a fresh tile where you press".into(),
            },
            Opt::PatternRecolor(r) => {
                if r {
                    "paint the pattern's glyphs in the brush colors".into()
                } else {
                    "paint the pattern in the colors it was made with".into()
                }
            }
            Opt::Size => format!(
                "brush size: click or drag{} {}",
                key(Action::BrushSmaller),
                keymap.key_for(Action::BrushBigger).map(|k| format!("[{k}]")).unwrap_or_default()
            ),
            Opt::Preset(d) => format!(
                "{} filter preset{}",
                if d < 0 { "previous" } else { "next" },
                key(if d < 0 { Action::ToolStyle } else { Action::ToolOption })
            ),
            Opt::Knob(i) => {
                let k = Knob::ALL[i % Knob::ALL.len()];
                format!("{}: {} · click or drag", k.name(), k.blurb())
            }
            Opt::AllLayers(all) => format!(
                "change {} (a selection limits it to the selection)",
                if all { "every unlocked, visible layer" } else { "the current layer only" }
            ),
            Opt::Rerender(true) => "re-fit glyphs to the filtered look (blocks mix the 16 colors)".into(),
            Opt::Rerender(false) => "keep the glyphs, map each color to the nearest of the 16".into(),
            Opt::Target(t) => format!(
                "replace the color in {}{opt_key}",
                match t {
                    Target::Fg => "foregrounds (ink) only",
                    Target::Bg => "backgrounds only",
                    Target::Both => "foregrounds and backgrounds",
                }
            ),
            Opt::Tolerance => "tolerance: near colors count too and keep their shading · 0 = exact".into(),
            Opt::UsedPage(_) => "more colors the piece uses".into(),
            Opt::GradShape(GradShape::Linear) => "linear: the ramp runs along your drag".into(),
            Opt::GradShape(GradShape::Radial) => "radial: rings out from where the drag starts".into(),
            Opt::GradStyle(s) => format!(
                "{}{opt_key}",
                match s {
                    GradStyle::Shades => "shades: solid colors with ░▒▓ mixes between (CP437-safe)",
                    GradStyle::Halves => "half blocks: ▀▄, two colors a cell, twice the steps up and down",
                    GradStyle::Smooth => "smooth: truecolor per cell (Modern documents only)",
                    GradStyle::Dither => "dither: the shade steps with an ordered pattern between them",
                }
            ),
            Opt::GradRamp(Ramp::Brush) => "ramp: your foreground to your background".into(),
            Opt::GradRamp(r) => format!("ramp: {}", r.name()),
            Opt::GradReverse => "run the ramp the other way (right-drag does too)".into(),
        },
        Hit::Act(Action::ApplyFx) if ts.tool == Tool::Filters => "apply the filter as one undo step  [Enter]".into(),
        Hit::Act(Action::ApplyFx) => "replace the color as one undo step  [Enter]".into(),
        Hit::Act(Action::ColorDialog) if ts.tool == Tool::Recolor => {
            format!("the new color is the brush FG · click to choose it{}", key(Action::ColorDialog))
        }
        Hit::UsedColor(c) => format!("replace {} (the piece uses it)", color_label(c)),
        Hit::FxFrom => "the color to replace: click it on the canvas, or a color chip below".into(),
        Hit::Act(Action::BrushStudio) => {
            format!("brush studio: presets, size, softness, texture, taper…{}", key(Action::BrushStudio))
        }
        Hit::Act(Action::PatternBrowse) => {
            format!("browse the patterns: built-in and saved{}", key(Action::PatternBrowse))
        }
        Hit::Act(Action::PatternFromSelection) => {
            format!("paint with the selection as a pattern (select an area first){}", key(Action::PatternFromSelection))
        }
        Hit::Act(Action::PatternSave) => {
            format!("save this pattern to your library{}", key(Action::PatternSave))
        }
        Hit::Act(Action::FramesPanel) => {
            format!("animation frames: filmstrip, play, onion skin, timing{}", key(Action::FramesPanel))
        }
        Hit::Act(Action::OnionPrev) => {
            format!("onion skin: the previous frame shows dimmed through empty cells{}", key(Action::OnionPrev))
        }
        Hit::Act(Action::OnionNext) => {
            format!("onion skin: the next frame shows dimmed through empty cells{}", key(Action::OnionNext))
        }
        Hit::Act(a @ (Action::HoldMore | Action::HoldLess)) => {
            format!("{} (a tick is 1/fps of a second){}", a.title(), key(a))
        }
        Hit::Act(Action::CommandPalette) => format!("commands: every command, search by name{}", key(Action::CommandPalette)),
        Hit::Act(a) => format!("{}{}", a.title(), key(a)),
        Hit::Replay(h) => match h {
            ReplayHit::Play => "play / pause the replay  [Space] · clicking the canvas does too".into(),
            ReplayHit::Scrub => "click or drag to jump through the replay  [← → step · Home End]".into(),
            ReplayHit::Speed(i) => match SPEEDS[i.min(SPEEDS.len() - 1)].1 {
                Speed::Fit(s) => format!("play the whole piece in {s} seconds (short ones as drawn)"),
                Speed::Times(x) => format!("play {x}× as fast as it was drawn"),
            },
            ReplayHit::SkipIdle => "cut long pauses down to a second".into(),
            ReplayHit::HideUndone => "show only the work that survived (skip what was undone)".into(),
            ReplayHit::Export(true) => "save the replay as an animated GIF beside the piece".into(),
            ReplayHit::Export(false) => "save the replay as an asciinema .cast beside the piece".into(),
        },
        Hit::Color(_) => match slot {
            Slot::Fg => "set the foreground · Alt-click sets the background".into(),
            Slot::Bg => "set the background · Alt-click sets the foreground".into(),
        },
        Hit::Slot(s) if s == slot => "active slot: click again for all colors".into(),
        Hit::Slot(Slot::Fg) => "make palette clicks set the foreground".into(),
        Hit::Slot(Slot::Bg) => "make palette clicks set the background".into(),
        Hit::Swap => format!("swap foreground and background{}", key(Action::SwapColors)),
        Hit::Glyph(i) => format!("brush '{}' — drag to paint, or press {} to place", ts.glyph(i), (i + 1) % 10),
        Hit::CharsetPrev => format!("previous character set{}", key(Action::CharsetPrev)),
        Hit::CharsetNext => format!("next character set{}", key(Action::CharsetNext)),
        Hit::Layer(_) => "make this the layer you draw on".into(),
        Hit::Frame(i) => format!(
            "show frame {} to draw on it{} {}",
            i + 1,
            key(Action::FramePrev),
            keymap.key_for(Action::FrameNext).map(|k| format!("[{k}]")).unwrap_or_default()
        ),
        Hit::LayerEye(_) => "show / hide the layer".into(),
        Hit::LayerOp(op) => {
            let a = match op {
                LayerOp::Add => Action::LayerAdd,
                LayerOp::Duplicate => Action::LayerDuplicate,
                LayerOp::Remove => Action::LayerRemove,
                LayerOp::Up => Action::LayerMoveUp,
                LayerOp::Down => Action::LayerMoveDown,
                LayerOp::Merge => Action::LayerMerge,
                LayerOp::Rename => Action::LayerRename,
                LayerOp::Panel => Action::LayersPanel,
            };
            format!("{}{}", a.title(), key(a))
        }
        Hit::Minimap => "click or drag to jump there".into(),
        Hit::Recent(_) => "recently viewed: click to select, click again to view it".into(),
        Hit::RecentStep(_) => "the next / previous recently viewed piece".into(),
        Hit::TakePart => "take a part: drag a box over the piece and it becomes a paste".into(),
        Hit::RecentStudio => "the sourcing studio, on this piece: cut its letters into a font".into(),
        // The app fills these in (they name files); see `exporter::tip`.
        Hit::Export(_) => String::new(),
        Hit::PatternStep(d) => format!(
            "{} pattern{}",
            if d < 0 { "previous" } else { "next" },
            key(if d < 0 { Action::ToolStyle } else { Action::ToolOption })
        ),
    }
}
