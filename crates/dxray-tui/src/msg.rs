//! Messages received by the terminal model from input and discovery workers.

use std::path::PathBuf;

use crate::entry::Entry;
use crate::key::Key;
use dxray_core::Origin;

/// A root or library discovered while walking a launcher inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanLocation {
    pub origin: Origin,
    pub path: PathBuf,
}

/// One diagnostic emitted while walking a launcher inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanDiagnostic {
    pub origin: Origin,
    pub root: Option<PathBuf>,
    pub library: Option<PathBuf>,
    pub message: String,
}

impl ScanDiagnostic {
    #[cfg(test)]
    pub(crate) fn testing(message: impl Into<String>) -> Self {
        Self {
            origin: Origin::new("test", ""),
            root: None,
            library: None,
            message: message.into(),
        }
    }
}

/// A message for the model.
#[derive(Debug, Clone)]
pub enum Msg {
    Key(Key),
    /// Terminal size, also sent at startup.
    Resize(u16, u16),
    /// A launcher installation was found and is about to be read.
    Root(ScanLocation),
    /// A discovered library is about to be read, including empty ones.
    Library(ScanLocation),
    /// One fully examined game. Boxed to keep the message enum small.
    Game(Box<Entry>),
    /// Something could not be read. The scan continues.
    Problem(ScanDiagnostic),
    /// Something was read and says less than it looks like it does.
    Note(ScanDiagnostic),
    /// Discovery completed; input may still keep the channel open.
    Finished,
    /// Time passed. Drives the spinner and nothing else.
    Tick,
}

#[cfg(test)]
impl Msg {
    pub(crate) fn test_problem(message: impl Into<String>) -> Self {
        Self::Problem(ScanDiagnostic::testing(message))
    }

    pub(crate) fn test_note(message: impl Into<String>) -> Self {
        Self::Note(ScanDiagnostic::testing(message))
    }
}
