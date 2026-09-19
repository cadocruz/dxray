//! Whole install layouts, built on disk and ranked end to end.
//!
//! The assertions are about **order**. A list that contains the shipping binary
//! somewhere below the crash handler is not a ranking, and a test that only
//! checks membership would pass on one.

use super::{MAX_DEPTH, MAX_EXECUTABLES, MAX_LOCAL_LIBRARIES, candidates};
use crate::game::{Candidate, Note, Reason};
use crate::testutil::TempDir;

/// The ranking, as `stem: score`, best first. Everything below asserts against
/// this rather than against a single pick.
fn order(survey: &crate::game::Survey) -> Vec<String> {
    survey
        .candidates
        .iter()
        .map(|c| {
            format!(
                "{}: {}",
                c.path.file_name().unwrap_or_default().to_string_lossy(),
                c.score()
            )
        })
        .collect()
}

fn reasons_for(survey: &crate::game::Survey, name: &str) -> Vec<Reason> {
    let found = survey.candidates.iter().find(|c| c.path.ends_with(name));
    match found {
        Some(candidate) => candidate.reasons.clone(),
        None => panic!("{name} is missing from {:?}", order(survey)),
    }
}

fn has_note(survey: &crate::game::Survey, wanted: fn(&Note) -> bool) -> bool {
    survey.notes.iter().any(wanted)
}

#[test]
fn an_unreal_install_ranks_the_shipping_binary_above_the_launcher_that_starts_it() {
    // The shape this whole slice exists for: a small launcher at the root that
    // spawns the real binary four directories down, the engine's crash reporter
    // beside it, and no name list anywhere deciding between them.
    let dir = TempDir::new("unreal");
    dir.image("Satisfactory.exe", &["KERNEL32.dll"], &[]);
    dir.image(
        "FactoryGame/Binaries/Win64/FactoryGame-Win64-Shipping.exe",
        &["KERNEL32.dll", "d3d12.dll"],
        &[],
    );
    dir.image(
        "Engine/Binaries/Win64/CrashReportClient.exe",
        &["KERNEL32.dll"],
        &[],
    );
    dir.write("FactoryGame/Binaries/Win64/nvngx_dlss.dll", "x");

    let survey = candidates(dir.path(), Some("Satisfactory")).expect("survey");

    assert_eq!(
        order(&survey),
        [
            "FactoryGame-Win64-Shipping.exe: 220",
            "Satisfactory.exe: 30",
            "CrashReportClient.exe: 0",
        ],
        "shipping binary, then launcher, then the engine's own tool"
    );
    assert!(
        !reasons_for(&survey, "CrashReportClient.exe")
            .iter()
            .any(|r| matches!(r, Reason::UnrealBinariesDirectory { .. })),
        "Engine/Binaries/Win64 is the engine's, not the game's"
    );
}

#[test]
fn the_upscaler_beside_the_shipping_binary_is_not_credited_to_the_launcher_at_the_root() {
    // The bug predicted when `dxray-core` was finished: an Unreal game keeps
    // its DLSS libraries in `Binaries/Win64`, not at the install root, and
    // evidence gathered from the root and pinned on a binary three levels down
    // would make every executable in the tree look equally equipped.
    let dir = TempDir::new("perdir");
    dir.image("Launcher.exe", &["KERNEL32.dll"], &[]);
    dir.image(
        "Game/Binaries/Win64/Game-Win64-Shipping.exe",
        &["d3d12.dll"],
        &[],
    );
    dir.write("Game/Binaries/Win64/nvngx_dlss.dll", "x");

    let survey = candidates(dir.path(), None).expect("survey");

    assert!(
        reasons_for(&survey, "Game-Win64-Shipping.exe")
            .iter()
            .any(|r| matches!(r, Reason::ShipsGraphicsLibraries { .. })),
        "the binary in that directory gets the credit"
    );
    assert!(
        !reasons_for(&survey, "Launcher.exe")
            .iter()
            .any(|r| matches!(r, Reason::ShipsGraphicsLibraries { .. })),
        "and the one four directories above it does not"
    );
}

#[test]
fn a_unity_game_outranks_its_own_crash_handler_in_the_same_directory() {
    // Unity's `.exe` is a stub: it imports `UnityPlayer.dll` and nothing
    // graphical, so the strongest signal in the module is silent on it. The
    // `_Data` directory and the one link into `UnityPlayer.dll` are what keep
    // it above a handler that shares every neighbour it has.
    let dir = TempDir::new("unity");
    dir.image("MyGame.exe", &["UnityPlayer.dll", "KERNEL32.dll"], &[]);
    dir.image("UnityPlayer.dll", &["d3d11.dll", "d3d12.dll"], &[]);
    dir.image("UnityCrashHandler64.exe", &["KERNEL32.dll"], &[]);
    dir.dir("MyGame_Data");
    dir.write("nvngx_dlss.dll", "x");

    let survey = candidates(dir.path(), Some("My Game")).expect("survey");

    assert_eq!(
        order(&survey),
        ["MyGame.exe: 215", "UnityCrashHandler64.exe: 15"],
        "the handler keeps the directory's credit and it is nowhere near enough"
    );
    let reasons: Vec<String> = reasons_for(&survey, "MyGame.exe")
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("through UnityPlayer.dll")),
        "the renderer behind the stub is named, got {reasons:?}"
    );
}

#[test]
fn a_directory_holding_only_redistributables_reports_them_and_says_it_found_nothing() {
    // `vcredist_x64.exe` sinks on its own merits — it has no evidence — rather
    // than by appearing on a list of names that would one day hide a real game.
    // The executables are still listed: "there is nothing here that looks like
    // a game" is an answer, and an empty list is not the same answer.
    let dir = TempDir::new("redist");
    dir.image(
        "_CommonRedist/vcredist/vcredist_x64.exe",
        &["KERNEL32.dll"],
        &[],
    );
    dir.image("_CommonRedist/DirectX/DXSETUP.exe", &["KERNEL32.dll"], &[]);

    let survey = candidates(dir.path(), Some("Some Game")).expect("survey");

    assert_eq!(order(&survey), ["DXSETUP.exe: 0", "vcredist_x64.exe: 0"]);
    assert!(
        !survey.has_evidence(),
        "nothing here argues for being the game"
    );
    assert!(
        survey.best().is_some(),
        "and the files are still reported rather than hidden"
    );
}

#[test]
fn a_lone_executable_that_imports_nothing_at_all_is_ranked_and_declared_unevidenced() {
    // The degenerate case. There is exactly one answer available and no
    // evidence for it, and the report has to be able to say both things.
    let dir = TempDir::new("silent");
    dir.image("thing.exe", &[], &[]);

    let survey = candidates(dir.path(), None).expect("survey");

    assert_eq!(order(&survey), ["thing.exe: 0"]);
    assert!(!survey.has_evidence());
    assert!(
        has_note(&survey, |n| matches!(n, Note::NoRendererImported { .. })),
        "and that no import table backed any of it, got {:?}",
        survey
            .notes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_java_game_is_ranked_on_structure_and_the_listing_says_that_is_all_it_had() {
    // Project Zomboid draws from `jre64/bin/java.exe`, buried in a
    // subdirectory. Neither it nor the launcher imports a graphics API, because
    // the JVM loads the renderer with LoadLibrary long after startup. The
    // ranking is still worth having and must not be printed as though an import
    // table stood behind it.
    let dir = TempDir::new("java");
    dir.image("ProjectZomboid64.exe", &["KERNEL32.dll"], &[]);
    dir.image("ProjectZomboid32.exe", &["KERNEL32.dll"], &[]);
    dir.image("jre64/bin/java.exe", &["KERNEL32.dll", "jli.dll"], &[]);

    let survey = candidates(dir.path(), Some("Project Zomboid")).expect("survey");

    assert_eq!(
        order(&survey),
        [
            "ProjectZomboid32.exe: 15",
            "ProjectZomboid64.exe: 15",
            "java.exe: 0",
        ],
        "both builds survive; neither is picked over the other"
    );
    let note = survey
        .notes
        .iter()
        .find(|n| matches!(n, Note::NoRendererImported { .. }))
        .map(ToString::to_string)
        .expect("a structure-only ranking has to declare itself");
    assert!(note.contains("structure alone"), "got {note:?}");
}

#[test]
fn a_game_that_ties_with_an_unrelated_program_says_so_instead_of_winning_on_the_alphabet() {
    // Importing a renderer is not a games-only signal. An Electron main process
    // does it as a matter of course, and so does at least one shipped security
    // product. Put such a binary beside a game whose only evidence is its own
    // import — no engine layout to carry it — and they tie at 100, with the top
    // slot going to whichever sorts first. Nothing in the numbers says that was
    // a coin toss, so the note has to.
    let dir = TempDir::new("tie");
    dir.image(
        "Fortress.exe",
        &["KERNEL32.dll", "d3d9.dll", "dxgi.dll"],
        &[],
    );
    dir.image("hlgame.exe", &["KERNEL32.dll", "d3d9.dll"], &[]);

    let survey = candidates(dir.path(), None).expect("survey");

    assert_eq!(
        order(&survey),
        ["Fortress.exe: 100", "hlgame.exe: 100"],
        "the alphabet decided this, and that is the problem"
    );
    let note = survey
        .notes
        .iter()
        .find(|n| matches!(n, Note::TiedAtTheTop { .. }))
        .map(ToString::to_string)
        .expect("the tie has to be declared");
    assert!(
        note.contains("does not choose between them"),
        "got {note:?}"
    );
}

#[test]
fn a_binary_below_the_depth_limit_is_reported_missing_rather_than_silently_dropped() {
    // The wrong-answer shape this bound exists to avoid: a truncated walk that
    // reports the launcher because it never reached the real binary, with
    // nothing in the output to say the walk stopped early.
    let dir = TempDir::new("deep");
    let mut path = String::new();
    for _ in 0..=MAX_DEPTH {
        path.push_str("down/");
    }
    dir.image(&format!("{path}buried.exe"), &["d3d12.dll"], &[]);
    dir.image("top.exe", &["KERNEL32.dll"], &[]);

    let survey = candidates(dir.path(), None).expect("survey");

    assert_eq!(
        order(&survey),
        ["top.exe: 0"],
        "the buried one was not reached"
    );
    let note = survey
        .notes
        .iter()
        .find(|n| matches!(n, Note::DepthLimited { .. }))
        .map(ToString::to_string)
        .expect("the truncation has to be reported");
    assert!(
        note.contains("missing from this list"),
        "and it has to say the answer may be the part that is gone, got {note:?}"
    );
    assert!(
        survey.is_incomplete(),
        "and it has to be incomplete to every caller, whatever the part that was \
         read says for itself: `top.exe` scores zero, so a rule that asked for \
         evidence before believing a truncation would call this exact survey \
         complete — and this survey is a game that went unread"
    );
}

#[test]
fn a_directory_with_more_executables_than_the_budget_stops_and_says_where() {
    // Tens of thousands of files is a normal game directory. Hitting this limit
    // is not, which is why the note says so rather than trimming quietly.
    let dir = TempDir::new("many");
    for i in 0..=MAX_EXECUTABLES {
        dir.write(&format!("f{i:04}.exe"), "not a pe image");
    }

    let survey = candidates(dir.path(), None).expect("survey");

    assert_eq!(
        survey.candidates.len(),
        MAX_EXECUTABLES,
        "the budget is a hard stop"
    );
    assert!(
        has_note(&survey, |n| matches!(n, Note::ExecutableLimited { .. })),
        "and it is declared"
    );
}

#[test]
fn an_executable_that_cannot_be_parsed_stays_in_the_list_with_the_reason_attached() {
    // "The game is the corrupt one" is an answer somebody needs, and a binary
    // dropped for failing to parse is a binary the report never mentions.
    let dir = TempDir::new("broken");
    dir.write("broken.exe", "MZ but nothing else");
    dir.image("Game.exe", &["d3d11.dll"], &[]);

    let survey = candidates(dir.path(), None).expect("survey");

    assert_eq!(order(&survey), ["Game.exe: 100", "broken.exe: 0"]);
    assert!(
        has_note(&survey, |n| matches!(n, Note::Unparsed { .. })),
        "the failure is reported beside the answer, got {:?}",
        survey
            .notes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_directory_that_is_not_there_fails_rather_than_coming_back_as_an_empty_ranking() {
    // The one case with no partial answer worth returning: an empty list from a
    // missing directory would read as an install with no executables in it.
    let error = candidates(
        std::path::Path::new("/dxray-no-such-install-anywhere"),
        None,
    )
    .expect_err("a missing root is a failure");

    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

/// A permission error on one folder must not sink a survey that is otherwise
/// complete, for the same reason one corrupt manifest does not sink a Steam
/// library.
///
/// Unix only, because taking read permission away from a directory is the only
/// portable way to build the case and Windows does not spell it the same way.
/// The behaviour under test is not platform-specific; the fixture is.
#[cfg(unix)]
#[test]
fn a_subdirectory_that_cannot_be_read_costs_itself_and_no_other_part_of_the_tree() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("unreadable");
    dir.image("Game.exe", &["d3d12.dll"], &[]);
    let closed = dir.dir("closed");
    dir.image("closed/inner.exe", &["KERNEL32.dll"], &[]);
    std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o000)).expect("chmod");

    // Running as root, the mode is advisory and the directory still opens. The
    // test would then assert nothing, so it says so rather than passing quietly.
    let locked = std::fs::read_dir(&closed).is_err();
    let survey = candidates(dir.path(), None);
    let _ = std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o755));
    let survey = survey.expect("the root is still listable");

    assert!(
        locked,
        "this process can read a 000 directory, so the fixture proves nothing"
    );
    assert_eq!(order(&survey), ["Game.exe: 100"]);
    assert!(
        has_note(&survey, |n| matches!(n, Note::Unreadable { .. })),
        "the folder that could not be opened is named, got {:?}",
        survey
            .notes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_name_of_the_directory_is_used_when_the_caller_supplies_no_name_at_all() {
    // A non-Steam install has no manifest to draw a title from, and the folder
    // somebody chose to put the game in is the only name evidence there is.
    let dir = TempDir::new("named");
    let stem = dir
        .path()
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned();
    dir.image(&format!("{stem}.exe"), &["KERNEL32.dll"], &[]);

    let survey = candidates(dir.path(), None).expect("survey");

    assert!(
        survey
            .candidates
            .first()
            .is_some_and(Candidate::has_evidence),
        "the directory named it, got {:?}",
        order(&survey)
    );
}

#[test]
fn the_walk_budgets_are_the_numbers_they_are_documented_to_be() {
    // Every other test here builds its fixture *from* these constants, so the
    // fixture resizes with the constant and the limit is proved to exist while
    // its value is pinned by nothing. Changing 8 to 3 or 512 to 4 broke no test.
    // These are the values, and the reasons they are not other values:
    //
    //   MAX_DEPTH 32 — far outside anything an installer lays down, so that
    //   reaching it means the tree is generated or extracted rather than
    //   shipped. It was 8, and 8 is inside the range of real games: two installs
    //   of five in a measured library truncated at it on ordinary asset trees
    //   at depth nine. Raising it costs no bound, because MAX_DIRECTORIES is
    //   what bounds the work and the walk does not follow symlinks.
    //
    //   MAX_EXECUTABLES 512 — a large install has tens. The number is far past
    //   anything real on purpose: reaching it means the directory is wrong, not
    //   the budget, and a smaller one would truncate ordinary installs and
    //   report the truncation as if it were that same anomaly.
    //
    //   MAX_LOCAL_LIBRARIES 16 — one link deep through the imports that sit in
    //   the executable's own directory. A stub loader names its engine DLL
    //   somewhere in a normal-sized import table, not necessarily first.
    assert_eq!(MAX_DEPTH, 32);
    assert_eq!(MAX_EXECUTABLES, 512);
    assert_eq!(MAX_LOCAL_LIBRARIES, 16);
}
