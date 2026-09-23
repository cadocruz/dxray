//! A terminal browser for the Steam and Heroic libraries this project can
//! inspect. The first frame is drawn before discovery starts, and games stream
//! in from a background thread as they are read.
//!
//! A separate binary so `dxray` stays free of a terminal stack; the split is
//! enforced by `crates/dxray-cli/tests/dependencies.rs`.
//!
//! The event loop is this crate's: `ratatui_tea` runs a `Cmd` synchronously on
//! the calling thread, so a scan inside one would freeze the screen. Two
//! dependency behaviours shape the rest: `SelectList` does not scroll, so the
//! model hands it only the visible slice; and `ratatui::restore()` never shows
//! the cursor again, so [`screen::restore`] does.

pub mod app;
pub mod entry;
pub mod key;
pub mod layout;
pub mod msg;
pub mod run;
pub mod screen;
pub mod view;

pub use key::Key;
pub use msg::Msg;
