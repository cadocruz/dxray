//! What a binary's dependencies say about the graphics APIs it can reach.
//!
//! Pure modules decide and never open a file; their IO halves only read and
//! hand over: [`analysis`] and [`evidence`], [`game`] and [`install`], [`vdf`]
//! and [`steam`], [`nvapi`] and [`proton`]. [`launcher`] names a game and the
//! launcher that supplied it, [`steam`] or [`heroic`], and walks them all;
//! [`inspect`](mod@inspect) establishes what is known about one game found.
//!
//! Nothing here claims which renderer a process ends up using: only what the
//! image is able to reach, and where that was observed.

pub mod analysis;
pub mod evidence;
pub mod game;
pub mod heroic;
pub mod inspect;
pub mod install;
pub mod launcher;
pub mod nvapi;
pub mod proton;
pub mod steam;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod vdf;

mod paths;

pub use analysis::{Evidence, FileVersion, Finding, Linked, Signal, Source, Verdict, analyse};
// Re-exported so consumers can match on a version without `dxray-pe`.
pub use dxray_pe::{Version, VersionInfo};
pub use evidence::LibraryCache;
pub use game::{Candidate, Reason, Survey};
pub use inspect::{Inspection, inspect};
pub use install::candidates;
pub use launcher::{
    Catalogue, Game, Identity, Inventory, InventoryDiagnostic, InventoryEntry, InventoryLibrary,
    InventoryRoot, Launcher, Libraries, Origin, Visitor, walk,
};

#[cfg(test)]
mod testutil;
