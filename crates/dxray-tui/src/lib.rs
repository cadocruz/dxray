//! A terminal browser for the Steam and Heroic libraries this project can
//! inspect, for all discovered games at once.
//!
//! # What it does
//!
//! This crate is a working, separate terminal binary in the workspace. It
//! opens a themed browser promptly, starts Steam and Heroic discovery only
//! after that first frame, and streams completed game records back to the
//! terminal thread. The user can filter by name or launcher id, move through the game list,
//! move focus to a scrollable evidence panel, and inspect renderer and NVAPI
//! findings for the selected game.
//!
//! Run it with `cargo run -p dxray-tui` from an interactive terminal.
//! `Esc` clears the filter before quitting, and `Ctrl-C` always quits; `q` is
//! deliberately searchable text rather than a quit key.
//!
//! # Why this is a second binary
//!
//! This binary brings in a substantially larger terminal stack than `dxray`,
//! which makes the split more load-bearing than it first looked. `dxray` is the
//! thing that goes in a script, and a script never passes `--tui`, so a `--tui`
//! flag would have made every scripted invocation compile a terminal library it
//! will never open.
//!
//! The split is enforced by `crates/dxray-cli/tests/dependencies.rs` rather
//! than by intention, because intention is what gets edited away.
//!
//! # The shape of the program, and the constraint that chose it
//!
//! `ratatui_tea::Program` is a shell and not a runtime: it has `new`, `init`,
//! `send` and `draw`, and no `run`. The event loop belongs to this crate.
//!
//! That turns out to be the only workable arrangement anyway, because
//! `ratatui_tea::Cmd::once` executes **synchronously**, inside
//! `into_messages`, on the thread that called `send` — and so does
//! `Cmd::tick`, which sleeps on that same thread. Scanning a Steam library is
//! hundreds of directory walks and PE reads. Put that in a `Cmd` and the screen
//! freezes for the length of the scan: no spinner, no cursor, no `q`. There is
//! no timer in that crate that can drive a spinner either, for the same reason.
//!
//! So the scan is a thread that posts one message per game down the channel
//! `ratatui_tea::channel` hands out, and the loop drains that channel into the
//! program between frames. The list fills as answers arrive.
//!
//! **Time to first frame does not depend on how big the library is.** That is
//! the property the architecture exists for, and the plan is for `run::first_frame`
//! to be a separate function that is not given the channel at all, so the
//! property is a fact about a signature rather than a claim in a comment.
//!
//! # What is here and what is not
//!
//! Written: [`app`] (the `Model`, selection and scroll), [`layout`], [`view`],
//! [`run`], [`screen`], [`entry`], [`key`] and [`msg`]. There is no NVAPI module
//! here: the Proton verdict a game gets is `dxray_core::proton::Answer`, carried
//! on an [`Entry`](entry::Entry) and shown by [`view`].
//!
//! Two findings that shape this implementation, both confirmed by reading the
//! dependencies' own source:
//!
//! - `ratatui_bubbletea_components::SelectList` **does not scroll**. Its
//!   `Widget` impl renders `(0..area.height).zip(self.items.iter())`, always
//!   from item zero, so selecting the hundredth game in a thirty-row pane draws
//!   the first thirty and no cursor. The model has to keep the scroll offset and
//!   hand the component only the visible slice, with the selection index
//!   translated into that slice.
//! - `ratatui::restore()` disables raw mode and leaves the alternate screen and
//!   **never shows the cursor again**, while `Terminal::draw` hides it on every
//!   frame that sets no cursor position. Cursor visibility is a terminal-wide
//!   setting rather than a per-buffer one, so ratatui's own helper — and the
//!   panic hook it installs — can hand back a shell with an invisible cursor.
//!   `screen::restore` must do three things, each independent of the last
//!   succeeding: disable raw mode, leave the alternate screen, show the cursor.
//!
//! # Limits of the current validation
//!
//! Rendering tests construct representative [`Entry`](entry::Entry)s directly, so they show
//! that the layout is rendered correctly independently of discovery. The scan
//! has also been exercised against a mounted Distrobox Steam/Heroic home, but
//! not against every supported Steam, Proton, Heroic or game layout. Discovery
//! checks XDG, Flatpak and Snap paths plus a bounded conventional Distrobox
//! layout on external mounts; `DXRAY_STEAM_ROOT` and `DXRAY_HEROIC_CONFIG`
//! remain the explicit answer for a custom home. Cancellation is cooperative:
//! quitting can wait for a filesystem operation that the background scan has
//! already started.

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
