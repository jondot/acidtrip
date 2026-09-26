//! Headless end-to-end test and screenshot harness for acidtrip.
//!
//! Launches a real program in a pseudo-terminal, emulates the terminal with
//! `vt100` (answering the queries crossterm makes at startup), drives it with
//! keys and SGR mouse events, and renders the screen to PNG with the VGA
//! 8x16 font from `acidtrip-core`.

pub mod fake_claude;
pub mod keys;
pub mod responder;
pub mod script;
pub mod session;

pub use fake_claude::FakeClaude;
pub use keys::Mods;
pub use script::{RunOptions, RunReport, default_app_bin, run_script, workspace_root};
pub use session::{CellInfo, DEFAULT_BG, DEFAULT_FG, MouseButton, MouseEv, MouseKind, Session, index_rgb};
