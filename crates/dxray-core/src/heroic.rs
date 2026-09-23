//! Discovery of games installed by Heroic, from its local caches and the install
//! list of the Legendary it bundles. Only explicit install records are read; a
//! directory called `Games` is never taken for a game.

use std::{
    collections::HashSet,
    fmt, fs, io,
    path::{Path, PathBuf},
};

/// An installed game. Steam's manifests and Heroic's caches describe the same
/// thing, so they hand back the same type — see [`launcher`](crate::launcher).
pub use crate::launcher::Game;

use crate::launcher::{Catalogue, Identity, Libraries, Origin};

/// The launcher this module implements. Each game names its backend too, such
/// as `"Heroic / GOG"`.
pub const ORIGIN: Origin = Origin::new("heroic", "Heroic");

/// The Heroic backend that supplied a game record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Store {
    Epic,
    Gog,
    Amazon,
}

impl Store {
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.origin().label()
    }

    /// What a game from this backend says it came from. The key is `"heroic"`
    /// for all three, since it names the scanner, not the shop.
    #[must_use]
    pub const fn origin(self) -> Origin {
        match self {
            Self::Epic => Origin::new("heroic", "Heroic / Epic"),
            Self::Gog => Origin::new("heroic", "Heroic / GOG"),
            Self::Amazon => Origin::new("heroic", "Heroic / Amazon"),
        }
    }
}

/// Heroic as a [`Launcher`](crate::launcher::Launcher).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heroic;

/// The one Heroic launcher, for [`launcher::all`](crate::launcher::all).
pub static HEROIC: Heroic = Heroic;

impl crate::launcher::Launcher for Heroic {
    fn origin(&self) -> Origin {
        ORIGIN
    }

    fn roots(&self) -> Vec<PathBuf> {
        roots()
    }

    fn candidate_roots(&self) -> Vec<PathBuf> {
        candidate_roots()
    }

    /// One library per root: the configuration root itself, the directory the
    /// records are read from. Never fails; [`roots`] checked `store_cache`.
    fn libraries(&self, root: &Path) -> Libraries {
        Libraries {
            paths: vec![root.to_path_buf()],
            notes: Vec::new(),
            problems: Vec::new(),
        }
    }

    /// Cannot fail wholesale: every cache file is optional, so failures arrive
    /// as problems.
    fn games(&self, library: &Path) -> Catalogue {
        let scan = games(library);
        Catalogue {
            games: scan.games,
            problems: scan.problems.iter().map(ToString::to_string).collect(),
            notes: scan.notes,
        }
    }
}

/// One input that could not be trusted.
#[derive(Debug)]
pub enum Error {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Json {
        path: PathBuf,
        source: String,
    },
    BadRecord {
        path: PathBuf,
        id: String,
        reason: &'static str,
    },
    MissingInstall {
        path: PathBuf,
        id: String,
        install_dir: PathBuf,
    },
    /// A record that names a game but no directory. A problem only when no other
    /// cache supplies the same `(backend, id)`; see `settle_missing_installs`.
    NoInstallPath {
        path: PathBuf,
        id: String,
        /// The backend whose cache this record came from, so the match is on the
        /// same `(backend, id)` a [`Game`] is built with.
        store: Store,
        /// The title the record names, for the sentence a reader gets.
        title: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Json { path, source } => write!(f, "{}: {source}", path.display()),
            Self::BadRecord { path, id, reason } => {
                write!(f, "{}: Heroic record {id:?} {reason}", path.display())
            }
            Self::MissingInstall {
                path,
                id,
                install_dir,
            } => write!(
                f,
                "{}: Heroic record {id:?} names missing install {}",
                path.display(),
                install_dir.display()
            ),
            Self::NoInstallPath { path, id, .. } => write!(
                f,
                "{}: Heroic record {id:?} has no install_path",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {}

/// A complete partial scan: valid games plus every unusable record.
#[derive(Debug, Default)]
pub struct Scan {
    pub games: Vec<Game>,
    /// Records that could not be used and cost something. Should move an exit
    /// code.
    pub problems: Vec<Error>,
    /// Records that could not be used and cost nothing, worded with the reason.
    /// Must not move an exit code.
    pub notes: Vec<String>,
}

/// Every Heroic installation on this machine: XDG, Flatpak, bounded Distrobox
/// homes on mounted volumes, and `DXRAY_HEROIC_CONFIG`.
///
/// A candidate qualifies when it has a `store_cache` directory, which Heroic
/// creates on its first launch whether or not a store is signed in. An install
/// with nothing in it is listed with zero games. Each configuration is its own
/// launcher, so a mounted Distrobox cache is never hidden by the host's.
#[must_use]
pub fn roots() -> Vec<PathBuf> {
    existing_roots(candidate_roots())
}

/// Every path [`roots`] considers, existing or not, in order, so a caller that
/// found no Heroic can say where it looked. Distrobox homes are always included.
#[must_use]
pub fn candidate_roots() -> Vec<PathBuf> {
    candidates_from(configured_candidates(), crate::paths::container_homes())
}

/// The candidates that come from this process's environment.
fn configured_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        candidates.push(Path::new(&config_home).join("heroic"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = Path::new(&home);
        candidates.push(home.join(".config/heroic"));
        candidates.push(home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"));
    }
    if let Some(configs) = std::env::var_os("DXRAY_HEROIC_CONFIG") {
        candidates.extend(std::env::split_paths(&configs));
    }
    candidates
}

fn candidates_from(
    mut candidates: Vec<PathBuf>,
    container_homes: impl IntoIterator<Item = PathBuf>,
) -> Vec<PathBuf> {
    // Each configuration is its own source of records, so container caches join
    // the host's rather than replacing it.
    for home in container_homes {
        candidates.push(home.join(".config/heroic"));
        candidates.push(home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"));
    }
    candidates
}

/// A [`roots`] whose environment is supplied rather than read, for tests.
#[cfg(test)]
fn roots_from_candidates(
    candidates: Vec<PathBuf>,
    container_homes: impl IntoIterator<Item = PathBuf>,
) -> Vec<PathBuf> {
    existing_roots(candidates_from(candidates, container_homes))
}

fn existing_roots(candidates: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|path| path.join("store_cache").is_dir())
        .filter(|path| seen.insert(fs::canonicalize(path).unwrap_or_else(|_| path.clone())))
        .collect()
}

/// Reads installed records under `root`: `store_cache` install maps, library
/// lists where only `is_installed: true` counts, and the install list of the
/// Legendary that Heroic bundles. The title, an absolute install path and the
/// directory itself must all exist; a DLC is never a game.
#[must_use]
pub fn games(root: &Path) -> Scan {
    let mut scan = Scan::default();
    let mut seen = HashSet::new();
    for (store, install_info, library) in [
        (
            Store::Epic,
            "store_cache/legendary_install_info.json",
            "store_cache/legendary_library.json",
        ),
        (
            Store::Gog,
            "store_cache/gog_install_info.json",
            "store_cache/gog_library.json",
        ),
        (
            Store::Amazon,
            "store_cache/nile_install_info.json",
            "store_cache/nile_library.json",
        ),
    ] {
        read_install_map(root.join(install_info), store, &mut scan, &mut seen);
        read_library(root.join(library), store, &mut scan, &mut seen);
    }
    // Heroic points `LEGENDARY_CONFIG_PATH` here; it can hold a path the cache lost.
    read_install_map(
        root.join("legendaryConfig/legendary/installed.json"),
        Store::Epic,
        &mut scan,
        &mut seen,
    );
    scan.games.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.identity.cmp(&right.identity))
    });
    settle_missing_installs(&mut scan);
    scan
}

/// Moves each "has no `install_path`" record to a note when the same
/// `(backend, id)` was supplied with a directory by another cache, and leaves it
/// a problem otherwise.
///
/// Heroic's install map and library list expire separately, so one can lose a
/// path the other still has. Matching on the record's identity rather than its
/// title keeps a different record sharing a title from excusing a lost game.
fn settle_missing_installs(scan: &mut Scan) {
    let found: HashSet<(Origin, &str)> = scan
        .games
        .iter()
        .filter_map(|game| match &game.identity {
            Identity::Native(id) => Some((game.origin, id.as_str())),
            Identity::SteamApp(_) => None,
        })
        .collect();
    let mut notes = Vec::new();
    scan.problems.retain(|problem| {
        let Error::NoInstallPath {
            id, store, title, ..
        } = problem
        else {
            return true;
        };
        if !found.contains(&(store.origin(), id.as_str())) {
            return true;
        }
        // Worded with both halves: the record is wrong, and nothing is missing.
        notes.push(format!(
            "{problem}; {title:?} was found from another source, so nothing is \
             missing from this listing"
        ));
        false
    });
    scan.notes.append(&mut notes);
}

fn read_install_map(
    path: PathBuf,
    store: Store,
    scan: &mut Scan,
    seen: &mut HashSet<(Store, Identity, PathBuf)>,
) {
    let Some(value) = read_json(&path, scan) else {
        return;
    };
    let Some(records) = value.object() else {
        scan.problems.push(Error::BadRecord {
            path,
            id: "<root>".to_owned(),
            reason: "is not a JSON object",
        });
        return;
    };
    for (id, record) in records {
        if id.starts_with("__") {
            continue;
        }
        add_record(record, id, store, &path, scan, seen);
    }
}

fn read_library(
    path: PathBuf,
    store: Store,
    scan: &mut Scan,
    seen: &mut HashSet<(Store, Identity, PathBuf)>,
) {
    let Some(value) = read_json(&path, scan) else {
        return;
    };
    let Some(records) = value
        .get("games")
        .or_else(|| value.get("library"))
        .and_then(Json::array)
    else {
        // An empty object is an ordinary cache that has not synchronised yet.
        if value.is_empty_object() {
            return;
        }
        scan.problems.push(Error::BadRecord {
            path,
            id: "<root>".to_owned(),
            reason: "has no games or library array",
        });
        return;
    };
    for record in records {
        if record.get("is_installed").and_then(Json::bool) != Some(true) {
            continue;
        }
        let Some(id) = record
            .get("app_name")
            .and_then(Json::string)
            .filter(|id| !id.is_empty())
        else {
            scan.problems
                .push(bad(&path, "<library entry>", "has no app_name"));
            continue;
        };
        add_record(record, id, store, &path, scan, seen);
    }
}

fn read_json(path: &Path, scan: &mut Scan) -> Option<Json> {
    if !path.is_file() {
        return None;
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) => {
            scan.problems.push(Error::Io {
                path: path.to_path_buf(),
                source,
            });
            return None;
        }
    };
    match Json::parse(&text) {
        Ok(value) => Some(value),
        Err(source) => {
            scan.problems.push(Error::Json {
                path: path.to_path_buf(),
                source,
            });
            None
        }
    }
}

fn add_record(
    record: &Json,
    id: &str,
    store: Store,
    path: &Path,
    scan: &mut Scan,
    seen: &mut HashSet<(Store, Identity, PathBuf)>,
) {
    if record.get("is_dlc").and_then(Json::bool) == Some(true) {
        return;
    }
    match record_to_game(record, id, store, path) {
        Ok(game) if seen.insert((store, game.identity.clone(), game.install_dir.clone())) => {
            scan.games.push(game);
        }
        Ok(_) => {}
        Err(problem) => scan.problems.push(problem),
    }
}

fn record_to_game(record: &Json, id: &str, store: Store, path: &Path) -> Result<Game, Error> {
    let title = record
        .pointer(&["game", "title"])
        .or_else(|| record.get("title"))
        .and_then(Json::string)
        .filter(|title| !title.trim().is_empty())
        .ok_or_else(|| bad(path, id, "has no game title"))?;
    let install = record
        .pointer(&["install", "install_path"])
        .or_else(|| record.get("install_path"))
        .and_then(Json::string)
        .filter(|install| !install.is_empty())
        .ok_or_else(|| Error::NoInstallPath {
            path: path.to_path_buf(),
            id: id.to_owned(),
            store,
            title: title.to_owned(),
        })?;
    let install_dir = PathBuf::from(install);
    if !install_dir.is_absolute() {
        return Err(bad(path, id, "has a relative install_path"));
    }
    if !install_dir.is_dir() {
        return Err(Error::MissingInstall {
            path: path.to_path_buf(),
            id: id.to_owned(),
            install_dir,
        });
    }
    Ok(Game {
        identity: Identity::Native(id.to_owned()),
        name: title.to_owned(),
        install_dir,
        origin: store.origin(),
    })
}

fn bad(path: &Path, id: &str, reason: &'static str) -> Error {
    Error::BadRecord {
        path: path.to_path_buf(),
        id: id.to_owned(),
        reason,
    }
}

/// The tiny JSON tree Heroic discovery needs, kept local so the CLI stays
/// dependency-free. Only strings, booleans and structure are interpreted.
#[derive(Debug)]
enum Json {
    Object(Vec<(String, Self)>),
    Array(Vec<Self>),
    String(String),
    Bool(bool),
    Other,
}

impl Json {
    fn parse(text: &str) -> Result<Self, String> {
        let mut parser = JsonParser::new(text);
        let value = parser.value(0)?;
        parser.space();
        if parser.at_end() {
            Ok(value)
        } else {
            Err("trailing data".to_owned())
        }
    }

    fn is_empty_object(&self) -> bool {
        matches!(self, Self::Object(entries) if entries.is_empty())
    }

    fn object(&self) -> Option<&[(String, Self)]> {
        match self {
            Self::Object(entries) => Some(entries),
            Self::Array(_) | Self::String(_) | Self::Bool(_) | Self::Other => None,
        }
    }

    fn string(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            Self::Object(_) | Self::Array(_) | Self::Bool(_) | Self::Other => None,
        }
    }

    fn array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(values) => Some(values),
            Self::Object(_) | Self::String(_) | Self::Bool(_) | Self::Other => None,
        }
    }

    fn bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::Object(_) | Self::Array(_) | Self::String(_) | Self::Other => None,
        }
    }

    fn get(&self, key: &str) -> Option<&Self> {
        self.object()?
            .iter()
            .find_map(|(found, value)| (found == key).then_some(value))
    }

    fn pointer(&self, keys: &[&str]) -> Option<&Self> {
        keys.iter().try_fold(self, |value, key| value.get(key))
    }
}

/// How deep a cache document may nest before it is treated as malformed. A real
/// cache is about six levels deep; the bound stops a file of `[` from
/// overflowing the stack.
const MAX_DEPTH: usize = 64;

/// Byte cursor over the document, with the text beside it so a multi-byte
/// character can be decoded from its own bytes.
struct JsonParser<'a> {
    text: &'a str,
    input: &'a [u8],
    at: usize,
}

impl<'a> JsonParser<'a> {
    const fn new(input: &'a str) -> Self {
        Self {
            text: input,
            input: input.as_bytes(),
            at: 0,
        }
    }
    fn at_end(&self) -> bool {
        self.at == self.input.len()
    }
    fn space(&mut self) {
        while self.input.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        self.space();
        if self.input.get(self.at) == Some(&byte) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn required(&mut self, byte: u8) -> Result<(), String> {
        self.take(byte)
            .then_some(())
            .ok_or_else(|| format!("expected {:?}", char::from(byte)))
    }

    /// `depth` is how many containers are already open, so a document nested
    /// exactly [`MAX_DEPTH`] deep parses and one more is an error.
    fn value(&mut self, depth: usize) -> Result<Json, String> {
        self.space();
        match self.input.get(self.at).copied() {
            Some(b'{') => self.object_value(depth),
            Some(b'"') => self.string_value().map(Json::String),
            Some(b'[') => self.array_value(depth),
            Some(b't') => self.literal(b"true").map(|()| Json::Bool(true)),
            Some(b'f') => self.literal(b"false").map(|()| Json::Bool(false)),
            Some(b'n') => self.literal(b"null").map(|()| Json::Other),
            Some(_) => {
                self.atom()?;
                Ok(Json::Other)
            }
            None => Err("expected value".to_owned()),
        }
    }

    fn array_value(&mut self, depth: usize) -> Result<Json, String> {
        if depth >= MAX_DEPTH {
            return Err(Self::too_deep());
        }
        self.required(b'[')?;
        let mut values = Vec::new();
        self.space();
        if self.take(b']') {
            return Ok(Json::Array(values));
        }
        loop {
            values.push(self.value(depth + 1)?);
            if self.take(b']') {
                return Ok(Json::Array(values));
            }
            self.required(b',')?;
        }
    }

    fn object_value(&mut self, depth: usize) -> Result<Json, String> {
        if depth >= MAX_DEPTH {
            return Err(Self::too_deep());
        }
        self.required(b'{')?;
        let mut entries = Vec::new();
        self.space();
        if self.take(b'}') {
            return Ok(Json::Object(entries));
        }
        loop {
            self.space();
            let key = self.string_value()?;
            self.required(b':')?;
            let value = self.value(depth + 1)?;
            entries.push((key, value));
            if self.take(b'}') {
                return Ok(Json::Object(entries));
            }
            self.required(b',')?;
        }
    }

    fn too_deep() -> String {
        format!("nested past {MAX_DEPTH} levels")
    }

    fn literal(&mut self, expected: &[u8]) -> Result<(), String> {
        self.space();
        if self.input.get(self.at..self.at + expected.len()) == Some(expected) {
            self.at += expected.len();
            Ok(())
        } else {
            Err("invalid literal".to_owned())
        }
    }

    fn atom(&mut self) -> Result<(), String> {
        self.space();
        let start = self.at;
        while self
            .input
            .get(self.at)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(*byte, b',' | b'}' | b']'))
        {
            self.at += 1;
        }
        (self.at > start)
            .then_some(())
            .ok_or_else(|| "expected atom".to_owned())
    }

    fn string_value(&mut self) -> Result<String, String> {
        self.space();
        self.required(b'"')?;
        let mut out = String::new();
        loop {
            let byte = *self
                .input
                .get(self.at)
                .ok_or_else(|| "unterminated string".to_owned())?;
            self.at += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escaped = *self
                        .input
                        .get(self.at)
                        .ok_or_else(|| "unterminated escape".to_owned())?;
                    self.at += 1;
                    let character = match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{0008}',
                        b'f' => '\u{000c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        other => return Err(format!("bad escape {other:?}")),
                    };
                    out.push(character);
                }
                0..=31 => return Err("control byte in string".to_owned()),
                byte if byte.is_ascii() => out.push(char::from(byte)),
                _ => {
                    // Decoded from its own bytes: validating the rest of the file
                    // per accented letter was quadratic. `str::get` errors where an
                    // index would panic.
                    let start = self.at - 1;
                    let character = self
                        .text
                        .get(start..)
                        .and_then(|rest| rest.chars().next())
                        .ok_or_else(|| "invalid UTF-8 in string".to_owned())?;
                    self.at = start + character.len_utf8();
                    out.push(character);
                }
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let first = self.unicode_unit()?;
        if !(0xd800..=0xdbff).contains(&first) {
            return char::from_u32(u32::from(first))
                .ok_or_else(|| "invalid unicode scalar".to_owned());
        }
        if self.input.get(self.at..self.at + 2) != Some(b"\\u") {
            return Err("unpaired high surrogate".to_owned());
        }
        self.at += 2;
        let second = self.unicode_unit()?;
        if !(0xdc00..=0xdfff).contains(&second) {
            return Err("unpaired high surrogate".to_owned());
        }
        let scalar = 0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00);
        char::from_u32(scalar).ok_or_else(|| "invalid unicode scalar".to_owned())
    }

    fn unicode_unit(&mut self) -> Result<u16, String> {
        let digits = self
            .input
            .get(self.at..self.at + 4)
            .ok_or_else(|| "short unicode escape".to_owned())?;
        self.at += 4;
        let text = std::str::from_utf8(digits).map_err(|_| "invalid unicode escape".to_owned())?;
        u16::from_str_radix(text, 16).map_err(|_| "invalid unicode escape".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, Json, MAX_DEPTH, Store, games, roots_from_candidates};
    use crate::testutil::TempDir;

    #[test]
    fn scans_the_current_legendary_cache_shape_and_ignores_bookkeeping() {
        let root = TempDir::new("heroic-cache");
        let install = root.dir("games/star-rail");
        root.write(
            "store_cache/legendary_install_info.json",
            &format!(r#"{{"game-id":{{"game":{{"title":"Honkai: Star Rail"}},"install":{{"install_path":"{}"}}}},"__timestamp":{{"game-id":1}}}}"#, install.display()),
        );
        let scan = games(root.path());
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert_eq!(scan.games.len(), 1);
        assert_eq!(scan.games[0].name, "Honkai: Star Rail");
        assert_eq!(scan.games[0].origin, Store::Epic.origin());
        assert_eq!(scan.games[0].install_dir, install);
    }

    #[test]
    fn a_stale_record_for_a_game_another_cache_supplied_is_a_note_not_a_problem() {
        // Measured on a real machine: a stale "Fortnite" record moved the exit
        // code while Fortnite itself was found from another cache.
        let root = TempDir::new("heroic-stale");
        let install = root.dir("games/fortnite");
        // One `app_name`, two caches: the install map has the path, the list lost it.
        root.write(
            "store_cache/legendary_install_info.json",
            &format!(
                r#"{{"Fortnite":{{"game":{{"title":"Fortnite"}},"install":{{"install_path":"{}"}}}}}}"#,
                install.display()
            ),
        );
        root.write(
            "store_cache/legendary_library.json",
            r#"{"library":[{"app_name":"Fortnite","title":"Fortnite","is_installed":true}]}"#,
        );

        let scan = games(root.path());

        assert_eq!(scan.games.len(), 1, "the game is found: {:?}", scan.games);
        assert!(
            scan.problems.is_empty(),
            "nothing was lost, so nothing moves the exit code, got {:?}",
            scan.problems
        );
        let note = scan.notes.first().expect("the record is still reported");
        assert!(
            note.contains("has no install_path") && note.contains("another source"),
            "and the note says both halves, or a stale entry and a lost game \
             read alike: {note:?}"
        );
    }

    #[test]
    fn legendary_own_install_list_supplies_a_game_the_cache_lost() {
        // Measured: Rocket League's cache record had `install: null`, and only
        // Legendary's `installed.json` still named the directory.
        let root = TempDir::new("heroic-legendary");
        let install = root.dir("games/rocketleague");
        root.write(
            "store_cache/legendary_install_info.json",
            r#"{"Sugar":{"game":{"title":"Rocket League®"},"install":null}}"#,
        );
        root.write(
            "legendaryConfig/legendary/installed.json",
            &format!(
                r#"{{"Sugar":{{"app_name":"Sugar","install_path":"{0}","is_dlc":false,"platform":"Windows","title":"Rocket League®","version":"1"}},"SugarPack":{{"app_name":"SugarPack","install_path":"{0}","is_dlc":true,"title":"A DLC","version":"1"}}}}"#,
                install.display()
            ),
        );

        let scan = games(root.path());

        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert_eq!(scan.games.len(), 1, "a DLC is not a game: {:?}", scan.games);
        assert_eq!(scan.games[0].name, "Rocket League®");
        assert_eq!(scan.games[0].origin, Store::Epic.origin());
        assert_eq!(scan.games[0].install_dir, install);
        assert!(
            scan.notes
                .iter()
                .any(|note| note.contains("another source")),
            "the cache record is still reported, got {:?}",
            scan.notes
        );
    }

    #[test]
    fn a_different_record_that_merely_shares_a_title_does_not_excuse_a_lost_game() {
        // Two records sharing a title: the lost GOG game must not be excused by the
        // Epic copy.
        let root = TempDir::new("heroic-namesake");
        let install = root.dir("games/epic-hades");
        root.write(
            "store_cache/legendary_install_info.json",
            &format!(
                r#"{{"Hades":{{"game":{{"title":"Hades"}},"install":{{"install_path":"{}"}}}}}}"#,
                install.display()
            ),
        );
        root.write(
            "store_cache/gog_library.json",
            r#"{"library":[{"app_name":"1207658691","title":"Hades","is_installed":true}]}"#,
        );

        let scan = games(root.path());

        assert_eq!(scan.games.len(), 1, "only one of them resolved");
        assert!(
            scan.notes.is_empty(),
            "a namesake is not the same record, so nothing is excused, got {:?}",
            scan.notes
        );
        assert!(
            scan.problems.iter().any(
                |problem| matches!(problem, Error::NoInstallPath { id, .. } if id == "1207658691")
            ),
            "the GOG record still costs the exit code it earned, got {:?}",
            scan.problems
        );
    }

    #[test]
    fn a_record_with_no_install_path_for_a_game_nothing_else_supplied_stays_a_problem() {
        // Here the missing path really cost a game: nothing else supplies it.
        let root = TempDir::new("heroic-lost");
        root.write(
            "store_cache/legendary_library.json",
            r#"{"library":[{"app_name":"Fortnite","title":"Fortnite","is_installed":true}]}"#,
        );

        let scan = games(root.path());

        assert!(scan.games.is_empty());
        assert!(scan.notes.is_empty(), "got {:?}", scan.notes);
        assert!(
            scan.problems
                .iter()
                .any(|problem| matches!(problem, Error::NoInstallPath { .. })),
            "a game nothing supplied is a game this tool did not report, got {:?}",
            scan.problems
        );
    }

    #[test]
    fn nesting_is_accepted_up_to_the_limit_and_refused_one_level_past_it() {
        // Tests the bound, not the overflow, which depends on stack layout. The
        // value is pinned as a literal too, since the fixture is built from it.
        assert_eq!(MAX_DEPTH, 64);

        let brackets = |levels: usize| format!("{}{}", "[".repeat(levels), "]".repeat(levels));

        assert!(
            Json::parse(&brackets(MAX_DEPTH)).is_ok(),
            "a document nested exactly {MAX_DEPTH} deep is within the bound"
        );
        let error =
            Json::parse(&brackets(MAX_DEPTH + 1)).expect_err("one level past it is refused");
        assert!(
            error.contains(&format!("nested past {MAX_DEPTH} levels")),
            "and it has to say why, got {error:?}"
        );

        let braces = |levels: usize| {
            let mut text = String::new();
            for _ in 0..levels {
                text.push_str("{\"k\":");
            }
            text.push('1');
            for _ in 0..levels {
                text.push('}');
            }
            text
        };
        assert!(
            Json::parse(&braces(MAX_DEPTH)).is_ok(),
            "objects are bounded on the same counter as arrays"
        );
        assert!(
            Json::parse(&braces(MAX_DEPTH + 1)).is_err(),
            "and refused on it too"
        );
    }

    #[test]
    fn a_pathologically_nested_cache_file_is_a_reported_problem_not_a_dead_scan() {
        // One hostile file must not abort the scan; it is named instead.
        let root = TempDir::new("heroic-deep");
        root.write(
            "store_cache/legendary_library.json",
            &"[".repeat(MAX_DEPTH * 4),
        );

        let scan = games(root.path());

        assert!(scan.games.is_empty());
        assert!(
            scan.problems
                .iter()
                .any(|problem| matches!(problem, Error::Json { .. })),
            "the file has to come back as a parse failure, got {:?}",
            scan.problems
        );
    }

    #[test]
    fn missing_or_relative_installs_are_reported_not_invented() {
        let root = TempDir::new("heroic-missing");
        root.write(
            "store_cache/gog_install_info.json",
            r#"{"gone":{"game":{"title":"Gone"},"install":{"install_path":"/dxray-no-game"}},"relative":{"game":{"title":"Relative"},"install":{"install_path":"games/relative"}}}"#,
        );
        let scan = games(root.path());
        assert!(scan.games.is_empty());
        assert_eq!(scan.problems.len(), 2, "{:?}", scan.problems);
    }

    #[test]
    fn empty_backend_caches_are_not_reported_as_bad_records() {
        let root = TempDir::new("heroic-empty");
        root.write("store_cache/legendary_library.json", "{}");
        root.write("store_cache/gog_library.json", "{}");
        let scan = games(root.path());
        assert!(scan.games.is_empty());
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
    }

    #[test]
    fn mounted_configuration_is_kept_when_the_host_cache_is_empty() {
        let fixture = TempDir::new("heroic-root-discovery");
        let host = fixture.dir("host/.config/heroic/store_cache");
        let mounted_home = fixture.dir("mounted/data/distrobox/gaming");
        let mounted = mounted_home.join(".config/heroic/store_cache");
        std::fs::create_dir_all(&mounted).expect("mounted cache");

        let roots = roots_from_candidates(
            vec![host.parent().expect("host config").to_path_buf()],
            vec![mounted_home],
        );
        assert_eq!(roots.len(), 2);
        assert!(
            roots
                .iter()
                .any(|root| root == host.parent().expect("host config"))
        );
        assert!(
            roots
                .iter()
                .any(|root| root == mounted.parent().expect("mounted config"))
        );
    }

    #[test]
    fn preserves_utf8_titles_and_json_escapes() {
        let root = TempDir::new("heroic-utf8");
        let install = root.dir("games/pokemon");
        root.write(
            "store_cache/nile_install_info.json",
            &format!(r#"{{"id":{{"game":{{"title":"Pokémon \u0026 Friends"}},"install":{{"install_path":"{}"}}}}}}"#, install.display()),
        );
        let scan = games(root.path());
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert_eq!(scan.games[0].name, "Pokémon & Friends");
    }

    #[test]
    fn scans_an_installed_library_record_when_install_info_is_absent() {
        let root = TempDir::new("heroic-library-only");
        let install = root.dir("games/library-game");
        root.write(
            "store_cache/legendary_library.json",
            &format!(
                r#"{{"library":[{{"app_name":"library-id","title":"Library Game","is_installed":true,"install":{{"install_path":"{}"}}}},{{"app_name":"owned-only","title":"Owned only","is_installed":false,"install":{{"install_path":"{}"}}}}]}}"#,
                install.display(),
                install.display()
            ),
        );
        let scan = games(root.path());
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert_eq!(scan.games.len(), 1);
        assert_eq!(scan.games[0].identity.to_string(), "library-id");
        assert_eq!(scan.games[0].origin, Store::Epic.origin());
    }

    #[test]
    fn games_array_and_surrogate_pairs_are_supported() {
        let root = TempDir::new("heroic-games-array");
        let install = root.dir("games/rocket");
        root.write(
            "store_cache/gog_library.json",
            &format!(
                r#"{{"games":[{{"app_name":"rocket","title":"Rocket \uD83D\uDE80","is_installed":true,"install":{{"install_path":"{}"}}}}]}}"#,
                install.display()
            ),
        );
        let scan = games(root.path());
        assert!(scan.problems.is_empty(), "{:?}", scan.problems);
        assert_eq!(scan.games[0].name, "Rocket 🚀");
        assert_eq!(scan.games[0].origin, Store::Gog.origin());
    }

    #[test]
    fn an_installation_is_a_store_cache_directory_and_not_a_bare_config_folder() {
        // An empty `store_cache` is a real installation (Heroic creates it on first
        // launch); a `heroic` folder without one is not.
        let fixture = TempDir::new("heroic-install-rule");
        let installed = fixture.dir("installed/.config/heroic/store_cache");
        let installed = installed.parent().expect("installed config").to_path_buf();
        let bare = fixture.dir("bare/.config/heroic");

        let roots = roots_from_candidates(vec![installed.clone(), bare.clone()], Vec::new());

        assert_eq!(
            roots,
            vec![installed],
            "an empty cache is an installation with no games, and a bare {} is not an installation",
            bare.display()
        );
    }
}
