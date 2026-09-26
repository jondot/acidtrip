//! Draw together: the app's side of a shared session (the network side is
//! the `acidtrip-net` crate). The shared document is the open one; its
//! history records every change, which goes out from [`App::together_tick`],
//! and edits from the others come in there too.
//!
//! Hosting, the app is the one place edits get their order: its own go out
//! as made, and a guest's are applied here, then passed on to everyone.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use acidtrip_net::{Event, Options, PeerId, PeerInfo, Session};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::actions::Action;
use crate::app::{App, Level};
use crate::dialogs;
use crate::tab::Tab;
use crate::ui::widgets::LineInput;

/// Cursor updates go out at most this often.
const CURSOR_EVERY: Duration = Duration::from_millis(50);

#[derive(Default)]
pub struct Together {
    /// The TOGETHER panel is showing in the sidebar.
    pub panel: bool,
    pub session: Option<Session>,
    /// The shared document: opening another one leaves the session.
    doc_id: Option<uuid::Uuid>,
    /// Hosting: what guests join with.
    pub ticket: Option<String>,
    /// Everyone in the session, the host first.
    pub peers: Vec<PeerInfo>,
    /// Your id (the host is 0).
    pub you: PeerId,
    /// Where the others are pointing, in canvas cells.
    pub cursors: BTreeMap<PeerId, (u32, u32)>,
    /// Connection progress or the last problem, for the panel.
    pub status: String,
    /// The join field (a pasted ticket).
    pub input: LineInput,
    /// Keys go to the join field.
    pub focus: bool,
    /// Guest: edits sent and not yet back from the host, oldest first.
    /// Their echoes aren't logged again for replay.
    unechoed: VecDeque<acidtrip_core::Transaction>,
    sent_cursor: Option<(u32, u32)>,
    cursor_sent_at: Option<Instant>,
}

impl Together {
    pub fn active(&self) -> bool {
        self.session.is_some()
    }

    pub fn hosting(&self) -> bool {
        self.session.as_ref().is_some_and(|s| s.is_host())
    }

    /// Joined and in (the host's document arrived).
    pub fn connected(&self) -> bool {
        self.doc_id.is_some()
    }

    pub fn peer(&self, id: PeerId) -> Option<&PeerInfo> {
        self.peers.iter().find(|p| p.id == id)
    }
}

/// The name the others see: config `ui.name`, else the login name.
fn my_name(app: &App) -> String {
    let n = app.config.ui.name.trim();
    if !n.is_empty() {
        return n.to_string();
    }
    ["USER", "USERNAME", "LOGNAME"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "someone".into())
}

fn options(app: &App) -> Options {
    // ACIDTRIP_NET_LOCAL=1: 127.0.0.1 only, no relays (tests, offline demos).
    Options { name: my_name(app), localhost: std::env::var("ACIDTRIP_NET_LOCAL").is_ok_and(|v| v == "1") }
}

/// A ticket in the clipboard, if there is one.
fn clipboard_ticket() -> Option<String> {
    let s = crate::share::paste_text()?;
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    acidtrip_net::parse_ticket(&clean).is_ok().then_some(clean)
}

impl App {
    pub fn together_run(&mut self, a: Action) {
        match a {
            Action::TogetherPanel => {
                self.together.panel = !self.together.panel;
                if self.together.panel {
                    self.reveal_sidebar();
                } else {
                    self.together.focus = false;
                }
            }
            Action::TogetherHost => self.together_host(),
            Action::TogetherJoin => {
                let text = self.together.input.text.trim().to_string();
                if self.together.active() {
                    self.flash("you're in a session already — leave it first", Level::Info);
                } else if text.is_empty() {
                    self.together_paste_ticket();
                } else {
                    self.together_join(text);
                }
            }
            Action::TogetherPasteTicket => self.together_paste_ticket(),
            Action::TogetherCopyTicket => match self.together.ticket.clone() {
                Some(t) => match crate::share::copy_text(&t) {
                    Ok(()) => self.flash("ticket copied: send it to whoever you want to draw with", Level::Ok),
                    Err(e) => self.flash(format!("can't copy: {e:#}"), Level::Error),
                },
                None => self.flash("host a session first: its ticket is what others join with", Level::Info),
            },
            Action::TogetherLeave => {
                if self.together.active() {
                    let msg = if self.together.hosting() { "session ended" } else { "left the session" };
                    self.together_leave();
                    self.flash(msg, Level::Info);
                } else {
                    self.flash("not in a session", Level::Info);
                }
            }
            _ => {}
        }
    }

    fn show_together(&mut self) {
        self.together.panel = true;
        self.reveal_sidebar();
    }

    fn together_host(&mut self) {
        self.show_together();
        if self.together.active() {
            self.flash("you're in a session already", Level::Info);
            return;
        }
        let opts = options(self);
        let t = self.tab_mut();
        t.history.set_shared(true);
        let doc = t.doc.clone();
        let id = doc.meta.id;
        self.together.session = Some(Session::host(opts, doc));
        self.together.doc_id = Some(id);
        self.together.status = "starting…".into();
        self.together.focus = false;
        self.flash("starting the session…", Level::Info);
    }

    /// Focus the join field and fill it from the clipboard when that holds
    /// a ticket.
    fn together_paste_ticket(&mut self) {
        self.show_together();
        if self.together.active() {
            return;
        }
        self.together.focus = true;
        match clipboard_ticket() {
            Some(t) => {
                self.together.input = LineInput::new(&t);
                self.flash("ticket pasted — Enter or Join to join", Level::Info);
            }
            None => self.flash("paste the host's ticket here, then Enter", Level::Info),
        }
    }

    fn together_join(&mut self, ticket: String) {
        if let Err(e) = acidtrip_net::parse_ticket(&ticket) {
            self.together.status = format!("{e}");
            self.flash(format!("{e} — ask the host to copy it again"), Level::Warn);
            return;
        }
        let start = move |app: &mut App| match Session::join(options(app), &ticket) {
            Ok(s) => {
                app.together.session = Some(s);
                app.together.status = "connecting…".into();
                app.together.focus = false;
                app.flash("joining…", Level::Info);
            }
            Err(e) => app.flash(format!("{e:#}"), Level::Error),
        };
        if self.tab().dirty() {
            let name = self.tab().title();
            self.dialogs.push(Box::new(dialogs::prompt::ConfirmDialog::new(
                &format!("Joining replaces {name}, which has unsaved changes. Join anyway?"),
                Box::new(start),
            )));
        } else {
            start(self);
        }
    }

    /// Leave (or end) the session and stop sharing the document.
    pub fn together_leave(&mut self) {
        if let Some(s) = self.together.session.take() {
            s.leave();
        }
        if let Some(t) = self.tabs.iter_mut().find(|t| Some(t.doc.meta.id) == self.together.doc_id) {
            t.history.set_shared(false);
        }
        let panel = self.together.panel;
        let input = std::mem::take(&mut self.together.input);
        let status = std::mem::take(&mut self.together.status);
        self.together = Together { panel, input, status, ..Default::default() };
    }

    /// A key while the join field has focus; false passes it on.
    pub fn together_key(&mut self, k: KeyEvent) -> bool {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Esc => self.together.focus = false,
            KeyCode::Enter => self.together_run(Action::TogetherJoin),
            KeyCode::Char('v') if ctrl => match crate::share::paste_text() {
                Some(s) => self.together.input.paste(s.trim()),
                None => self.flash("the clipboard is empty", Level::Info),
            },
            _ => return self.together.input.key(&k),
        }
        true
    }

    /// Pasted text: into the join field when it has focus, and a ticket
    /// pasted anywhere else opens it ready to join.
    pub fn together_paste(&mut self, s: &str) -> bool {
        let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        if self.together.focus {
            self.together.input.paste(&clean);
            return true;
        }
        if self.together.active() || acidtrip_net::parse_ticket(&clean).is_err() {
            return false;
        }
        self.show_together();
        self.together.focus = true;
        self.together.input = LineInput::new(&clean);
        self.flash("that's a session ticket — Enter or Join to join", Level::Info);
        true
    }

    /// Send what changed here, take in what the others did. True when
    /// something needs drawing.
    pub fn together_tick(&mut self) -> bool {
        if self.together.session.is_none() {
            return false;
        }
        let mut redraw = false;
        // Another document opened (or new): it isn't shared.
        if let Some(id) = self.together.doc_id
            && self.tab().doc.meta.id != id
        {
            let hosting = self.together.hosting();
            self.together_leave();
            let what = if hosting { "ended the session" } else { "left the session" };
            self.flash(format!("{what}: another document is open now"), Level::Warn);
            return true;
        }
        self.together_send_changes();
        while let Some(ev) = self.together.session.as_ref().and_then(|s| s.poll()) {
            redraw = true;
            match ev {
                Event::Ticket(t) => {
                    // On the clipboard right away, ready to paste in a chat.
                    let copied = crate::share::copy_text(&t).is_ok();
                    self.together.ticket = Some(t);
                    self.together.status = "send the ticket to others".into();
                    if copied {
                        self.flash("ticket copied: send it to whoever you want to draw with", Level::Ok);
                    }
                }
                Event::Welcome { you, doc, rejoined } => {
                    self.together.you = you;
                    self.together.doc_id = Some(doc.meta.id);
                    self.together.cursors.clear();
                    self.together.unechoed.clear();
                    let mut tab = Tab::new(*doc, None);
                    tab.history.set_shared(true);
                    if rejoined {
                        let old = self.tab();
                        (tab.cursor, tab.scroll, tab.layer, tab.zoom) = (old.cursor, old.scroll, old.layer, old.zoom);
                        tab.clamp();
                    }
                    let old = self.tab().doc.meta.id;
                    acidtrip_io::recovery::clear(&self.paths.recovery_dir(), old);
                    self.tabs = vec![tab];
                    self.active = 0;
                    self.tools.floating = None;
                    self.tools.anchor = None;
                    self.together.status = String::new();
                    // A first join is announced with the peer list, which
                    // names the host and comes right after.
                    if rejoined {
                        self.flash("back in the session", Level::Ok);
                    }
                }
                Event::Edit(tx) => {
                    let hosting = self.together.hosting();
                    let echo = !hosting && self.together.unechoed.front() == Some(&tx);
                    if echo {
                        self.together.unechoed.pop_front();
                    }
                    if let Some(t) = self.tabs.iter_mut().find(|t| Some(t.doc.meta.id) == self.together.doc_id) {
                        if echo {
                            t.history.apply_echo(&mut t.doc, &tx);
                        } else {
                            t.history.apply_remote(&mut t.doc, &tx);
                        }
                        t.clamp();
                    }
                    if hosting && let Some(s) = &self.together.session {
                        s.edit(tx);
                    }
                }
                Event::Peers(p) => {
                    let before: Vec<PeerId> = self.together.peers.iter().map(|p| p.id).collect();
                    let now: Vec<PeerId> = p.iter().map(|p| p.id).collect();
                    // Say who came and went (not on the first list).
                    if before.is_empty() && !self.together.hosting() {
                        let host = p.iter().find(|q| q.id == acidtrip_net::HOST).map(|q| q.name.as_str());
                        self.flash(format!("joined {}'s drawing", host.unwrap_or("the host")), Level::Ok);
                    } else if !before.is_empty() {
                        if let Some(n) = p.iter().find(|q| !before.contains(&q.id) && q.id != self.together.you) {
                            self.flash(format!("{} joined", n.name), Level::Ok);
                        }
                        if let Some(g) = self.together.peers.iter().find(|q| !now.contains(&q.id)) {
                            self.flash(format!("{} left", g.name), Level::Info);
                        }
                    }
                    self.together.cursors.retain(|id, _| now.contains(id));
                    if p.len() > 1 {
                        // Someone's here: the "send the ticket" hint is done.
                        self.together.status.clear();
                    }
                    self.together.peers = p;
                }
                Event::Cursor { peer, at } => match at {
                    Some(at) => {
                        self.together.cursors.insert(peer, at);
                    }
                    None => {
                        self.together.cursors.remove(&peer);
                    }
                },
                Event::Status(s) => self.together.status = s,
                Event::Ended(reason) => {
                    self.together_leave();
                    self.together.status = reason.clone();
                    self.flash(reason, Level::Warn);
                    return true;
                }
            }
        }
        self.together_send_cursor();
        redraw
    }

    /// Changes made here since the last tick, in the order made.
    fn together_send_changes(&mut self) {
        let Some(id) = self.together.doc_id else {
            return;
        };
        let Some(t) = self.tabs.iter_mut().find(|t| t.doc.meta.id == id) else {
            return;
        };
        let changes = t.history.take_changes();
        if let Some(s) = &self.together.session {
            for tx in changes {
                if !s.is_host() {
                    self.together.unechoed.push_back(tx.clone());
                }
                s.edit(tx);
            }
        }
    }

    fn together_send_cursor(&mut self) {
        if !self.together.connected() {
            return;
        }
        let at = if self.key_cursor { Some(self.tab().cursor) } else { self.canvas_hover() };
        let at = at.map(|(x, y)| (x as u32, y as u32));
        let t = &mut self.together;
        if at == t.sent_cursor || t.cursor_sent_at.is_some_and(|s| s.elapsed() < CURSOR_EVERY) {
            return;
        }
        if let Some(s) = &t.session {
            s.cursor(at);
        }
        t.sent_cursor = at;
        t.cursor_sent_at = Some(Instant::now());
    }
}
