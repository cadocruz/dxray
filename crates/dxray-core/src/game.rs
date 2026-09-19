//! Which executable in an install directory is the game? Rank, and explain.
//!
//! Pure in the same sense as [`analysis`](crate::analysis): nothing here opens
//! a file. It reads the *shape* of a path — the stem, the directories above it,
//! the names of the directories beside it — because that shape is evidence, but
//! it never asks the filesystem a question. [`install`](crate::install) does
//! that and hands the answers over as [`Observed`].
//!
//! Four ideas shape everything below.
//!
//! **Rank and explain; never pick.** A real install holds a launcher at the
//! root, the shipping binary several directories down, a crash handler, and
//! installers for Visual C++ that were never removed. The launcher and the game
//! are both correct answers to different questions, so the result is an ordered
//! list where every entry carries the reasons it scored as it did. A tool that
//! names one and hides the other is lying by omission.
//!
//! **No exclusion lists.** `vcredist_x64.exe` and `UnityCrashHandler64.exe` sink
//! because they carry no evidence, not because they are on a list of names.
//! Every such list is a guess that eventually hides a real game — the same
//! argument [`steam::games`](crate::steam::games) makes for not filtering
//! manifests. Every reason in [`Reason`] is something observed about the file or
//! its directory, and a binary with none of them scores zero on its own merits.
//!
//! **Evidence is per-directory, and that is not the install root.** An Unreal
//! game's executable lives in `Binaries/Win64` and its upscaler DLLs live there
//! with it, not at the root. Attributing root-level neighbours to a binary three
//! levels down would make every executable in the tree look equally equipped, so
//! [`Observed::directory`] is the verdict for the candidate's **own** directory
//! and nothing else.
//!
//! **The strongest signal is missing for a whole class of real games, and the
//! answer has to say so.** A Java or LWJGL title, an Electron game, a .NET one —
//! none of them import a graphics API, because the runtime loads it with
//! `LoadLibrary` after startup. The structural signals still rank them, but a
//! ranking with no import table under it must not be presented as confidently
//! as one that has it, so [`Note::NoRendererImported`] rides back with the list
//! and [`Survey::ranked`] raises it where no caller can forget to.
//!
//! **A directory-level signal cannot break a tie inside its own directory.**
//! Every executable in one folder shares its neighbours, so `nvngx_dlss.dll`
//! sitting beside both the game and the crash handler says the same thing about
//! both. Signals of that kind are weighted lowest on purpose: they discriminate
//! across directories, which is the only place they carry information.

use std::io;
use std::path::{Path, PathBuf};

use crate::analysis::{Source, Verdict};

/// The weight of each reason, in one table so the ranking can be read at a
/// glance rather than reconstructed from a `match`.
///
/// The numbers are not measurements. They are an ordering, chosen so that the
/// cases in `tests` come out in the order a person would defend, and every one
/// of them is a place where a real install could prove this module wrong. They
/// are public so that a caller who disagrees can say by how much.
///
/// # Every weight here is positive, and that is load-bearing
///
/// Because no reason is worth nothing and none is worth less than nothing, a
/// candidate that [`assess`] built scores zero **exactly** when its reasons are
/// empty. [`Candidate::score`] and [`Candidate::has_evidence`] therefore cannot
/// disagree, and neither can the two surfaces built on them: the TUI marks a
/// row `no evidence` from [`Survey::has_evidence`], which counts reasons across
/// the install, while its detail pane says "nothing observed argues that this
/// is the game" from the reasons of the one executable that ranked first. A
/// weight of zero, or a negative one cancelling a positive, would let a row
/// carry no marker while the pane under it said nothing was observed.
///
/// A new reason worth zero therefore does not belong here; it belongs in
/// [`Note`], which is where this module puts something worth saying that is not
/// worth scoring. `tests::every_reason_assess_can_build_is_worth_more_than_nothing`
/// is the guard.
///
/// The one arm of [`Reason::weight`] that returns zero — a `LinksRenderer`
/// carrying [`Source::Neighbour`] — is not an exception to this. [`assess`]
/// cannot build it, and the zero exists precisely so that a caller who
/// hand-builds one gets no credit for evidence that variant does not mean.
pub mod weight {
    /// The image's own import table names a graphics API. The process cannot
    /// start without it.
    pub const RENDERER_IMPORT: i32 = 100;

    /// A subdirectory named `<stem>_Data` sits beside `<stem>.exe`.
    ///
    /// As strong as a load-time import, and deliberately so. Unity's player
    /// refuses to start without that directory and the name is derived from the
    /// executable's own, so the pairing cannot arise by accident. It has to be
    /// this strong: a Unity game's `.exe` is a stub that imports no graphics API
    /// at all, and a table that ranked imports above layout would rank every
    /// Unity game below its own crash handler the moment the handler picked up
    /// any other signal.
    pub const UNITY_DATA: i32 = 100;

    /// The image delay-loads a graphics API: real, but the call may never
    /// happen.
    pub const RENDERER_DELAY_IMPORT: i32 = 80;

    /// A library in the image's own directory, which the image imports, reaches
    /// a graphics API. Weaker than reaching one directly, because the chain has
    /// one more link in it that this crate did not watch being followed.
    pub const RENDERER_INDIRECT: i32 = 70;

    /// The image sits in `Binaries/Win64` or `Binaries/Win32`.
    pub const UNREAL_BINARIES: i32 = 55;

    /// The stem ends `-Shipping`, which is what Unreal calls a release build.
    ///
    /// Together with [`UNREAL_BINARIES`] this outweighs a lone load-time import,
    /// and that is the intended answer: `Binaries/Win64/Thing-Win64-Shipping.exe`
    /// is a packaging convention that essentially nothing but Unreal produces,
    /// while a launcher built on an embedded browser imports `d3d11.dll` for its
    /// own compositor and means nothing by it.
    pub const SHIPPING_SUFFIX: i32 = 50;

    /// The stem and the game's name are the same word.
    ///
    /// Deliberately below every structural reason, because a name is the
    /// weakest kind of identity evidence there is and its absence has to cost
    /// nothing: `PUBG: BATTLEGROUNDS` ships `TslGame.exe`, which resembles its
    /// own title not at all. A signal that can be right about a game and say
    /// nothing about the next one must never be able to outvote the layout.
    pub const NAME_EXACT: i32 = 30;

    /// One of the stem and the game's name contains the other.
    pub const NAME_PARTIAL: i32 = 15;

    /// The image's own directory ships graphics libraries — an upscaler, a
    /// vendor SDK, the Direct3D 12 Agility runtime.
    ///
    /// Lowest, because it is a fact about the directory rather than about any
    /// binary in it. See the module documentation.
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

/// Where the name a stem was compared against came from.
///
/// Kept because the two are worth different amounts of trust and a reader
/// should not have to guess which one was used. A name out of a Steam manifest
/// is evidence from a different source; the directory's own name is the same
/// install talking about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameFrom {
    /// Supplied by the caller — from a Steam manifest, or typed.
    Supplied,
    /// The name of the directory that was surveyed.
    Directory,
}

/// One observed fact that argues this executable is the game.
///
/// Every variant is something seen in the file or in the directory it sits in.
/// There is no variant meaning "this name looks like a launcher", and there must
/// not be: that is the guess-list this module exists to avoid.
///
/// There are no negative reasons either. A binary with nothing to say about
/// itself scores zero and sinks, which is the same outcome with none of the risk
/// of scoring a real game below zero for having an unlucky name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// The image's own import or delay-import table names a graphics API.
    ///
    /// **Never built from a neighbouring file.** A `d3d12core.dll` in the
    /// directory is a fact about the directory, shared by every executable in
    /// it, and it arrives as [`Reason::ShipsGraphicsLibraries`] instead.
    LinksRenderer {
        /// Every API reached, in the order [`analyse`](crate::analyse) reports.
        apis: Vec<String>,
        /// The strongest way any of them was reached.
        source: Source,
    },
    /// A library in the image's own directory, which the image imports, reaches
    /// a graphics API.
    ///
    /// This is the signal that keeps Unity games above their own crash
    /// handlers on evidence rather than on layout alone: `Game.exe` imports
    /// `UnityPlayer.dll`, and it is `UnityPlayer.dll` that imports `d3d11.dll`.
    /// The chain is followed exactly one link, and only into files sitting in
    /// the candidate's own directory.
    ///
    /// Only raised when the image reaches no renderer directly, so the two
    /// never stack into a double count of one capability.
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
    /// The executable sits in `Binaries/Win64` or `Binaries/Win32`.
    ///
    /// Not raised for `Engine/Binaries/Win64`, which is where an Unreal install
    /// keeps the engine's own tools — the crash reporter above all. That is a
    /// statement about the layout, not a guess about the name of the binary
    /// inside it: the game's own `Binaries` directory is a sibling of `Engine`,
    /// never a child of it.
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
    /// The stable spelling used in machine-readable output.
    ///
    /// Rendered by callers instead of `Debug`, which is free to change, and
    /// alongside the [`Display`](std::fmt::Display) sentence rather than instead
    /// of it: the sentence is for a person and is allowed to be reworded, this
    /// is for a program and is not.
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
            // `Source::Neighbour` cannot reach this variant — `assess` builds it
            // from import tables only — and is scored at nothing rather than at
            // something flattering, so that a future caller constructing one by
            // hand gets no credit for evidence this variant does not mean.
            Self::LinksRenderer { source, .. } => match source {
                Source::Import => weight::RENDERER_IMPORT,
                Source::DelayImport => weight::RENDERER_DELAY_IMPORT,
                // Reached through a local library. The ranking builds its own
                // evidence without this source and reaches the same conclusion
                // by its own route, as `Reason::LinksRendererThrough`, so this
                // arm is not taken today. It carries the same weight as that
                // reason so the two cannot drift if the evidence ever arrives
                // here instead.
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
    /// Every reason found, strongest first. Empty means nothing was observed —
    /// which is the honest answer for a Visual C++ redistributable, and is
    /// reported as such rather than hidden.
    pub reasons: Vec<Reason>,
}

impl Candidate {
    /// The sum of the weights of the reasons.
    ///
    /// Computed rather than stored, so the score and the reasons printed beside
    /// it cannot drift apart. There is no way to hold a `Candidate` whose number
    /// is not exactly the arithmetic of the sentences under it.
    #[must_use]
    pub fn score(&self) -> i32 {
        self.reasons.iter().map(Reason::weight).sum()
    }

    /// True when something was observed about this executable.
    #[must_use]
    pub fn has_evidence(&self) -> bool {
        !self.reasons.is_empty()
    }

    /// True when an import table — this image's own, or that of a library
    /// beside it — named a graphics API.
    ///
    /// The distinction the whole [`Note::NoRendererImported`] caveat turns on:
    /// a structural signal says where a file sits, and only this one says the
    /// loader will be asked for a renderer.
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

/// Something true about a survey that makes it worth less than it looks.
///
/// The same idea as [`steam::Note`](crate::steam::Note) and for the same
/// reason. A truncated walk that reports the launcher because it never reached
/// the real binary is the wrong-answer shape this crate exists to refuse, so
/// every limit that was hit is carried back with the answer.
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
    /// the tree is still worth ranking, and a permission error on one folder
    /// must not sink a survey that is otherwise complete.
    Unreadable { path: PathBuf, source: io::Error },
    /// An executable was found but could not be read or parsed. It stays in the
    /// list with whatever its path still says about it, because "the game is
    /// the corrupt one" is an answer somebody needs.
    Unparsed { path: PathBuf, source: io::Error },
    /// Not one executable in the survey reaches a graphics API through its
    /// import tables, so the ranking rests on directory structure alone.
    ///
    /// This is the caveat that keeps the ranking honest for an entire class of
    /// real games. A Java or LWJGL title (`Project Zomboid`, `Minecraft`), an
    /// Electron game, a .NET or `MonoGame` one — none of them import a renderer,
    /// because the runtime loads it with `LoadLibrary` long after startup. The
    /// process that actually draws `Project Zomboid` is `jre64/bin/java.exe`,
    /// buried in a subdirectory, and it imports nothing graphical either.
    ///
    /// It is also a finding in its own right rather than only an apology. An
    /// executable that imports no graphics API behaves differently under an
    /// injector than one that imports `d3d11.dll`, and that is worth telling
    /// somebody whether or not they asked which binary is the game.
    NoRendererImported {
        /// How many executables were read and found to import none.
        executables: usize,
    },
    /// Two or more executables share the highest score, so the evidence does
    /// not choose between them.
    ///
    /// The one case where printing a list best-first is itself a claim the
    /// evidence does not support. Whatever is at the top of a tie got there by
    /// the presentation rule in [`Survey::ranked`] — shallower path, then
    /// alphabetical — and a reader has no way to tell that from a result the
    /// evidence actually separated. This slice exists to rank rather than pick,
    /// and an unreported tie at the top is picking with extra steps.
    ///
    /// **Raised without any idea of what caused the tie**, and that is the
    /// design. Two builds of one game shipped side by side tie because both are
    /// real. A game ties with an Electron application, or with a security
    /// product, because importing `d3d11.dll` is ordinary behaviour for a great
    /// deal of software that is not a game. A rule that had to recognise the
    /// category would one day meet a category it did not recognise; a tie is a
    /// tie, and the sentence is the same either way.
    ///
    /// Only raised when the shared score is above zero. A directory of
    /// installers where everything ties at nothing is already described by
    /// [`Survey::has_evidence`], in a sentence that says more than this one
    /// would.
    TiedAtTheTop {
        /// How many candidates share the top score. Always at least two.
        count: usize,
        /// The score they share.
        score: i32,
    },
}

impl Note {
    /// True when the note means something was **not read**, rather than that
    /// what was read says less than usual.
    ///
    /// The line between "not read" and "read, and says little", and the two
    /// sides of it are not the same kind of thing. Every surface asks it
    /// through [`Survey::is_incomplete`], which is the same line for
    /// `dxray --game` and for a listing.
    ///
    /// A walk that stopped at a limit, a directory that could not be opened, a
    /// file that would not parse — in each of those the answer printed may be
    /// wrong because the right answer was never looked at, and a script has to
    /// be able to tell. [`Note::NoRendererImported`] is
    /// the other kind: everything was read, and this is what it says.
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
///
/// Two lists rather than a `Vec<Candidate>` and a silent truncation, for the
/// reason spelled out on [`Note`]: the shape of a wrong answer here is a short
/// list that looks complete.
#[derive(Debug, Default)]
pub struct Survey {
    /// Best first. Ties broken by path, so two runs over one install produce
    /// the same list in the same order and can be diffed against each other.
    pub candidates: Vec<Candidate>,
    pub notes: Vec<Note>,
}

impl Survey {
    /// Sorts `candidates`, raises [`Note::NoRendererImported`] if it applies,
    /// and builds the survey.
    ///
    /// Both happen here rather than in a separate call the caller makes
    /// afterwards. A `Survey` whose caller forgot to rank it is a ranked list in
    /// name only, and one whose caller forgot the caveat is the confident wrong
    /// answer this module exists to refuse. Neither is reachable.
    #[must_use]
    pub fn ranked(mut candidates: Vec<Candidate>, mut notes: Vec<Note>) -> Self {
        // Descending score; then, among equals, the shallower path and then the
        // alphabetical one. Not `sort_by_key` with a negated score: the path is
        // borrowed from the element being compared.
        //
        // The depth is a **presentation** rule and not a signal. It scores
        // nothing and appears in no explanation: it decides only the order of
        // candidates the evidence could not separate, and it is there because
        // the alternative sorted a launcher at the install root below a
        // redistributable four directories down for no reason a reader could
        // see. Between two files with identical evidence there is no right
        // answer, and this at least puts the one nearer the front door first.
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

    /// The highest-ranked executable, if any were found at all.
    ///
    /// Returns the top of the list even when it carries no evidence. Deciding
    /// that a zero-scoring candidate is not worth returning would be this
    /// module picking, and it does not pick — [`Candidate::has_evidence`] lets
    /// the caller say so in its own words.
    #[must_use]
    pub fn best(&self) -> Option<&Candidate> {
        self.candidates.first()
    }

    /// True when at least one executable had something to say for itself.
    ///
    /// False for a directory holding only redistributables, which is a real
    /// state worth reporting and not the same as an empty directory.
    #[must_use]
    pub fn has_evidence(&self) -> bool {
        self.candidates.iter().any(Candidate::has_evidence)
    }

    /// The notes saying part of this directory was never looked at.
    ///
    /// The selection every surface prints from, so that a reader cannot be
    /// shown one set of unread-file notes by `--game` and a different set by
    /// `--steam` for the same directory. Exactly [`Note::is_incomplete`],
    /// applied in one place.
    pub fn incomplete_notes(&self) -> impl Iterator<Item = &Note> {
        self.notes.iter().filter(|note| note.is_incomplete())
    }

    /// True when part of this directory was never looked at.
    ///
    /// The one question every surface asks about a survey's completeness:
    /// `dxray --game`, the `--installed` and `--steam` listings, and the
    /// terminal browser. There is no second, narrower version of it for
    /// listings.
    ///
    /// # Why a listing may not narrow this by evidence
    ///
    /// A listing inspects everything a launcher declares, Proton builds and
    /// container sysroots included, so it is tempting to charge a truncation
    /// only to entries that carry evidence of being a game — the tool
    /// directories would stop costing an exit code on a healthy machine. It is
    /// unsound, and not by a margin that tuning could close. `has_evidence` is
    /// only ever consulted here when the walk already truncated, and a
    /// truncated walk is itself an explanation for finding no evidence: the
    /// evidence may be the part that was not read. The question is asked in
    /// exactly the situation where it cannot answer.
    ///
    /// Measured: a Steam install holding a zero-evidence `launcher.exe` at its
    /// root and the real `Game-Win64-Shipping.exe` below the bound listed as
    /// one game, with no caveat and an exit code of 0, while the row beside it
    /// said nothing there carried evidence of being a game. That is the
    /// wrong-answer shape [`install`](crate::install) bounds the walk to avoid,
    /// one level up.
    ///
    /// Tool directories are therefore addressed where the problem is — a bound
    /// no real install reaches, see
    /// [`MAX_DEPTH`](crate::install::MAX_DEPTH) — and not by teaching the exit
    /// code to guess which truncations mattered.
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

/// [`Note::TiedAtTheTop`], if the highest score is shared.
///
/// Called on an already-sorted list, so the tied candidates are the run at the
/// front and counting stops at the first lower score.
///
/// A score of zero is left alone deliberately: every candidate in a directory
/// of installers ties there, and [`Survey::has_evidence`] already gives the
/// caller a better sentence for that than "five things tie at nothing" would be.
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

/// Everything [`install`](crate::install) saw about one executable.
///
/// Built by the IO half, or by hand in a test. Deliberately a plain struct of
/// already-gathered facts: every judgement made about them is in [`assess`],
/// where it can be tested against an awkward layout without a disk or a game.
#[derive(Debug, Default, Clone)]
pub struct Observed {
    /// The executable, spelled as it should be reported.
    pub path: PathBuf,
    /// The same file relative to the directory that was surveyed. This is what
    /// the layout rules read, so that an install that happens to live under a
    /// folder called `Binaries` does not hand every binary in it a reason.
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

/// Reads `observed` and lists every reason it is the game.
///
/// `supplied_name` is the title from a Steam manifest, or whatever the caller
/// knows. `directory_name` is the name of the directory that was surveyed. Both
/// are optional and the better of the two matches is used, once — a stem that
/// resembles both is one piece of evidence, not two.
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
        // Only when nothing was reached directly. A binary that imports
        // `d3d12.dll` *and* ships a library that imports it too has one
        // capability, not two, and scoring it twice would let a well-stocked
        // directory outrank a game on the strength of the same fact counted
        // again.
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

    // Strongest first, stably, so two reasons of equal weight keep the order
    // they were gathered in and the sentence under a candidate reads the same
    // way on every run.
    reasons.sort_by_key(|reason| -reason.weight());
    Candidate {
        path: observed.path.clone(),
        reasons,
    }
}

/// The renderers the image itself reaches, if any.
///
/// Findings whose only support is a neighbouring file are dropped here rather
/// than filtered upstream, so that a caller who builds an [`Observed`] by hand
/// and fills `own` with a full verdict still cannot smuggle a directory listing
/// in as an import.
fn links_renderer(own: &Verdict) -> Option<Reason> {
    let mut apis = Vec::new();
    let mut strongest = None;
    for finding in &own.renderers {
        let source = finding.strength();
        // A finding whose strongest support is a followed link belongs to
        // `links_renderer_through`, which can name the library it went through.
        // A finding supported only by a neighbouring file is not evidence the
        // image loads anything at all.
        if source == Source::Neighbour || source == Source::Linked {
            continue;
        }
        apis.push(finding.name.clone());
        strongest = Some(strongest.map_or(source, |best: Source| best.min(source)));
    }
    // `Source` is ordered strongest-first, so the minimum is the strongest.
    strongest.map(|source| Reason::LinksRenderer { apis, source })
}

/// The renderers reached one link away, merged into a single reason.
///
/// Read out of the same [`Verdict`] [`links_renderer`] reads, so the score and
/// the verdict printed beside it cannot disagree about one binary. They did:
/// a Unity stub scored for reaching a renderer through `UnityPlayer.dll` while
/// the verdict under that score said no graphics API was determined.
///
/// Merged rather than one reason per library: three local libraries that each
/// reach Direct3D 11 are one capability observed three times, and scoring each
/// of them would let a directory full of engine DLLs outrank a game.
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

/// Unity names the player's data directory after the executable, so
/// `MyGame.exe` is shipped with `MyGame_Data`. Matched case-insensitively,
/// because the pairing survives a copy onto a case-preserving filesystem and a
/// case-sensitive match would lose it.
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
/// `Engine/Binaries/Win64` is excluded: it holds the engine's own tools, the
/// crash reporter among them, and they are not the game. The exclusion is
/// structural rather than a name list — a packaged Unreal title keeps the
/// game's `Binaries` under a project directory that is a *sibling* of `Engine`,
/// never a child of it — but it is the one place in this module where a
/// directory name is trusted, and it is worth knowing that.
///
/// # The cost, which is real and is accepted
///
/// A game whose own directory is called `Engine` loses this reason and scores
/// 55 lower than it should. It does not lose its *place*: the exclusion has
/// never been load-bearing for a top-ranked result, because the tools it keeps
/// out have no renderer import, no shipping suffix and no name match, and the
/// shipping binary beside them wins on those instead. What is lost is a line of
/// explanation.
///
/// The obvious repair is to ask the tree instead of the name — in a packaged
/// title *both* `Engine/Binaries` and `<Game>/Binaries` exist, so an `Engine`
/// that is the only directory hosting a `Binaries` folder is more likely a game
/// than an engine. It is not done, for two reasons that are worse than the
/// defect:
///
/// A title that is **half downloaded**, with only `Engine/` extracted so far,
/// has exactly that shape, and the repair would hand Epic's crash reporter the
/// game's own layout bonus in a state that occurs on real machines every day.
///
/// And the answer would depend on **how much of the tree was read**. The walk
/// is bounded; a truncation before reaching `<Game>/Binaries` would silently
/// change the reasons printed for an unrelated candidate in a different
/// directory. A signal whose value moves with the budget is not a signal.
///
/// So the narrower rule stays, and the limitation is documented here and in the
/// README rather than traded for a wrong answer in a commoner case.
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

/// The better of the two name matches, or none.
///
/// The supplied name wins a tie because it is evidence from somewhere else. The
/// directory's own name is the same install talking about itself, which is
/// still worth something — it is how a non-Steam install gets any name evidence
/// at all — but it is not independent.
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
/// `Cyberpunk 2077` and `Cyberpunk2077.exe` are the same name written twice, so
/// they match exactly. `Game-Win64-Shipping.exe` in a game called `Game` is a
/// partial match, because the stem *begins* with the title.
///
/// **Only at the beginning.** A substring found anywhere would match `java.exe`
/// against a folder called `Java Development Kit`, and `edit.exe` against
/// `Skyrim Special Edition`, which is not a coincidence that happens rarely —
/// it happened twice in the first dozen fixtures written against this function.
/// A name that lifts an unrelated installer off a score of zero is worse than a
/// name signal that misses, because the miss costs a real game nothing: every
/// other reason still fires and this one was never allowed to decide anything.
///
/// The known miss is a leading article. `The Witcher 3: Wild Hunt` and
/// `witcher3.exe` are the same game and this reports nothing about them, for
/// the same reason it reports nothing about `TslGame.exe` in
/// `PUBG: BATTLEGROUNDS`: there is no rule short of a synonym table that gets
/// those right, and inventing one here would be the guess-list this module
/// refuses.
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

/// Lowercase, letters and digits only. Everything a person puts between the
/// words of a title — spaces, colons, hyphens, trademark signs — is noise when
/// the question is whether two spellings are the same name.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The graphics libraries the executable's own directory ships.
///
/// Drawn from the renderer and feature findings whose support is a neighbouring
/// file. Local overrides are deliberately left out: a `dxgi.dll` next to a game
/// is a mod loader, and which executable it was installed for is exactly what it
/// does not say.
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
