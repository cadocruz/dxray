//! Finding a Steam install, its libraries and the games in them.
//!
//! The IO half of Steam discovery, kept as thin as [`evidence`](crate::evidence)
//! is: it opens files, hands the bytes to [`vdf`], and does the one
//! thing that cannot be done without a disk — deciding which candidate paths
//! actually exist. Every judgement about what the parsed keys *mean* is written
//! out below in plain functions so it can be read and argued with.
//!
//! The native, Flatpak, Snap and mounted-Distrobox candidate paths have been
//! exercised against one real Steam installation. The tests still carry the
//! portability burden: they prove the file-layout assumptions and edge cases
//! without claiming every Steam layout has been observed.
//!
//! Three things here exist because the obvious version quietly finds nothing:
//!
//! **`libraryfolders.vdf` has had two incompatible schemas.** The old one maps
//! `"1"` straight to a path string; the current one maps `"1"` to a block with
//! a `"path"` key. Reading only one of them finds no extra libraries on half
//! the installs in the world and reports that as a machine with one library.
//!
//! **Not every key in that file is a library.** The old schema mixes
//! `"TimeNextStatsReport"` and `"ContentStatsID"` in beside the numbered
//! entries. Anything whose key is not a plain number is skipped, or the
//! scanner ends up chasing a timestamp as though it were a directory.
//!
//! **The common roots are usually the same directory.** `~/.steam/steam`,
//! `~/.steam/root` and `~/.local/share/Steam` are symlinks to one place on a
//! normal install. Without resolving them, every game is found and reported
//! three times.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::launcher::{Catalogue, Identity, Libraries, Origin};
use crate::vdf::{self, Object, Value};

pub mod launch;

/// One installed game. Steam's manifests and Heroic's caches describe the same
/// thing, so they hand back the same type — see [`launcher`](crate::launcher).
pub use crate::launcher::Game;

/// The launcher this module implements, and the name its messages carry.
pub const ORIGIN: Origin = Origin::new("steam", "Steam");

/// Steam as a [`Launcher`](crate::launcher::Launcher).
///
/// A unit struct rather than a set of free functions with a marker, because a
/// trait object needs something to be. The free functions below stay the
/// module's real API; the adapter only collapses their typed errors into the
/// worded lists the trait promises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Steam;

/// The one Steam launcher, for [`launcher::all`](crate::launcher::all).
pub static STEAM: Steam = Steam;

impl crate::launcher::Launcher for Steam {
    fn origin(&self) -> Origin {
        ORIGIN
    }

    fn roots(&self) -> Vec<PathBuf> {
        roots()
    }

    fn candidate_roots(&self) -> Vec<PathBuf> {
        candidate_roots()
    }

    /// A root whose index cannot be read yields no libraries and one problem.
    ///
    /// What the collapse gives up is telling "this root's index is unusable"
    /// from "one library the index named is unusable". Both arrive as a problem
    /// against the root, and nothing in [`Libraries`] lets a consumer branch on
    /// which of the two it was. The message names the file either way, so the
    /// sentence a person reads is the same; what is gone is the distinction a
    /// program could act on.
    ///
    /// This used to claim the collapse was lossless for every caller there is.
    /// That is a claim about files this one cannot see, and the commit that
    /// gave the trait its second consumer said so itself: the consumers had
    /// begun labelling failures by where they arrived from, one word apart. A
    /// caller that needs the distinction back calls [`libraries`] directly and
    /// gets the typed [`Error`]; the trait implementation is the wording, not
    /// the API.
    fn libraries(&self, root: &Path) -> Libraries {
        match libraries(root) {
            Ok(index) => Libraries {
                paths: index.libraries,
                notes: index.notes.iter().map(ToString::to_string).collect(),
                problems: Vec::new(),
            },
            Err(error) => Libraries {
                paths: Vec::new(),
                notes: Vec::new(),
                problems: vec![error.to_string()],
            },
        }
    }

    fn games(&self, library: &Path) -> Catalogue {
        match games(library) {
            Ok(scan) => Catalogue {
                games: scan.games,
                problems: scan.problems.iter().map(ToString::to_string).collect(),
                notes: Vec::new(),
            },
            Err(error) => Catalogue {
                games: Vec::new(),
                problems: vec![error.to_string()],
                notes: Vec::new(),
            },
        }
    }
}

/// What can go wrong while reading a Steam directory.
///
/// Every variant carries the file it came from. Without that, "missing key
/// installdir" sends a person looking through several hundred manifests.
#[derive(Debug)]
pub enum Error {
    /// The file could not be read.
    Io { path: PathBuf, source: io::Error },
    /// The file was read but is not valid `KeyValues`.
    Vdf { path: PathBuf, source: vdf::Error },
    /// The file parsed, but a key this code needs is not in it.
    MissingKey { path: PathBuf, key: &'static str },
    /// The key is there and its content cannot be used.
    BadValue {
        path: PathBuf,
        key: &'static str,
        value: String,
    },
    /// An index file exists and holds nothing at all.
    ///
    /// Its own variant because the alternative is the failure this crate was
    /// written to refuse: an empty `libraryfolders.vdf` handed back as "one
    /// library, no problems" is a truncated file reported as a healthy machine.
    /// The parser's job is to say what is in the file; deciding that *nothing*
    /// is suspicious is this layer's.
    NoLibraries { path: PathBuf },
}

impl Error {
    /// The file the failure is about.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Io { path, .. }
            | Self::Vdf { path, .. }
            | Self::MissingKey { path, .. }
            | Self::BadValue { path, .. }
            | Self::NoLibraries { path } => path,
        }
    }
}

/// How much of a rejected value to quote back.
///
/// A manifest is an untrusted file. Printing an unbounded value means a
/// corrupt one can push megabytes through a terminal, and the first line of it
/// is all anybody reads anyway.
const VALUE_EXCERPT: usize = 60;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Vdf { path, source } => write!(f, "{}: {source}", path.display()),
            Self::MissingKey { path, key } => {
                write!(f, "{}: no \"{key}\" key", path.display())
            }
            Self::BadValue { path, key, value } => {
                let excerpt: String = value.chars().take(VALUE_EXCERPT).collect();
                let ellipsis = if excerpt.len() < value.len() {
                    "..."
                } else {
                    ""
                };
                write!(
                    f,
                    "{}: unusable \"{key}\" value {excerpt:?}{ellipsis}",
                    path.display()
                )
            }
            Self::NoLibraries { path } => write!(
                f,
                "{}: the library index is empty, which is what a truncated or \
                 overwritten one looks like",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Vdf { source, .. } => Some(source),
            Self::MissingKey { .. } | Self::BadValue { .. } | Self::NoLibraries { .. } => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Every candidate Steam install on this machine that exists.
///
/// Duplicates are removed by resolving symlinks, because the usual Linux
/// install has three of these names pointing at one directory and a caller
/// that trusts the list would report every game three times. The first spelling
/// of each real directory is the one kept, so `~/.steam/steam` wins over the
/// path it resolves to and the output stays recognisable.
///
/// An empty result means no Steam install was found *at a path this code knows
/// to look at*. It does not mean there is none: a Steam moved somewhere custom
/// is invisible here. See [`candidate_roots`] for what is searched.
#[must_use]
pub fn roots() -> Vec<PathBuf> {
    let mut out = existing_roots(candidate_roots());
    // A mounted Distrobox home is a fallback, not a second account to merge
    // into an ordinary host scan.  This keeps a chosen native/Flatpak/custom
    // Steam authoritative and makes callers that isolate HOME deterministic.
    if out.is_empty() {
        out = existing_roots(container_candidate_roots());
    }
    out
}

/// The candidates that are Steam installs, first spelling of each kept.
///
/// One pass, compared by [`identity`]. A second pass comparing the paths
/// literally used to follow this one and could never remove anything, since
/// two candidates spelled the same way answer `identity` the same way.
fn existing_roots(candidates: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for candidate in candidates {
        if !is_root(&candidate) {
            continue;
        }
        if seen.insert(identity(&candidate)) {
            out.push(candidate);
        }
    }
    out
}

/// Every path [`roots`] considers, existing or not, in priority order.
///
/// Public so that a caller which found nothing can say *where* it looked,
/// which is the difference between a useful message and "no Steam found".
///
/// **Unverified.** This list is the conventional one; it has not been checked
/// against an install. A Steam in a custom prefix, a non-default Flatpak data
/// directory, or a Snap install will not be here. On Windows the authoritative
/// answer lives in the registry under `HKCU\Software\Valve\Steam\SteamPath`,
/// which this crate does not read — see the module docs.
#[must_use]
pub fn candidate_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();

    #[cfg(unix)]
    {
        if let Some(home) = std::env::var_os("HOME") {
            out.extend(unix_home_roots(Path::new(&home)));
        }
        out.push(PathBuf::from("/usr/local/games/Steam"));
        // A custom root cannot be inferred from a directory name. Honour an
        // explicit, platform-native list rather than walking mounted drives.
        if let Some(roots) = std::env::var_os("DXRAY_STEAM_ROOT") {
            out.extend(std::env::split_paths(&roots));
        }
    }

    #[cfg(windows)]
    {
        // These three are GUESSES at the default install locations, and
        // nothing more. They cover a Steam that was installed where the
        // installer offered to put it, and they miss every Steam that was not.
        //
        // The authoritative answer is the registry value
        // `HKCU\Software\Valve\Steam\SteamPath`, which this crate deliberately
        // does NOT read: it would cost either a new dependency or a shell-out
        // to `reg.exe`, and this tool's subject is Steam under Proton on Linux,
        // where Windows support is a bonus. If you are running this on Windows
        // and it cannot find your library, that registry value is where your
        // Steam actually is, and reading it is the change to make.
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(dir) = std::env::var_os(var) {
                out.push(Path::new(&dir).join("Steam"));
            }
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            out.push(Path::new(&home).join("Steam"));
        }
    }

    out
}

/// Steam paths below bounded, conventional Distrobox homes on mounted volumes.
#[cfg(unix)]
fn container_candidate_roots() -> Vec<PathBuf> {
    crate::paths::container_homes()
        .into_iter()
        .flat_map(|home| unix_home_roots(&home))
        .collect()
}

#[cfg(not(unix))]
fn container_candidate_roots() -> Vec<PathBuf> {
    Vec::new()
}

/// The Steam directories that can sit under a Unix home, in priority order.
///
/// Split out from [`candidate_roots`] so the path building can be tested
/// without an environment. Kept private: the list is a guess, and exporting it
/// would invite somebody to treat it as a fact.
#[cfg(unix)]
fn unix_home_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        home.join(".steam/steam"),
        home.join(".steam/root"),
        home.join(".local/share/Steam"),
        // Flatpak keeps the whole home under its own prefix.
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        // Some Flatpak revisions have used the app data directory directly.
        home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
        // Snap's writable data home is separate from the normal XDG home.
        home.join("snap/steam/common/.local/share/Steam"),
    ];
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(Path::new(&data_home).join("Steam"));
    }
    roots
}

/// A directory is a Steam root only when it contains a Steam library or its
/// library index. A leftover `Steam` directory must not become a silent empty
/// library merely because its name happens to match a candidate.
fn is_root(path: &Path) -> bool {
    path.join("steamapps").is_dir() || LIBRARY_INDEX.iter().any(|index| path.join(index).is_file())
}

/// Where `libraryfolders.vdf` is looked for, in order.
///
/// Two places because Steam has written it to both over the years and neither
/// is guaranteed to be the one present. Trying only `steamapps` on an install
/// that keeps it in `config` finds one library and calls that the whole
/// machine. **Unverified against a real install.**
const LIBRARY_INDEX: [&str; 2] = ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"];

/// Something true about an answer that makes it worth less than it looks.
///
/// A note is not a failure. It is the difference between an answer that is
/// right and an answer that is right *and complete*, and it exists because the
/// two are not distinguishable from the file alone.
///
/// Paired with [`Error`] across this module by severity, and the pairing is the
/// whole point: [`Scan::problems`] and [`Index::notes`] look alike because both
/// ride alongside a partial answer, but a problem means something could not be
/// read and should move an exit code, while a note means everything was read
/// and there may simply be less of it than the user expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// The index parsed but declared no numbered entries at all.
    ///
    /// **This is genuinely ambiguous and cannot be resolved from the file.** An
    /// old-schema index on a single-library machine holds only bookkeeping keys
    /// and no numbered entries, which is a healthy state. A current-schema
    /// index truncated or half-overwritten just past its bookkeeping keys looks
    /// byte for byte the same and means every library on every other drive has
    /// been lost.
    ///
    /// Telling them apart would take a list of known bookkeeping key names, and
    /// that is the guess-list this crate refuses everywhere else — see the note
    /// against filtering non-game manifests in [`games`]. So the ambiguity is
    /// reported instead of decided. The sentence this renders to is true of
    /// both cases without claiming to know which one is in front of it.
    NoLibraryEntries { path: PathBuf },
}

impl Note {
    /// The file the note is about.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::NoLibraryEntries { path } => path,
        }
    }
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLibraryEntries { path } => write!(
                f,
                "{} declared no library entries; only the Steam root is being scanned. \
                 On an old single-library install this is normal; on a newer one it means \
                 the index lost its entries.",
                path.display()
            ),
        }
    }
}

/// The library directories belonging to one Steam root, and what the caller
/// should know about how complete that list is.
///
/// Shaped to match [`Scan`] because a reader will meet both: an answer, plus
/// the things beside it that qualify the answer. They differ in severity, and
/// deliberately so — see [`Note`].
#[derive(Debug, Default)]
pub struct Index {
    /// The root first, then whatever the index declared, deduplicated.
    pub libraries: Vec<PathBuf>,
    /// Reasons this list may be shorter than the machine really has.
    pub notes: Vec<Note>,
}

/// The library directories belonging to `root`, the root itself first.
///
/// The root is always a library — its own `steamapps/common` holds games
/// whether or not the index file mentions it — so it leads the list and is
/// deduplicated against whatever the index says.
///
/// Paths are returned **as the file declared them**, without checking that they
/// exist. A library on an external drive that is currently unplugged is still
/// what Steam believes, and silently dropping it turns "your D: drive is not
/// mounted" into "you own fewer games than you do". The failure surfaces from
/// [`games`] instead, naming the path.
///
/// An index that declares no numbered entries yields the root and a
/// [`Note::NoLibraryEntries`]. That case used to be indistinguishable from a
/// healthy single-library install, which meant a truncated index reported
/// perfect health — the exact silent-wrong-answer shape this crate exists to
/// avoid. It is reported rather than decided because from the file alone it
/// cannot be decided.
///
/// # Errors
///
/// Fails if the index file exists but cannot be read or parsed, if a numbered
/// entry in it has no usable path, or if the file holds nothing at all
/// ([`Error::NoLibraries`]). A *missing* index file is not an error: it yields
/// `[root]`, because the root's own library does not depend on the index to be
/// there.
pub fn libraries(root: &Path) -> Result<Index> {
    let mut index = Index::default();
    if root.join("steamapps").is_dir() {
        index.libraries.push(root.to_path_buf());
    }

    let Some((path, text)) = read_first(root, &LIBRARY_INDEX)? else {
        return Ok(index);
    };
    let parsed = vdf::parse(&text).map_err(|source| Error::Vdf {
        path: path.clone(),
        source,
    })?;

    // A file with nothing in it at all is still an error rather than a note.
    // Every index, of either schema, has *something* in it — the old one its
    // bookkeeping keys, the new one at least the root as entry "0" — so an
    // empty one has no reading under which it is healthy. Unverified against a
    // real freshly-created install, which is the one case that could prove this
    // wrong.
    let root_object = parsed.as_object().filter(|o| !o.is_empty());
    let Some(root_object) = root_object else {
        return Err(Error::NoLibraries { path });
    };
    let folders = inner_block_of(root_object, "libraryfolders", &path)?;
    if folders.is_empty() {
        return Err(Error::NoLibraries { path });
    }

    let declared = library_paths(folders, &path)?;
    // Bookkeeping keys and nothing else. Reported, not judged: see `Note`.
    if declared.is_empty() {
        index.notes.push(Note::NoLibraryEntries { path });
    }
    for path in declared {
        index.libraries.push(PathBuf::from(path));
    }

    dedup_paths(&mut index.libraries);
    Ok(index)
}

/// Pulls the library paths out of a parsed `libraryfolders.vdf`, in file order.
///
/// Handles both schemas, and skips every entry whose key is not a plain
/// number, which is what keeps `TimeNextStatsReport` out of the results.
///
/// Every entry is walked rather than looked up by index, because these keys are
/// not unique: `KeyValues` is a list and a file that says `"1"` twice has two
/// entries. A lookup by index would take one and lose the other, and losing one
/// costs a whole Steam library without saying anything.
fn library_paths(folders: &Object, path: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for (key, value) in folders {
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let declared = match value {
            // The old schema: the index maps straight to the path.
            Value::String(declared) => declared.clone(),
            // The current schema: the index maps to a block describing the
            // library, and the path is one key inside it.
            Value::Object(entry) => entry
                .get_str("path")
                .ok_or_else(|| Error::MissingKey {
                    path: path.to_path_buf(),
                    key: "path",
                })?
                .to_owned(),
        };
        // An empty path would resolve to the working directory, which then gets
        // scanned as though Steam had said it was a library.
        if declared.is_empty() {
            return Err(Error::BadValue {
                path: path.to_path_buf(),
                key: "path",
                value: String::new(),
            });
        }
        out.push(declared);
    }
    Ok(out)
}

/// What one library turned out to hold: the games, and the manifests that
/// could not be read.
///
/// Two lists rather than a `Vec<Result<Game>>`, for one reason: every caller
/// wants the partition, and the `Vec<Result<_>>` shape makes
/// `.filter_map(Result::ok)` the shortest thing to write. That one call
/// silently drops every corrupt manifest, which is precisely the failure this
/// type exists to prevent. Naming both lists makes ignoring the second one a
/// visible decision instead of a default.
#[derive(Debug, Default)]
pub struct Scan {
    /// Ordered by application id, so two scans of one machine can be diffed
    /// against each other. `read_dir` promises nothing about order, and a
    /// report that shuffles itself between runs cannot show what changed.
    pub games: Vec<Game>,
    /// One entry per manifest that could not be read, parsed or understood,
    /// each naming its file.
    pub problems: Vec<Error>,
}

/// Every game installed in `library`, and every manifest that defeated the
/// reader.
///
/// A corrupt manifest does not stop the scan. It used to, and that was wrong
/// for the same reason a scan of seven thousand files does not abort on the
/// first unreadable one: losing every other game in the library is a far bigger
/// wrong answer than the one bad file it was protecting against. What must not
/// happen is losing the bad file *quietly*, which is why the failures come back
/// beside the games rather than being dropped.
///
/// Manifests for things that are not games — Proton builds, Steamworks
/// redistributables, soundtracks — are **not** filtered out, and no filter
/// should be added here. Any list of names to exclude is a guess that will one
/// day hide a real game. The next stage decides which executable in a directory
/// is the game, and a redistributable folder has no game executable in it, so
/// the problem answers itself one layer up where there is evidence instead of a
/// name list.
///
/// # Errors
///
/// Fails only if `library/steamapps` cannot be listed at all — the library is
/// not there, or cannot be read. That is a statement about the library rather
/// than about any file in it, and it is the one case where there is no partial
/// answer worth returning.
pub fn games(library: &Path) -> Result<Scan> {
    let steamapps = library.join("steamapps");
    let entries = fs::read_dir(&steamapps).map_err(|source| Error::Io {
        path: steamapps.clone(),
        source,
    })?;

    let mut scan = Scan::default();
    let mut manifests = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => {
                if is_manifest_name(&entry.file_name().to_string_lossy()) {
                    manifests.push(entry.path());
                }
            }
            // One unreadable directory entry among many. Recorded rather than
            // fatal, for the same reason as a bad manifest.
            Err(source) => scan.problems.push(Error::Io {
                path: steamapps.clone(),
                source,
            }),
        }
    }
    // Sorted before reading so the problems come back in a stable order too.
    manifests.sort();

    for manifest in manifests {
        match read_manifest(&manifest, library) {
            Ok(game) => scan.games.push(game),
            Err(problem) => scan.problems.push(problem),
        }
    }
    scan.games.sort_by_key(|g| g.identity.steam_appid());
    Ok(scan)
}

/// Whether a directory entry is an application manifest.
///
/// Matched case-insensitively because these files live on NTFS as often as on
/// ext4, and a Windows install written as `appmanifest_570.ACF` after a restore
/// is still a manifest.
fn is_manifest_name(name: &str) -> bool {
    // `get` rather than slicing: a name whose twelfth byte falls inside a
    // multi-byte character would panic on a slice, and a hostile directory can
    // contain any name at all.
    name.get(..12)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("appmanifest_"))
        && Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("acf"))
}

/// Reads one `.acf` into a [`Game`].
fn read_manifest(path: &Path, library: &Path) -> Result<Game> {
    let text = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let parsed = vdf::parse(&text).map_err(|source| Error::Vdf {
        path: path.to_path_buf(),
        source,
    })?;
    let state = inner_block(&parsed, "AppState", path)?;

    let appid_text = required(state, "appid", path)?;
    let appid: u32 = appid_text.trim().parse().map_err(|_| Error::BadValue {
        path: path.to_path_buf(),
        key: "appid",
        value: appid_text.to_owned(),
    })?;

    let install_dir_name = required(state, "installdir", path)?;
    check_install_dir(install_dir_name, path)?;

    Ok(Game {
        identity: Identity::SteamApp(appid),
        name: required(state, "name", path)?.to_owned(),
        // Resolved rather than left as the bare `installdir` name because that
        // name is meaningless without the library it belongs to, and every
        // caller would otherwise rebuild this join and one of them would get it
        // wrong. Not checked for existence — see `Game::install_dir`.
        install_dir: library
            .join("steamapps")
            .join("common")
            .join(install_dir_name),
        origin: ORIGIN,
    })
}

/// Rejects an `installdir` that is not a single directory name.
///
/// A manifest is a file on disk that anything can write. `Path::join` with an
/// absolute value throws away everything to its left, and a value of `..`
/// climbs out of the library, so either would hand back a path outside
/// `steamapps/common` that reads as though it were inside it. Real values are
/// always one plain name.
fn check_install_dir(value: &str, path: &Path) -> Result<()> {
    let name = Path::new(value);
    let mut parts = name.components();
    let single =
        matches!(parts.next(), Some(std::path::Component::Normal(_))) && parts.next().is_none();
    if single {
        return Ok(());
    }
    Err(Error::BadValue {
        path: path.to_path_buf(),
        key: "installdir",
        value: value.to_owned(),
    })
}

/// A required leaf key, or an error naming it and the file it is missing from.
fn required<'a>(block: &'a Object, key: &'static str, path: &Path) -> Result<&'a str> {
    let value = block.get_str(key).ok_or_else(|| Error::MissingKey {
        path: path.to_path_buf(),
        key,
    })?;
    if value.is_empty() {
        // An empty `installdir` joins to the bare `steamapps/common`, and an
        // empty `name` prints as a blank line. Both read as data rather than
        // as the damage they are.
        return Err(Error::BadValue {
            path: path.to_path_buf(),
            key,
            value: String::new(),
        });
    }
    Ok(value)
}

/// The single named block a Steam file wraps everything in.
///
/// Falls back to the sole top-level block when the name does not match, because
/// the name has changed before — `libraryfolders.vdf` was `LibraryFolders` —
/// and a file with exactly one block in it leaves no room for ambiguity about
/// which one was meant. If there is more than one, the name has to match.
fn inner_block<'a>(parsed: &'a Value, name: &'static str, path: &Path) -> Result<&'a Object> {
    let root = parsed.as_object().ok_or_else(|| Error::MissingKey {
        path: path.to_path_buf(),
        key: name,
    })?;
    inner_block_of(root, name, path)
}

/// [`inner_block`] for a caller that has already unwrapped the root object.
fn inner_block_of<'a>(root: &'a Object, name: &'static str, path: &Path) -> Result<&'a Object> {
    if let Some(block) = root.get_object(name) {
        return Ok(block);
    }
    let mut blocks = root.iter().filter_map(|(_, v)| v.as_object());
    match (blocks.next(), blocks.next()) {
        (Some(only), None) => Ok(only),
        _ => Err(Error::MissingKey {
            path: path.to_path_buf(),
            key: name,
        }),
    }
}

/// Reads the first of `relatives` under `root` that is there.
///
/// A file that is absent is `Ok(None)`; a file that is present and unreadable
/// is an error. The distinction matters: "there is no index" is a normal state
/// for a fresh install, while "the index is there and I was refused" is a
/// scan that did not happen and must not be reported as one that found nothing.
fn read_first(root: &Path, relatives: &[&str]) -> Result<Option<(PathBuf, String)>> {
    for relative in relatives {
        let path = root.join(relative);
        match fs::read_to_string(&path) {
            Ok(text) => return Ok(Some((path, text))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(Error::Io { path, source }),
        }
    }
    Ok(None)
}

/// Removes repeats, keeping the first spelling of each.
///
/// Compared by [`identity`], not by text, and that is the whole point of this
/// function. On a normal Linux install the root is `~/.steam/steam`, which is a
/// symlink, while the index file inside it declares the same library by its
/// resolved name `~/.local/share/Steam`. The two strings are different and the
/// directory is one, so a textual comparison keeps both and every game in the
/// main library is reported twice.
fn dedup_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = HashSet::new();
    paths.retain(|p| seen.insert(identity(p)));
}

/// What makes two paths the same directory.
///
/// The resolved path when it can be resolved, and the path as written when it
/// cannot. The fallback is what keeps a library on an unmounted drive in the
/// list: `canonicalize` fails on a path that is not there, and treating that
/// failure as "not a directory" would drop the entry silently — the exact
/// failure this module exists to refuse.
fn identity(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{Error, Game, Identity, Note, ORIGIN, candidate_roots, games, libraries, roots};
    use crate::testutil::TempDir;
    use std::path::{Path, PathBuf};

    /// Builds a Steam-shaped directory tree. Everything these tests know about
    /// the real layout is written here, once, so a reader can check the
    /// assumption in one place rather than in fifteen.
    struct Install(TempDir);

    impl Install {
        fn new(tag: &str) -> Self {
            Self(TempDir::new(tag))
        }

        fn root(&self) -> &Path {
            self.0.path()
        }

        fn index(&self, contents: &str) -> &Self {
            self.0.write("steamapps/libraryfolders.vdf", contents);
            self
        }

        fn manifest(&self, appid: u32, contents: &str) -> &Self {
            self.0
                .write(&format!("steamapps/appmanifest_{appid}.acf"), contents);
            self
        }
    }

    /// Scans `library` and returns its single reported problem.
    ///
    /// Asserts there is exactly one, because a test that broke one manifest and
    /// got two failures is testing something other than what it says it is.
    fn only_problem(library: &Path) -> Error {
        let mut scan = games(library).expect("the library itself is readable");
        assert_eq!(
            scan.problems.len(),
            1,
            "expected exactly one bad manifest, got {:?}",
            scan.problems
        );
        scan.problems.remove(0)
    }

    /// A well-formed manifest, so that each test can break exactly one thing.
    fn manifest(appid: u32, name: &str, installdir: &str) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{appid}\"\n\t\"name\"\t\t\"{name}\"\n\
             \t\"installdir\"\t\t\"{installdir}\"\n\t\"StateFlags\"\t\t\"4\"\n}}\n"
        )
    }

    #[test]
    fn the_current_library_schema_maps_each_index_to_a_block_with_a_path() {
        // What Steam writes today. Reading only the old schema here finds no
        // extra libraries at all and calls that a one-library machine.
        let install = Install::new("new-schema");
        install.index(
            "\"libraryfolders\"\n{\n\
             \t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/u/.local/share/Steam\"\n\
             \t\t\"apps\"\n\t\t{\n\t\t\t\"570\"\t\t\"12345678\"\n\t\t}\n\t}\n\
             \t\"1\"\n\t{\n\t\t\"path\"\t\t\"/mnt/games/SteamLibrary\"\n\t}\n}\n",
        );

        let found = libraries(install.root()).expect("a well-formed index parses");

        assert!(found.notes.is_empty(), "a healthy index needs no caveat");
        assert_eq!(
            found.libraries,
            [
                install.root().to_path_buf(),
                PathBuf::from("/home/u/.local/share/Steam"),
                PathBuf::from("/mnt/games/SteamLibrary"),
            ],
            "the root leads, then the declared libraries in file order"
        );
    }

    #[test]
    fn the_old_library_schema_maps_each_index_straight_to_a_path_string() {
        // Installs that have not been touched in years still have this file,
        // and a reader that only knows the block form finds nothing in it.
        let install = Install::new("old-schema");
        install.index(
            "\"LibraryFolders\"\n{\n\
             \t\"TimeNextStatsReport\"\t\t\"1580000000\"\n\
             \t\"ContentStatsID\"\t\t\"-1234567890123456789\"\n\
             \t\"1\"\t\t\"/mnt/games/SteamLibrary\"\n\
             \t\"2\"\t\t\"D:\\\\SteamLibrary\"\n}\n",
        );

        let found = libraries(install.root()).expect("the old schema parses");

        assert!(
            found.notes.is_empty(),
            "entries were declared, so no caveat"
        );
        assert_eq!(
            found.libraries,
            [
                install.root().to_path_buf(),
                PathBuf::from("/mnt/games/SteamLibrary"),
                PathBuf::from(r"D:\SteamLibrary"),
            ],
            "the non-numeric keys are not libraries, and the escaped path unescapes"
        );
    }

    #[test]
    fn a_library_declared_on_a_drive_that_is_not_mounted_is_still_reported() {
        // Dropping it silently turns "your external drive is unplugged" into
        // "you own fewer games than you do". The failure has to surface where
        // it can name the path, which is when the library is scanned.
        let install = Install::new("unmounted");
        install.index("\"libraryfolders\" { \"0\" { \"path\" \"/dxray-no-such-library\" } }");

        let found = libraries(install.root()).expect("parses");
        assert!(
            found
                .libraries
                .contains(&PathBuf::from("/dxray-no-such-library")),
            "a declared library is reported as declared, got {found:?}"
        );

        let error = games(Path::new("/dxray-no-such-library")).expect_err("nothing to scan");
        assert!(
            matches!(error, Error::Io { .. }),
            "and scanning it says so, got {error}"
        );
        assert!(
            error.to_string().contains("/dxray-no-such-library"),
            "naming the path, got {error}"
        );
    }

    #[test]
    fn a_root_that_is_also_listed_in_its_own_index_is_not_scanned_twice() {
        // The current schema lists the root as entry "0". Without the dedup
        // every game in the main library appears in the report twice.
        let install = Install::new("dedup");
        let root = install.root().display().to_string();
        install.index(&format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{root}\" }} }}"
        ));

        let found = libraries(install.root()).expect("parses");

        assert_eq!(
            found.libraries.len(),
            1,
            "one library, not two, got {found:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_library_reached_by_a_symlink_is_the_same_library_as_its_target() {
        // This is what every normal Linux install looks like: the root is
        // `~/.steam/steam`, a symlink, while the index inside it declares the
        // same directory by its resolved name. The two strings differ and the
        // directory is one, so a textual dedup keeps both and every game in the
        // main library is listed twice.
        let dir = TempDir::new("symlink");
        let real = dir.dir("real");
        std::fs::create_dir_all(real.join("steamapps")).expect("steamapps");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        std::fs::write(
            real.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
                real.display()
            ),
        )
        .expect("index");

        let found = libraries(&link).expect("parses");

        assert_eq!(
            found.libraries.len(),
            1,
            "the link and its target are one library, got {found:?}"
        );
    }

    #[test]
    fn an_install_with_no_index_file_still_reports_its_own_library() {
        // A fresh install, or one whose index has been deleted. Erroring here
        // would report a machine with games on it as unreadable.
        let install = Install::new("no-index");
        install.0.dir("steamapps");

        assert_eq!(
            libraries(install.root())
                .expect("a missing index is not a failure")
                .libraries,
            [install.root().to_path_buf()]
        );
    }

    #[test]
    fn a_truncated_index_is_a_failure_and_not_a_machine_with_one_library() {
        // The whole point. A half-parsed index reads as "no other libraries",
        // which is a wrong answer that looks exactly like a right one.
        let install = Install::new("truncated-index");
        install.index("\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\"/mnt/g\"\n");

        let error = libraries(install.root()).expect_err("truncation must not parse");

        assert!(
            matches!(error, Error::Vdf { .. }),
            "reported as a parse failure, got {error}"
        );
        assert!(
            error.to_string().contains("libraryfolders.vdf"),
            "naming the file, got {error}"
        );
    }

    #[test]
    fn an_index_file_that_exists_and_says_nothing_is_a_failure() {
        // An empty file parses cleanly — it is a well-formed empty document —
        // and handing back [root] for it would report a machine whose index
        // was truncated or overwritten as a healthy one-library install. The
        // parser says what is in the file; this layer decides nothing is wrong.
        for contents in ["", "   \n\t\n", "// everything was lost\n"] {
            let install = Install::new("empty-index");
            install.index(contents);

            let error = libraries(install.root()).expect_err(&format!(
                "an index of {contents:?} must not read as success"
            ));

            assert!(matches!(error, Error::NoLibraries { .. }), "got {error}");
        }
    }

    #[test]
    fn an_index_block_with_no_entries_at_all_is_a_failure() {
        // `"libraryfolders" {}` is what the file looks like after it has been
        // half rewritten. Even the current schema always lists the root as
        // entry "0", so a block with nothing in it is never a healthy state.
        let install = Install::new("empty-block");
        install.index("\"libraryfolders\"\n{\n}\n");

        let error = libraries(install.root()).expect_err("an empty block is not one library");

        assert!(matches!(error, Error::NoLibraries { .. }), "got {error}");
    }

    #[test]
    fn an_index_that_declares_no_entries_is_told_apart_from_one_that_declares_some() {
        // The defect this pair exists to hold shut. Before the note, a
        // bookkeeping-only index and a healthy current-schema index produced
        // byte-identical output — "1 library, exit 0" — so a new-schema file
        // truncated just past its bookkeeping keys lost every library on every
        // other drive and reported perfect health.
        //
        // Both halves are asserted on purpose. A test for the note alone would
        // pass on an implementation that noted every file, which would be just
        // as useless in the other direction.
        let bookkeeping_only = Install::new("no-entries");
        bookkeeping_only.index("\"libraryfolders\" { \"contentstatsid\" \"-123456789\" }");

        let declares_one = Install::new("one-entry");
        declares_one.index(&format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
            declares_one.root().display()
        ));

        let noted = libraries(bookkeeping_only.root()).expect("it parses; it is just thin");
        let clean = libraries(declares_one.root()).expect("parses");

        assert_eq!(
            noted.notes.len(),
            1,
            "an index that declared nothing must say so"
        );
        assert!(
            matches!(noted.notes[0], Note::NoLibraryEntries { .. }),
            "got {:?}",
            noted.notes
        );
        assert!(
            clean.notes.is_empty(),
            "an index that declared an entry must NOT be noted, got {:?}",
            clean.notes
        );

        // And the note does not cost the answer: the root is still a library.
        assert_eq!(noted.libraries, [bookkeeping_only.root().to_path_buf()]);
    }

    #[test]
    fn the_note_names_the_file_and_claims_nothing_about_which_case_it_is() {
        // The sentence has to be true of a healthy old install and of a
        // truncated new one at the same time, because from the file alone the
        // two cannot be told apart. Anything that picked a side would be the
        // guess this module refuses to make.
        let note = Note::NoLibraryEntries {
            path: PathBuf::from("/s/steamapps/libraryfolders.vdf"),
        };
        let text = note.to_string();

        assert!(text.contains("/s/steamapps/libraryfolders.vdf"), "{text}");
        assert!(
            text.contains("only the Steam root is being scanned"),
            "it must say what the consequence is, got {text}"
        );
        assert!(
            text.contains("normal") && text.contains("lost its entries"),
            "both readings must be offered, neither chosen, got {text}"
        );
    }

    #[test]
    fn an_old_schema_index_with_only_stats_keys_is_a_healthy_one_library_machine() {
        // Deliberately NOT an error. An old client on a machine with a single
        // library writes exactly this — two bookkeeping keys and no numbered
        // entries — and failing it would break every such install. It does earn
        // a note, because the same bytes are also what a truncated new-schema
        // file looks like and this module will not guess between them.
        // Unverified: no old client was available to check.
        let install = Install::new("stats-only");
        install.index(
            "\"LibraryFolders\"\n{\n\t\"TimeNextStatsReport\"\t\"1580000000\"\n\
             \t\"ContentStatsID\"\t\"-1\"\n}\n",
        );

        let found = libraries(install.root()).expect("this is a normal file, not a failure");

        assert_eq!(
            found.libraries,
            [install.root().to_path_buf()],
            "the root library, and no error"
        );
        assert!(
            matches!(found.notes.as_slice(), [Note::NoLibraryEntries { .. }]),
            "the install keeps working and sees a caveat, got {:?}",
            found.notes
        );
    }

    #[test]
    fn a_repeated_index_key_costs_no_library() {
        // KeyValues is a list, not a map, so "1" can appear twice. Looking the
        // entries up by index would silently keep one and drop the other, and
        // a dropped entry is a whole library's worth of games gone with no
        // message. This is the sharpest silent failure in the format.
        let install = Install::new("repeated-key");
        install.index(
            "\"libraryfolders\"\n{\n\
             \t\"1\"\t{ \"path\" \"/mnt/a\" }\n\
             \t\"1\"\t{ \"path\" \"/mnt/b\" }\n}\n",
        );

        let found = libraries(install.root()).expect("parses");

        assert!(
            found.libraries.contains(&PathBuf::from("/mnt/a"))
                && found.libraries.contains(&PathBuf::from("/mnt/b")),
            "both entries must survive, got {found:?}"
        );
    }

    #[test]
    fn a_numbered_library_entry_with_no_path_key_is_a_failure() {
        // A block that parsed but says nothing about where the library is.
        // Skipping it would lose a whole drive's worth of games in silence.
        let install = Install::new("no-path");
        install.index("\"libraryfolders\" { \"0\" { \"label\" \"games\" } }");

        let error = libraries(install.root()).expect_err("a library needs a path");

        assert!(
            matches!(error, Error::MissingKey { key: "path", .. }),
            "got {error}"
        );
    }

    #[test]
    fn manifests_become_games_with_their_install_directory_resolved() {
        // The bare `installdir` is meaningless without the library it belongs
        // to, so the join happens once here rather than in every caller.
        let install = Install::new("games");
        install.manifest(570, &manifest(570, "Dota 2", "dota 2 beta"));
        install.manifest(220, &manifest(220, "Half-Life 2", "Half-Life 2"));

        let scan = games(install.root()).expect("the library is readable");

        assert!(
            scan.problems.is_empty(),
            "nothing was wrong with either file"
        );
        assert_eq!(
            scan.games,
            [
                Game {
                    identity: Identity::SteamApp(220),
                    name: "Half-Life 2".to_owned(),
                    install_dir: install.root().join("steamapps/common/Half-Life 2"),
                    origin: ORIGIN,
                },
                Game {
                    identity: Identity::SteamApp(570),
                    name: "Dota 2".to_owned(),
                    install_dir: install.root().join("steamapps/common/dota 2 beta"),
                    origin: ORIGIN,
                },
            ],
            "ordered by appid so two scans can be diffed against each other"
        );
    }

    #[test]
    fn every_download_type_is_listed_because_the_field_does_not_mean_what_it_looks_like() {
        // `DownloadType` looks structural — 0 shared content, 1 a tool, 3 an
        // application the user bought — and a classifier was once built on
        // exactly that reading. On a real machine shipped games were observed
        // carrying 1, the same value Proton Experimental carries, so the field
        // marks real games as tooling and cannot filter anything. This test is
        // the guard against reading it again and concluding otherwise: the
        // inventory holds all four manifests whatever the key says, and no
        // function in this module looks at it.
        let install = Install::new("application-kind");
        install.manifest(10, &manifest_with_download_type(10, "Game", "game", "3"));
        install.manifest(20, &manifest_with_download_type(20, "Tool", "tool", "1"));
        install.manifest(
            30,
            &manifest_with_download_type(30, "Shared", "shared", "0"),
        );
        install.manifest(40, &manifest(40, "Legacy Game", "legacy"));

        let scan = games(install.root()).expect("inventory");
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert_eq!(
            scan.games
                .iter()
                .map(|game| game.identity.clone())
                .collect::<Vec<_>>(),
            vec![
                Identity::SteamApp(10),
                Identity::SteamApp(20),
                Identity::SteamApp(30),
                Identity::SteamApp(40),
            ],
            "the tool, the shared content and the manifest with no DownloadType \
             at all are installed applications like any other"
        );
    }

    #[test]
    fn a_manifest_missing_the_keys_a_game_needs_fails_and_names_the_file() {
        // A manifest without `installdir` is what a partially written one looks
        // like. Treating it as a game with an empty directory would point the
        // next stage at `steamapps/common` and scan the whole library as one
        // game's folder.
        let install = Install::new("missing-keys");
        install.manifest(
            570,
            "\"AppState\"\n{\n\t\"appid\"\t\"570\"\n\t\"name\"\t\"Dota 2\"\n}\n",
        );

        let error = only_problem(install.root());

        assert!(
            matches!(
                error,
                Error::MissingKey {
                    key: "installdir",
                    ..
                }
            ),
            "got {error}"
        );
        assert!(
            error.to_string().contains("appmanifest_570.acf"),
            "naming the file, or a person searches several hundred of them: {error}"
        );
    }

    #[test]
    fn a_manifest_with_an_empty_installdir_is_rejected_rather_than_joined() {
        // `join("")` resolves to the bare `steamapps/common`, which is a real
        // directory and would be reported as this game's folder.
        let install = Install::new("empty-installdir");
        install.manifest(570, &manifest(570, "Dota 2", ""));

        let error = only_problem(install.root());

        assert!(
            matches!(
                error,
                Error::BadValue {
                    key: "installdir",
                    ..
                }
            ),
            "got {error}"
        );
    }

    #[test]
    fn an_installdir_that_climbs_out_of_the_library_is_rejected() {
        // A manifest is a file anything can write, and `join` with an absolute
        // value discards everything to its left. A real `installdir` is always
        // one plain directory name.
        let install = Install::new("escape");
        install.manifest(1, &manifest(1, "Up", "../../../etc"));
        install.manifest(2, &manifest(2, "Absolute", "/etc"));

        let scan = games(install.root()).expect("the library is readable");

        assert!(
            scan.games.is_empty(),
            "neither is a game, got {:?}",
            scan.games
        );
        assert_eq!(
            scan.problems.len(),
            2,
            "both are reported, not just the first"
        );
        assert!(
            scan.problems.iter().all(|p| matches!(
                p,
                Error::BadValue {
                    key: "installdir",
                    ..
                }
            )),
            "got {:?}",
            scan.problems
        );
    }

    #[test]
    fn one_corrupt_manifest_does_not_cost_the_user_every_other_game() {
        // This used to abort the whole library, and that was the wrong trade:
        // losing forty games to protect against one bad file is a far bigger
        // wrong answer than the one it was guarding. The bad file still has to
        // be reported — losing it quietly is the other failure — so both come
        // back together and the caller cannot take one without seeing the other.
        let install = Install::new("partial");
        install.manifest(570, &manifest(570, "Dota 2", "dota 2 beta"));
        install.manifest(220, &manifest(220, "Half-Life 2", "Half-Life 2"));
        install.manifest(
            999,
            "\"AppState\"\n{\n\t\"appid\"\t\"999\"\n\t\"name\"\t\"Trunc",
        );

        let scan = games(install.root()).expect("the library is readable");

        assert_eq!(
            scan.games
                .iter()
                .map(|g| g.identity.clone())
                .collect::<Vec<_>>(),
            [Identity::SteamApp(220), Identity::SteamApp(570)],
            "the readable games survive"
        );
        assert_eq!(scan.problems.len(), 1, "and the broken one is still named");
        assert!(
            scan.problems[0].to_string().contains("appmanifest_999.acf"),
            "got {}",
            scan.problems[0]
        );
    }

    #[test]
    fn a_manifest_whose_appid_is_not_a_number_fails_rather_than_becoming_zero() {
        // `parse().unwrap_or_default()` would give every corrupt manifest the
        // appid 0 and make them collide with each other in the sort.
        let install = Install::new("bad-appid");
        install.manifest(570, &manifest_with_appid("not a number"));

        let error = only_problem(install.root());

        assert!(
            matches!(error, Error::BadValue { key: "appid", .. }),
            "got {error}"
        );
    }

    fn manifest_with_appid(appid: &str) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\"{appid}\"\n\t\"name\"\t\"X\"\n\
             \t\"installdir\"\t\"X\"\n}}\n"
        )
    }

    fn manifest_with_download_type(
        appid: u32,
        name: &str,
        install_dir: &str,
        download_type: &str,
    ) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\"{appid}\"\n\t\"name\"\t\"{name}\"\n\
             \t\"installdir\"\t\"{install_dir}\"\n\t\"DownloadType\"\t\"{download_type}\"\n}}\n"
        )
    }

    #[test]
    fn a_truncated_manifest_is_a_failure_and_not_a_library_with_no_games() {
        // The shape this module was written to refuse. A lenient parse returns
        // an AppState with no installdir, which is indistinguishable from a
        // game that is not installed.
        let install = Install::new("truncated-acf");
        install.manifest(
            570,
            "\"AppState\"\n{\n\t\"appid\"\t\"570\"\n\t\"name\"\t\"Dot",
        );

        let error = only_problem(install.root());

        assert!(matches!(error, Error::Vdf { .. }), "got {error}");
    }

    #[test]
    fn files_in_steamapps_that_are_not_manifests_are_ignored() {
        // `steamapps` also holds `common`, `downloading`, `shadercache`,
        // `sourcemods` and a workshop directory. Trying to parse those as
        // manifests would fail the scan on every real install.
        let install = Install::new("clutter");
        install.manifest(570, &manifest(570, "Dota 2", "dota 2 beta"));
        install.0.dir("steamapps/common/dota 2 beta");
        install.0.dir("steamapps/downloading");
        install
            .0
            .write("steamapps/libraryfolders.vdf", "\"libraryfolders\" {}");

        let scan = games(install.root()).expect("the clutter is skipped");

        assert!(
            scan.problems.is_empty(),
            "clutter is not a problem to report"
        );
        assert_eq!(
            scan.games.len(),
            1,
            "only the manifest counted, got {:?}",
            scan.games
        );
    }

    #[test]
    fn a_library_with_no_steamapps_directory_is_an_error_rather_than_no_games() {
        // Distinguishing "there are no games here" from "this is not a Steam
        // library" is the difference between a report and a shrug.
        let dir = TempDir::new("not-a-library");

        let error = games(dir.path()).expect_err("no steamapps means not a library");

        assert!(matches!(error, Error::Io { .. }), "got {error}");
    }

    #[test]
    fn every_root_that_is_reported_is_a_directory_that_exists() {
        // The only claim about roots that can be checked without a Steam
        // install: whatever comes back has to be real. Whether the candidate
        // list covers the places Steam is actually installed cannot be tested
        // here and is not tested anywhere — see the module docs.
        for root in roots() {
            assert!(
                root.is_dir(),
                "roots() must filter to existing directories, got {}",
                root.display()
            );
        }
    }

    #[test]
    fn the_candidate_list_is_not_empty_and_has_no_repeats() {
        // A typo that produced the same path twice would show every game
        // twice; an empty list would report every machine as Steam-less.
        let candidates = candidate_roots();

        assert!(!candidates.is_empty(), "somewhere has to be searched");
        let mut unique = candidates.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            candidates.len(),
            "no candidate may be listed twice, got {candidates:?}"
        );
    }

    #[test]
    fn a_leftover_directory_named_steam_is_not_a_root_or_library() {
        let directory = TempDir::new("not-a-steam-root");
        assert!(
            !super::is_root(directory.path()),
            "an arbitrary existing directory must not become an empty library"
        );
        directory.dir("steamapps");
        assert!(super::is_root(directory.path()));
    }

    #[cfg(unix)]
    #[test]
    fn the_unix_candidates_are_built_under_the_home_that_was_given() {
        // A regression guard on the spelling — `.steam/steam` is not
        // `.steam/Steam` on a case-sensitive filesystem — and nothing more.
        // That these six are where Steam installs itself is an assumption
        // this machine cannot check.
        let found = super::unix_home_roots(Path::new("/home/u"));

        assert_eq!(
            found,
            [
                PathBuf::from("/home/u/.steam/steam"),
                PathBuf::from("/home/u/.steam/root"),
                PathBuf::from("/home/u/.local/share/Steam"),
                PathBuf::from("/home/u/.var/app/com.valvesoftware.Steam/.local/share/Steam"),
                PathBuf::from("/home/u/.var/app/com.valvesoftware.Steam/data/Steam"),
                PathBuf::from("/home/u/snap/steam/common/.local/share/Steam"),
            ]
        );
    }
}
