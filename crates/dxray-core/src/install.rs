//! Walking a game's install directory to find the executables in it: the IO
//! half of [`game`](crate::game).
//!
//! The walk is bounded, and every bound that bites becomes a [`Note`]. It is
//! breadth-first, which is a compromise: no order finds the game inside a
//! budget, so the budget is set far beyond real installs.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use dxray_pe::Pe;

use crate::analysis::{Evidence, Verdict, analyse};
use crate::evidence::LibraryCache;
use crate::game::{Candidate, Note, Observed, Survey, assess};

/// How far below the surveyed directory the walk goes; the root is depth zero.
/// Far outside real installs: eight truncated two of five real games. The work
/// is bounded by [`MAX_DIRECTORIES`] whatever the depth, and symlinks are never
/// followed.
pub const MAX_DEPTH: usize = 32;

/// How many directories the walk will list before giving up.
pub const MAX_DIRECTORIES: usize = 4096;

/// How many executables will be read and ranked. Far past anything real, so
/// hitting it says something is wrong with the directory.
pub const MAX_EXECUTABLES: usize = 512;

/// How many local libraries one executable's imports are followed into, one
/// link deep: enough to see the renderer behind a Unity stub.
pub const MAX_LOCAL_LIBRARIES: usize = 16;

/// Every executable under `dir`, ranked best-first, each carrying its reasons.
/// `game_name` is one signal; the directory's own name is always a second, weaker
/// one. Nothing is filtered out.
///
/// # Errors
///
/// Fails only if `dir` itself cannot be listed. An unreadable subdirectory is a
/// [`Note::Unreadable`] beside a partial answer.
pub fn candidates(dir: &Path, game_name: Option<&str>) -> io::Result<Survey> {
    let root = Listing::read(dir)?;
    let directory_name = dir.file_name().map(|n| n.to_string_lossy().into_owned());

    let mut walk = Walk::new(dir.to_path_buf());
    walk.run(root, game_name, directory_name.as_deref());
    Ok(walk.finish())
}

/// The names in one directory, split by kind and sorted, read once and shared.
struct Listing {
    path: PathBuf,
    files: Vec<String>,
    directories: Vec<String>,
}

impl Listing {
    fn read(path: &Path) -> io::Result<Self> {
        let mut files = Vec::new();
        let mut directories = Vec::new();
        for entry in fs::read_dir(path)? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            // `file_type` does not follow symlinks, so a link pointing back up
            // the tree is never descended into and cannot loop forever.
            match entry.file_type() {
                Ok(t) if t.is_dir() => directories.push(name),
                Ok(_) => files.push(name),
                Err(_) => {}
            }
        }
        files.sort_unstable();
        directories.sort_unstable();
        Ok(Self {
            path: path.to_path_buf(),
            files,
            directories,
        })
    }
}

/// The state of one walk: the queue, the budgets and what has been found.
struct Walk {
    root: PathBuf,
    queue: VecDeque<(PathBuf, usize)>,
    candidates: Vec<Candidate>,
    notes: Vec<Note>,
    listed: usize,
    skipped_for_depth: usize,
    stopped: bool,
}

impl Walk {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            queue: VecDeque::new(),
            candidates: Vec::new(),
            notes: Vec::new(),
            listed: 0,
            skipped_for_depth: 0,
            stopped: false,
        }
    }

    /// Ranks the executables in `root`, queues its subdirectories, and keeps
    /// going until the queue runs dry or a budget does.
    fn run(&mut self, root: Listing, game_name: Option<&str>, directory_name: Option<&str>) {
        let mut current = (root, 0usize);
        loop {
            let (listing, depth) = current;
            self.listed += 1;
            self.enqueue(&listing, depth);
            self.rank(&listing, game_name, directory_name);
            if self.stopped {
                break;
            }
            match self.next() {
                Some(next) => current = next,
                None => break,
            }
        }
        self.wrap_up();
    }

    /// The next directory to list, skipping the ones that cannot be read.
    fn next(&mut self) -> Option<(Listing, usize)> {
        while let Some((path, depth)) = self.queue.pop_front() {
            if self.listed >= MAX_DIRECTORIES {
                self.notes.push(Note::DirectoryLimited {
                    limit: MAX_DIRECTORIES,
                });
                self.queue.clear();
                return None;
            }
            match Listing::read(&path) {
                Ok(listing) => return Some((listing, depth)),
                Err(source) => self.notes.push(Note::Unreadable { path, source }),
            }
        }
        None
    }

    fn enqueue(&mut self, listing: &Listing, depth: usize) {
        if depth >= MAX_DEPTH {
            self.skipped_for_depth += listing.directories.len();
            return;
        }
        for name in &listing.directories {
            self.queue.push_back((listing.path.join(name), depth + 1));
        }
    }

    fn wrap_up(&mut self) {
        if self.skipped_for_depth > 0 {
            self.notes.push(Note::DepthLimited {
                limit: MAX_DEPTH,
                skipped: self.skipped_for_depth,
            });
        }
    }

    fn finish(self) -> Survey {
        Survey::ranked(self.candidates, self.notes)
    }

    /// Reads every executable in one directory and assesses it.
    fn rank(&mut self, listing: &Listing, game_name: Option<&str>, directory_name: Option<&str>) {
        let executables: Vec<&String> = listing.files.iter().filter(|n| is_executable(n)).collect();
        if executables.is_empty() {
            return;
        }

        // One verdict for the directory, shared by every binary in it.
        let directory = analyse(&Evidence {
            neighbours: listing.files.clone(),
            ..Evidence::default()
        });
        // One cache per directory, since links are only followed within one.
        let mut links = LibraryCache::default();

        for name in executables {
            if self.candidates.len() >= MAX_EXECUTABLES {
                self.notes.push(Note::ExecutableLimited {
                    limit: MAX_EXECUTABLES,
                });
                self.stopped = true;
                return;
            }
            let path = listing.path.join(name);
            let observed = self.observe(path, listing, &directory, &mut links);
            self.candidates
                .push(assess(&observed, game_name, directory_name));
        }
    }

    /// Everything known about one executable, gathered.
    fn observe(
        &mut self,
        path: PathBuf,
        listing: &Listing,
        directory: &Verdict,
        links: &mut LibraryCache,
    ) -> Observed {
        let relative = path.strip_prefix(&self.root).unwrap_or(&path).to_path_buf();
        let mut observed = Observed {
            path: path.clone(),
            relative,
            sibling_directories: listing.directories.clone(),
            own: Verdict::default(),
            directory: directory.clone(),
        };

        let named = match read_names(&path) {
            Ok(named) => named,
            // Unparsable, but still an executable: ranked on its path, with a note.
            Err(source) => {
                self.notes.push(Note::Unparsed { path, source });
                return observed;
            }
        };
        // The same one-link chase the verdict uses.
        let linked = links.follow(&path, &named.imports, &named.delay_imports, &listing.files);
        observed.own = analyse(&Evidence {
            imports: named.imports,
            delay_imports: named.delay_imports,
            linked,
            ..Evidence::default()
        });
        observed
    }
}

/// The import tables of one image.
struct Named {
    imports: Vec<String>,
    delay_imports: Vec<String>,
}

fn read_names(path: &Path) -> io::Result<Named> {
    let bytes = fs::read(path)?;
    let pe = Pe::parse(&bytes).map_err(invalid_data)?;
    Ok(Named {
        imports: pe.imports().map_err(invalid_data)?,
        delay_imports: pe.delay_imports().map_err(invalid_data)?,
    })
}

/// A parse failure as an [`io::Error`], for the same reason
/// [`evidence`](crate::evidence) does it: every caller is already handling one.
fn invalid_data(error: dxray_pe::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// True for a file named like a program. Only `.exe`: a game shipped as a DLL
/// behind a stub launcher is out of scope, and would be found as the stub.
fn is_executable(name: &str) -> bool {
    has_extension(name, "exe")
}

fn has_extension(name: &str, extension: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
}

#[cfg(test)]
mod tests;
