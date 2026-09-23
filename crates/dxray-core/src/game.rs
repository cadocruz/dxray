//! Which executable in an install directory is the game? Rank, and explain.
//!
//! Pure: nothing here opens a file. [`install`](crate::install) walks the
//! directory and hands the facts over as [`Observed`].
//!
//! - **Rank and explain; never pick.** The result is an ordered list where
//!   every entry carries the reasons it scored as it did.
//! - **No exclusion lists.** A redistributable sinks because it carries no
//!   evidence, not because its name is on a list.
//! - **Evidence is per directory.** [`Observed::directory`] describes the
//!   candidate's own directory only, never the install root.
//! - **Say when the strongest signal is missing.** Java, Electron and .NET
//!   games import no graphics API, so [`Note::NoRendererImported`] travels
//!   with such a ranking.

use std::io;
use std::path::{Path, PathBuf};

use crate::analysis::{Source, Verdict};

/// The weight of each reason. The numbers are an ordering, not measurements.
///
/// Every weight is positive, so a candidate scores zero exactly when it has no
/// reasons, and [`Candidate::score`] and [`Candidate::has_evidence`] cannot
/// disagree. Something worth saying but not scoring belongs in [`Note`].
pub mod weight {
    /// The image's own import table names a graphics API. The process cannot
    /// start without it.
    pub const RENDERER_IMPORT: i32 = 100;

    /// A subdirectory named `<stem>_Data` sits beside `<stem>.exe`. As strong
    /// as an import: a Unity executable is a stub that imports no graphics API.
    pub const UNITY_DATA: i32 = 100;

    /// The image delay-loads a graphics API: real, but the call may never
    /// happen.
    pub const RENDERER_DELAY_IMPORT: i32 = 80;

    /// A library beside the image, which the image imports, reaches a graphics
    /// API: one link weaker than reaching it directly.
    pub const RENDERER_INDIRECT: i32 = 70;

    /// The image sits in `Binaries/Win64` or `Binaries/Win32`.
    pub const UNREAL_BINARIES: i32 = 55;

    /// The stem ends `-Shipping`, Unreal's release build. With
    /// [`UNREAL_BINARIES`] it outweighs a lone import, which an embedded
    /// browser launcher also has.
    pub const SHIPPING_SUFFIX: i32 = 50;

    /// The stem and the game's name are the same word. Below every structural
    /// reason: names often differ (`TslGame.exe` is PUBG), so a name must not
    /// outvote the layout.
    pub const NAME_EXACT: i32 = 30;

    /// One of the stem and the game's name contains the other.
    pub const NAME_PARTIAL: i32 = 15;

    /// The image's own directory ships graphics libraries. Lowest: it says the
    /// same about every executable in that directory.
    pub const SHIPS_GRAPHICS_LIBRARIES: i32 = 15;
}

/// How closely a stem and a name match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resemblance {
    /// The same word once punctuation and case are dropped.
    Exact,
    /// One contains the other.
    Partial,
}

/// Where the name a stem was compared against came from. A supplied name is
/// independent evidence; the directory's own name is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameFrom {
    /// Supplied by the caller — from a Steam manifest, or typed.
    Supplied,
    /// The name of the directory that was surveyed.
    Directory,
}

/// One observed fact that argues this executable is the game. There are no
/// negative reasons and no "looks like a launcher" reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The image's own import or delay-import table names a graphics API.
    /// Never built from a neighbouring file.
    LinksRenderer {
        /// Every API reached, in the order [`analyse`](crate::analyse) reports.
        apis: Vec<String>,
        /// The strongest way any of them was reached.
        source: Source,
    },
    /// A library beside the image, which the image imports, reaches a graphics
    /// API, as a Unity stub does through `UnityPlayer.dll`. Followed one link,
    /// and only raised when the image reaches no renderer itself.
    LinksRendererThrough {
        /// The local libraries that reached one, sorted.
        libraries: Vec<String>,
        /// Every API they reach between them.
        apis: Vec<String>,
    },
    /// A subdirectory named after the executable, in Unity's `<stem>_Data`
    /// shape, sits beside it.
    UnityDataDirectory {
        /// The directory as it was spelled on disk.
        directory: String,
    },
    /// The executable sits in `Binaries/Win64` or `Binaries/Win32`, but not
    /// under `Engine`, which holds the engine's own tools.
    UnrealBinariesDirectory {
        /// The two directory names as they were spelled, joined by a slash.
        directory: String,
    },
    /// The stem ends `-Shipping`.
    ShippingSuffix,
    /// The stem resembles the game's name.
    NameResembles {
        /// The name it was compared against.
        name: String,
        how: Resemblance,
        from: NameFrom,
    },
    /// The executable's own directory ships graphics libraries.
    ShipsGraphicsLibraries {
        /// The library names, as they were spelled in the directory.
        libraries: Vec<String>,
    },
}

impl Reason {
    /// The stable spelling used in machine-readable output; the `Display`
    /// sentence is for people and may be reworded.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::LinksRenderer { .. } => "links-renderer",
            Self::LinksRendererThrough { .. } => "links-renderer-through",
            Self::UnityDataDirectory { .. } => "unity-data-directory",
            Self::UnrealBinariesDirectory { .. } => "unreal-binaries-directory",
            Self::ShippingSuffix => "shipping-suffix",
            Self::NameResembles { .. } => "name-resembles",
            Self::ShipsGraphicsLibraries { .. } => "ships-graphics-libraries",
        }
    }

    /// What this reason contributes to the score. See [`weight`].
    #[must_use]
    pub fn weight(&self) -> i32 {
        match self {
            // `assess` never builds this with `Source::Neighbour`; a hand-built
            // one gets no credit.
            Self::LinksRenderer { source, .. } => match source {
                Source::Import => weight::RENDERER_IMPORT,
                Source::DelayImport => weight::RENDERER_DELAY_IMPORT,
                // Not built today; weighted like `LinksRendererThrough` so the
                // two cannot drift.
                Source::Linked => weight::RENDERER_INDIRECT,
                Source::Neighbour => 0,
            },
            Self::LinksRendererThrough { .. } => weight::RENDERER_INDIRECT,
            Self::UnityDataDirectory { .. } => weight::UNITY_DATA,
            Self::UnrealBinariesDirectory { .. } => weight::UNREAL_BINARIES,
            Self::ShippingSuffix => weight::SHIPPING_SUFFIX,
            Self::NameResembles { how, .. } => match how {
                Resemblance::Exact => weight::NAME_EXACT,
                Resemblance::Partial => weight::NAME_PARTIAL,
            },
            Self::ShipsGraphicsLibraries { .. } => weight::SHIPS_GRAPHICS_LIBRARIES,
        }
    }
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LinksRenderer { apis, source } => {
                write!(f, "links {} ({})", join_or(apis), source.as_str())
            }
            Self::LinksRendererThrough { libraries, apis } => write!(
                f,
                "links {} through {}, in its own directory",
                join_or(apis),
                libraries.join(", ")
            ),
            Self::UnityDataDirectory { directory } => write!(
                f,
                "has a matching \"{directory}\" beside it, which is Unity's layout"
            ),
            Self::UnrealBinariesDirectory { directory } => write!(
                f,
                "sits in {directory}, which is how Unreal packages a game"
            ),
            Self::ShippingSuffix => {
                write!(
                    f,
                    "its name ends \"-Shipping\", Unreal's name for a release build"
                )
            }
            Self::NameResembles { name, how, from } => {
                let verb = match how {
                    Resemblance::Exact => "is",
                    Resemblance::Partial => "resembles",
                };
                let source = match from {
                    NameFrom::Supplied => "the game's name",
                    NameFrom::Directory => "the directory's name",
                };
                write!(f, "its name {verb} {source}, \"{name}\"")
            }
            Self::ShipsGraphicsLibraries { libraries } => {
                write!(f, "its own directory ships {}", libraries.join(", "))
            }
        }
    }
}

/// `"a"`, `"a or b"`, `"a, b or c"`.
fn join_or(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

/// One executable, and why it ranked where it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The executable, spelled as it will be reported.
    pub path: PathBuf,
    /// Every reason found, strongest first. Empty means nothing was observed.
    pub reasons: Vec<Reason>,
}

impl Candidate {
    /// The sum of the reasons' weights, computed so it always matches them.
    #[must_use]
    pub fn score(&self) -> i32 {
        self.reasons.iter().map(Reason::weight).sum()
    }

    /// True when something was observed about this executable.
    #[must_use]
    pub fn has_evidence(&self) -> bool {
        !self.reasons.is_empty()
    }

    /// True when an import table, this image's own or a library's beside it,
    /// named a graphics API.
    #[must_use]
    pub fn reaches_renderer(&self) -> bool {
        self.reasons.iter().any(|reason| {
            matches!(
                reason,
                Reason::LinksRenderer { .. } | Reason::LinksRendererThrough { .. }
            )
        })
    }
}

/// Something true about a survey that makes it worth less than it looks. Every
/// limit the walk hit is carried back with the answer.
#[derive(Debug)]
pub enum Note {
    /// Directories below [`MAX_DEPTH`](crate::install::MAX_DEPTH) were not
    /// descended into.
    DepthLimited {
        limit: usize,
        /// How many directories were left unvisited.
        skipped: usize,
    },
    /// The walk stopped after [`MAX_DIRECTORIES`](crate::install::MAX_DIRECTORIES).
    DirectoryLimited { limit: usize },
    /// The walk stopped after [`MAX_EXECUTABLES`](crate::install::MAX_EXECUTABLES).
    ExecutableLimited { limit: usize },
    /// A directory below the root could not be listed. Not fatal: the rest of
    /// the tree is still ranked.
    Unreadable { path: PathBuf, source: io::Error },
    /// An executable could not be read or parsed. It stays in the list, ranked
    /// on its path alone.
    Unparsed { path: PathBuf, source: io::Error },
    /// No executable reaches a graphics API through its import tables, so the
    /// ranking rests on directory structure alone. Normal for Java, Electron
    /// and .NET games, which load their renderer at run time.
    NoRendererImported {
        /// How many executables were read and found to import none.
        executables: usize,
    },
    /// Two or more executables share the highest score above zero, so the
    /// order among them is presentation, not evidence. Raised without guessing
    /// what caused the tie.
    TiedAtTheTop {
        /// How many candidates share the top score. Always at least two.
        count: usize,
        /// The score they share.
        score: i32,
    },
}

impl Note {
    /// True when something was not read, rather than read and found to say
    /// little. [`Survey::is_incomplete`] applies it for every surface.
    #[must_use]
    pub fn is_incomplete(&self) -> bool {
        match self {
            Self::DepthLimited { .. }
            | Self::DirectoryLimited { .. }
            | Self::ExecutableLimited { .. }
            | Self::Unreadable { .. }
            | Self::Unparsed { .. } => true,
            Self::NoRendererImported { .. } | Self::TiedAtTheTop { .. } => false,
        }
    }
}

impl std::fmt::Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DepthLimited { limit, skipped } => write!(
                f,
                "stopped at {limit} directories deep; {skipped} {} not searched, so a \
                 binary below that is missing from this list entirely",
                if *skipped == 1 {
                    "directory was"
                } else {
                    "directories were"
                }
            ),
            Self::DirectoryLimited { limit } => write!(
                f,
                "stopped after {limit} directories; the rest of the tree was not \
                 searched, so a binary in it is missing from this list entirely"
            ),
            Self::ExecutableLimited { limit } => write!(
                f,
                "stopped after {limit} executables; any found later was not ranked, \
                 so the best one may not be in this list at all"
            ),
            Self::Unreadable { path, source } => {
                write!(f, "{}: not searched: {source}", path.display())
            }
            Self::Unparsed { path, source } => {
                write!(f, "{}: ranked on its path alone: {source}", path.display())
            }
            Self::NoRendererImported { executables } => write!(
                f,
                "{}, so this ranking rests on directory structure alone and is weaker \
                 than one backed by an import table. That is also a finding: a Java, \
                 Electron or .NET game loads its renderer at run time, where no import \
                 table can show it.",
                if *executables == 1 {
                    "the only executable here reaches no graphics API through its import \
                     tables"
                        .to_owned()
                } else {
                    format!(
                        "none of the {executables} executables here reaches a graphics API \
                         through its import tables"
                    )
                }
            ),
            Self::TiedAtTheTop { count, score } => write!(
                f,
                "{count} executables share the top score of {score}, so the evidence does \
                 not choose between them. Whichever is listed first was put there by path \
                 depth and then alphabetical order, which is presentation and not a \
                 finding — read all {count} before believing any of them."
            ),
        }
    }
}

/// The ranked executables in one directory, and what qualifies the ranking.
#[derive(Debug, Default)]
pub struct Survey {
    /// Best first. Ties broken by path, so two runs over one install produce
    /// the same list in the same order and can be diffed against each other.
    pub candidates: Vec<Candidate>,
    pub notes: Vec<Note>,
}

impl Survey {
    /// Sorts `candidates` and raises the notes that apply, so no caller can
    /// forget either.
    #[must_use]
    pub fn ranked(mut candidates: Vec<Candidate>, mut notes: Vec<Note>) -> Self {
        // Descending score, then shallower path, then alphabetical. Depth only
        // orders what the evidence could not separate; it is not a signal.
        candidates.sort_by(|a, b| {
            b.score()
                .cmp(&a.score())
                .then_with(|| depth(a).cmp(&depth(b)))
                .then_with(|| a.path.cmp(&b.path))
        });
        if !candidates.is_empty() && !candidates.iter().any(Candidate::reaches_renderer) {
            notes.push(Note::NoRendererImported {
                executables: candidates.len(),
            });
        }
        if let Some(tie) = tie_at_the_top(&candidates) {
            notes.push(tie);
        }
        Self { candidates, notes }
    }

    /// The highest-ranked executable, even when it carries no evidence.
    #[must_use]
    pub fn best(&self) -> Option<&Candidate> {
        self.candidates.first()
    }

    /// True when at least one executable had something to say for itself.
    /// False for a directory of redistributables, which is not an empty one.
    #[must_use]
    pub fn has_evidence(&self) -> bool {
        self.candidates.iter().any(Candidate::has_evidence)
    }

    /// The notes saying part of this directory was never looked at: exactly
    /// [`Note::is_incomplete`], applied in one place.
    pub fn incomplete_notes(&self) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(|note| note.is_incomplete())
    }

    /// True when part of this directory was never looked at: the one
    /// completeness question every surface asks.
    ///
    /// Not narrowed by evidence. A truncated walk may have missed the very
    /// executable that carried it, so "no evidence" cannot excuse a truncation.
    #[must_use]
    pub fn is_incomplete(&self) -> bool {
        self.incomplete_notes().next().is_some()
    }
}

/// How many path components a candidate sits behind. Used only to break a tie
/// between candidates the evidence ranked equal. See [`Survey::ranked`].
fn depth(candidate: &Candidate) -> usize {
    candidate.path.components().count()
}

/// [`Note::TiedAtTheTop`], if the highest score is shared. `candidates` is
/// already sorted. A shared score of zero is left to [`Survey::has_evidence`].
fn tie_at_the_top(candidates: &[Candidate]) -> Option<Note> {
    let best = candidates.first()?;
    let score = best.score();
    if score <= 0 {
        return None;
    }
    let count = candidates.iter().take_while(|c| c.score() == score).count();
    if count < 2 {
        return None;
    }
    Some(Note::TiedAtTheTop { count, score })
}

/// Everything [`install`](crate::install) saw about one executable, as plain
/// facts; the judgement is in [`assess`].
#[derive(Debug, Default, Clone)]
pub struct Observed {
    /// The executable, spelled as it should be reported.
    pub path: PathBuf,
    /// The same file relative to the surveyed directory, which is what the
    /// layout rules read.
    pub relative: PathBuf,
    /// Names of the subdirectories sitting beside the executable.
    pub sibling_directories: Vec<String>,
    /// The verdict on the image's **own** import tables, with no neighbours fed
    /// in. Neighbours are per-directory and arrive in [`Self::directory`].
    pub own: Verdict,
    /// The verdict on the files in the executable's **own** directory, which is
    /// shared by every executable in that directory and by none outside it.
    pub directory: Verdict,
}

/// Reads `observed` and lists every reason it is the game. The better of the
/// two name matches is used, once.
#[must_use]
pub fn assess(
    observed: &Observed,
    supplied_name: Option<&str>,
    directory_name: Option<&str>,
) -> Candidate {
    let mut reasons = Vec::new();
    let stem = observed
        .relative
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    if let Some(reason) = links_renderer(&observed.own) {
        reasons.push(reason);
    } else if let Some(reason) = links_renderer_through(&observed.own) {
        // Only when nothing was reached directly: one capability is not scored
        // twice.
        reasons.push(reason);
    }

    if let Some(directory) = unity_data_directory(&stem, &observed.sibling_directories) {
        reasons.push(Reason::UnityDataDirectory { directory });
    }
    if let Some(directory) = unreal_binaries_directory(&observed.relative) {
        reasons.push(Reason::UnrealBinariesDirectory { directory });
    }
    if has_shipping_suffix(&stem) {
        reasons.push(Reason::ShippingSuffix);
    }
    if let Some(reason) = name_resemblance(&stem, supplied_name, directory_name) {
        reasons.push(reason);
    }
    if let Some(reason) = ships_graphics_libraries(&observed.directory) {
        reasons.push(reason);
    }

    // Strongest first, stably, so equal weights keep their order.
    reasons.sort_by_key(|reason| -reason.weight());
    Candidate {
        path: observed.path.clone(),
        reasons,
    }
}

/// The renderers the image itself reaches, if any. Neighbour-only findings are
/// dropped here, so a hand-built [`Observed`] cannot pass one off as an import.
fn links_renderer(own: &Verdict) -> Option<Reason> {
    let mut apis = Vec::new();
    let mut strongest = None;
    for finding in &own.renderers {
        let source = finding.strength();
        // Linked findings belong to `links_renderer_through`; neighbour-only
        // ones are not evidence the image loads anything.
        if source == Source::Neighbour || source == Source::Linked {
            continue;
        }
        apis.push(finding.name.clone());
        strongest = Some(strongest.map_or(source, |best: Source| best.min(source)));
    }
    // `Source` is ordered strongest-first, so the minimum is the strongest.
    strongest.map(|source| Reason::LinksRenderer { apis, source })
}

/// The renderers reached one link away, merged into one reason: several local
/// libraries reaching Direct3D 11 are one capability.
fn links_renderer_through(own: &Verdict) -> Option<Reason> {
    let mut libraries = Vec::new();
    let mut apis: Vec<String> = Vec::new();
    for finding in &own.renderers {
        for signal in &finding.signals {
            if signal.source != Source::Linked {
                continue;
            }
            libraries.push(signal.library.clone());
            if !apis.contains(&finding.name) {
                apis.push(finding.name.clone());
            }
        }
    }
    if libraries.is_empty() {
        return None;
    }
    libraries.sort_unstable();
    libraries.dedup();
    Some(Reason::LinksRendererThrough { libraries, apis })
}

/// Unity names the data directory after the executable: `MyGame.exe` ships
/// with `MyGame_Data`. Matched case-insensitively.
fn unity_data_directory(stem: &str, siblings: &[String]) -> Option<String> {
    if stem.is_empty() {
        return None;
    }
    let wanted = format!("{stem}_Data");
    siblings
        .iter()
        .find(|name| name.eq_ignore_ascii_case(&wanted))
        .cloned()
}

/// The last two directory names, when they are Unreal's packaging shape.
///
/// `Engine/Binaries/Win64` is excluded: it holds the engine's tools. A game
/// whose own directory is called `Engine` loses this reason, which is accepted:
/// inferring it from the rest of the tree would depend on how much of the tree
/// was read, and would misfire on a half-downloaded install.
fn unreal_binaries_directory(relative: &Path) -> Option<String> {
    let names: Vec<String> = relative
        .parent()?
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();

    let [rest @ .., binaries, platform] = names.as_slice() else {
        return None;
    };
    if !binaries.eq_ignore_ascii_case("Binaries") {
        return None;
    }
    if !platform.eq_ignore_ascii_case("Win64") && !platform.eq_ignore_ascii_case("Win32") {
        return None;
    }
    if rest
        .last()
        .is_some_and(|d| d.eq_ignore_ascii_case("Engine"))
    {
        return None;
    }
    Some(format!("{binaries}/{platform}"))
}

/// The suffix Unreal gives a release build.
fn has_shipping_suffix(stem: &str) -> bool {
    stem.len() >= SHIPPING.len()
        && stem
            .get(stem.len() - SHIPPING.len()..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(SHIPPING))
}

const SHIPPING: &str = "-Shipping";

/// The better of the two name matches, or none. The supplied name wins a tie,
/// being evidence from outside the install.
fn name_resemblance(stem: &str, supplied: Option<&str>, directory: Option<&str>) -> Option<Reason> {
    let candidates = [
        (supplied, NameFrom::Supplied),
        (directory, NameFrom::Directory),
    ];
    let mut best: Option<Reason> = None;
    for (name, from) in candidates {
        let Some(name) = name else { continue };
        let Some(how) = resembles(stem, name) else {
            continue;
        };
        let reason = Reason::NameResembles {
            name: name.to_owned(),
            how,
            from,
        };
        if best.as_ref().is_none_or(|b| b.weight() < reason.weight()) {
            best = Some(reason);
        }
    }
    best
}

/// The shortest run of letters that is worth calling a match.
const MIN_NAME_OVERLAP: usize = 3;

/// How closely a file stem and a title match, once punctuation is dropped.
///
/// Only at the beginning: a substring anywhere would match `java.exe` against
/// `Java Development Kit`. A leading article (`The Witcher 3` and
/// `witcher3.exe`) is a known miss.
fn resembles(stem: &str, name: &str) -> Option<Resemblance> {
    let stem = squash(stem);
    let name = squash(name);
    if stem.len() < MIN_NAME_OVERLAP || name.len() < MIN_NAME_OVERLAP {
        return None;
    }
    if stem == name {
        return Some(Resemblance::Exact);
    }
    if stem.starts_with(&name) || name.starts_with(&stem) {
        return Some(Resemblance::Partial);
    }
    None
}

/// Lowercase letters and digits only.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The graphics libraries the executable's own directory ships. Local
/// overrides are left out: a `dxgi.dll` does not say which executable it serves.
fn ships_graphics_libraries(directory: &Verdict) -> Option<Reason> {
    let mut libraries: Vec<String> = directory
        .renderers
        .iter()
        .chain(&directory.features)
        .flat_map(|finding| &finding.signals)
        .filter(|signal| signal.source == Source::Neighbour)
        .map(|signal| signal.library.clone())
        .collect();
    if libraries.is_empty() {
        return None;
    }
    libraries.sort_unstable();
    libraries.dedup();
    Some(Reason::ShipsGraphicsLibraries { libraries })
}

#[cfg(test)]
mod tests;
