//! Everything this project can say about one game, gathered on the scanning
//! thread and sent whole, so the thread that draws never opens a file. Every
//! absent field is absent for a reason the screen can print.

use std::io;
use std::path::PathBuf;

use dxray_core::analysis::Verdict;
use dxray_core::game::{Candidate, Note, Survey};
use dxray_core::launcher::{Game, Identity, Origin};

/// One game, its best executable, what that executable can reach, and what
/// Proton does to its NVAPI.
#[derive(Debug, Clone)]
pub struct Entry {
    /// What the launcher calls this game, and so what can be asked about it.
    pub identity: Identity,
    /// The launcher, and backend, that supplied this game.
    pub origin: Origin,
    pub name: String,
    pub install_dir: PathBuf,
    /// The library this game was found in, since two installs can declare the
    /// same one.
    pub library: PathBuf,
    /// The highest-ranked executable, or why there is none.
    pub best: Best,
    /// The survey's caveats, already worded and carried in full.
    pub notes: Vec<String>,
    /// The tie sentence, when the survey reported one. Also in `notes`; kept
    /// apart so the list column can draw `tied`.
    pub tie: Option<String>,
    /// Whether this install carries any evidence of being a game, exactly as
    /// [`Survey::has_evidence`] answers it, or `None` when it could not be read.
    /// `None` is not "no evidence": that question was never put.
    pub carries_evidence: Option<bool>,
    /// True when survey notes or a root error mean an incomplete search. A
    /// missing directory keeps its diagnostic without the marker, as in the CLI.
    pub incomplete: bool,
    pub nvapi: dxray_core::proton::Answer,
}

/// The executable the ranking chose, or the reason it chose none.
#[derive(Debug, Clone)]
pub enum Best {
    /// One was ranked. It may still carry no evidence at all — see
    /// [`Ranked::has_evidence`].
    Ranked(Box<Ranked>),
    /// The directory listed fine and holds no program: ordinary mid-download.
    NoExecutable,
    /// The install directory itself could not be walked.
    Unwalkable(String),
}

/// One executable and everything concluded about it.
#[derive(Debug, Clone)]
pub struct Ranked {
    pub path: PathBuf,
    pub score: i32,
    /// Why it ranked where it did, strongest first, already worded.
    pub reasons: Vec<String>,
    /// How many executables were ranked in total, which the score means nothing
    /// without.
    pub of: usize,
    /// What the image can reach, or why it could not be read. An error is not an
    /// empty verdict.
    pub verdict: Result<Verdict, String>,
}

impl Ranked {
    /// True when something was observed about this executable, which the detail
    /// pane asks. Not [`Entry::carries_evidence`], which asks about the install.
    #[must_use]
    pub fn has_evidence(&self) -> bool {
        !self.reasons.is_empty()
    }
}

impl Entry {
    /// Assembles the record from what `dxray-core` produced. Nothing
    /// launcher-specific is left to do here.
    #[must_use]
    pub fn build(
        game: Game,
        library: PathBuf,
        survey: io::Result<Survey>,
        nvapi: dxray_core::proton::Answer,
    ) -> Self {
        let Game {
            identity,
            name,
            install_dir,
            origin,
        } = game;
        let survey = match survey {
            Ok(survey) => survey,
            Err(error) => {
                let incomplete = dxray_core::inspect::is_incomplete_error(&error);
                return Self {
                    identity,
                    origin,
                    name,
                    install_dir,
                    library,
                    best: Best::Unwalkable(error.to_string()),
                    notes: Vec::new(),
                    tie: None,
                    // Nothing was read, so nothing can be said either way.
                    carries_evidence: None,
                    incomplete,
                    nvapi,
                };
            }
        };

        // Asked of core, in the same call `--installed` and `--game` make, so
        // no two surfaces can mark one install differently.
        let incomplete = survey.is_incomplete();
        // Asked once, of core, before the survey is taken apart below.
        let carries_evidence = Some(survey.has_evidence());
        let tie = survey.notes.iter().find_map(|note| match note {
            Note::TiedAtTheTop { .. } => Some(note.to_string()),
            _ => None,
        });
        let notes = survey.notes.iter().map(ToString::to_string).collect();
        let of = survey.candidates.len();
        let best = match survey.candidates.into_iter().next() {
            None => Best::NoExecutable,
            Some(candidate) => Best::Ranked(Box::new(rank(candidate, of))),
        };

        Self {
            identity,
            origin,
            name,
            install_dir,
            library,
            best,
            notes,
            tie,
            carries_evidence,
            incomplete,
            nvapi,
        }
    }

    /// True only when this install was read and nothing in it argues it is a
    /// game: [`inspect::lacks_evidence`](dxray_core::inspect::lacks_evidence),
    /// the rule the CLI listing uses too.
    #[must_use]
    pub fn lacks_evidence(&self) -> bool {
        dxray_core::inspect::lacks_evidence(self.carries_evidence)
    }

    /// What the launcher calls this game, as a person would type it.
    #[must_use]
    pub fn id_label(&self) -> String {
        self.identity.to_string()
    }

    /// The one-line answer for the list column. A file not read, a directory
    /// with no program and an image that recognises nothing each get their words.
    #[must_use]
    pub fn headline(&self) -> String {
        match &self.best {
            Best::Unwalkable(_) => "directory unreadable".to_owned(),
            Best::NoExecutable => "no executable here".to_owned(),
            Best::Ranked(ranked) => match &ranked.verdict {
                Err(_) => "executable unreadable".to_owned(),
                Ok(verdict) => verdict.headline(),
            },
        }
    }

    /// True when `needle`, already lowercased, appears in this game's name or
    /// its application id.
    #[must_use]
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        self.name.to_lowercase().contains(needle) || self.id_label().contains(needle)
    }
}

/// Reads the chosen executable and words its reasons.
fn rank(candidate: Candidate, of: usize) -> Ranked {
    let score = candidate.score();
    let reasons = candidate.reasons.iter().map(ToString::to_string).collect();
    // Read here, on the scanning thread, and never again. The verdict is a fact
    // about a file that the main loop has no business opening.
    let verdict = dxray_core::Evidence::from_executable(&candidate.path)
        .map(|evidence| {
            let mut verdict = dxray_core::analyse(&evidence);
            // Versions are read here, on the scanning thread, never by the drawer.
            dxray_core::evidence::stamp_versions(
                &mut verdict,
                &candidate.path,
                &evidence.neighbours,
            );
            verdict
        })
        .map_err(|error| error.to_string());
    Ranked {
        path: candidate.path,
        score,
        reasons,
        of,
        verdict,
    }
}

#[cfg(test)]
mod tests {
    use std::{io, path::PathBuf};

    use super::{Best, Entry};
    use dxray_core::game::{Candidate, Note, Survey};
    use dxray_core::{Identity, launcher::Game, proton::Answer, steam};

    fn game() -> Game {
        Game {
            identity: Identity::SteamApp(9999),
            name: "Still downloading".to_owned(),
            install_dir: PathBuf::from("/games/not-downloaded-yet"),
            origin: steam::ORIGIN,
        }
    }

    fn nvapi() -> Answer {
        Answer {
            script: None,
            verdict: "not configured".to_owned(),
            available: None,
            condition: Vec::new(),
        }
    }

    /// A survey that hit the depth bound, over `reasons` worth of evidence.
    fn truncated(reasons: Vec<dxray_core::game::Reason>) -> Survey {
        Survey::ranked(
            vec![Candidate {
                path: PathBuf::from("/games/entry/thing.exe"),
                reasons,
            }],
            vec![Note::DepthLimited {
                limit: 32,
                skipped: 3,
            }],
        )
    }

    #[test]
    fn a_truncated_walk_of_a_real_game_is_marked_incomplete_in_the_browser() {
        // The browser is a listing, and a game it listed whose directory was
        // only partly searched is a game whose named executable may be wrong.
        let entry = Entry::build(
            game(),
            PathBuf::from("/library"),
            Ok(truncated(vec![dxray_core::game::Reason::ShippingSuffix])),
            nvapi(),
        );

        assert!(
            entry.incomplete,
            "evidence of a game plus an unread subtree is an incomplete answer"
        );
    }

    #[test]
    fn a_truncated_walk_that_found_no_evidence_is_marked_incomplete_too() {
        // No evidence cannot excuse a truncation: the truncation may be why.
        let entry = Entry::build(
            game(),
            PathBuf::from("/library"),
            Ok(truncated(Vec::new())),
            nvapi(),
        );

        assert!(
            entry.incomplete,
            "no evidence plus an unread subtree is the shape where the evidence \
             may be what went unread"
        );
        assert!(
            entry
                .notes
                .iter()
                .any(|note| note.contains("directories deep")),
            "and the truncation is still carried, got {:?}",
            entry.notes
        );
    }

    #[test]
    fn a_missing_install_is_visible_without_claiming_the_scan_was_incomplete() {
        // Steam keeps a manifest before its directory exists: named, but not
        // marked as partly searched.
        let entry = Entry::build(
            game(),
            PathBuf::from("/steam"),
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "directory is absent",
            )),
            nvapi(),
        );

        assert!(
            matches!(entry.best, Best::Unwalkable(ref text) if text.contains("directory is absent"))
        );
        assert!(!entry.incomplete, "a missing install is not a partial scan");
    }

    #[test]
    fn a_permission_denied_install_is_marked_as_incomplete() {
        let entry = Entry::build(
            game(),
            PathBuf::from("/steam"),
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "permission denied",
            )),
            nvapi(),
        );

        assert!(
            matches!(entry.best, Best::Unwalkable(ref text) if text.contains("permission denied"))
        );
        assert!(
            entry.incomplete,
            "an unreadable existing install is incomplete"
        );
    }

    #[test]
    fn another_install_read_error_is_marked_as_incomplete() {
        let entry = Entry::build(
            game(),
            PathBuf::from("/steam"),
            Err(io::Error::other("stale mount")),
            nvapi(),
        );

        assert!(matches!(entry.best, Best::Unwalkable(ref text) if text.contains("stale mount")));
        assert!(
            entry.incomplete,
            "only NotFound is the normal absent-install case"
        );
    }
}
