//! Frame layout: tab bar, canvas, sidebar, status bar, dialogs.

pub mod canvas;
pub mod minimap;
pub mod sidebar;
pub mod thumbs;
pub mod together;
pub mod widgets;

use ratatui::Frame;
use ratatui::buffer::Cell;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::actions::Action;
use crate::app::{App, Level};
use crate::tools_ctl::Tool;
use crate::ui::sidebar::Hit;
use widgets::theme;

pub fn draw(f: &mut Frame, app: &mut App) {
    // Replaying: the canvas (and everything reading the doc) shows the
    // replay's document for this one frame; the live one is swapped back after.
    let live_view = app.replay.as_mut().map(|r| {
        let t = &mut app.tabs[app.active];
        std::mem::swap(&mut t.doc, r.tl.doc_mut());
        let kept = (t.cursor, t.scroll, t.layer, t.selection.take());
        t.clamp();
        kept
    });
    draw_frame(f, app);
    if let (Some(r), Some((cursor, scroll, layer, selection))) = (app.replay.as_mut(), live_view) {
        let t = &mut app.tabs[app.active];
        std::mem::swap(&mut t.doc, r.tl.doc_mut());
        (t.cursor, t.scroll, t.layer, t.selection) = (cursor, scroll, layer, selection);
    }
}

/// Terminals this wide always have room for the sidebar.
pub const WIDE: u16 = 100;
/// Narrower than this, the sidebar never shows (the canvas would vanish).
pub const NARROW_SIDEBAR_MIN: u16 = sidebar::WIDTH + 20;

fn draw_frame(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.screen_w = area.width;
    f.render_widget(Block::default().style(Style::new().bg(theme::BG)), area);
    let [top, main, status] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(area);
    let room = area.width >= WIDE || (app.narrow_sidebar && area.width >= NARROW_SIDEBAR_MIN);
    let side_w = if app.show_sidebar && room { sidebar::WIDTH } else { 0 };
    let [canvas_area, side] = Layout::horizontal([Constraint::Min(1), Constraint::Length(side_w)]).areas(main);

    let mut overlay = match &app.playback {
        Some(p) => p.overlay(),
        None if app.replay.is_some() => vec![],
        None => app.tools.preview(app.tab()),
    };
    // Brush ghost: show what a click will paint, under the mouse.
    if app.playback.is_none()
        && app.replay.is_none()
        && app.dialogs.is_empty()
        && overlay.is_empty()
        && app.mouse_idle()
        && matches!(app.tools.tool, Tool::Brush | Tool::Fill)
        && app.tools.opts.brush_mode == acidtrip_core::tools::PaintMode::Char
        && let Some((hx, hy)) = app.hover
    {
        overlay.push((hx, hy, app.tab().doc.conform(app.tools.brush.cell())));
    }
    {
        let t = &mut app.tabs[app.active];
        let z = if t.zoom { 2 } else { 1 };
        t.clamp_scroll(canvas_area.width as usize / z, canvas_area.height as usize / z);
    }
    let pending = app.tools.pending_selection(app.tab()).filter(|_| app.replay.is_none());
    let typing = app.typing_mode();
    let view = canvas::CanvasView {
        tab: app.tab(),
        overlay: &overlay,
        pending_selection: pending,
        grid: app.grid,
        show_cursor: !typing && app.cursor_visible() && app.dialogs.is_empty() && app.replay.is_none(),
        onion: app.replay.is_none() && app.playback.is_none(),
    };
    app.geom = canvas::draw(f.buffer_mut(), canvas_area, &view);
    together::draw_cursors(f.buffer_mut(), &app.geom, &app.together);
    if let Some(p) = &app.playback {
        p.mask(f.buffer_mut(), &app.geom);
    }

    app.sidebar_hits.clear();
    app.minimap_rect = None;
    let mut previews_out = None;
    if side_w > 0 {
        let mut hits = vec![];
        let g = app.geom;
        if app.tools.tool == Tool::Pen {
            let key = crate::dialogs::brushes::PreviewKey::new(
                &app.tools,
                sidebar::PREVIEW_W as usize,
                sidebar::PREVIEW_H as usize,
            );
            crate::dialogs::brushes::cached_preview(&mut app.side_preview, key, &app.tabs[app.active].doc);
        }
        let preview = app.side_preview.as_ref().map(|(_, c)| c.as_slice()).unwrap_or_default();
        let sv = sidebar::SidebarView {
            tab: app.tab(),
            tools: &app.tools,
            keymap: &app.keymap,
            slot: app.color_slot,
            minimap: app.show_minimap,
            view: (g.scroll.0, g.scroll.1, g.cols, g.rows),
            brush_preview: preview,
            hover: app.side_hover.filter(|_| app.dialogs.is_empty()),
            art: app.art_mode().then_some(&app.artboard),
            recent: &app.recent,
            together: app.together.panel.then_some(&app.together),
            replay: app.replay.as_ref(),
            open: app.side_open,
        };
        let previews = sidebar::draw(f, side, &sv, &mut hits);
        app.sidebar_hits = hits;
        app.minimap_rect = previews.minimap.map(|(_, g)| g);
        previews_out = Some(previews);
    }
    // Taken before the status bar too: a message shown whole above it may
    // cover the minimap.
    let snapshots = if app.thumbs.is_some() { snapshot(f, previews_out.as_ref()) } else { vec![] };
    let mut hits = vec![];
    draw_status(f, status, app, &mut hits);
    draw_top(f, top, app, &mut hits);
    app.sidebar_hits.extend(hits);

    if typing && app.dialogs.is_empty() {
        let (cx, cy) = app.tab().cursor;
        let g = app.geom;
        if cx >= g.scroll.0 && cy >= g.scroll.1 {
            let (dx, dy) = (cx - g.scroll.0, cy - g.scroll.1);
            let (sx, sy) = if g.zoom { (dx * 2, dy * 2) } else { (dx, dy) };
            if sx < g.area.width as usize && sy < g.area.height as usize {
                f.set_cursor_position((g.area.x + sx as u16, g.area.y + sy as u16));
            }
        }
    }

    // Dialogs draw bottom to top; each gets the full area.
    let mut dialogs = std::mem::take(&mut app.dialogs);
    let mut top = None;
    for d in dialogs.iter_mut() {
        widgets::take_popup_area();
        d.draw(f, area, app);
        top = widgets::take_popup_area();
    }
    app.dialog_area = top.map(|r| (dialogs.len(), r));
    // A dialog callback may have pushed new dialogs during draw (none do), keep order.
    dialogs.append(&mut app.dialogs);
    app.dialogs = dialogs;

    if let Some(p) = previews_out {
        draw_pixel_previews(f, app, &p, &snapshots);
    }
    if let (Some(th), Some(d)) = (app.thumbs.as_mut(), app.dialogs.last_mut()) {
        d.pixels(f, th);
    }
}

/// Real pixels where the terminal can show images. Placed after dialogs, and
/// only where no dialog drew: an image sits above the text layer, so it would
/// cover a popup, while a covered preview keeps its half-block fallback.
fn draw_pixel_previews(f: &mut Frame, app: &mut App, previews: &sidebar::Previews, snapshots: &[(Rect, Vec<Cell>)]) {
    let Some(th) = app.thumbs.as_mut() else { return };
    let untouched = |f: &mut Frame, r: Rect| {
        let buf = f.buffer_mut();
        snapshots
            .iter()
            .find(|(sr, _)| *sr == r)
            .is_some_and(|(_, cells)| r.positions().zip(cells).all(|(p, c)| buf.cell(p).is_some_and(|b| b == c)))
    };
    let t = &app.tabs[app.active];
    let g = app.geom;
    let src = thumbs::Source {
        doc_id: t.doc.meta.id,
        // Each replay step is its own picture.
        revision: app.replay.as_ref().map_or(t.history.revision(), |r| u64::MAX - r.tl.pos() as u64),
        canvas: &t.doc.canvas,
        palette: &t.doc.meta.palette,
    };
    th.begin_frame();
    th.retain_layers(t.doc.canvas.layers.len());
    for (i, r) in &previews.layers {
        if untouched(f, *r) {
            th.layer(f, *r, &src, *i, t.scroll.1);
        }
    }
    th.retain_recent(crate::recent::STRIP);
    for (k, &(r, i, dim)) in previews.recent.iter().enumerate() {
        if untouched(f, r)
            && let (Some(d), Some(piece)) = (app.recent.doc(i), app.recent.pieces.get(i))
        {
            let key = piece.key().bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
            th.recent(f, k, r, key, &d.canvas, &d.meta.palette, dim);
        }
    }
    if let Some((area, _)) = previews.minimap
        && untouched(f, area)
    {
        let view = (g.scroll.0, g.scroll.1, g.cols, g.rows);
        if let Some(mg) = th.minimap(f, area, &src, view) {
            app.minimap_rect = Some(mg);
        }
    }
}

/// Copies of the preview areas as the sidebar drew them, to spot dialogs.
fn snapshot(f: &mut Frame, previews: Option<&sidebar::Previews>) -> Vec<(Rect, Vec<Cell>)> {
    let Some(p) = previews else { return vec![] };
    let buf = f.buffer_mut();
    let grab = |r: Rect| (r, r.positions().filter_map(|pos| buf.cell(pos).cloned()).collect());
    p.layers
        .iter()
        .map(|(_, r)| grab(*r))
        .chain(p.recent.iter().map(|(r, _, _)| grab(*r)))
        .chain(p.minimap.map(|(r, _)| grab(r)))
        .collect()
}

/// Top bar: file name left, the tile set centered (ACiDDraw kept its F-key
/// set up top), logo right.
fn draw_top(f: &mut Frame, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    f.render_widget(Paragraph::new("").style(Style::new().bg(theme::PANEL)), area);
    let t = app.tab();
    let dot = if t.dirty() { "● " } else { "  " };
    // The name is a button: a click saves (asks for a name when untitled),
    // so saving works with the mouse alone.
    let save = Hit::Act(Action::Save);
    let bg = if app.side_hover == Some(save) && app.dialogs.is_empty() { theme::BORDER } else { theme::PANEL };
    let title = Line::from(vec![
        Span::styled(format!(" {dot}"), Style::new().fg(theme::WARN).bg(bg)),
        Span::styled(format!("{} ", t.title()), Style::new().fg(theme::TEXT).bg(bg).add_modifier(Modifier::BOLD)),
    ]);
    let title_w = title.width() as u16;
    let title_r = Rect::new(area.x, area.y, title_w.min(area.width), 1);
    f.render_widget(Paragraph::new(title), title_r);
    hits.push((title_r, save));

    let logo = " acidtrip ";
    let logo_w = logo.chars().count() as u16;
    let logo_r = Rect::new(area.right().saturating_sub(logo_w), area.y, logo_w, 1);
    let hot = app.side_hover == Some(Hit::Act(Action::CommandPalette)) && app.dialogs.is_empty();
    let st = Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD | Modifier::ITALIC);
    f.render_widget(Paragraph::new(Span::styled(logo, if hot { st.bg(theme::BORDER) } else { st })), logo_r);

    let ts = &app.tools;
    let set = &ts.charsets[ts.charset % ts.charsets.len()];
    let pal = &t.doc.meta.palette;
    let (fg, bg) = (canvas::rgb(ts.brush.fg, pal), canvas::rgb(ts.brush.bg, pal));
    let mut bar = vec![Span::styled(format!("set {:>2}  ", ts.charset + 1), Style::new().fg(theme::DIM))];
    for (i, ch) in set.chars.iter().enumerate() {
        let key = if i == 9 { '0' } else { char::from(b'1' + i as u8) };
        let st = if *ch == ts.brush.ch {
            Style::new().fg(theme::BG).bg(theme::ACCENT2)
        } else {
            Style::new().fg(theme::DIM)
        };
        bar.push(Span::styled(key.to_string(), st));
        bar.push(Span::styled(ch.to_string(), Style::new().fg(fg).bg(bg)));
        bar.push(Span::raw(" "));
    }
    let bar_w: u16 = bar.iter().map(|s| s.width() as u16).sum();
    // Centered, unless that would collide with the title or logo.
    let centered = area.x + area.width.saturating_sub(bar_w) / 2;
    let x = centered.max(area.x + title_w + 1);
    if x + bar_w + logo_w <= area.right() {
        f.render_widget(Paragraph::new(Line::from(bar)), Rect::new(x, area.y, bar_w, 1));
    }
    // The logo opens the command palette: every command, one click away.
    hits.push((logo_r, Hit::Act(Action::CommandPalette)));
}

fn draw_status(f: &mut Frame, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    let t = app.tab();
    let d = &t.doc;
    let pal = &d.meta.palette;
    let b = app.tools.brush;
    let tool = app.tools.tool;
    let mode = if app.replay.is_some() {
        "REPLAY"
    } else if app.art_mode() {
        "ART"
    } else if tool == Tool::Text {
        if t.insert_mode { "TYPE·INS" } else { "TYPE" }
    } else {
        tool.name()
    };
    let pos = app.hover.filter(|_| tool != Tool::Text).unwrap_or(t.cursor);
    let kind = match d.meta.kind {
        acidtrip_core::DocKind::Classic => "CLASSIC",
        acidtrip_core::DocKind::Modern => "MODERN",
    };
    // (span, button, drop order): on a narrow terminal a message would be
    // cut off, so parts with a drop order give way, highest first.
    let mut parts: Vec<(Span, Option<Hit>, u8)> = vec![
        (
            Span::styled(
                format!(" {mode} "),
                Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD),
            ),
            None,
            0,
        ),
        (
            Span::styled(
                format!(" {}{} ", b.ch, b.ch),
                Style::new().fg(canvas::rgb(b.fg, pal)).bg(canvas::rgb(b.bg, pal)),
            ),
            None,
            0,
        ),
        (Span::styled(format!(" {:>3},{:<3}", pos.0, pos.1), Style::new().fg(theme::TEXT)), None, 2),
        (under_cursor(app), None, 3),
        (Span::raw(" "), None, 1),
    ];
    // Size, mode and iCE are buttons: they join the sidebar's hit list, so
    // they light up under the mouse and show a tip.
    let flip =
        if d.meta.kind == acidtrip_core::DocKind::Classic { Action::ConvertModern } else { Action::ConvertClassic };
    let ice = if d.meta.ice { "iCE" } else { "no iCE" };
    for (text, hit) in [
        (format!("{}x{}", d.width(), d.height()), Hit::Act(Action::CanvasSize)),
        (kind.to_string(), Hit::Act(flip)),
        (ice.to_string(), Hit::Act(Action::ToggleIce)),
    ] {
        let hot = app.side_hover == Some(hit) && app.dialogs.is_empty();
        let st = if hot {
            Style::new().fg(theme::TEXT).bg(theme::BORDER)
        } else {
            Style::new().fg(theme::DIM).add_modifier(Modifier::UNDERLINED)
        };
        parts.push((Span::styled(text, st), Some(hit), 1));
        parts.push((Span::raw(" "), None, 1));
    }
    parts.push((
        Span::styled(format!(" L{}/{} ", t.layer + 1, d.canvas.layers.len()), Style::new().fg(theme::DIM)),
        None,
        1,
    ));
    // Which frame is showing: the frame strip lives in the sidebar, which a
    // narrow terminal hides, and the onion skin makes frames look alike.
    if d.frame_count() > 1 {
        parts.push((
            Span::styled(format!("F{}/{} ", d.current_frame() + 1, d.frame_count()), Style::new().fg(theme::DIM)),
            None,
            1,
        ));
    }
    if t.zoom {
        parts.push((Span::styled("ZOOM ", Style::new().fg(theme::WARN)), None, 0));
    }
    {
        let (text, st) = together::status_chip(&app.together);
        let hit = Hit::Act(Action::TogetherPanel);
        let st = if app.side_hover == Some(hit) { st.bg(theme::BORDER) } else { st };
        parts.push((Span::styled(text, st), Some(hit), 0));
    }
    // The message under the mouse shows whole above the bar, not as a tip.
    let tip = app
        .side_hover
        .filter(|h| *h != Hit::Message)
        .filter(|_| app.dialogs.is_empty() && app.mouse_idle())
        .filter(|_| app.msg.as_ref().is_none_or(|(_, _, at)| *at < app.side_hover_at))
        .map(|h| match h {
            sidebar::Hit::Export(e) => crate::exporter::tip(app, e),
            h => sidebar::tip(h, &app.tools, &app.keymap, app.color_slot),
        });
    // The message gets what's left of the line; a long one ends in "…"
    // rather than stopping mid-word.
    let used = |parts: &[(Span, Option<Hit>, u8)]| parts.iter().map(|p| p.0.width()).sum::<usize>();
    let room = (area.width as usize).saturating_sub(used(&parts));
    if let Some(tip) = tip {
        parts.push((Span::styled(ellipsize(&format!("ⓘ {tip}"), room), Style::new().fg(theme::ACCENT2)), None, 0));
    } else if let Some(a) = &app.agent {
        let spin =
            ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][(a.started.elapsed().as_millis() / 100 % 10) as usize];
        // A mouse way to stop the run (Esc on the canvas is the key).
        let hit = Hit::Act(Action::AiStop);
        let st = if app.side_hover == Some(hit) { theme::WARN } else { theme::DIM };
        parts.push((Span::styled("✕ stop", Style::new().fg(st).add_modifier(Modifier::UNDERLINED)), Some(hit), 0));
        parts.push((Span::raw(" "), None, 0));
        let status = ellipsize(&format!("{spin} {}", a.status), room.saturating_sub(7));
        parts.push((Span::styled(status, Style::new().fg(theme::ACCENT2)), None, 0));
    } else if let Some((m, lvl, _)) = &app.msg {
        let c = match lvl {
            Level::Info => theme::TEXT,
            Level::Ok => theme::OK,
            Level::Warn => theme::WARN,
            Level::Error => theme::ERR,
        };
        // A warning must be readable whole; other news is cut off rather
        // than hide the read-outs. (A hover tip never pushes the chips away:
        // they may be what it's about.)
        let orders: &[u8] = if matches!(lvl, Level::Warn | Level::Error) { &[3, 2, 1] } else { &[] };
        let need = unicode_width::UnicodeWidthStr::width(m.as_str());
        for &order in orders {
            if used(&parts) + need <= area.width as usize {
                break;
            }
            parts.retain(|p| p.2 != order);
        }
        let room = (area.width as usize).saturating_sub(used(&parts));
        let shown = fit_message(m, room);
        // Cut short, it is a button: hovering shows it whole, a click opens
        // the message history.
        let cut = shown != *m;
        let hover = cut && app.side_hover == Some(Hit::Message) && app.dialogs.is_empty();
        let st = if hover { Style::new().fg(c).bg(theme::BORDER) } else { Style::new().fg(c) };
        parts.push((Span::styled(shown, st), cut.then_some(Hit::Message), 0));
        if hover {
            full_message(f, area, m, c);
        }
    }
    let mut x = area.x;
    let mut spans = Vec::with_capacity(parts.len());
    for (span, hit, _) in parts {
        let w = span.width() as u16;
        if let Some(hit) = hit {
            hits.push((Rect::new(x, area.y, w, 1), hit));
        }
        x += w;
        spans.push(span);
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(Style::new().bg(theme::PANEL)), area);
}

/// A message too long for the status bar, whole, in a box just above it
/// on the right (where the message sits).
fn full_message(f: &mut Frame, status: Rect, m: &str, color: ratatui::style::Color) {
    let screen = f.area();
    let w = (unicode_width::UnicodeWidthStr::width(m) as u16 + 4).clamp(24, screen.width);
    let rows = widgets::wrap_words(m, w.saturating_sub(4) as usize);
    let hint = "click: message history";
    let h = (rows.len() as u16 + 2).min(status.y.saturating_sub(screen.y));
    if h < 3 {
        return;
    }
    let r = Rect::new(screen.right() - w, status.y - h, w, h);
    let mut block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::new().fg(theme::BORDER))
        .style(Style::new().bg(theme::PANEL));
    if widgets::hint_fits_border(hint, w) {
        block = block
            .title_bottom(Line::from(Span::styled(format!(" {hint} "), Style::new().fg(theme::DIM))).right_aligned());
    }
    let inner = block.inner(r);
    f.render_widget(ratatui::widgets::Clear, r);
    f.render_widget(block, r);
    let lines: Vec<Line> = rows.into_iter().map(|t| Line::from(Span::styled(t, Style::new().fg(color)))).collect();
    f.render_widget(Paragraph::new(lines), Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner });
}

/// A status message that fits `room` columns: paths shrink to "…/name"
/// first (the name is what matters), longest first, then the end is cut.
fn fit_message(m: &str, room: usize) -> String {
    let width = |w: &[String]| w.iter().map(|s| s.chars().count()).sum::<usize>() + w.len().saturating_sub(1);
    let mut words: Vec<String> = m.split(' ').map(str::to_string).collect();
    while width(&words) > room {
        let longest = words
            .iter()
            .enumerate()
            .filter(|(_, w)| {
                (w.starts_with('/') || w.starts_with("~/") || w.matches('/').count() > 1) && !w.starts_with("…/")
            })
            .max_by_key(|(_, w)| w.chars().count())
            .map(|(i, _)| i);
        let Some(i) = longest else { break };
        let w = &words[i];
        let tail = w.trim_end_matches(['/', ':', ',', ')']);
        let name = tail.rsplit('/').next().unwrap_or(tail);
        words[i] = format!("…/{name}{}", &w[tail.len()..]);
    }
    let out = words.join(" ");
    if out.chars().count() <= room {
        return out;
    }
    let mut cut: String = out.chars().take(room.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// `s` cut to `room` columns, ending in "…" when it had to be cut.
fn ellipsize(s: &str, room: usize) -> String {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if s.width() <= room {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > room {
            break;
        }
        out.push(c);
        w += cw;
    }
    if room > 0 {
        out.push('…');
    }
    out
}

/// What's under the keyboard cursor: glyph, CP437 code and colors, drawn in
/// its own colors, so a cursor over a full block never hides the cell.
fn under_cursor(app: &App) -> Span<'static> {
    let t = app.tab();
    if !app.key_cursor {
        return Span::raw("");
    }
    let (x, y) = t.cursor;
    let c = t.doc.canvas.composite(x, y);
    let pal = &t.doc.meta.palette;
    let ch = if c.ch.is_control() || c.ch == '\u{0}' { ' ' } else { c.ch };
    let code = acidtrip_core::cp437::from_char(c.ch)
        .map(|b| format!("{b}"))
        .unwrap_or_else(|| format!("U+{:04X}", c.ch as u32));
    let label = |col: acidtrip_core::Color| match col {
        acidtrip_core::Color::Pal(i) => i.to_string(),
        acidtrip_core::Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    };
    Span::styled(
        format!(" [{ch}] {code} {}/{} ", label(c.fg), label(c.bg)),
        Style::new().fg(canvas::rgb(c.fg, pal)).bg(canvas::rgb(c.bg, pal)),
    )
}

#[cfg(test)]
mod tests {
    use super::{ellipsize, fit_message};

    #[test]
    fn long_messages_shorten_paths_first() {
        let m = "saved /tmp/acidtrip-flows/documents-19/s.xb";
        assert_eq!(fit_message(m, 80), m);
        assert_eq!(fit_message(m, 33), "saved …/s.xb");
        let m = "exported /a/long/folder/p.png, /a/long/folder/p.svg";
        assert_eq!(fit_message(m, 30), "exported …/p.png, …/p.svg");
        // Not paths: cut at the end.
        assert_eq!(fit_message("fg/bg swapped on every layer", 12), "fg/bg swapp…");
    }

    #[test]
    fn long_messages_end_in_an_ellipsis() {
        assert_eq!(ellipsize("short", 10), "short");
        assert_eq!(ellipsize("exactly10!", 10), "exactly10!");
        assert_eq!(ellipsize("a bit too long", 8), "a bit t…");
        assert_eq!(ellipsize("tip", 0), "");
    }
}
