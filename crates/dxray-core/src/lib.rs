//! What a binary's dependencies say about the graphics APIs it can reach.
//!
//! The crate is split in two along a line that is deliberately hard to cross.
//! [`analysis`] is a pure function from observed names to a [`Verdict`]: it
//! never opens a file, never looks at a path, and can therefore be tested
//! exhaustively without a disk or a Windows install. [`evidence`] is the only
//! half that touches the filesystem, and it is kept as thin as it can be so
//! that almost every interesting decision lands on the testable side.
//!
//! [`game`] and [`install`] are the third pair on that line, and they answer a
//! different question: given a game's install directory, which of the several
//! executables in it is the game? [`game`] ranks and explains without opening
//! anything; [`install`] does the walking. The answer is an ordered list and
//! never a single pick — the launcher at the root and the shipping binary four
//! folders down are both correct answers to different questions.
//!
//! [`vdf`] and [`steam`] are split along the same line for the same reason:
//! [`vdf`] parses Valve's `KeyValues` text and never opens a file, [`steam`]
//! opens the files and hands them over. Steam discovery carries one extra
//! caveat that the PE half does not — **none of it has been run against a real
//! Steam install**, because there was none on the machine it was written on.
//! The layout it believes in is documented at each place it is relied on.
//!
//! [`launcher`] is the one module that is not half of a pair. It holds the
//! concept [`steam`] and [`heroic`] were both implementations of without ever
//! naming it — a game, and which launcher supplied it — and the trait both of
//! them implement. It exists because that concept previously lived inside a
//! consumer, where the other consumer could not reach it. The traversal over
//! every launcher lives there too, so the two consumers react to one walk
//! rather than each writing their own.
//!
//! [`inspect`](mod@inspect) is the other half of that split, and answers a different
//! question: given one game the walk found, what does this crate know about it?
//! It is separate from the walk because a caller may not want the per-game work
//! at all, and it is a function rather than two calls at each call site because
//! it *was* two calls at each call site and they had already drifted apart.
//!
//! Nothing here claims to know which renderer a process *ends up* using. That
//! is decided at run time, on hardware this tool cannot see, by code it refuses
//! to execute. What it reports is what the image is *able* to reach, and where
//! that ability was observed.

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
// Re-exported because [`FileVersion::Stamped`] hands one out, and a consumer
// that cannot name the type cannot match on it. Neither binary should have to
// take a dependency on the PE parser to read a number this crate gave it.
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
