//! Everything this crate can establish about one installed game, in one
//! function so both surfaces ask the same questions the same way. How a fact
//! is shown is left to each surface.

use std::io;
use std::path::Path;

use crate::game::Survey;
use crate::launcher::Game;
use crate::proton::{Answer, Builds};

/// What is known about one installed game, beyond what its launcher said.
#[derive(Debug)]
pub struct Inspection {
    /// Every executable in the install directory, ranked, with reasons. The
    /// typed error survives: a missing directory is not a failed search.
    pub survey: io::Result<Survey>,
    /// What Proton will do to this game's NVAPI, or why that has no answer.
    /// Always a sentence, never a blank.
    pub nvapi: Answer,
}

/// Establishes what this crate knows about `game`, found in `library` under
/// the launcher installation `root`. `builds` is shared across a scan.
pub fn inspect(
    game: &Game,
    root: Option<&Path>,
    library: &Path,
    builds: &mut Builds,
) -> Inspection {
    Inspection {
        survey: crate::candidates(&game.install_dir, Some(&game.name)),
        nvapi: builds.answer_for(root, library, game),
    }
}

/// Whether an error reading a launcher-listed game's root makes the scan
/// incomplete. `NotFound` does not: it may be a download in progress.
#[must_use]
pub fn is_incomplete_error(error: &io::Error) -> bool {
    error.kind() != io::ErrorKind::NotFound
}

/// True only when this install was read and nothing in it argues it is a game.
/// `None` means it was never read, which is not an answer of "nothing".
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
        // The decoy sorts before the title, so only the title can decide.
        let install = TempDir::new("inspect-title");
        install.write("Dota 2.exe", "MZ");
        install.write("CrashReporter.exe", "MZ");

        let facts = inspect(
            &game(install.path().to_path_buf()),
            None,
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
        // Typed, so the caller can tell a download in progress from a broken mount.
        let facts = inspect(
            &game(PathBuf::from("/dxray-inspect-no-such-directory")),
            None,
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
        // Unread is not "nothing".
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
            None,
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
