//! Replay: watch how the piece was drawn. The canvas shows the document at
//! a moment of its edit log (the live document is untouched); the sidebar's
//! replay panel plays, scrubs and exports it.
//!
//! Replay is a view of the whole document rather than a drawing tool, so it
//! is a mode like the art board: its panel takes the tool options' place,
//! and anything that edits leaves it first.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use acidtrip_core::replay::{Speed, Timeline, TimelineOptions};
use acidtrip_io::format::{self, ReplayExport};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::actions::Action;
use crate::app::{App, Level};

/// Speed chips: label and speed.
pub const SPEEDS: [(&str, Speed); 4] = [
    (" 1× ", Speed::Times(1.0)),
    (" 4× ", Speed::Times(4.0)),
    (" 16× ", Speed::Times(16.0)),
    (" 30s ", Speed::Fit(30)),
];
/// "fit to 30 s" by default: any piece plays in half a minute.
const DEFAULT_SPEED: usize = 3;

/// A control in the replay panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayHit {
    Play,
    /// The scrubber: click or drag along it.
    Scrub,
    Speed(usize),
    SkipIdle,
    HideUndone,
    /// Export: true = GIF, false = asciinema.
    Export(bool),
}

pub struct ReplayView {
    pub tl: Timeline,
    pub playing: bool,
    pub speed: usize,
    /// Playhead in virtual ms.
    pub t: f64,
    last: Instant,
    doc_id: uuid::Uuid,
    /// Live history revision the timeline was built from.
    revision: u64,
    /// When the timeline last caught up with a shared session.
    grown: Instant,
}

impl ReplayView {
    pub fn factor(&self) -> f64 {
        SPEEDS[self.speed].1.factor(self.tl.duration())
    }

    fn seek_step(&mut self, pos: usize) {
        self.playing = false;
        self.tl.seek(pos);
        self.t = self.tl.time_at(self.tl.pos()) as f64;
    }

    fn seek_t(&mut self, t: f64) {
        self.t = t.clamp(0.0, self.tl.duration() as f64);
        self.tl.seek_time(self.t as u64);
    }

    /// Fraction of the way through (for the scrubber).
    pub fn progress(&self) -> f64 {
        let d = self.tl.duration();
        if d == 0 { 1.0 } else { (self.t / d as f64).clamp(0.0, 1.0) }
    }
}

/// `m:ss`.
pub fn clock(ms: f64) -> String {
    let s = (ms / 1000.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Actions that keep the replay on screen: views and help. Anything else
/// (edits, tools, files) goes back to the live document first.
fn keeps_replay(a: Action) -> bool {
    matches!(
        a,
        Action::Replay
            | Action::Sidebar
            | Action::Minimap
            | Action::Grid
            | Action::Zoom
            | Action::Help
            | Action::CommandPalette
            | Action::PageUp
            | Action::PageDown
    )
}

impl App {
    pub fn replay_on(&self) -> bool {
        self.replay.is_some()
    }

    /// Start or leave replay. Pieces without an edit log (never drawn
    /// here, or saved in another format) get the modem reveal instead.
    pub fn set_replay(&mut self, on: bool) {
        if on == self.replay_on() {
            return;
        }
        if !on {
            self.replay = None;
            self.flash("replay off · back to the live piece", Level::Info);
            return;
        }
        let t = self.tab();
        let opts = TimelineOptions::default();
        let Some(tl) = Timeline::new(t.history.log(), opts) else {
            self.playback = Some(crate::dialogs::playback::Playback::new(self.tab(), 14400));
            self.flash("no edit history here (only .acid keeps it) · modem reveal instead", Level::Info);
            return;
        };
        let (doc_id, revision) = (t.doc.meta.id, t.history.revision());
        self.reveal_sidebar();
        self.replay = Some(ReplayView {
            tl,
            playing: true,
            speed: DEFAULT_SPEED,
            t: 0.0,
            last: Instant::now(),
            doc_id,
            revision,
            grown: Instant::now(),
        });
        self.flash("replay · Space play/pause · ← → step · Home End · Esc back to drawing", Level::Info);
    }

    /// Called before running an action: edits leave the replay first.
    pub fn replay_before(&mut self, a: Action) {
        if self.replay_on() && !keeps_replay(a) {
            self.replay = None;
        }
    }

    pub fn toggle_replay_play(&mut self) {
        let Some(r) = &mut self.replay else { return };
        r.playing = !r.playing;
        if r.playing && r.t >= r.tl.duration() as f64 {
            r.seek_t(0.0);
        }
        r.last = Instant::now();
    }

    /// A key while replaying; false = not ours.
    pub fn replay_key(&mut self, k: KeyEvent) -> bool {
        if k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return false;
        }
        let Some(r) = &mut self.replay else { return false };
        match k.code {
            KeyCode::Char(' ') => self.toggle_replay_play(),
            KeyCode::Left => r.seek_step(r.tl.pos().saturating_sub(1)),
            KeyCode::Right => r.seek_step(r.tl.pos() + 1),
            KeyCode::Home => r.seek_step(0),
            KeyCode::End => r.seek_step(r.tl.len()),
            KeyCode::Esc => self.set_replay(false),
            _ => return false,
        }
        true
    }

    /// A click (or drag, for the scrubber) in the replay panel.
    pub fn replay_click(&mut self, h: ReplayHit, rect_x: u16, width: u16, col: u16) {
        let frac = self.mouse_frac.0;
        let Some(r) = &mut self.replay else { return };
        match h {
            ReplayHit::Play => self.toggle_replay_play(),
            ReplayHit::Scrub => {
                let f = ((col.saturating_sub(rect_x)) as f32 + frac) / width.max(1) as f32;
                r.playing = false;
                let d = r.tl.duration() as f64;
                r.seek_t(f.clamp(0.0, 1.0) as f64 * d);
            }
            ReplayHit::Speed(i) => {
                r.speed = i.min(SPEEDS.len() - 1);
                let msg = format!("replay speed {}", SPEEDS[r.speed].0.trim());
                self.flash(msg, Level::Info);
            }
            ReplayHit::SkipIdle | ReplayHit::HideUndone => {
                let mut o = r.tl.options();
                if h == ReplayHit::SkipIdle {
                    o.skip_idle = !o.skip_idle;
                } else {
                    o.hide_undone = !o.hide_undone;
                }
                r.tl.set_options(o);
                r.t = r.tl.time_at(r.tl.pos()) as f64;
                let msg = match h {
                    ReplayHit::SkipIdle if o.skip_idle => "long pauses are cut",
                    ReplayHit::SkipIdle => "pauses play in real time",
                    _ if o.hide_undone => "undone work is hidden",
                    _ => "undone work is shown (and its undo)",
                };
                self.flash(msg, Level::Info);
            }
            ReplayHit::Export(gif) => self.export_replay(gif),
        }
    }

    /// Where an export of this replay goes: beside the file.
    fn replay_path(&self, ext: &str) -> PathBuf {
        match &self.tab().file {
            Some(p) => {
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                p.with_file_name(format!("{stem}-replay.{ext}"))
            }
            None => PathBuf::from(format!("untitled-replay.{ext}")),
        }
    }

    pub fn export_replay(&mut self, gif: bool) {
        let Some(r) = &self.replay else { return };
        let o = ReplayExport { timeline: r.tl.options(), speed: SPEEDS[r.speed].1, scale: 1 };
        let log = self.tab().history.log();
        let bytes = if gif { format::replay_gif(log, &o) } else { format::replay_cast(log, &o) };
        let path = self.replay_path(if gif { "gif" } else { "cast" });
        match bytes.and_then(|b| acidtrip_io::library::write_atomic(&path, &b)) {
            Ok(()) => self.flash(format!("wrote {}", path.display()), Level::Ok),
            Err(e) => self.flash(format!("replay export failed: {e:#}"), Level::Error),
        }
    }

    /// Advance playback; true when the screen changed.
    pub fn replay_tick(&mut self) -> bool {
        let Some(r) = &mut self.replay else { return false };
        let t = &self.tabs[self.active];
        if t.doc.meta.id != r.doc_id {
            self.replay = None;
            return true;
        }
        if t.history.revision() != r.revision && t.history.is_shared() {
            // Drawing together: the others' edits keep coming. The timeline
            // grows (once a second) and stays where it is.
            if r.grown.elapsed() >= Duration::from_secs(1)
                && let Some(tl) = Timeline::new(t.history.log(), r.tl.options())
            {
                r.tl = tl;
                r.revision = t.history.revision();
                r.grown = Instant::now();
                let at = r.t;
                r.seek_t(at);
            }
        } else if t.history.revision() != r.revision {
            // Edited meanwhile (an AI run, say): replay what is there now.
            if let Some(tl) = Timeline::new(t.history.log(), r.tl.options()) {
                r.tl = tl;
                r.revision = t.history.revision();
                let n = r.tl.len();
                r.seek_step(n);
                return true;
            }
            self.replay = None;
            return true;
        }
        let now = Instant::now();
        let dt = now.duration_since(r.last).as_secs_f64() * 1000.0;
        r.last = now;
        if !r.playing {
            return false;
        }
        let end = r.tl.duration() as f64;
        let to = r.t + dt * r.factor();
        r.seek_t(to);
        if to >= end {
            r.playing = false;
        }
        true
    }

    pub fn replay_animating(&self) -> bool {
        self.replay.as_ref().is_some_and(|r| r.playing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_format() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(61_400.0), "1:01");
    }

    #[test]
    fn edits_leave_replay() {
        assert!(keeps_replay(Action::Replay));
        assert!(!keeps_replay(Action::Undo));
        assert!(!keeps_replay(Action::ToolBrush));
    }
}
