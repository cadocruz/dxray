//! Finding the `proton` script a game last ran under, and reading what its
//! prefix recorded. The IO half of [`nvapi`](crate::nvapi).
//!
//! Proton writes `config_info` at the top of a game's prefix
//! (`<library>/steamapps/compatdata/<appid>`), and its lines name the build's
//! directories. Cutting one at `/files/` (`/dist/` before 9.0) gives the root;
//! a candidate counts only if a `proton` script is really there.

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

/// What Proton unpacks itself into: `files` since 9.0, `dist` before.
const DIST: [&str; 2] = ["/files/", "/dist/"];

/// What can go wrong while looking for the script.
#[derive(Debug)]
pub enum Error {
    /// A file is there and could not be read.
    Io { path: PathBuf, source: io::Error },
    /// A directory was handed over as a Proton install and holds no `proton`.
    NoScript { root: PathBuf },
    /// The game has no compatibility prefix: the ordinary state of a game never
    /// launched under Proton, not a fault.
    NoPrefix { path: PathBuf },
    /// `config_info` was read and no line in it names a Proton directory.
    NoProtonNamed { path: PathBuf },
    /// It names directories and none holds a `proton` script: an uninstalled or
    /// renamed build.
    ProtonGone { path: PathBuf, named: Vec<PathBuf> },
    /// It names more than one Proton. Refused rather than picking one.
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

    /// Whether this is a game that has never been run, rather than something
    /// that went wrong.
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

/// The compatibility prefix Steam would give `appid` inside `library`, built
/// rather than searched for and not checked for existence.
#[must_use]
pub fn compatdata(library: &Path, appid: u32) -> PathBuf {
    library
        .join("steamapps")
        .join("compatdata")
        .join(appid.to_string())
}

/// The `proton` script the prefix at `compatdata` (the per-game directory, not
/// its `pfx`) last ran under.
///
/// # Errors
///
/// Fails when there is no prefix ([`Error::NoPrefix`]), when `config_info`
/// cannot be read, or when it names no Proton, a missing one, or two.
pub fn from_prefix(compatdata: &Path) -> Result<PathBuf> {
    let path = compatdata.join("config_info");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        // No prefix, or no `config_info` in one: no build is recorded either way.
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

/// Reads a script for [`nvapi::scan`](crate::nvapi::scan). Invalid UTF-8 is
/// replaced, since it only appears in comments.
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

/// Every Proton root a `config_info` could be naming: every cut of every line,
/// settled later by looking for the script.
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

/// What one game's NVAPI question came to, with more than any one caller
/// prints.
#[derive(Debug, Clone)]
pub struct Answer {
    /// The launcher script the answer was drawn from, shown beside the verdict
    /// so it can be checked.
    pub script: Option<PathBuf>,
    /// The sentence, ready to print.
    pub verdict: String,
    /// Whether Proton offers this game NVAPI; `None` when that is not settled.
    /// Only fit for colouring the sentence beside it.
    pub available: Option<bool>,
    /// The condition the flag is set under, quoted verbatim for the reader to
    /// apply.
    pub condition: Vec<String>,
}

impl Answer {
    /// A game that is not a Steam application, so Steam's Proton policy does not
    /// apply. The sentence names the launcher.
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
    /// The NVAPI answer for a game from any launcher; only a Steam application
    /// has a prefix to read. `root` is where its launch options live.
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
