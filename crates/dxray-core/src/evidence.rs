//! The only half of this crate that touches the filesystem, kept thin: it
//! produces names, and [`analysis`](crate::analysis) judges them. Besides the
//! image itself it reads the libraries beside it that it imports; see
//! [`LibraryCache`].

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
    /// Reads `path` as a PE image and lists the directory it sits in. Parse
    /// failures come back as [`io::ErrorKind::InvalidData`].
    ///
    /// # Errors
    ///
    /// Fails if `path` cannot be read, is not a PE image, or has malformed
    /// import tables. An unlistable directory is not an error.
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

/// Reads each library beside an executable at most once, for both questions
/// asked of it: what it imports, and what version it carries.
///
/// Scoped to one directory, and it enforces that itself: the key is a library
/// name, so entering another directory drops everything.
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
    /// `None` when the file's import tables cannot be read. Recorded, so a broken
    /// DLL costs one read.
    linked: Option<Linked>,
    /// Never [`FileVersion::Elsewhere`] or [`FileVersion::Unread`]: the file
    /// was found in the directory and an attempt was made on it.
    version: FileVersion,
}

impl LibraryCache {
    /// Points the cache at the directory `path` sits in, forgetting another
    /// directory's files. `None` for a path with no directory.
    fn enter<'a>(&mut self, path: &'a Path) -> Option<&'a Path> {
        let dir = directory_of(path)?;
        if self.dir.as_deref() != Some(dir) {
            self.read.clear();
            self.listing = None;
            self.dir = Some(dir.to_owned());
        }
        Some(dir)
    }

    /// The names of the files beside `path`, listing the directory once however
    /// many of its files are inspected. `path` itself is filtered out here.
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

    /// The library `lower` names, read at most once per directory. `None` when
    /// no file of that name sits beside the executable.
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

    /// Follows an image's imports one step, into libraries sitting beside it,
    /// which is how a Unity stub's renderer is found. Unreadable libraries are
    /// skipped: they are supporting evidence nobody asked about.
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
            // Lowercased: PE import names are inconsistent about case.
            let lower = name.to_ascii_lowercase();
            if seen.contains(&lower) {
                continue;
            }
            // Only `.dll` files are followed. A `.exe` beside a game is another
            // candidate for being the game, not a library this one loads.
            if Path::new(&lower).extension() != Some("dll".as_ref()) {
                continue;
            }
            // A name not on disk does not spend the budget, which counts reads.
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
    /// this directory's reads. See [`stamp_versions`].
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

/// One library read: its import tables and its version, from one read. The two
/// fail independently.
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

/// The one-link chase for a single executable, with no cache. A caller walking
/// a directory should hold a [`LibraryCache`] instead.
#[must_use]
pub fn linked_libraries(
    path: &Path,
    imports: &[String],
    delay_imports: &[String],
    neighbours: &[String],
) -> Vec<Linked> {
    LibraryCache::default().follow(path, imports, delay_imports, neighbours)
}

/// Fills in the version of every file the verdict's signals name: the uncached
/// form of [`LibraryCache::stamp`].
///
/// Done after the verdict, so only the handful of files it names are read. The
/// rule has no exceptions: a file of that name beside the executable reports its
/// version, and anything else is [`Elsewhere`](FileVersion::Elsewhere).
/// `neighbours` supplies the directory's own spelling of each name.
pub fn stamp_versions(verdict: &mut Verdict, path: &Path, neighbours: &[String]) {
    LibraryCache::default().stamp(verdict, path, neighbours);
}

/// The directory `path` sits in: `.` for a bare file name, `None` for a root
/// or a bare prefix.
fn directory_of(path: &Path) -> Option<&Path> {
    match path.parent() {
        // A root, or a path that is nothing but a prefix: nothing sits beside it.
        None => None,
        Some(dir) if dir.as_os_str().is_empty() => Some(Path::new(".")),
        Some(dir) => Some(dir),
    }
}

/// A parse failure as an [`io::Error`], keeping the original error as the
/// source.
fn invalid_data(error: dxray_pe::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// File names sitting beside `path`, excluding `path` itself and every
/// subdirectory. An unreadable directory yields an empty list: the neighbours
/// are supporting evidence. The uncached form of [`LibraryCache::neighbours`].
#[must_use]
pub fn neighbours_of(path: &Path) -> Vec<String> {
    LibraryCache::default().neighbours(path)
}

/// Every file name directly in `dir`, sorted, with subdirectories left out. An
/// unreadable directory yields an empty list.
fn list_directory(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut names = Vec::new();
    for entry in entries.flatten() {
        // `file_type` does not follow symlinks: a link named `dxgi.dll` shadows
        // the system copy like a file.
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        names.push(entry.file_name().to_string_lossy().into_owned());
    }

    // Sorted, since `read_dir` promises no order and scans should be diffable.
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
        // Supporting evidence: learning nothing must not sink the record.
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

    /// The shape that exposed this: Honkai: Star Rail, a Unity stub whose
    /// renderer is one link on.
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
        // Only imported libraries are followed, never a stray DLL in the folder.
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
        // The executable's own import table is the stronger claim.
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
        // Built from the constant, so it proves the budget is a hard stop; the
        // number itself is pinned in `install::tests`.
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
        // A DLL that is not a PE at all must cost nothing.
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
