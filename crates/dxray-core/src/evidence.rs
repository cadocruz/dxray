//! The only half of this crate that touches the filesystem.
//!
//! Kept deliberately thin. Everything it produces is a list of names, and every
//! judgement made about those names lives in [`analysis`](crate::analysis),
//! where it can be tested against a hundred awkward cases without a disk, a
//! Windows install or a game.
//!
//! It reads one file more than the one it was asked about: a library sitting
//! beside the executable that the executable imports. What comes back is still
//! only names, which library and what it imports in turn, so the judgement
//! stays where it was. See [`LibraryCache`].

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use dxray_pe::Pe;

use crate::analysis::{Evidence, FileVersion, Linked, Verdict};

// Bounded by the same constant the ranking uses, so the two cannot disagree
// about how far a link is chased.
use crate::install::MAX_LOCAL_LIBRARIES;

impl Evidence {
    /// Reads `path` as a PE image and lists the directory it sits in.
    ///
    /// Parse failures come back as [`io::ErrorKind::InvalidData`] rather than
    /// as a separate error type, because every caller of this function is
    /// already handling an `io::Error` from the read and a second error type
    /// buys nothing but a `From` impl.
    ///
    /// # Errors
    ///
    /// Fails if `path` cannot be read, or if it is not a PE image, or if its
    /// import tables are malformed. A directory that cannot be listed is *not*
    /// an error — see [`neighbours_of`].
    pub fn from_executable(path: &Path) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        let pe = Pe::parse(&bytes).map_err(invalid_data)?;
        let imports = pe.imports().map_err(invalid_data)?;
        let delay_imports = pe.delay_imports().map_err(invalid_data)?;
        let neighbours = neighbours_of(path);
        let linked = linked_libraries(path, &imports, &delay_imports, &neighbours);
        Ok(Self {
            imports,
            delay_imports,
            neighbours,
            linked,
        })
    }
}

/// Reads each library beside an executable at most once, however many questions
/// are asked about it.
///
/// Two questions are asked of the same files — what does this library import in
/// turn, and what version does it carry — and both are answered from one read,
/// because both come out of one `Pe`. Before this, a directory of 803 files had
/// each of its three DLSS runtimes opened 805 times: every record listed the
/// directory afresh and nothing was remembered between records. The same folder
/// now costs one read per library.
///
/// **Scoped to one directory**, and it enforces that itself rather than
/// trusting callers to. The key is a library name, so two directories sharing
/// one cache would attribute one `UnityPlayer.dll` to the other's games;
/// `enter` drops everything the moment the directory changes, which
/// also keeps a recursive walk's memory to one folder's worth.
#[derive(Default)]
pub struct LibraryCache {
    /// The directory every entry below was read from, once anything has been.
    dir: Option<std::path::PathBuf>,
    /// That directory's file names, listed once. See [`Self::neighbours`].
    listing: Option<Vec<String>>,
    read: HashMap<String, Library>,
}

/// One library read once: what it imports, and what version it carries.
struct Library {
    /// `None` when the file is not a PE image whose import tables this can
    /// read. Recorded rather than retried, so a broken DLL costs one read and
    /// not one per executable in the folder.
    linked: Option<Linked>,
    /// Never [`FileVersion::Elsewhere`] or [`FileVersion::Unread`]: the file
    /// was found in the directory and an attempt was made on it.
    version: FileVersion,
}

impl LibraryCache {
    /// Points the cache at the directory `path` sits in, forgetting another
    /// directory's files.
    ///
    /// Returns the directory, or `None` for a path that has none — a root, or a
    /// path that is nothing but a prefix.
    fn enter<'a>(&mut self, path: &'a Path) -> Option<&'a Path> {
        let dir = directory_of(path)?;
        if self.dir.as_deref() != Some(dir) {
            self.read.clear();
            self.listing = None;
            self.dir = Some(dir.to_owned());
        }
        Some(dir)
    }

    /// The names of the files sitting beside `path`, listing the directory once
    /// however many files in it are inspected.
    ///
    /// `path` itself is filtered out here rather than left out of the listing,
    /// because every file in the folder shares this listing and each of them
    /// has a different name to exclude. That filtering is string comparisons;
    /// the thing worth not repeating is the `read_dir`.
    ///
    /// This is the multiplier the open count does not show. Listing an 803-file
    /// directory once per record is 803 listings of 803 entries, and on a
    /// Windows mount — where every one of them crosses a 9p boundary — it is
    /// what a user actually waits for once the duplicate reads are gone.
    #[must_use]
    pub fn neighbours(&mut self, path: &Path) -> Vec<String> {
        let Some(dir) = self.enter(path) else {
            return Vec::new();
        };
        if self.listing.is_none() {
            self.listing = Some(list_directory(dir));
        }
        let self_name = path.file_name();
        self.listing
            .iter()
            .flatten()
            .filter(|name| Some(name.as_ref()) != self_name)
            .cloned()
            .collect()
    }

    /// The library `lower` names, read at most once per directory.
    ///
    /// `None` when no file of that name sits beside the executable, which is
    /// the ordinary answer for `d3d12.dll` and every other name the loader
    /// takes from `System32`.
    fn library(&mut self, dir: &Path, lower: &str, neighbours: &[String]) -> Option<&Library> {
        if !self.read.contains_key(lower) {
            let on_disk = neighbours
                .iter()
                .find(|name| name.eq_ignore_ascii_case(lower))?;
            let library = read_library(&dir.join(on_disk), on_disk);
            self.read.insert(lower.to_owned(), library);
        }
        self.read.get(lower)
    }

    /// Follows an image's imports one step, into libraries sitting beside it.
    ///
    /// A Unity executable is a stub that imports `UnityPlayer.dll` and nothing
    /// graphical, and the renderer is one link further on. Only the same
    /// directory is looked in: a library resolved out of `System32` describes
    /// Windows rather than the game, and one three folders away is not a name
    /// the loader would resolve this way at all.
    ///
    /// Unreadable libraries are skipped rather than reported. This is a
    /// supporting signal gathered from files nobody asked about, and a note for
    /// every DLL in a game directory that failed to parse would bury the answer.
    ///
    /// `path` is the image being inspected, not the directory: the cache has to
    /// know which folder it is in, and deriving that here is one fewer thing a
    /// caller can get wrong.
    pub fn follow(
        &mut self,
        path: &Path,
        imports: &[String],
        delay_imports: &[String],
        neighbours: &[String],
    ) -> Vec<Linked> {
        let Some(dir) = self.enter(path) else {
            return Vec::new();
        };
        let dir = dir.to_owned();

        let mut out = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for name in imports.iter().chain(delay_imports) {
            if seen.len() >= MAX_LOCAL_LIBRARIES {
                break;
            }
            // Lowercased so an import spelled `UNITYPLAYER.DLL` still finds the
            // file spelled `UnityPlayer.dll`; PE import names are famously
            // inconsistent about case.
            let lower = name.to_ascii_lowercase();
            if seen.contains(&lower) {
                continue;
            }
            // Only `.dll` files are followed. A `.exe` beside a game is another
            // candidate for being the game, not a library this one loads.
            if Path::new(&lower).extension() != Some("dll".as_ref()) {
                continue;
            }
            // `None` means no file of that name is here, which does not spend
            // any of the budget below: the bound counts libraries read, and a
            // name the loader takes from Windows was never read.
            let Some(linked) = self
                .library(&dir, &lower, neighbours)
                .map(|library| library.linked.clone())
            else {
                continue;
            };
            seen.push(lower);
            if let Some(linked) = linked {
                out.push(linked);
            }
        }
        out
    }

    /// Fills in the version of every file the verdict's signals name, reusing
    /// whatever this directory has already been read for.
    ///
    /// See [`stamp_versions`] for why this happens after the verdict and why it
    /// judges nothing.
    pub fn stamp(&mut self, verdict: &mut Verdict, path: &Path, neighbours: &[String]) {
        let Some(dir) = self.enter(path) else {
            return;
        };
        let dir = dir.to_owned();

        let findings = verdict
            .renderers
            .iter_mut()
            .chain(&mut verdict.infrastructure)
            .chain(&mut verdict.features)
            .chain(&mut verdict.local_overrides);
        for finding in findings {
            for signal in &mut finding.signals {
                let lower = signal.library.to_ascii_lowercase();
                signal.version = self
                    .library(&dir, &lower, neighbours)
                    .map_or(FileVersion::Elsewhere, |library| library.version);
            }
        }
    }
}

/// One library read: its import tables, and its version, from a single read.
///
/// The two halves fail independently on purpose. An image whose import tables
/// are malformed may still carry a perfectly good version resource, and there
/// is no reason for one broken table to cost the other answer.
fn read_library(path: &Path, spelled: &str) -> Library {
    let unreadable = Library {
        linked: None,
        version: FileVersion::Unreadable,
    };
    let Ok(bytes) = fs::read(path) else {
        return unreadable;
    };
    let Ok(pe) = Pe::parse(&bytes) else {
        return unreadable;
    };
    Library {
        linked: match (pe.imports(), pe.delay_imports()) {
            (Ok(imports), Ok(delay_imports)) => Some(Linked {
                library: spelled.to_owned(),
                imports,
                delay_imports,
            }),
            _ => None,
        },
        version: match pe.version_info() {
            // `0.0.0.0` arrives here as the version it is and leaves as one.
            Ok(Some(info)) => FileVersion::Stamped(info),
            Ok(None) => FileVersion::Unstamped,
            Err(_) => FileVersion::Unreadable,
        },
    }
}

/// The one-link chase for a single executable, with no cache to share.
///
/// Public because a caller that has already read the image - and does not want
/// to read it twice - still has to reach the same conclusion this does. Two
/// callers doing their own chase is how the verdict and the ranking came to
/// disagree in the first place.
///
/// A caller walking a whole directory should hold a [`LibraryCache`] instead:
/// this reads every library again for every executable.
#[must_use]
pub fn linked_libraries(
    path: &Path,
    imports: &[String],
    delay_imports: &[String],
    neighbours: &[String],
) -> Vec<Linked> {
    LibraryCache::default().follow(path, imports, delay_imports, neighbours)
}

/// Fills in the version of every file the verdict's signals name.
///
/// The uncached form of [`LibraryCache::stamp`], for a caller with one file to
/// ask about. Between them they are the **only** place in the project that
/// reads a version off disk for a library other than the one under inspection,
/// because this project has twice paid for one rule implemented in two places.
///
/// A caller walking a whole directory should hold a [`LibraryCache`] instead:
/// this reads every named library again for every executable.
///
/// # Why after the verdict, not during the evidence
///
/// A real game directory holds hundreds of DLLs and reading a version means
/// reading the whole file; `nvngx_dlss.dll` alone runs to tens of megabytes.
/// Versioning every neighbour would be most of the cost of the scan spent on
/// files nobody will be told about. So [`analyse`](crate::analyse) decides what
/// matters first and this reads exactly the files it named — at most a handful,
/// and each of them once even when several findings point at the same file.
///
/// # Why it judges nothing
///
/// The rule is one line and has no exceptions: *if a file of that name sits
/// beside the executable, report its version; otherwise say so*. No category is
/// treated specially. That is what makes `d3d12.dll` come back
/// [`Elsewhere`](FileVersion::Elsewhere) in an ordinary game — the loader takes
/// it from `System32` and its version describes Windows — while the same name
/// in a directory where somebody dropped a proxy DLL comes back stamped, which
/// is precisely the case where the number is worth having.
///
/// `neighbours` is passed in rather than listed again so the spelling matched
/// against is the directory's own. An import table saying `D3D12.dll` has to
/// find a file called `d3d12.dll`, and on the Linux machines this tool runs on
/// nothing else will do that for us.
pub fn stamp_versions(verdict: &mut Verdict, path: &Path, neighbours: &[String]) {
    LibraryCache::default().stamp(verdict, path, neighbours);
}

/// The directory `path` sits in.
///
/// `Path::parent` of a bare file name is an empty path, which `read_dir`
/// rejects and `join` mishandles; the directory meant is the current one. One
/// function because three callers here need the same answer and three spellings
/// of it is two too many.
fn directory_of(path: &Path) -> Option<&Path> {
    match path.parent() {
        // A root, or a path that is nothing but a prefix. There is no directory
        // "beside" it and pretending it is the current one would list a folder
        // that has nothing to do with the question.
        None => None,
        Some(dir) if dir.as_os_str().is_empty() => Some(Path::new(".")),
        Some(dir) => Some(dir),
    }
}

/// A parse failure as an [`io::Error`]. The original error is kept as the
/// source, so the message a caller prints still names the field that was wrong
/// rather than flattening to "invalid data".
fn invalid_data(error: dxray_pe::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// File names sitting beside `path`, excluding `path` itself and every
/// subdirectory.
///
/// An unreadable directory yields an empty list rather than an error. The
/// neighbours are one supporting signal among several, and a permission error
/// on a directory means the tool learned nothing there — not that it failed to
/// read the file it was asked about. Turning it into an error would sink a
/// record that is otherwise complete.
///
/// Subdirectories are excluded because the question they answer is about
/// loader search order, and the loader looks in the directory itself.
///
/// The uncached form of [`LibraryCache::neighbours`]. A caller walking a whole
/// directory should hold a cache: this lists the folder again for every file in
/// it, which is quadratic in its entries and is what a Windows mount charges
/// for once the duplicate reads are gone.
#[must_use]
pub fn neighbours_of(path: &Path) -> Vec<String> {
    LibraryCache::default().neighbours(path)
}

/// Every file name directly in `dir`, sorted, with subdirectories left out.
///
/// An unreadable directory yields an empty list rather than an error. The
/// neighbours are one supporting signal among several, and a permission error
/// on a directory means the tool learned nothing there — not that it failed to
/// read the file it was asked about. Turning it into an error would sink a
/// record that is otherwise complete.
fn list_directory(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for entry in entries.flatten() {
        // `file_type` does not follow symlinks, so a link to a directory is
        // still reported as a file here. That is the right answer: a link named
        // `dxgi.dll` shadows the system copy exactly as a real file would.
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        names.push(entry.file_name().to_string_lossy().into_owned());
    }

    // Sorted so a verdict does not reorder itself between runs: `read_dir`
    // makes no promise about order and two scans of one directory should be
    // diffable against each other.
    names.sort_unstable();
    names
}

#[cfg(test)]
mod version_tests;

#[cfg(test)]
mod tests {
    use super::neighbours_of;
    use crate::analysis::Evidence;
    use crate::testutil::TempDir;
    use std::path::Path;

    #[test]
    fn the_executable_itself_is_not_one_of_its_own_neighbours() {
        // Otherwise a game shipped as `dxgi.dll` — which happens, plenty of
        // launchers are DLLs — would be reported as its own injector.
        let dir = TempDir::new("self");
        let exe = dir.file("dxgi.dll");
        dir.file("version.dll");

        assert_eq!(
            neighbours_of(&exe),
            ["version.dll"],
            "the file under inspection must not appear beside itself"
        );
    }

    #[test]
    fn subdirectories_are_not_listed_as_neighbours() {
        // The loader searches the directory, not the tree below it. A folder
        // named `d3d12.dll` would otherwise read as a proxy DLL.
        let dir = TempDir::new("subdir");
        let exe = dir.file("game.exe");
        dir.dir("d3d12.dll");
        dir.file("winmm.dll");

        assert_eq!(neighbours_of(&exe), ["winmm.dll"]);
    }

    #[test]
    fn a_directory_that_cannot_be_listed_yields_no_neighbours_rather_than_an_error() {
        // Neighbours are supporting evidence. A path that is not there means
        // nothing was learned from the directory, which must not sink a record
        // that is otherwise complete.
        let missing = Path::new("/dxray-no-such-directory-anywhere/game.exe");

        assert!(
            neighbours_of(missing).is_empty(),
            "an unlistable directory is silence, not failure"
        );
    }

    #[test]
    fn a_file_that_is_not_a_pe_image_fails_as_invalid_data() {
        // The caller distinguishes "could not read" from "read it, it is not a
        // binary", and both arrive as io::Error so there is one thing to match.
        let dir = TempDir::new("notpe");
        let path = dir.file("notes.txt");

        let error = Evidence::from_executable(&path).expect_err("not a PE image");

        assert_eq!(
            error.kind(),
            std::io::ErrorKind::InvalidData,
            "a parse failure is invalid data, got {error}"
        );
    }
}

#[cfg(test)]
mod linked_tests {
    use crate::analysis::{Evidence, Source, analyse};
    use crate::testutil::TempDir;

    /// The shape that exposed this: Honkai: Star Rail, read on a real machine.
    ///
    /// A Unity executable is a stub. It imports `UnityPlayer.dll` and nothing
    /// graphical, and the engine one link on is what reaches Direct3D. Before
    /// the link was followed here, the ranking scored such a binary 170 —
    /// crediting it for reaching a renderer through `UnityPlayer.dll` — while
    /// the verdict printed beside that score said "no graphics API determined".
    /// Two layers answering one question differently, and the confident half
    /// was the wrong one.
    #[test]
    fn a_renderer_reached_through_a_local_library_is_in_the_verdict() {
        let dir = TempDir::new("linked");
        let exe = dir.image("StarRail.exe", &["KERNEL32.dll", "UnityPlayer.dll"], &[]);
        dir.image("UnityPlayer.dll", &["d3d11.dll", "dxgi.dll"], &[]);

        let evidence = Evidence::from_executable(&exe).expect("the stub is a readable PE");
        let verdict = analyse(&evidence);

        assert!(
            !verdict.renderers.is_empty(),
            "a Unity stub reaches its renderer one link on, and saying otherwise \
             contradicts the score printed beside it"
        );
        let finding = &verdict.renderers[0];
        assert_eq!(finding.name, "Direct3D 11");
        assert_eq!(finding.signals[0].source, Source::Linked);
        assert_eq!(
            finding.signals[0].library, "UnityPlayer.dll",
            "the signal names the library this executable actually imports, \
             because that is the file a reader would go and open"
        );
    }

    #[test]
    fn a_library_in_the_directory_that_the_executable_does_not_import_is_not_followed() {
        // The loader resolves what the import table names. A DLL that merely
        // sits in the folder is a neighbour, and reading it as a link would let
        // any stray engine DLL decide the verdict for an unrelated binary.
        let dir = TempDir::new("unimported");
        let exe = dir.image("launcher.exe", &["KERNEL32.dll"], &[]);
        dir.image("UnityPlayer.dll", &["d3d11.dll"], &[]);

        let verdict = analyse(&Evidence::from_executable(&exe).expect("readable"));

        assert!(
            verdict.renderers.is_empty(),
            "only imported libraries are followed, got {:?}",
            verdict.renderers
        );
    }

    #[test]
    fn a_direct_import_outranks_the_same_api_reached_through_a_library() {
        // Both are true at once for plenty of engines. The order is the point:
        // the executable's own import table is the stronger claim, and the
        // detail pane shows the strongest signal first.
        let dir = TempDir::new("both");
        let exe = dir.image("game.exe", &["d3d11.dll", "Engine.dll"], &[]);
        dir.image("Engine.dll", &["d3d11.dll"], &[]);

        let verdict = analyse(&Evidence::from_executable(&exe).expect("readable"));

        let d3d11 = verdict
            .renderers
            .iter()
            .find(|f| f.name == "Direct3D 11")
            .expect("found either way");
        assert_eq!(d3d11.signals[0].source, Source::Import);
    }

    #[test]
    fn exactly_the_local_library_budget_is_followed_and_no_more() {
        // The budget was untested in both directions: raising it changed
        // nothing any test could see, and lowering it to one would silently
        // reduce the link chase to whichever library the import table happens to
        // name first. A stub loader's engine DLL is regularly not the first
        // import, so that is a wrong verdict, not a smaller one.
        //
        // This fixture is built from the constant, so it proves the budget is a
        // hard stop rather than what the number is; the number itself is pinned
        // as a literal in `install::tests`.
        use super::{LibraryCache, MAX_LOCAL_LIBRARIES, neighbours_of};

        let dir = TempDir::new("budget");
        let names: Vec<String> = (0..=MAX_LOCAL_LIBRARIES)
            .map(|i| format!("engine{i:02}.dll"))
            .collect();
        for name in &names {
            dir.image(name, &["d3d11.dll"], &[]);
        }
        let exe = dir.image("game.exe", &["KERNEL32.dll"], &[]);

        let neighbours = neighbours_of(&exe);
        let linked = LibraryCache::default().follow(&exe, &names, &[], &neighbours);

        assert_eq!(
            linked.len(),
            MAX_LOCAL_LIBRARIES,
            "every library up to the budget is read, and the one past it is not"
        );
    }

    #[test]
    fn an_unreadable_local_library_is_skipped_rather_than_failing_the_read() {
        // A game directory is full of DLLs this tool has no business parsing.
        // One that is not a PE at all must cost nothing: the executable was
        // read successfully and that is the question that was asked.
        let dir = TempDir::new("badlib");
        let exe = dir.image("game.exe", &["KERNEL32.dll", "Broken.dll"], &[]);
        dir.write("Broken.dll", "not a PE image at all");

        let evidence = Evidence::from_executable(&exe).expect("the executable is still readable");

        assert!(
            evidence.linked.is_empty(),
            "an unparsable library is skipped"
        );
    }
}
