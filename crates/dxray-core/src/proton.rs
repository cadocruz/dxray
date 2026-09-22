//! Finding the `proton` script a game actually ran under.
//!
//! The IO half of [`nvapi`](crate::nvapi), kept as thin as
//! [`evidence`](crate::evidence) and [`steam`](crate::steam) are: it decides
//! which candidate paths exist, reads one text file, and hands the bytes over.
//! Every judgement about what the text *means* is on the pure side.
//!
//! Knowing the policy is only half an answer. Proton's policy changed direction
//! twice and split in two, so the question is never "what does Proton do to
//! this game" but "what does *this* Proton do to this game" — and which Proton
//! a game ran under is written down in the prefix Proton built for it.
//!
//! # Where that is written down
//!
//! Steam gives each game a compatibility prefix at
//! `<library>/steamapps/compatdata/<appid>/`. Proton writes a `config_info`
//! file at the top of it whose lines name the directories the build it used
//! lives in — its fonts, its libraries, its template prefix. Those lines look
//! like `<proton root>/files/share/fonts/`, so cutting one at its `/files/`
//! gives the Proton root, and the script is `proton` directly inside it.
//!
//! **The marker is not always `files`.** Proton 8.0 and every release before it
//! unpacked into `dist/` rather than `files/`; 9.0 renamed it. A resolver that
//! knows only `files` cannot identify any prefix last run under Proton 8.0 or
//! earlier — which is exactly the era whose policy runs the *other* way round,
//! so the games it fails on are the ones where being wrong costs most. Both
//! spellings are accepted here.
//!
//! Every candidate root is checked for an actual `proton` file rather than
//! trusted, because a line can be cut in more than one place — a user whose
//! home directory is called `files` produces two candidates from one line — and
//! the file being there is evidence while the string is only a guess.
//!
//! # None of this has been run against a real prefix
//!
//! There was no Steam, no Proton and no `compatdata` on the machine this was
//! written on. The layout above is taken from Proton's own source, which is
//! where `config_info` is written and where the directory names are set, and
//! the tests prove that this code does what that source says — not that a real
//! install matches. The one thing the source cannot settle is what a
//! `config_info` written by a build nobody has read looks like.

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::nvapi::{Applied, Decision, Environment, Reading, Resolution, UserSettings};
use crate::steam::launch::Launches;

/// The file name of the launcher script inside a Proton install.
const SCRIPT: &str = "proton";

/// What Proton unpacks itself into, newest spelling first.
///
/// `files` since Proton 9.0, `dist` before it. Both are kept because a prefix
/// records the build that last ran it, and plenty of prefixes on a real machine
/// were last touched years ago.
const DIST: [&str; 2] = ["/files/", "/dist/"];

/// What can go wrong while looking for the script.
#[derive(Debug)]
pub enum Error {
    /// A file is there and could not be read.
    Io { path: PathBuf, source: io::Error },
    /// A directory was handed over as a Proton install and holds no `proton`.
    NoScript { root: PathBuf },
    /// The game has no compatibility prefix.
    ///
    /// Its own variant, and the only one that is not a fault: a game that has
    /// never been launched under Proton has no prefix, which is the ordinary
    /// state of most of a Steam library. A caller that counted this as a
    /// failure would report a healthy machine as a broken scan.
    NoPrefix { path: PathBuf },
    /// `config_info` was read and no line in it names a Proton directory.
    NoProtonNamed { path: PathBuf },
    /// It names directories and none of them holds a `proton` script.
    ///
    /// What an uninstalled or renamed Proton looks like. Distinct from
    /// [`Error::NoProtonNamed`] because the fix is different: this one names a
    /// build that is gone, and a person can see which.
    ProtonGone { path: PathBuf, named: Vec<PathBuf> },
    /// It names more than one Proton, and they are not the same directory.
    ///
    /// Refused rather than resolved by picking the first. A prefix belongs to
    /// one build, so two means the file is not what this code believes it is,
    /// and the wrong build's policy is a confident wrong answer.
    Ambiguous { path: PathBuf, named: Vec<PathBuf> },
}

impl Error {
    /// The file or directory the failure is about.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Io { path, .. }
            | Self::NoPrefix { path }
            | Self::NoProtonNamed { path }
            | Self::ProtonGone { path, .. }
            | Self::Ambiguous { path, .. } => path,
            Self::NoScript { root } => root,
        }
    }

    /// Whether this is the ordinary state of a game that has never been run
    /// rather than something that went wrong.
    ///
    /// Callers that sweep a whole library use it to keep an untouched game out
    /// of the failure count. Nothing else should: it is the one variant that
    /// does not mean a thing on disk defeated this code.
    #[must_use]
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::NoPrefix { .. })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::NoScript { root } => write!(
                f,
                "{}: no {SCRIPT} script here, so this is not a Proton install",
                root.display()
            ),
            Self::NoPrefix { path } => write!(
                f,
                "{}: no compatibility prefix, so this game has not been run under Proton \
                 and there is no build to read a policy out of",
                path.display()
            ),
            Self::NoProtonNamed { path } => write!(
                f,
                "{}: no line in it names a Proton directory, so the build this prefix used \
                 cannot be identified",
                path.display()
            ),
            Self::ProtonGone { path, named } => write!(
                f,
                "{}: the Proton it names is not there any more ({})",
                path.display(),
                list(named)
            ),
            Self::Ambiguous { path, named } => write!(
                f,
                "{}: it names more than one Proton ({}), and reading the wrong one would \
                 report a policy this game never ran under",
                path.display(),
                list(named)
            ),
        }
    }
}

fn list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// The launcher script inside a Proton install, if it is there.
#[must_use]
pub fn script_in(root: &Path) -> Option<PathBuf> {
    let script = root.join(SCRIPT);
    script.is_file().then_some(script)
}

/// Takes either a Proton install directory or the script itself.
///
/// Both because both are things a person has in front of them: the directory is
/// what Steam shows in its library, and the script is what a path copied out of
/// a `config_info` line points at.
///
/// # Errors
///
/// Fails when a directory holds no `proton` script.
pub fn resolve(target: &Path) -> Result<PathBuf> {
    if target.is_dir() {
        return script_in(target).ok_or_else(|| Error::NoScript {
            root: target.to_path_buf(),
        });
    }
    Ok(target.to_path_buf())
}

/// The compatibility prefix Steam would give `appid` inside `library`.
///
/// Built rather than searched for, and **not** checked for existence: the
/// caller needs the path in the message when it turns out not to be there.
#[must_use]
pub fn compatdata(library: &Path, appid: u32) -> PathBuf {
    library
        .join("steamapps")
        .join("compatdata")
        .join(appid.to_string())
}

/// The `proton` script the prefix at `compatdata` last ran under.
///
/// `compatdata` is the per-game directory, the one named after the application
/// id, not the `pfx` inside it.
///
/// # Errors
///
/// Fails when there is no prefix ([`Error::NoPrefix`], which is the ordinary
/// state of a game nobody has launched), when `config_info` cannot be read,
/// when no line in it names a Proton, when the Proton it names has gone, or
/// when it names two.
pub fn from_prefix(compatdata: &Path) -> Result<PathBuf> {
    let path = compatdata.join("config_info");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        // No prefix at all, and no `config_info` inside one that does exist,
        // both mean the same thing to a caller: there is no build recorded
        // here. They are reported against different paths so the message says
        // which.
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(Error::NoPrefix {
                path: if compatdata.is_dir() {
                    path
                } else {
                    compatdata.to_path_buf()
                },
            });
        }
        Err(source) => return Err(Error::Io { path, source }),
    };

    let named = roots_named(&text);
    if named.is_empty() {
        return Err(Error::NoProtonNamed { path });
    }
    let mut found: Vec<PathBuf> = Vec::new();
    for root in &named {
        if script_in(root).is_some() && !found.iter().any(|seen| same(seen, root)) {
            found.push(root.clone());
        }
    }
    match found.len() {
        0 => Err(Error::ProtonGone { path, named }),
        1 => Ok(found[0].join(SCRIPT)),
        _ => Err(Error::Ambiguous { path, named: found }),
    }
}

/// The script the game with `appid` in `library` last ran under.
///
/// # Errors
///
/// As [`from_prefix`].
pub fn for_game(library: &Path, appid: u32) -> Result<PathBuf> {
    from_prefix(&compatdata(library, appid))
}

/// Reads a script for [`nvapi::scan`](crate::nvapi::scan).
///
/// Bytes that are not valid UTF-8 are replaced rather than refused. A launcher
/// script is ASCII apart from the game titles in its comments, those comments
/// are stripped before anything is read out of them, and refusing the file over
/// one bad byte in a comment would lose the whole policy.
///
/// # Errors
///
/// Fails when the file cannot be read.
pub fn read(script: &Path) -> Result<String> {
    let bytes = fs::read(script).map_err(|source| Error::Io {
        path: script.to_path_buf(),
        source,
    })?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Every Proton root a `config_info` could be naming.
///
/// Every cut of every line, not the first one that matches: a line can hold the
/// marker more than once, and which occurrence is the real boundary is settled
/// by looking for the script, not by picking one.
fn roots_named(text: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        for marker in DIST {
            let mut from = 0;
            while let Some(at) = line[from..].find(marker) {
                let cut = from + at;
                if cut > 0 {
                    let root = PathBuf::from(&line[..cut]);
                    if !out.contains(&root) {
                        out.push(root);
                    }
                }
                from = cut + 1;
            }
        }
    }
    out
}

/// Whether two paths are the same directory, resolving symlinks when they can
/// be resolved and comparing the text when they cannot.
fn same(a: &Path, b: &Path) -> bool {
    let resolve = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    resolve(a) == resolve(b)
}

/// What one game's NVAPI question came to.
///
/// Carries more than any one caller prints. A listing that sweeps a library has
/// a line per game and shows the sentence; a detail pane has room for the
/// condition's source. Which of these a caller renders is a presentation
/// decision, and making it a *data* decision is what put two copies of this
/// type in two binaries that then had to be kept in step by hand.
#[derive(Debug, Clone)]
pub struct Answer {
    /// The launcher script the answer was drawn from, when one was found.
    ///
    /// Shown beside the verdict and never instead of it. Which Proton ran a
    /// game decides the answer - the policy changed direction twice across
    /// releases - so a verdict with no build behind it is not checkable by the
    /// person reading it.
    pub script: Option<PathBuf>,
    /// The sentence, ready to print.
    pub verdict: String,
    /// `Some(true)` when Proton offers this game NVAPI, `Some(false)` when it
    /// withholds it, `None` when that is not settled.
    ///
    /// Only fit for colouring a word the sentence beside it spells out. Three
    /// very different states answer `None` - a condition this crate declines to
    /// evaluate, a build with no per-game policy, and an absence of knowledge -
    /// and nothing may read this as telling them apart.
    pub available: Option<bool>,
    /// The condition the flag is set under, quoted verbatim.
    ///
    /// Verbatim rather than summarised because the reader is the one who gets
    /// to apply it: they know whether their machine has an NVIDIA module
    /// loaded, and this crate has decided on purpose not to look.
    pub condition: Vec<String>,
}

impl Answer {
    /// A game that is not a Steam application, so Steam's Proton policy cannot
    /// be applied to it. Every Heroic title is this.
    ///
    /// The launcher is named rather than assumed, because "Heroic" was hard
    /// coded here while Heroic was the only non-Steam launcher, and the next
    /// one would have inherited a sentence that was false about it. The refusal
    /// itself is the point: this is an answer, not a blank, and a list holding
    /// Steam and Heroic games side by side needs a sentence in every row.
    #[must_use]
    pub fn not_applicable(origin: crate::launcher::Origin) -> Self {
        Self {
            script: None,
            verdict: format!(
                "not applicable: a {} game has no Steam AppID, and without one there is no \
                 compatibility prefix to read a Proton policy out of",
                origin.label()
            ),
            available: None,
            condition: Vec::new(),
        }
    }

    /// A game whose Proton could not be found or read at all.
    fn undetermined(reason: &str) -> Self {
        Self {
            script: None,
            verdict: format!("not determined: {reason}"),
            available: None,
            condition: Vec::new(),
        }
    }
}

/// One Proton build, read once per scan.
struct Build {
    reading: Reading,
    settings: UserSettings,
}

/// Everything a scan reads once and many games share: each Proton build, and
/// each Steam installation's launch options.
#[derive(Default)]
pub struct Builds {
    read: HashMap<PathBuf, std::result::Result<Build, String>>,
    launches: HashMap<PathBuf, Launches>,
}

impl Builds {
    /// The NVAPI answer for a game from any launcher. Only a Steam application
    /// has a prefix to read; any other game is told the question does not
    /// apply.
    ///
    /// `root` is the Steam installation the game came from, which is where its
    /// launch options live.
    pub fn answer_for(
        &mut self,
        root: Option<&Path>,
        library: &Path,
        game: &crate::launcher::Game,
    ) -> Answer {
        match game.identity.steam_appid() {
            Some(appid) => self.answer(root, library, appid),
            None => Answer::not_applicable(game.origin),
        }
    }

    /// What the Proton that last ran `appid` in `library` does to its NVAPI,
    /// with the launch options from `root` applied. Never an error: every
    /// failure is a sentence.
    pub fn answer(&mut self, root: Option<&Path>, library: &Path, appid: u32) -> Answer {
        let prefix = compatdata(library, appid);
        let script = match from_prefix(&prefix) {
            Ok(script) => script,
            Err(error) => return Answer::undetermined(&error.to_string()),
        };
        let build = self.read.entry(script.clone()).or_insert_with(|| {
            read(&script)
                .map_err(|error| error.to_string())
                .and_then(|source| crate::nvapi::scan(&source).map_err(|error| error.to_string()))
                .map(|reading| Build {
                    reading,
                    settings: user_settings_beside(&script),
                })
        });
        let build = match build {
            Ok(build) => build,
            Err(error) => {
                return Answer {
                    script: Some(script),
                    verdict: format!("not determined: this script was not understood: {error}"),
                    available: None,
                    condition: Vec::new(),
                };
            }
        };
        // Without a Steam root there are no launch options to read.
        let launch = root.map_or(Ok(Vec::new()), |root| {
            self.launches
                .entry(root.to_path_buf())
                .or_insert_with(|| crate::steam::launch::launches(root))
                .environment(appid)
        });
        let appid = appid.to_string();
        let environment = Environment::new(launch, &build.settings);
        let decision = crate::nvapi::decide(&build.reading, &appid);
        let resolution = crate::nvapi::resolve(&build.reading, &appid, &environment);
        let recorded = build
            .reading
            .recorded_line
            .and_then(|line| recorded(&prefix, line));
        compose(script, &decision, &resolution, recorded)
    }
}

/// The answer the default policy, the launch environment and the last launch
/// give between them.
fn compose(
    script: PathBuf,
    decision: &Decision,
    resolution: &Resolution,
    recorded: Option<bool>,
) -> Answer {
    let condition = || {
        decision
            .condition()
            .map(crate::nvapi::Condition::source)
            .unwrap_or_default()
    };
    let (mut verdict, mut available, condition) = match resolution {
        Resolution::Default => (decision.to_string(), decision.available(), condition()),
        Resolution::Computed { use_nvapi, applied } => {
            let how: Vec<String> = applied.iter().map(Applied::describe).collect();
            (
                format!(
                    "NVAPI is {}: {}; the script alone says: {decision}",
                    if *use_nvapi { "offered" } else { "withheld" },
                    how.join(", ")
                ),
                Some(*use_nvapi),
                Vec::new(),
            )
        }
        Resolution::Undetermined(why) => (
            format!("not determined: {why}; the script alone says: {decision}"),
            None,
            condition(),
        ),
    };
    if let Some(recorded) = recorded {
        let spelled = if recorded { "True" } else { "False" };
        let _ = write!(verdict, "; the last launch recorded use_nvapi={spelled}");
        if available.is_some_and(|expected| expected != recorded) {
            verdict.push_str(
                ", which disagrees: the options changed since, or something this reader \
                 does not see sets it",
            );
            available = None;
        }
    }
    Answer {
        script: Some(script),
        verdict,
        available,
        condition,
    }
}

/// `user_settings.py` in the build's directory, which Proton imports.
fn user_settings_beside(script: &Path) -> UserSettings {
    match fs::read_to_string(script.with_file_name("user_settings.py")) {
        Ok(text) => crate::nvapi::user_settings(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => UserSettings::Absent,
        Err(_) => UserSettings::Unreadable,
    }
}

/// The `use_nvapi` value the last launch wrote to `config_info`, at `line`.
fn recorded(compatdata: &Path, line: usize) -> Option<bool> {
    let text = fs::read_to_string(compatdata.join("config_info")).ok()?;
    match text.lines().nth(line.checked_sub(1)?)?.trim() {
        "True" => Some(true),
        "False" => Some(false),
        _ => None,
    }
}
