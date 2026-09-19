//! Everything this crate can establish about one installed game.
//!
//! [`launcher::walk`](crate::launcher::walk) answers "what is installed". This
//! answers "what is true about this one", and the two are kept apart on
//! purpose: a caller listing a machine's libraries may not want the per-game
//! work done at all, and folding it into the traversal would make that
//! impossible to decline.
//!
//! # Why this is a function and not two calls at each call site
//!
//! It was two calls at each call site, and the two sites had already drifted.
//! The terminal browser asked [`candidates`](crate::candidates) with the game's
//! title; the command line asked it with `None`. The title is a ranking signal,
//! so the same directory could be handed to both surfaces and come back with a
//! different executable named as the game — one answer per surface to a
//! question that has one answer.
//!
//! Nothing announced that. Both calls looked correct in isolation, which is
//! what a duplicated sequence looks like right up until the day it is not. The
//! same shape is how this repository's dependency classification came to exist
//! in two tables, only one of which had heard of `_nvngx.dll`.
//!
//! So the sequence lives here, once. The next fact worth establishing about a
//! game is added to [`Inspection`] and both surfaces get it, or neither does.
//!
//! # What is deliberately *not* here
//!
//! Anything that decides how a fact is shown: which of the survey's notes are
//! worth a row, how a score is worded, what a missing directory should look
//! like on a terminal. The browser streams these facts into a list as it finds
//! them and the listing prints them when it is finished, and those are two
//! legitimate presentations of one set of facts.

use std::io;
use std::path::Path;

use crate::game::Survey;
use crate::launcher::Game;
use crate::proton::{Answer, Builds};

/// What is known about one installed game, beyond what its launcher said.
#[derive(Debug)]
pub struct Inspection {
    /// Every executable in the install directory, ranked, with reasons.
    ///
    /// Still a `Result`, and the typed [`io::Error`] survives rather than being
    /// worded here, because the two failures are not the same failure: a
    /// missing install directory has no survey to report, while other errors
    /// prevent a reliable search. [`is_incomplete_error`] shares that policy
    /// between launcher listings without discarding the diagnostic.
    pub survey: io::Result<Survey>,
    /// What Proton will do to this game's NVAPI, or why that has no answer.
    ///
    /// Never absent. A game with nothing said about its NVAPI reads as a game
    /// with nothing wrong with it, so the refusal is a sentence rather than a
    /// silence — and for a launcher with no Steam application id, a sentence
    /// naming that launcher.
    pub nvapi: Answer,
}

/// Establishes what this crate knows about `game`, found in `library`.
///
/// `builds` is threaded through rather than built here so that one reading per
/// Proton build serves a whole scan. A library where a hundred games share one
/// Proton would otherwise tokenise the same two thousand lines a hundred times.
///
/// The game's title is always handed to the ranking. It is a real signal — an
/// executable whose name resembles the title outranks one that does not — and
/// the caller that used to withhold it was getting a worse answer to the same
/// question for no stated reason.
pub fn inspect(game: &Game, library: &Path, builds: &mut Builds) -> Inspection {
    Inspection {
        survey: crate::candidates(&game.install_dir, Some(&game.name)),
        nvapi: builds.answer_for(library, game),
    }
}

/// Whether an error reading a launcher-listed game's root makes the scan incomplete.
///
/// `NotFound` leaves the diagnostic visible but does not mark the scan incomplete.
/// It can mean a pending download, removal, or an unavailable mount; the error
/// alone does not establish which. Other errors prevent a reliable search.
///
/// This policy does not apply to an explicitly requested game directory or to
/// unreadable subdirectories recorded in a survey's notes.
#[must_use]
pub fn is_incomplete_error(error: &io::Error) -> bool {
    error.kind() != io::ErrorKind::NotFound
}

/// True only when this install was read and nothing in it argues it is a game.
///
/// The argument is [`Survey::has_evidence`]'s answer for the install, or `None`
/// where the install directory could not be read and the question was never
/// put. Both surfaces that order and mark a library by evidence — the terminal
/// browser's list and the `--installed`/`--steam` listing — ask exactly this,
/// so neither can decide on its own that an unread install belongs beside the
/// ones that were read and found to argue nothing.
///
/// That distinction is the whole of the policy. `None` is not "no evidence": it
/// is an unanswered question, and putting it where an answer of "nothing"
/// belongs is the conflation this project refuses. A surface that demoted it
/// would be hiding a failure under a finding.
///
/// It says what was observed and stops there. It does not say "tool": a field
/// that claimed to mark tooling is what this project already retired for
/// marking shipped games as tooling, and a rule that names a category will one
/// day meet a category it does not know.
#[must_use]
pub fn lacks_evidence(carries_evidence: Option<bool>) -> bool {
    carries_evidence == Some(false)
}

#[cfg(test)]
mod tests {
    use super::{inspect, lacks_evidence};
    use crate::game::{NameFrom, Reason};
    use crate::launcher::{Game, Identity};
    use crate::proton::Builds;
    use crate::testutil::TempDir;
    use std::path::{Path, PathBuf};

    fn game(install_dir: PathBuf) -> Game {
        Game {
            identity: Identity::SteamApp(570),
            name: "Dota 2".to_owned(),
            install_dir,
            origin: crate::steam::ORIGIN,
        }
    }

    #[test]
    fn the_title_is_handed_to_the_ranking_rather_than_withheld_by_one_caller() {
        // The bug this function exists to make unrepresentable. Both surfaces
        // ask which executable in a directory is the game; one of them used to
        // ask without the title, which is a ranking signal, so the same
        // directory could come back with two different answers.
        //
        // The decoy is named to sort *before* the title under byte order, and
        // the assertion is on the reason rather than on the winner. Without the
        // title neither file scores anything, no tie is noted at zero, and the
        // order falls through to the path — so a decoy named `unins000.exe`
        // let `Dota 2.exe` win on `D` being 0x44 and `u` 0x75, and the test
        // passed with the title withheld. A test that a tie-break can satisfy
        // is not guarding the thing it names.
        let install = TempDir::new("inspect-title");
        install.write("Dota 2.exe", "MZ");
        install.write("CrashReporter.exe", "MZ");

        let facts = inspect(
            &game(install.path().to_path_buf()),
            Path::new("/not/a/library"),
            &mut Builds::default(),
        );
        let survey = facts.survey.expect("the directory is listable");
        let best = survey.best().expect("two executables were written");

        assert!(
            best.reasons.iter().any(|reason| matches!(
                reason,
                Reason::NameResembles {
                    from: NameFrom::Supplied,
                    ..
                }
            )),
            "the title has to reach the ranking as evidence, or the two \
             surfaces answer one question differently: {:?}",
            best.reasons
        );
        assert!(
            best.path.ends_with("Dota 2.exe"),
            "and that evidence has to be what decides the answer: {:?}",
            best.path
        );
    }

    #[test]
    fn a_directory_that_is_not_there_comes_back_typed_rather_than_worded() {
        // The caller decides whether a missing directory costs an exit code —
        // mid-download is normal, a broken mount is not — and it cannot decide
        // that from a sentence.
        let facts = inspect(
            &game(PathBuf::from("/dxray-inspect-no-such-directory")),
            Path::new("/not/a/library"),
            &mut Builds::default(),
        );

        let error = facts.survey.expect_err("the directory does not exist");
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::NotFound,
            "the kind is what tells a mid-download install from a broken mount"
        );
    }

    #[test]
    fn an_install_that_was_never_read_is_not_an_install_that_argued_nothing() {
        // The one line both surfaces order and mark by. A directory that could
        // not be walked was never asked the question, and demoting it would put
        // an unanswered question where an answer of "nothing" belongs — which
        // is the exact conflation between an absent answer and a wrong one that
        // this project refuses.
        assert!(
            lacks_evidence(Some(false)),
            "read, and nothing in it argues it is a game"
        );
        assert!(
            !lacks_evidence(None),
            "unread is not empty; it stays with the games and says why"
        );
        assert!(!lacks_evidence(Some(true)));
    }

    #[test]
    fn every_game_gets_an_nvapi_sentence_even_when_there_is_no_answer() {
        // A game with nothing said about its NVAPI reads as a game with nothing
        // wrong with it, so the absence is worded rather than left blank.
        let facts = inspect(
            &game(PathBuf::from("/dxray-inspect-no-such-directory")),
            Path::new("/not/a/library"),
            &mut Builds::default(),
        );

        assert!(!facts.nvapi.verdict.is_empty());
        assert!(
            facts.nvapi.available.is_none(),
            "an unread prefix must not colour a word either way"
        );
    }
}
