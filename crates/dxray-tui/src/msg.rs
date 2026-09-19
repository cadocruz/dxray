//! Everything that can reach the model, from either side of it.
//!
//! One type for keystrokes and scan results together, because the loop has one
//! receiver. Two sources — a thread blocking in `event::read` and a thread
//! walking a launcher's libraries — are merged by having both send here, which
//! is what lets the main loop wait on a single
//! [`std::sync::mpsc::Receiver`] instead of polling two things and sleeping
//! between them.

use std::path::PathBuf;

use crate::entry::Entry;
use crate::key::Key;

/// A message for the model.
#[derive(Debug, Clone)]
pub enum Msg {
    Key(Key),
    /// The terminal's new size. Also sent once at startup, because the model
    /// needs to know how many rows the list has before the user resizes
    /// anything.
    Resize(u16, u16),
    /// A launcher installation was found and is about to be read.
    Root(PathBuf),
    /// A library inside one of those installations is about to be read.
    ///
    /// Sent for every library the launcher structurally recognises, whether or
    /// not it turns out to hold a game: an empty Steam library and an installed
    /// Heroic with nothing installed are the same situation and are counted the
    /// same way.
    Library(PathBuf),
    /// One game, fully examined.
    ///
    /// Boxed because it is by far the largest thing here and an enum is as big
    /// as its widest variant: unboxed, every keystroke would move several
    /// hundred bytes through the channel.
    Game(Box<Entry>),
    /// Something could not be read. The scan continues.
    Problem(String),
    /// Something was read and says less than it looks like it does.
    Note(String),
    /// The scan is over. Not the same as the channel closing: the scanning
    /// thread finishing is news the screen has to show, and the input thread
    /// keeps the channel open long after.
    Finished,
    /// Time passed. Drives the spinner and nothing else.
    Tick,
}
