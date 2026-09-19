//! Everything this project can say about one game, gathered into one value.
//!
//! Built on the scanning thread and sent whole down the channel, so the main
//! loop never opens a file. That is not tidiness: the main loop is the one that
//! draws, and anything it reads from a disk is a frame the user does not get.
//!
//! Every field that can be absent is absent for a reason the screen can print.
//! There is no `Option` here that renders as a blank — a game with nothing
//! known about it is a real state, and the detail pane says which kind of
//! nothing it is.

use std::io;
use std::path::PathBuf;

use dxray_core::analysis::Verdict;
use dxray_core::game::{Candidate, Note, Survey};
use dxray_core::launcher::{Game, Identity, Origin};

/// One game, its best executable, what that executable can reach, and what
/// Proton does to its NVAPI.
#[derive(Debug, Clone)]
pub struct Entry {
    /// What the launcher calls this game, and therefore what can be asked
    /// about it.
    ///
    /// One field where there used to be three — a `u32` that was zero for
    /// Heroic, an `Option<String>` beside it, and an enum saying which to
    /// believe. Zero is a real Steam appid, so the placeholder could not be
    /// told from a game, and keeping the three in step was left to whoever
    /// touched them next. [`Identity`] is the core type that makes that
    /// impossible to get wrong.
    pub identity: Identity,
    /// The launcher, and backend, that supplied this game.
    pub origin: Origin,
    pub name: String,
    pub install_dir: PathBuf,
    /// The library this game was found in. Kept because two Steam installs can
    /// declare the same library, and a reader looking at a duplicate-looking
    /// title needs to see which shelf it came off.
    pub library: PathBuf,
    /// The highest-ranked executable, or why there is none.
    pub best: Best,
    /// The survey's caveats, already worded. Carried in full rather than
    /// summarised: these are the sentences that say the answer above them may
    /// be wrong, and a list view is exactly where they would get dropped.
    pub notes: Vec<String>,
    /// Set when the survey reported a tie at the top, with the sentence that
    /// says so.
    ///
    /// Held apart from `notes` only because the list column needs a marker it
    /// can draw beside a headline. The sentence is still in `notes` as well, so
    /// the detail pane cannot show a tie marker with no explanation under it.
    /// The list column draws it as `tied`.
    pub tie: Option<String>,
    /// Whether this install carries any evidence of being a game, exactly as
    /// [`Survey::has_evidence`] answers it, or `None` when the install
    /// directory could not be read and the question therefore has no answer.
    ///
    /// The one question this browser orders and marks by. It is asked of
    /// `dxray-core` and never re-derived here: the same call decides the CLI's
    /// "no executable here carries any evidence of being the game", so the two
    /// surfaces cannot end up disagreeing about one install.
    ///
    /// Deliberately not read off [`Ranked`], which knows only about the
    /// executable that ranked first. The two do agree today, and only for an
    /// arithmetic reason: the survey sorts by score, every weight in
    /// [`weight`](dxray_core::game::weight) is positive, and so a candidate
    /// with reasons always outranks one without. Deriving the install's answer
    /// from the top candidate's would make it a consequence of the ranking's
    /// numbers rather than of the question asked, and the day a reason worth
    /// nothing is added it would quietly become wrong. Asking core is the same
    /// price and stays right.
    ///
    /// `None` is not "no evidence". An install that could not be walked was
    /// never asked, and drawing it beside the directories that were read and
    /// found empty is exactly the conflation this project refuses.
    pub carries_evidence: Option<bool>,
    /// True when survey notes or a root error indicate an incomplete search.
    /// Shares the root-error policy with CLI launcher listings: a missing
    /// directory retains its diagnostic without the `not searched in full`
    /// marker. Survey notes use [`Survey::is_incomplete`], the one question
    /// `--game` and the CLI listings ask too.
    pub incomplete: bool,
    pub nvapi: dxray_core::proton::Answer,
}

/// The executable the ranking chose, or the reason it chose none.
#[derive(Debug, Clone)]
pub enum Best {
    /// One was ranked. It may still carry no evidence at all — see
    /// [`Ranked::has_evidence`].
    Ranked(Box<Ranked>),
    /// The directory listed fine and holds no program.
    ///
    /// The ordinary state of a game that is mid-download, which is why it is a
    /// finding here and not an error.
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
    /// How many executables were ranked in total. The denominator the score
    /// means nothing without: "100" is a different claim in a directory of one
    /// than in a directory of forty.
    pub of: usize,
    /// What the image can reach, or why that could not be read.
    ///
    /// A missing verdict is not the same as an empty one. An empty verdict says
    /// the file was read and recognises nothing; this says the file was not
    /// read, and the screen prints the error rather than a blank line.
    pub verdict: Result<Verdict, String>,
}

impl Ranked {
    /// True when something was observed about **this executable**.
    ///
    /// Not the same question as [`Entry::carries_evidence`], which asks whether
    /// anything in the whole install argues it is a game. This one is asked by
    /// the detail pane, which is printing one executable's reasons and needs to
    /// know whether it has any to print.
    ///
    /// They answer alike on everything `dxray-core` builds, because no reason
    /// is worth nothing and the survey sorts by score — see
    /// [`weight`](dxray_core::game::weight), where that premise is written down
    /// and held by a test. That agreement is a fact about the weights, not
    /// about the two questions, so neither call is derived from the other: each
    /// is asked of its own subject and stays right when the other's changes.
    #[must_use]
    pub fn has_evidence(&self) -> bool {
        !self.reasons.is_empty()
    }
}

impl Entry {
    /// Assembles the record from what `dxray-core` produced.
    ///
    /// One constructor for every launcher, where there used to be one per
    /// launcher and a private one underneath holding eight positional
    /// arguments. There is nothing launcher-specific left to do here: the game
    /// carries its own identity and origin, and the NVAPI answer was already
    /// decided — by [`Builds::answer_for`](dxray_core::proton::Builds::answer_for),
    /// which is the one place in the project that knows a Heroic game cannot be
    /// asked.
    ///
    /// `survey` is consumed rather than borrowed because every sentence in it
    /// is rendered here and nothing downstream wants the structure back.
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
    /// game.
    ///
    /// The browser's ordering and its `no evidence` marker both turn on this
    /// one call. False for an install that could not be read: that is an
    /// unanswered question, not an answer of "nothing", and the headline
    /// already says which.
    ///
    /// The rule itself is
    /// [`inspect::lacks_evidence`](dxray_core::inspect::lacks_evidence) and is
    /// not restated here. The `--installed` listing demotes and marks by the
    /// same question, and two surfaces that each spelled out what `None` means
    /// could be changed apart — one browser showing an unreadable install
    /// beside the games while the listing buried it.
    #[must_use]
    pub fn lacks_evidence(&self) -> bool {
        dxray_core::inspect::lacks_evidence(self.carries_evidence)
    }

    /// What the launcher calls this game, as a person would type it.
    #[must_use]
    pub fn id_label(&self) -> String {
        self.identity.to_string()
    }

    /// The one-line answer for the list column.
    ///
    /// Worded so that the three ways of knowing nothing stay apart: a file that
    /// was not read, a directory with no program in it, and an image that was
    /// read and recognises nothing all get their own words. A blank would say
    /// all three at once and would be wrong about two of them.
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

    /// True when `needle` — already lowercased by the caller — appears in this
    /// game's name or in its application id.
    ///
    /// The id is matched as well as the name because an appid is the one thing
    /// about a game that is unambiguous, and somebody who arrived here from a
    /// Proton bug report has the number and not the title.
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
            // Versions belong to the scanning thread for the same reason the
            // verdict does: every file the main loop opens is a frame nobody
            // gets. Read through the one shared function, never here.
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
        // The absence of evidence cannot narrow this, because the truncation is
        // itself a candidate explanation for the absence: the shipping binary
        // may be the thing below the bound. An entry whose survey found only a
        // zero-evidence `launcher.exe` and stopped early is precisely the
        // answer most likely to be wrong, so it is the last one to mark clean.
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
        // Steam deliberately retains a manifest while its download directory
        // has not appeared. The TUI must name that fact, but it must not draw
        // the marker reserved for a directory that existed and was only partly
        // searched.
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
