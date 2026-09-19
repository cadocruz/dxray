//! Walking a game's install directory to find the executables in it.
//!
//! The IO half of [`game`](crate::game), kept as thin as
//! [`evidence`](crate::evidence) is. It lists directories, reads images and
//! hands the names over; every judgement about what those names mean is in
//! [`game::assess`](crate::game::assess), where it can be tested against an
//! awkward layout without a disk or a game.
//!
//! **The walk is bounded, and it says when a bound was hit.** A game directory
//! can hold tens of thousands of files, so there are four limits below and each
//! of them produces a [`Note`] when it bites. That matters more here than
//! anywhere else in the crate: a silently truncated scan that reports the
//! launcher because it never reached the real binary is a confident wrong
//! answer, which is the one thing this crate must not produce.
//!
//! **The order is breadth-first, and that is a compromise rather than a
//! solution.** Shallow directories are searched first, so a truncated walk
//! keeps the launcher and loses the shipping binary — the wrong half. Depth
//! first would lose a different half. There is no order that guarantees finding
//! the game inside a budget, so the budget is set high enough that a real
//! install does not come near it and the truncation is reported loudly when it
//! happens anyway.

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use dxray_pe::Pe;

use crate::analysis::{Evidence, Verdict, analyse};
use crate::evidence::LibraryCache;
use crate::game::{Candidate, Note, Observed, Survey, assess};

/// How far below the surveyed directory the walk goes. The root is depth zero.
///
/// Thirty-two, which is the same kind of number as [`MAX_EXECUTABLES`]: far
/// outside anything a game installer produces, so that hitting it is itself the
/// finding rather than a statement about the budget. Nothing an installer lays
/// down goes thirty-two levels below its own root; a tree that does is
/// generated, extracted or somebody's container sysroot.
///
/// It was eight — "around six, plus two spare" — and eight is *inside* the
/// range of real games. Measured across a real library, two installs of five
/// truncated at it, both on ordinary asset trees sitting at depth nine: a
/// jQuery `images/` folder inside a game's web UI, and a localised audio
/// package. A bound that fires on two games in five is not a sign that
/// something is wrong with the directory, and an exit code drawn from it says
/// nothing.
///
/// The raise costs no guarantee, because depth was never what bounded the work.
/// [`MAX_DIRECTORIES`] bounds it whatever shape the tree has, and no real
/// install came near that; and `Listing::read` uses `file_type`, which does not
/// follow symlinks, so a link pointing back up the tree cannot make the walk
/// run away no matter how deep the limit is.
pub const MAX_DEPTH: usize = 32;

/// How many directories the walk will list before giving up.
pub const MAX_DIRECTORIES: usize = 4096;

/// How many executables will be read and ranked.
///
/// A large install has tens. Five hundred is far past anything real, which is
/// the point: hitting it means something is wrong with the directory rather
/// than with the limit, and the [`Note`] says so.
pub const MAX_EXECUTABLES: usize = 512;

/// How many local libraries one executable's imports will be followed into.
///
/// The chain is followed exactly one link — into libraries the image imports
/// *and* that sit in its own directory — because that is what it takes to see
/// the renderer behind a stub loader like Unity's. Two links would start
/// reading the whole dependency graph of a game for a signal that is already
/// the weakest renderer evidence this module reports.
pub const MAX_LOCAL_LIBRARIES: usize = 16;

/// Every executable under `dir`, ranked best-first, each carrying its reasons.
///
/// `game_name` is the title from a Steam manifest or whatever else the caller
/// knows; `None` simply drops one signal rather than changing how the others
/// are weighed. The directory's own name is always used as a second, weaker
/// name to compare against, so a non-Steam install is not left with no name
/// evidence at all.
///
/// Nothing is filtered out. A directory holding nothing but a Visual C++
/// redistributable comes back with that redistributable in it, scoring zero and
/// saying why — see [`Survey::has_evidence`].
///
/// # Errors
///
/// Fails only if `dir` itself cannot be listed. A subdirectory that cannot be
/// read is a [`Note::Unreadable`] beside a partial answer, because losing the
/// rest of the tree over one unreadable folder is the bigger wrong answer.
pub fn candidates(dir: &Path, game_name: Option<&str>) -> io::Result<Survey> {
    let root = Listing::read(dir)?;
    let directory_name = dir.file_name().map(|n| n.to_string_lossy().into_owned());

    let mut walk = Walk::new(dir.to_path_buf());
    walk.run(root, game_name, directory_name.as_deref());
    Ok(walk.finish())
}

/// The names in one directory, split by kind and sorted.
///
/// Read once per directory and shared by every executable in it, because the
/// neighbours are a fact about the directory and reading them again per binary
/// would be the same answer at N times the cost.
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

        // One verdict for the whole directory: the neighbours are the same for
        // every binary in it, and this is the evidence that must never be taken
        // from the install root and attributed to a binary three levels down.
        let directory = analyse(&Evidence {
            neighbours: listing.files.clone(),
            ..Evidence::default()
        });
        // One cache for the directory, so an engine DLL that fifty executables
        // all import is parsed once. It is scoped here because a link is only
        // ever followed within one directory.
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
            // An executable this tool cannot parse is still an executable in a
            // game directory. It stays in the list, ranked on what its path
            // says, and the reason it could not be read travels with the
            // survey rather than being dropped.
            Err(source) => {
                self.notes.push(Note::Unparsed { path, source });
                return observed;
            }
        };
        // The one-link chase, through the same function the verdict uses. Two
        // implementations of this rule is how the score and the verdict printed
        // beside it came to disagree about the same binary.
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
