//! The one concept Steam and Heroic share: a launcher that knows where games
//! are, and what may therefore be asked about them.
//!
//! Before this module the crate knew about two launchers and had no idea they
//! were the same kind of thing. The only place the shared concept existed was
//! `dxray_tui::entry::Source`, inside a consumer, which meant the CLI could not
//! import it and would have had to invent it a second time. That is the shape
//! of defect this project has already paid for twice — one rule with two
//! implementations, free to drift.
//!
//! Adding GOG Galaxy or Lutris is one new module and one line in [`all`].
//! Compiled in, never dynamically loaded: the workspace forbids `unsafe`, Rust
//! has no stable ABI, and a plugin boundary would trade type checking for a
//! whole subsystem this project does not need for two launchers.
//!
//! # The three places the launchers genuinely differ
//!
//! **A library level Heroic does not obviously have.** Steam is root, then
//! libraries, then games; Heroic is root, then games. The level is kept rather
//! than flattened, because it is load-bearing on the Steam side — a Proton
//! compatibility prefix lives at `<library>/steamapps/compatdata/<appid>`, so a
//! game with no library beside it cannot be asked the Proton question at all.
//! Heroic returns its configuration root as its single library, which is not a
//! ceremonial answer: it is the directory the record was read out of, and it is
//! already what the browser prints and counts for a Heroic machine.
//!
//! **One `games` returns a `Result` and the other cannot fail wholesale.**
//! Neither is lying, and the difference is real one layer down: Steam has a
//! single mandatory input per library (`steamapps` must be listable) while
//! Heroic reads up to six optional cache files and a missing one is an ordinary
//! empty cache. But no *caller* has ever used the distinction: both consumers
//! push the wholesale error onto the same list of worded problems that the
//! per-file failures go on, and carry on. So the trait returns a
//! [`Catalogue`] — games beside problems, never one or the other — and a
//! wholesale failure arrives as a problem with an empty game list. The typed
//! `Result` stays on [`steam::games`](crate::steam::games) for anyone who wants
//! it.
//!
//! **Identity is a number for Steam and a string for Heroic, and that decides
//! which questions have answers.** This is the difference that must survive,
//! and [`Identity`] is the type that makes it survive. A Steam game carries an
//! application id, which is the key to a `compatdata` prefix and therefore to a
//! real Proton NVAPI verdict. A Heroic game carries an opaque launcher id and
//! gets told, in words, that the question does not apply to it. The enum is
//! what stops that from being a convention: there is no way to build an
//! [`Identity::SteamApp`] out of a Heroic record, because the adapter has only
//! a `String` and this crate offers no conversion — and equally no way for the
//! Steam adapter to quietly lose the number, because [`Identity::Native`] is
//! not what [`crate::proton::Builds::answer_for`] matches on.
//!
//! The capability is asked for and *declined*, rather than being impossible to
//! phrase. That is deliberate. A list that shows Steam and Heroic games side by
//! side needs a sentence in the NVAPI row of every one of them, and "not
//! applicable: this launcher's games have no Steam `AppID`" is a better answer
//! than a blank — it is the same refusal to render an absence as emptiness that
//! the rest of this crate is built on.
//!
//! # The place they turned out not to differ
//!
//! For one round this trait had a fourth difference written against it: Steam
//! announced every library it found while Heroic announced a configuration root
//! only if scanning it had produced a game, so a single scan loop could not
//! serve both without smuggling a presentation flag into a discovery trait.
//! That was never a difference between the launchers. It was
//! [`heroic::roots`](crate::heroic::roots) answering "does this directory
//! exist" when the question is "is Heroic installed here", and a guard in the
//! terminal UI compensating one layer too late — which also meant a Heroic
//! install with nothing installed vanished from the counts while an empty Steam
//! library was listed. `roots` says what it means now, the guard is gone, and
//! the scan is one loop over [`all`].

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::fmt;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

/// Which launcher, and which of its backends, supplied a game.
///
/// Two strings and not an enum, so that a new launcher is a new module rather
/// than a new arm in a match that every consumer has to be recompiled against.
/// The `key` is for machines — dedup sets, and one day a command-line flag —
/// and never changes; the `label` is for people and may be reworded.
///
/// Both are `&'static str` because every origin this project has is known when
/// it is compiled: Steam has one, Heroic has one per backend. A launcher whose
/// backends are only discoverable at run time would need a `Cow` here, and
/// there is no such launcher to design against yet.
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

    /// The stable machine-readable name, such as `"steam"` or `"heroic"`.
    ///
    /// Shared by every backend of one launcher: Heroic's Epic, GOG and Amazon
    /// origins all answer `"heroic"`, because a dedup key wants to know which
    /// scanner found a directory, not which shop sold it.
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

/// What a launcher calls a game, and — through which variant it is — what can
/// therefore be asked about that game.
///
/// Not a string with a flag beside it. The whole point of the enum is that the
/// number and the capability travel together and cannot be separated by
/// anybody's carelessness downstream.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Identity {
    /// A Steam application id: the number in `appmanifest_<id>.acf`.
    ///
    /// The one identifier in this project that opens a door. It names a
    /// compatibility prefix under `steamapps/compatdata`, which names the
    /// Proton build the game last ran under, which is the only way to get a
    /// truthful NVAPI answer — the policy changed direction twice across
    /// releases, so "what does Proton do" is never answerable without knowing
    /// *which* Proton.
    SteamApp(u32),
    /// A launcher's own identifier, opaque here on purpose.
    ///
    /// Heroic's Epic, GOG and Amazon ids are all this. Nothing in this crate
    /// reads inside the string, because nothing in this crate knows what any of
    /// those id schemes mean, and a parser that guessed would be inventing
    /// facts about somebody's library.
    Native(String),
}

impl Identity {
    /// The Steam application id, when there is one.
    ///
    /// `None` is not a missing value to be papered over with a default. It is
    /// the answer "this game is not a Steam application", and every caller that
    /// matches on it is deciding what to say about a question that has no
    /// answer rather than picking a placeholder. The placeholder version of
    /// this — a `u32` that is zero for Heroic — is what the browser used to
    /// carry, and zero is a real appid.
    #[must_use]
    pub fn steam_appid(&self) -> Option<u32> {
        match self {
            Self::SteamApp(appid) => Some(*appid),
            Self::Native(_) => None,
        }
    }
}

/// Through [`Formatter::pad`](fmt::Formatter::pad) rather than `write!`, so
/// that `{:<8}` lays out a column. A `Display` that ignores the width silently
/// unaligns every listing that asks for one, and nothing warns about it.
impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SteamApp(appid) => f.pad(&appid.to_string()),
            Self::Native(id) => f.pad(id),
        }
    }
}

/// One installed game, whichever launcher found it.
///
/// Replaces the two structs that used to say this separately. They agreed on
/// three fields out of four and disagreed on the fourth in the one way that
/// mattered, which is exactly the situation in which two types quietly become
/// two behaviours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    /// What the launcher calls it, and what may be asked about it.
    pub identity: Identity,
    /// The title as the launcher stores it, which is UTF-8 and often not ASCII.
    pub name: String,
    /// The directory the game is installed in, resolved to an absolute path.
    ///
    /// **Whether it exists is launcher-defined and deliberately so.** Steam
    /// hands back the path its manifest declares without checking, because a
    /// game mid-download names a directory that is not there yet and that is a
    /// fact worth reporting rather than a reason to drop the entry. Heroic
    /// checks, because its cache outlives the games it describes and a record
    /// pointing at a deleted directory is stale metadata, not an install. Both
    /// are right about their own file format; nothing here should flatten them
    /// into one rule.
    pub install_dir: PathBuf,
    /// The launcher, and backend, this came from.
    pub origin: Origin,
}

/// One game in a completed launcher inventory.
///
/// This is the presentation-neutral result of a full [`walk`].  The game keeps
/// the identity and origin supplied by its launcher; `library` records the
/// location that makes Steam's Proton policy meaningful and lets consumers
/// present the same inventory without reconstructing that relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryEntry {
    pub game: Game,
    pub library: PathBuf,
}

/// A completed, presentation-neutral inventory.
///
/// Command-line and terminal consumers normally implement [`Visitor`]
/// directly: the command line renders as it accumulates and the TUI streams
/// events while discovery is still in progress.  This collector exists for
/// callers and tests that need the stable facts produced by the shared walk,
/// rather than either presentation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Inventory {
    pub entries: Vec<InventoryEntry>,
}

impl Inventory {
    /// Collect every game delivered by the shared launcher walk.
    ///
    /// Unlike a UI visitor this collector never cancels, so a returned value
    /// always represents the complete traversal of the launchers supplied.
    #[must_use]
    pub fn collect(launchers: &[&dyn Launcher]) -> Self {
        let mut inventory = Self::default();
        let _ = walk(launchers, &mut inventory);
        inventory
    }
}

/// The libraries inside one launcher root, and everything qualifying that list.
///
/// Three lists rather than a `Result`, because all three can be true at once: a
/// Steam root can hand back four libraries, a note saying its index declared
/// none, and a problem from a fifth that could not be read. A `Result` would
/// have to throw two of those away.
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

/// What one library turned out to hold.
///
/// Games beside problems, never one instead of the other. A `Vec<Result<Game>>`
/// makes `.filter_map(Result::ok)` the shortest thing to write, and that one
/// call silently drops every corrupt record — the failure this whole crate
/// exists to refuse.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Catalogue {
    pub games: Vec<Game>,
    /// One worded entry per record that could not be read, each naming its
    /// file.
    ///
    /// Already worded, and not a shared error enum, because the two launchers'
    /// error types have nothing in common beyond naming a file and rendering to
    /// a sentence — and both consumers call `to_string` on them in the next
    /// line anyway. A third enum wrapping the other two would be one more thing
    /// to keep in step by hand, which is the cost this module was written to
    /// stop paying.
    pub problems: Vec<String>,
    /// One worded entry per record that could not be used and cost nothing,
    /// each already saying why it cost nothing.
    ///
    /// The same division [`Libraries`] draws one level up, for the same reason:
    /// a stale cache entry for a game another source supplied is a fact about
    /// the launcher's bookkeeping, not a game this tool failed to read, and it
    /// must not move an exit code. It is still printed — an unusable record
    /// that disappears is indistinguishable from one that never existed.
    pub notes: Vec<String>,
}

/// A place games are installed from, that this tool can enumerate.
///
/// Object-safe on purpose: [`all`] hands back trait objects so a scan can be
/// written once and run over every launcher compiled in.
pub trait Launcher: Sync {
    /// The launcher itself, for prefixing its messages and naming it in a UI.
    ///
    /// A *game's* origin can be more specific than this — Heroic answers
    /// `"Heroic"` here while its games say `"Heroic / GOG"` — because the
    /// backend is a fact about a record and not about the scanner.
    fn origin(&self) -> Origin;

    /// Every installation of this launcher found on this machine.
    ///
    /// Empty means none was found *at a path this code knows to look at*, which
    /// is not the same as none existing.
    fn roots(&self) -> Vec<PathBuf>;

    /// Every path [`roots`](Launcher::roots) considers on this machine,
    /// existing or not, in the order it considers them.
    ///
    /// What a caller that found nothing prints. "No launcher found" on its own
    /// is unactionable: the install may well be somewhere real that this tool
    /// does not know to check, and only the list of candidates makes that
    /// visible.
    ///
    /// Required rather than defaulted to an empty list. A launcher that opted
    /// out would vanish silently from that message, which is the same defect
    /// one layer up: the answer would be short and nothing would say so.
    fn candidate_roots(&self) -> Vec<PathBuf>;

    /// The libraries belonging to one root.
    fn libraries(&self, root: &Path) -> Libraries;

    /// The games in one library.
    ///
    /// Everything the launcher declares installed, including its own tooling.
    /// There is deliberately no companion method asking whether an entry is a
    /// game: Steam's `DownloadType` was tried for that and marks shipped games
    /// as tools, so a launcher has no trustworthy answer to give. The question
    /// is answered one layer up, from the evidence an install actually carries
    /// ([`crate::Survey::has_evidence`]), and never by filtering here.
    fn games(&self, library: &Path) -> Catalogue;
}

/// Every launcher compiled into this build, in the order a scan visits them.
///
/// The registry. A new launcher is a new module implementing [`Launcher`] and
/// one more entry here — that is the whole extension point, and it is the
/// reason this is a trait rather than an enum with two arms.
#[must_use]
pub fn all() -> &'static [&'static dyn Launcher] {
    /// The list itself, as a `const` so that it lives for the whole program
    /// rather than for the call that builds it.
    const ALL: &[&dyn Launcher] = &[&crate::steam::STEAM, &crate::heroic::HEROIC];
    ALL
}

/// What a caller does with each thing [`walk`] finds, as it is found.
///
/// The traversal is one definition and the reactions to it are two. The
/// terminal UI's visitor drops each item onto a channel so the list fills while
/// the scan is still running; the command line's accumulates rows and prints
/// when the walk returns. Those are different presentations of one walk, and
/// before this trait they were two walks that had to be kept in step by hand —
/// including one copy of the library dedup rule each, which is the shape of
/// defect this module was written to stop paying for.
///
/// # Cancellation is a visitor's business, not a parameter
///
/// Every method returns [`ControlFlow`], and [`walk`] stops at the first
/// [`Break`](ControlFlow::Break). A visitor that can be cancelled polls its own
/// flag and breaks; a visitor that cannot returns
/// [`Continue`](ControlFlow::Continue) and never thinks about it. That is why
/// there is no cancellation argument here: a caller with nothing to cancel
/// writes nothing, rather than passing something that is always false.
///
/// The polling granularity is the same one the terminal UI earned — between
/// filesystem operations — because that is where these methods are called
/// from.
///
/// # The launcher names itself
///
/// Every method is handed the [`Origin`] or the [`Launcher`] the item came
/// from, and no visitor ever asks *which* launcher it is looking at. A message
/// prefixed from `origin` is right for a launcher that does not exist yet; a
/// message prefixed from a branch is right until somebody adds one.
pub trait Visitor {
    /// One installation of a launcher, before anything inside it is read.
    fn root(&mut self, origin: Origin, root: &Path) -> ControlFlow<()>;

    /// Everything in this root was read, and there may be less of it than the
    /// user expects. Must not move an exit code.
    fn note(&mut self, origin: Origin, note: &str) -> ControlFlow<()>;

    /// Something in this root could not be read. Should move an exit code.
    fn problem(&mut self, origin: Origin, problem: &str) -> ControlFlow<()>;

    /// One library, before its games are read.
    ///
    /// Announced first and catalogued second, in two calls rather than one,
    /// because reading a library can take as long as the disk it is on takes.
    /// A browser that fills as it goes has to be able to show the header while
    /// the read is still running, and folding the two together would have made
    /// every library appear only once it was finished — the progressive fill
    /// quietly becoming a series of stalls.
    fn library(&mut self, origin: Origin, library: &Path) -> ControlFlow<()>;

    /// What that library turned out to hold.
    ///
    /// Separate from [`library`](Visitor::library) for the reason above, and
    /// handed over whole rather than item by item because the two consumers
    /// order its contents differently: the listing names the games first and
    /// the unreadable records under them, the browser reports a problem the
    /// moment it has one. That ordering is presentation. What is *not*
    /// presentation is which libraries get read at all, and that stays in
    /// [`walk`].
    ///
    /// Both consumers must present every entry in this catalogue. The
    /// `launcher` supplies its origin, not a consumer-specific filter.
    fn catalogue(
        &mut self,
        launcher: &dyn Launcher,
        library: &Path,
        catalogue: Catalogue,
    ) -> ControlFlow<()>;
}

impl Visitor for Inventory {
    fn root(&mut self, _origin: Origin, _root: &Path) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn note(&mut self, _origin: Origin, _note: &str) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn problem(&mut self, _origin: Origin, _problem: &str) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn library(&mut self, _origin: Origin, _library: &Path) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn catalogue(
        &mut self,
        _launcher: &dyn Launcher,
        library: &Path,
        catalogue: Catalogue,
    ) -> ControlFlow<()> {
        self.entries
            .extend(catalogue.games.into_iter().map(|game| InventoryEntry {
                game,
                library: library.to_path_buf(),
            }));
        ControlFlow::Continue(())
    }
}

/// Visits every launcher in `launchers`, every installation of each, and every
/// library in each installation.
///
/// This is the shared inventory policy for CLI and TUI: deliver every entry
/// returned by each launcher's catalogue, with library deduplication below.
/// Consumers must not apply additional classification or deduplication.
/// Tools remain visible because the available Steam manifest metadata does
/// not reliably distinguish them from games.
///
/// [`Break`](ControlFlow::Break) means the visitor asked to stop and the rest
/// of the machine was not looked at. Nothing else in the walk can fail: a root
/// that cannot be indexed and a library that cannot be listed both arrive as
/// worded items, because one broken launcher must not hide the other's games.
///
/// Takes the launchers rather than calling [`all`] itself. That is what lets
/// one listing answer "everything installed" and a narrower one answer "what
/// Steam has" without a second implementation of either — and what lets a test
/// hand it a launcher whose libraries hold no games, which no real machine can
/// be relied upon to have.
///
/// # Libraries are deduplicated across the whole walk
///
/// [`steam::libraries`](crate::steam::libraries) removes the duplicates one
/// Steam root declares. Separate native and Flatpak installs can still name the
/// same library, and so can two launchers. Noticing that is this function's
/// job, and doing it here is why the counts and the games agree in both
/// consumers — each of them used to carry its own copy of this rule.
///
/// Roots are *not* deduplicated: two installs that share a library are still
/// two installs, and their notes and failures belong to each.
pub fn walk(launchers: &[&dyn Launcher], visitor: &mut dyn Visitor) -> ControlFlow<()> {
    let mut listed = HashSet::new();
    for launcher in launchers {
        let origin = launcher.origin();
        for root in launcher.roots() {
            visitor.root(origin, &root)?;
            let index = launcher.libraries(&root);
            // Notes before problems, and both before the libraries they
            // qualify: a caveat that explains why there is only one library
            // has to be readable above it rather than after it.
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

/// What makes two library paths the same directory: the resolved path when it
/// resolves, and the path as written when it does not.
///
/// The fallback is the load-bearing half. A library on a drive that is not
/// mounted cannot be canonicalised, and erasing it from the scan would turn
/// "your D: drive is not plugged in" into "you own fewer games than you do".
/// Keeping the literal path keeps it in the walk, where it fails loudly with
/// its own name attached.
#[must_use]
pub fn identity(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
