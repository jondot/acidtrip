//! Draw together on screen: the TOGETHER sidebar panel (host, join with a
//! pasted ticket, copy the ticket, who's here, leave), the others' cursors
//! on the canvas with their names, and the status bar chip.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::canvas::CanvasGeom;
use super::sidebar::Hit;
use super::widgets::theme;
use crate::actions::Action;
use crate::together::Together;

pub const ICON: &str = "⇄";

fn color([r, g, b]: [u8; 3]) -> Color {
    Color::Rgb(r, g, b)
}

struct Panel<'f, 'a, 'h> {
    f: &'f mut Frame<'a>,
    hits: &'h mut Vec<(Rect, Hit)>,
    hover: Option<Hit>,
    x0: u16,
    w: u16,
}

impl Panel<'_, '_, '_> {
    fn put(&mut self, x: u16, y: u16, w: u16, line: Line) {
        if w > 0 {
            self.f.render_widget(Paragraph::new(line), Rect::new(x, y, w, 1));
        }
    }

    /// A button: lit under the mouse.
    fn button(&mut self, x: u16, y: u16, label: &str, action: Action, strong: bool) -> u16 {
        let w = label.chars().count() as u16;
        let hit = Hit::Act(action);
        let st = match (self.hover == Some(hit), strong) {
            (true, _) => Style::new().fg(Color::White).bg(theme::BORDER),
            (false, true) => Style::new().fg(theme::BG).bg(theme::ACCENT2).add_modifier(Modifier::BOLD),
            (false, false) => Style::new().fg(theme::TEXT).bg(theme::PANEL_HI),
        };
        self.put(x, y, w, Line::from(Span::styled(label.to_string(), st)));
        self.hits.push((Rect::new(x, y, w, 1), hit));
        w
    }

    fn dim(&mut self, y: u16, s: &str) {
        let s: String = s.chars().take(self.w as usize).collect();
        self.put(self.x0, y, self.w, Line::from(Span::styled(s, Style::new().fg(theme::DIM))));
    }
}

/// The panel in the sidebar's tool options slot: a title bar, then rows,
/// in `area` (text starts at its left edge; the bar reaches one further).
pub fn panel(f: &mut Frame, hits: &mut Vec<(Rect, Hit)>, hover: Option<Hit>, area: Rect, t: &Together) {
    let (x0, y, w, rows) = (area.x, area.y, area.width, area.height - 1);
    let bar = Style::new().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD);
    f.render_widget(Paragraph::new("").style(bar), Rect::new(x0 - 1, y, w + 1, 1));
    let state = if t.hosting() {
        "hosting"
    } else if t.connected() {
        "joined"
    } else if t.active() {
        "joining"
    } else {
        ""
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {ICON} TOGETHER "), bar),
            Span::styled(state, Style::new().fg(theme::BG).bg(theme::ACCENT)),
        ])),
        Rect::new(x0, y, w - 3, 1),
    );
    let close = Hit::Act(Action::TogetherPanel);
    let st = if hover == Some(close) { Style::new().fg(Color::White).bg(theme::BORDER) } else { bar };
    f.render_widget(Paragraph::new(Span::styled(" ✕ ", st)), Rect::new(x0 + w - 3, y, 3, 1));
    hits.push((Rect::new(x0 + w - 3, y, 3, 1), close));

    let mut p = Panel { f, hits, hover, x0, w };
    let y = y + 1;
    let last = y + rows - 1;
    if !t.active() {
        p.button(x0, y, "  Host this drawing  ", Action::TogetherHost, true);
        // The join field: a click focuses it and pastes a ticket from the clipboard.
        let join_w = 6;
        let field = Rect::new(x0, y + 1, w - join_w - 1, 1);
        let hit = Hit::Act(Action::TogetherPasteTicket);
        if t.focus {
            t.input.render(p.f, field, "join› ", true);
        } else {
            let (text, st) = if t.input.text.is_empty() {
                ("paste a ticket".to_string(), Style::new().fg(theme::DIM))
            } else {
                (t.input.text.clone(), Style::new().fg(theme::TEXT))
            };
            let bg = if hover == Some(hit) { theme::BORDER } else { theme::PANEL_HI };
            let n = field.width.saturating_sub(6) as usize;
            let text: String = text.chars().take(n).collect();
            p.put(
                field.x,
                field.y,
                field.width,
                Line::from(vec![
                    Span::styled("join› ", Style::new().fg(theme::DIM).bg(bg)),
                    Span::styled(format!("{text:<n$}"), st.bg(bg)),
                ]),
            );
        }
        p.hits.push((field, hit));
        p.button(x0 + w - join_w, y + 1, " Join ", Action::TogetherJoin, !t.input.text.is_empty());
        let status = if t.status.is_empty() { "Host: others join by ticket." } else { t.status.as_str() };
        p.dim(y + 2, status);
        p.dim(y + 3, "Join: paste a ticket someone");
        p.dim(y + 4, "sent you, then press Join.");
        return;
    }
    let mut row = y;
    if let Some(ticket) = &t.ticket {
        let copy = " Copy ";
        let bw = copy.chars().count() as u16;
        let n = (w - bw - 1) as usize;
        let shown: String = ticket.chars().take(n.saturating_sub(1)).chain(std::iter::once('…')).collect();
        let hit = Hit::Act(Action::TogetherCopyTicket);
        let st = if hover == Some(hit) {
            Style::new().fg(Color::White).bg(theme::BORDER)
        } else {
            Style::new().fg(theme::ACCENT2)
        };
        p.put(x0, row, n as u16, Line::from(Span::styled(shown, st)));
        p.hits.push((Rect::new(x0, row, n as u16, 1), hit));
        p.button(x0 + w - bw, row, copy, Action::TogetherCopyTicket, true);
        row += 1;
    }
    // Who's here, in their colors, wrapping onto a second row.
    let mut x = x0;
    let rows_for_peers = if t.ticket.is_some() { 2 } else { 3 };
    let peer_end = row + rows_for_peers;
    for peer in &t.peers {
        let you = if peer.id == t.you { " (you)" } else { "" };
        let label = format!("● {}{you} ", peer.name);
        let lw = label.chars().count() as u16;
        if x + lw > x0 + w && x > x0 {
            row += 1;
            x = x0;
        }
        if row >= peer_end {
            break;
        }
        p.put(x, row, lw.min(x0 + w - x), Line::from(Span::styled(label, Style::new().fg(color(peer.color)))));
        x += lw;
    }
    let status_row = peer_end;
    if !t.status.is_empty() {
        p.dim(status_row, &t.status.clone());
    } else if t.connected() {
        let others = t.peers.len().saturating_sub(1);
        let s = match others {
            0 => "nobody else here yet".to_string(),
            1 => "1 other drawing with you".to_string(),
            n => format!("{n} others drawing with you"),
        };
        p.dim(status_row, &s);
    }
    let leave = if t.hosting() { " End session " } else { " Leave " };
    p.button(x0, last, leave, Action::TogetherLeave, false);
}

/// The others' cursors: their cell in their color, their name beside it.
pub fn draw_cursors(buf: &mut Buffer, g: &CanvasGeom, t: &Together) {
    let area = g.area;
    for (id, &(x, y)) in &t.cursors {
        if *id == t.you {
            continue;
        }
        let Some(peer) = t.peer(*id) else { continue };
        let (x, y) = (x as usize, y as usize);
        if x < g.scroll.0 || y < g.scroll.1 {
            continue;
        }
        let (dx, dy) = (x - g.scroll.0, y - g.scroll.1);
        let (sx, sy, cw) = if g.zoom { (dx * 2, dy * 2, 2) } else { (dx, dy, 1) };
        if sx >= area.width as usize || sy >= area.height as usize {
            continue;
        }
        let (sx, sy) = (area.x + sx as u16, area.y + sy as u16);
        let c = color(peer.color);
        for i in 0..cw {
            if sx + i < area.right()
                && let Some(cell) = buf.cell_mut((sx + i, sy))
            {
                cell.set_bg(c).set_fg(Color::Black);
            }
        }
        // The name to the right, or to the left at the edge.
        let name: String = format!(" {} ", peer.name);
        let nw = name.chars().count() as u16;
        let nx = if sx + cw + nw <= area.right() { sx + cw } else { sx.saturating_sub(nw).max(area.x) };
        let st = Style::new().fg(Color::Black).bg(c).add_modifier(Modifier::BOLD);
        buf.set_stringn(nx, sy, &name, (area.right() - nx) as usize, st);
    }
}

/// The status bar chip: opens the panel; in a session it says how many
/// are here.
pub fn status_chip(t: &Together) -> (String, Style) {
    let (text, fg) = if !t.active() {
        (format!(" {ICON} "), theme::DIM)
    } else if t.connected() {
        (format!(" {ICON} {} ", t.peers.len()), theme::ACCENT2)
    } else {
        (format!(" {ICON} … "), theme::WARN)
    };
    (text, Style::new().fg(fg).add_modifier(Modifier::BOLD))
}
