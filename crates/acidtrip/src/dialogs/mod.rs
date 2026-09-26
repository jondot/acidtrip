//! Modal dialogs. Each dialog gets keys/mouse while on top of the stack and
//! returns an [`Outcome`]; callbacks run against the whole [`App`].

use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::App;

pub mod ai;
pub mod block;
pub mod brushes;
pub mod canvas_size;
pub mod chars;
pub mod colors;
pub mod export;
pub mod export_row;
pub mod files;
pub mod fonts;
pub mod forms;
pub mod gallery;
pub mod harvest;
pub mod help;
pub mod import;
pub mod layers;
pub mod library;
pub mod messages;
pub mod palette;
pub mod patterns;
pub mod playback;
pub mod preview;
pub mod prompt;
pub mod recovery;
pub mod share;
pub mod stencils;
pub mod versions;

pub type Callback = Box<dyn FnOnce(&mut App)>;

pub enum Outcome {
    Keep,
    Close,
    /// Close, then run.
    Then(Callback),
    /// Stay open and run.
    KeepThen(Callback),
}

pub trait Dialog {
    fn draw(&mut self, f: &mut Frame, area: Rect, app: &App);
    fn key(&mut self, k: KeyEvent, app: &App) -> Outcome;
    fn mouse(&mut self, _m: MouseEvent, _app: &App) -> Outcome {
        Outcome::Keep
    }
    /// A click beside the dialog's popup: Esc, unless the dialog says otherwise.
    fn click_outside(&mut self, app: &App) -> Outcome {
        self.key(KeyEvent::new(crossterm::event::KeyCode::Esc, crossterm::event::KeyModifiers::NONE), app)
    }
    fn paste(&mut self, _s: &str) {}
    /// Needs periodic redraws (background progress).
    fn animating(&self) -> bool {
        false
    }
    /// After the frame is drawn, the top dialog may put real pixel images
    /// over what it drew (terminals with graphics only).
    fn pixels(&mut self, _f: &mut Frame, _thumbs: &mut crate::ui::thumbs::Thumbs) {}
}
