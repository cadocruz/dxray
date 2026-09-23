//! The one concept Steam and Heroic share: a launcher that knows where games
//! are, and what may therefore be asked about them. A new launcher is a new
//! module and one line in [`all`].
//!
//! They differ in three ways, and the types keep each one: Heroic has no real
//! library level, so its configuration root is its single library; one
//! launcher's scan can fail wholesale, so [`Catalogue`] carries games beside
//! problems; and only a Steam [`Identity`] opens a Proton prefix, so a Heroic
//! game is told in words that the NVAPI question does not apply.

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::fmt;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

/// Which launcher, and which of its backends, supplied a game: `key` for
/// machines, never changing, and `label` for people.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Origin {
    key: &'static str,
    label: &'static str,
}

impl Origin {
    /// `key` identifies the launcher to a machine, `label` names it to a
    /// person.
    #[must_use]
    pub const fn new(key: &'static str, label: &'static str) -> Self {
        Self { key, label }
    }

    /// The stable machine-readable name, such as `"steam"` or `"heroic"`, shared
    /// by every backend of one launcher.
    #[must_use]
    pub const fn key(self) -> &'static str {
        self.key
    }

    /// The name to print, such as `"Steam"` or `"Heroic / GOG"`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        self.label
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.label)
    }
}

/// What a launcher calls a game, and so what can be asked about it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Identity {
    /// A Steam application id: the number in `appmanifest_<id>.acf`, and the
    /// key to the game's Proton prefix.
    SteamApp(u32),
    /// A launcher's own identifier, opaque here on purpose.
    Native(String),
}

impl Identity {
    /// The Steam application id, when there is one. `None` means the game is
    /// not a Steam application; zero is a real appid.
    #[must_use]
    pub fn steam_appid(&self) -> Option<u32> {
        match self {
            Self::SteamApp(appid) => Some(*appid),
            Self::Native(_) => None,
        }
    }
}

/// Through [`Formatter::pad`](fmt::Formatter::pad), so `{:<8}` lays out a
/// column.
impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SteamApp(appid) => f.pad(&appid.to_string()),
            Self::Native(id) => f.pad(id),
        }
    }
}

/// One installed game, whichever launcher found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    /// What the launcher calls it, and what may be asked about it.
    pub identity: Identity,
    /// The title as the launcher stores it, which is UTF-8 and often not ASCII.
    pub name: String,
    /// Absolute install directory. Existence follows launcher policy: Steam
    /// preserves its manifest; Heroic rejects stale cache records.
    pub install_dir: PathBuf,
    /// The launcher, and backend, this came from.
    pub origin: Origin,
}

/// One game in a completed launcher inventory. `library` is where Steam's
/// Proton prefix lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    /// The launcher installation that led to this game.
    pub root: Option<PathBuf>,
    pub game: Game,
    pub library: PathBuf,
}

/// One launcher installation visited by an inventory walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryRoot {
    pub origin: Origin,
    pub path: PathBuf,
}

/// One library visited under a launcher installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryLibrary {
    pub origin: Origin,
    /// `None` preserves an out-of-order [`Visitor`] call without inventing a root.
    pub root: Option<PathBuf>,
    pub path: PathBuf,
}

/// A non-game fact reported during an inventory walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryDiagnostic {
    pub origin: Origin,
    pub root: Option<PathBuf>,
    /// Root-index diagnostics have no library; catalogue diagnostics do.
    pub library: Option<PathBuf>,
    pub message: String,
}

/// A completed inventory, for callers and tests that want every fact of the
/// walk rather than a [`Visitor`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Inventory {
    pub roots: Vec<InventoryRoot>,
    pub libraries: Vec<InventoryLibrary>,
    pub notes: Vec<InventoryDiagnostic>,
    pub problems: Vec<InventoryDiagnostic>,
    pub entries: Vec<InventoryEntry>,
    current_root: Option<PathBuf>,
}

impl Inventory {
    /// Collects the complete shared launcher walk without cancellation.
    #[must_use]
    pub fn collect(launchers: &[&dyn Launcher]) -> Self {
        let mut inventory = Self::default();
        let _ = walk(launchers, &mut inventory);
        inventory
    }
}

/// The libraries inside one launcher root, with the notes and problems that
/// qualify the list; all three can be true at once.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Libraries {
    /// The directories to scan for games.
    pub paths: Vec<PathBuf>,
    /// Everything was read, and there may be less of it than the user expects.
    /// Must not move an exit code.
    pub notes: Vec<String>,
    /// Something could not be read. Should move an exit code.
    pub problems: Vec<String>,
}

/// What one library turned out to hold: games beside problems, never one
/// instead of the other.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Catalogue {
    pub games: Vec<Game>,
    /// One worded entry per record that could not be read, each naming its
    /// file.
    pub problems: Vec<String>,
    /// One worded entry per record that could not be used and cost nothing,
    /// saying why. Printed, but does not move an exit code.
    pub notes: Vec<String>,
}

/// A place games are installed from, that this tool can enumerate.
/// Object-safe, so one scan runs over every launcher compiled in.
pub trait Launcher: Sync {
    /// The launcher itself. A game's origin can name a backend too, such as
    /// `"Heroic / GOG"`.
    fn origin(&self) -> Origin;

    /// Every installation found where this code looks. Empty is not proof
    /// there is none.
    fn roots(&self) -> Vec<PathBuf>;

    /// Every path [`roots`](Launcher::roots) considers, existing or not, in
    /// order, so a caller that found nothing can say where it looked.
    fn candidate_roots(&self) -> Vec<PathBuf>;

    /// The libraries belonging to one root.
    fn libraries(&self, root: &Path) -> Libraries;

    /// The games in one library: everything the launcher declares, tools
    /// included. Telling games apart is left to the evidence
    /// ([`crate::Survey::has_evidence`]).
    fn games(&self, library: &Path) -> Catalogue;
}

/// Every launcher compiled into this build, in the order a scan visits them.
#[must_use]
pub fn all() -> &'static [&'static dyn Launcher] {
    /// The list itself, as a `const` so that it lives for the whole program
    /// rather than for the call that builds it.
    const ALL: &[&dyn Launcher] = &[&crate::steam::STEAM, &crate::heroic::HEROIC];
    ALL
}

/// What a caller does with each thing [`walk`] finds, as it is found: the
/// terminal UI streams it, the command line collects and prints.
///
/// Every method returns [`ControlFlow`] and [`walk`] stops at the first
/// [`Break`](ControlFlow::Break), so cancellation is the visitor's own
/// business. Each item comes with the [`Origin`] or [`Launcher`] it came from.
pub trait Visitor {
    /// One installation of a launcher, before anything inside it is read.
    fn root(&mut self, origin: Origin, root: &Path) -> ControlFlow<()>;

    /// Everything in this root was read, and there may be less of it than the
    /// user expects. Must not move an exit code.
    fn note(&mut self, origin: Origin, note: &str) -> ControlFlow<()>;

    /// Something in this root could not be read. Should move an exit code.
    fn problem(&mut self, origin: Origin, problem: &str) -> ControlFlow<()>;

    /// One library, announced before its games are read, so a list that fills
    /// as it goes can show the header first.
    fn library(&mut self, origin: Origin, library: &Path) -> ControlFlow<()>;

    /// What that library turned out to hold. Every entry must be presented;
    /// the order is the consumer's.
    fn catalogue(
        &mut self,
        launcher: &dyn Launcher,
        library: &Path,
        catalogue: Catalogue,
    ) -> ControlFlow<()>;
}

impl Visitor for Inventory {
    fn root(&mut self, origin: Origin, root: &Path) -> ControlFlow<()> {
        let root = root.to_path_buf();
        self.roots.push(InventoryRoot {
            origin,
            path: root.clone(),
        });
        self.current_root = Some(root);
        ControlFlow::Continue(())
    }

    fn note(&mut self, origin: Origin, note: &str) -> ControlFlow<()> {
        self.notes.push(InventoryDiagnostic {
            origin,
            root: self.current_root.clone(),
            library: None,
            message: note.to_owned(),
        });
        ControlFlow::Continue(())
    }

    fn problem(&mut self, origin: Origin, problem: &str) -> ControlFlow<()> {
        self.problems.push(InventoryDiagnostic {
            origin,
            root: self.current_root.clone(),
            library: None,
            message: problem.to_owned(),
        });
        ControlFlow::Continue(())
    }

    fn library(&mut self, origin: Origin, library: &Path) -> ControlFlow<()> {
        self.libraries.push(InventoryLibrary {
            origin,
            root: self.current_root.clone(),
            path: library.to_path_buf(),
        });
        ControlFlow::Continue(())
    }

    fn catalogue(
        &mut self,
        launcher: &dyn Launcher,
        library: &Path,
        catalogue: Catalogue,
    ) -> ControlFlow<()> {
        let root = self.current_root.clone();
        let library = library.to_path_buf();
        let origin = launcher.origin();
        self.notes.extend(
            catalogue
                .notes
                .into_iter()
                .map(|message| InventoryDiagnostic {
                    origin,
                    root: root.clone(),
                    library: Some(library.clone()),
                    message,
                }),
        );
        self.problems.extend(
            catalogue
                .problems
                .into_iter()
                .map(|message| InventoryDiagnostic {
                    origin,
                    root: root.clone(),
                    library: Some(library.clone()),
                    message,
                }),
        );
        self.entries
            .extend(catalogue.games.into_iter().map(|game| InventoryEntry {
                root: root.clone(),
                game,
                library: library.clone(),
            }));
        ControlFlow::Continue(())
    }
}

/// Visits every launcher in `launchers`, every installation of each, and every
/// library in each installation: the one inventory policy both surfaces use.
///
/// Libraries are deduplicated across the whole walk, since separate installs
/// and launchers can name the same one; roots are not. Nothing fails the walk:
/// failures arrive as worded items. [`Break`](ControlFlow::Break) means the
/// visitor asked to stop.
pub fn walk(launchers: &[&dyn Launcher], visitor: &mut dyn Visitor) -> ControlFlow<()> {
    let mut listed = HashSet::new();
    for launcher in launchers {
        let origin = launcher.origin();
        for root in launcher.roots() {
            visitor.root(origin, &root)?;
            let index = launcher.libraries(&root);
            // Notes and problems first, above the libraries they qualify.
            for note in &index.notes {
                visitor.note(origin, note)?;
            }
            for problem in &index.problems {
                visitor.problem(origin, problem)?;
            }
            for library in index.paths {
                if !listed.insert(identity(&library)) {
                    continue;
                }
                visitor.library(origin, &library)?;
                let catalogue = launcher.games(&library);
                visitor.catalogue(*launcher, &library, catalogue)?;
            }
        }
    }
    ControlFlow::Continue(())
}

/// What makes two library paths the same directory: the resolved path, or the
/// path as written when it cannot be resolved, so an unmounted drive stays in
/// the walk.
#[must_use]
pub fn identity(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
