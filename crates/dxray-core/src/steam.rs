//! Finding a Steam install, its libraries and the games in them. The IO half:
//! it opens files and hands them to [`vdf`].
//!
//! `libraryfolders.vdf` has had two schemas (index to path, and index to a
//! block with `"path"`); both are read. Only numeric keys are libraries. The
//! usual roots are symlinks to one directory, so paths are compared resolved.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::launcher::{Catalogue, Identity, Libraries, Origin, identity};
use crate::vdf::{self, Object, Value};

pub mod launch;

/// One installed game. Steam's manifests and Heroic's caches describe the same
/// thing, so they hand back the same type — see [`launcher`](crate::launcher).
pub use crate::launcher::Game;

/// The launcher this module implements, and the name its messages carry.
pub const ORIGIN: Origin = Origin::new("steam", "Steam");

/// Steam as a [`Launcher`](crate::launcher::Launcher). The free functions below
/// are the typed API; the adapter words their errors.
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
    /// Callers that need the typed error call [`libraries`] directly.
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

/// What can go wrong while reading a Steam directory. Every variant names its
/// file.
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
    /// An index file exists and holds nothing: a truncated file, not a healthy
    /// machine with one library.
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

/// How much of a rejected value to quote back from an untrusted file.
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

/// Every candidate Steam install on this machine that exists, deduplicated by
/// resolved path with the first spelling kept. Empty means none was found where
/// this code looks, not that there is none.
#[must_use]
pub fn roots() -> Vec<PathBuf> {
    let mut out = existing_roots(candidate_roots());
    // Mounted Distrobox homes are only a fallback when the host has no Steam.
    if out.is_empty() {
        out = existing_roots(container_candidate_roots());
    }
    out
}

/// The candidates that are Steam installs, first spelling of each kept,
/// compared by [`identity`].
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

/// Every path [`roots`] considers, existing or not, in priority order, so a
/// caller that found nothing can say where it looked. On Windows the real
/// answer is the `SteamPath` registry value, which is not read.
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
        // Guesses at the default install locations only; the registry's
        // `SteamPath` is authoritative and is not read.
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

/// A directory is a Steam root only when it holds a library or a library
/// index, so a leftover `Steam` directory is not one.
fn is_root(path: &Path) -> bool {
    path.join("steamapps").is_dir() || LIBRARY_INDEX.iter().any(|index| path.join(index).is_file())
}

/// Where `libraryfolders.vdf` is looked for, in order. A current install keeps
/// it in `config`; older ones in `steamapps`.
const LIBRARY_INDEX: [&str; 2] = ["steamapps/libraryfolders.vdf", "config/libraryfolders.vdf"];

/// Something true about an answer that makes it worth less than it looks. A
/// note means everything was read; a problem ([`Error`]) means something was not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// The index parsed but declared no numbered entries. Ambiguous from the
    /// file alone: a healthy old single-library install, or a truncated new
    /// one. Reported, not decided.
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

/// The library directories belonging to one Steam root, and the notes that
/// qualify the list.
#[derive(Debug, Default)]
pub struct Index {
    /// The root first, then whatever the index declared, deduplicated.
    pub libraries: Vec<PathBuf>,
    /// Reasons this list may be shorter than the machine really has.
    pub notes: Vec<Note>,
}

/// The library directories belonging to `root`, the root itself first.
///
/// Paths are returned as the index declares them, unchecked: a library on an
/// unplugged drive stays in the list and fails later with its own name.
///
/// # Errors
///
/// Fails if the index exists but cannot be read or parsed, if an entry has no
/// usable path, or if the index is empty ([`Error::NoLibraries`]). A missing
/// index is not an error.
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

    // Every index of either schema has something in it, so an empty one is an
    // error rather than a note.
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

/// Pulls the library paths out of a parsed `libraryfolders.vdf`, in file order:
/// both schemas, non-numeric keys skipped, repeated keys all kept.
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
/// could not be read. Two lists, so dropping the failures is never the default.
#[derive(Debug, Default)]
pub struct Scan {
    /// Ordered by application id, so two scans can be diffed.
    pub games: Vec<Game>,
    /// One entry per manifest that could not be read, parsed or understood,
    /// each naming its file.
    pub problems: Vec<Error>,
}

/// Every game installed in `library`, and every manifest that defeated the
/// reader. A bad manifest does not stop the scan, and nothing is filtered out:
/// telling a game from a tool is left to the evidence one layer up.
///
/// # Errors
///
/// Fails only if `library/steamapps` cannot be listed at all.
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

/// Whether a directory entry is an application manifest, matched
/// case-insensitively.
fn is_manifest_name(name: &str) -> bool {
    // `get` rather than slicing, which could split a multi-byte character.
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
        // Joined here once, so no caller rebuilds it. Not checked for existence.
        install_dir: library
            .join("steamapps")
            .join("common")
            .join(install_dir_name),
        origin: ORIGIN,
    })
}

/// Rejects an `installdir` that is not a single directory name: an absolute
/// path or `..` would point outside `steamapps/common`.
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
        // Empty values would read as data rather than as the damage they are.
        return Err(Error::BadValue {
            path: path.to_path_buf(),
            key,
            value: String::new(),
        });
    }
    Ok(value)
}

/// The single named block a Steam file wraps everything in. Falls back to the
/// sole top-level block, since the name has changed before.
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

/// Reads the first of `relatives` under `root` that is there. Absent is
/// `Ok(None)`; present and unreadable is an error.
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

/// Removes repeats by [`identity`], keeping the first spelling: the root's
/// symlink and the index's resolved path are one library.
fn dedup_paths(paths: &mut Vec<PathBuf>) {
    let mut seen = HashSet::new();
    paths.retain(|p| seen.insert(identity(p)));
}

#[cfg(test)]
mod tests {
    use super::{Error, Game, Identity, Note, ORIGIN, candidate_roots, games, libraries, roots};
    use crate::testutil::TempDir;
    use std::path::{Path, PathBuf};

    /// A Steam-shaped directory tree, with the layout assumptions in one place.
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

    /// Scans `library` and returns its only reported problem.
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
        // An unplugged drive must fail later, naming its path, not vanish.
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
        // The usual Linux install: a symlinked root, and the index naming the
        // same directory by its resolved path.
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
        // An empty file parses cleanly, and is still not a healthy index.
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
        // A current index always lists the root as entry "0".
        let install = Install::new("empty-block");
        install.index("\"libraryfolders\"\n{\n}\n");

        let error = libraries(install.root()).expect_err("an empty block is not one library");

        assert!(matches!(error, Error::NoLibraries { .. }), "got {error}");
    }

    #[test]
    fn an_index_that_declares_no_entries_is_told_apart_from_one_that_declares_some() {
        // Both halves: a note for bookkeeping-only, none for a healthy index.
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
        // True of a healthy old install and of a truncated new one alike.
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
        // Not an error: an old single-library client writes exactly this.
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
        // KeyValues is a list: a repeated key is two libraries.
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
        // `DownloadType` marks real games as tools too, so nothing filters on it.
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
        // A partial manifest must not point the scan at `steamapps/common`.
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
        // `join` with an absolute value discards everything to its left.
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
        // A bad manifest is reported beside the games, not instead of them.
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
        // A lenient parse would look like a game that is not installed.
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
        // `steamapps` holds directories that are not manifests.
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
        // Whatever comes back must exist; coverage cannot be tested here.
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
        // A guard on the spelling only: `.steam/steam`, not `.steam/Steam`.
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
