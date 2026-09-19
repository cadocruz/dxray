//! `game` end to end: whole install layouts on disk, through the real binary.
//!
//! The unit tests in `src/game.rs` prove the listing renders what it is given.
//! These prove the walk, the ranking and the analysis line up behind one
//! command, and they assert **order** — a run that contains the shipping binary
//! somewhere below the crash handler has failed even though every name is
//! present.

mod common;

use common::{Image, TempDir, dxray, stdout_of};

/// Where each named executable appears in the output, so the assertions can be
/// about which came first rather than about which was mentioned.
fn position(text: &str, needle: &str) -> usize {
    text.find(needle)
        .unwrap_or_else(|| panic!("{needle} is missing from:\n{text}"))
}

/// An Unreal install: a launcher at the root, the shipping binary four
/// directories down with its upscalers beside it, the engine's crash reporter,
/// and two installers nobody removed.
fn unreal(dir: &TempDir) {
    dir.write(
        "FactoryGame.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );
    dir.write(
        "FactoryGame/Binaries/Win64/FactoryGame-Win64-Shipping.exe",
        &Image::x64()
            .importing(&["KERNEL32.dll", "d3d11.dll"])
            .delay_loading(&["d3d12.dll", "dxgi.dll"])
            .build(),
    );
    dir.write(
        "Engine/Binaries/Win64/CrashReportClient.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );
    dir.write(
        "_CommonRedist/vcredist/vcredist_x64.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );
    dir.write("FactoryGame/Binaries/Win64/nvngx_dlss.dll", b"x");
}

#[test]
fn the_shipping_binary_leads_the_ranking_and_is_the_one_that_gets_analysed() {
    // The whole slice in one command: find the executables, rank them on
    // evidence, explain the ranking, then report on the winner.
    let dir = TempDir::new("game-unreal");
    unreal(&dir);

    let out = dxray(["game", &dir.path().display().to_string()]);
    let text = stdout_of(&out);

    assert!(
        position(text, "FactoryGame-Win64-Shipping.exe") < position(text, "CrashReportClient.exe"),
        "the game outranks the engine's crash reporter, got:\n{text}"
    );
    assert!(
        position(text, "FactoryGame-Win64-Shipping.exe") < position(text, "vcredist_x64.exe"),
        "and the leftover installer, got:\n{text}"
    );
    assert!(
        text.contains("verdict   Direct3D 11 or Direct3D 12"),
        "the winner is then analysed in full, got:\n{text}"
    );
    assert_eq!(out.status.code(), Some(0), "nothing failed, got:\n{text}");
}

#[test]
fn every_executable_is_listed_with_its_score_and_not_only_the_winner() {
    // Rank and explain, do not pick. The launcher and the game are both
    // legitimate answers to different questions, and a tool that names one and
    // hides the other is lying by omission.
    let dir = TempDir::new("game-listed");
    unreal(&dir);

    let text = stdout_of(&dxray(["game", &dir.path().display().to_string()])).to_owned();

    for name in [
        "FactoryGame-Win64-Shipping.exe",
        "FactoryGame.exe",
        "CrashReportClient.exe",
        "vcredist_x64.exe",
    ] {
        assert!(text.contains(name), "{name} is missing from:\n{text}");
    }
    assert!(
        text.contains("links Direct3D 11 or Direct3D 12 (import)"),
        "the reason for the top score is printed, got:\n{text}"
    );
    assert!(
        text.contains("nothing observed argues that this is the game"),
        "and so is the absence of one, got:\n{text}"
    );
}

#[test]
fn a_unity_stub_beats_the_crash_handler_that_shares_every_file_it_has() {
    // The Unity `.exe` imports no graphics API — `UnityPlayer.dll` does the
    // drawing — so the strongest signal is silent and the layout has to carry
    // it. The handler sits in the same directory and shares its neighbours.
    let dir = TempDir::new("game-unity");
    dir.write(
        "Cuphead.exe",
        &Image::x64()
            .importing(&["UnityPlayer.dll", "KERNEL32.dll"])
            .build(),
    );
    dir.write(
        "UnityPlayer.dll",
        &Image::x64()
            .importing(&["KERNEL32.dll", "d3d11.dll"])
            .build(),
    );
    dir.write(
        "UnityCrashHandler64.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );
    dir.write("Cuphead_Data/resources.assets", b"x");
    dir.write("nvngx_dlss.dll", b"x");

    let text = stdout_of(&dxray(["game", &dir.path().display().to_string()])).to_owned();

    assert!(
        position(&text, "Cuphead.exe") < position(&text, "UnityCrashHandler64.exe"),
        "got:\n{text}"
    );
    assert!(
        text.contains("Cuphead_Data"),
        "the layout evidence is named, got:\n{text}"
    );
    assert!(
        text.contains("through UnityPlayer.dll"),
        "and so is the renderer behind the stub, got:\n{text}"
    );
}

#[test]
fn a_directory_of_installers_is_reported_as_holding_no_game_and_still_exits_zero() {
    // `vcredist_x64.exe` sinks because it has nothing to say for itself, not
    // because it is on a list of names. Finding no game is an answer, not a
    // failure: the directory was read and this is what is in it.
    let dir = TempDir::new("game-redist");
    dir.write(
        "vcredist_x64.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );
    dir.write(
        "DirectX/DXSETUP.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );

    let out = dxray(["game", &dir.path().display().to_string()]);
    let text = stdout_of(&out);

    assert!(
        text.contains("no executable here carries any evidence of being the game"),
        "got:\n{text}"
    );
    assert!(
        text.contains("vcredist_x64.exe") && text.contains("DXSETUP.exe"),
        "both are still listed rather than hidden, got:\n{text}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "an honest 'no game here' is not a failure, got:\n{text}"
    );
}

#[test]
fn a_game_whose_renderer_loads_at_run_time_is_ranked_and_the_listing_says_why_that_is_thin() {
    // Java, Electron and .NET games import no graphics API: the runtime loads
    // it with LoadLibrary long after startup. The ranking still works off the
    // structure, and it must not be printed as confidently as one an import
    // table stands behind.
    let dir = TempDir::new("game-java");
    dir.write(
        "ProjectZomboid64.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );
    dir.write(
        "ProjectZomboid32.exe",
        &Image::x86().importing(&["KERNEL32.dll"]).build(),
    );
    dir.write(
        "jre64/bin/java.exe",
        &Image::x64().importing(&["KERNEL32.dll", "jli.dll"]).build(),
    );

    let text = stdout_of(&dxray(["game", &dir.path().display().to_string()])).to_owned();

    assert!(
        text.contains("rests on directory structure alone"),
        "the caveat rides with the answer, got:\n{text}"
    );
    assert!(
        text.contains("Java, Electron or .NET"),
        "including the half that is a finding in its own right, got:\n{text}"
    );
    assert!(
        text.contains("ProjectZomboid32.exe") && text.contains("ProjectZomboid64.exe"),
        "both builds ship and both are launchable; neither is hidden, got:\n{text}"
    );
}

#[test]
fn the_json_line_appends_the_ranking_without_disturbing_the_frozen_keys() {
    // A harness reads the first eight keys positionally. `game` extends a
    // record; it does not reshape one.
    let dir = TempDir::new("game-json");
    unreal(&dir);

    let out = dxray(["game", "--json", &dir.path().display().to_string()]);
    let text = stdout_of(&out);
    let line = text.lines().next().expect("one line per directory");

    assert!(
        line.starts_with(r#"{"path":""#),
        "the record shape is unchanged, got {line}"
    );
    assert!(
        line.contains(r#","error":null,"verdict":"#),
        "the frozen eight still end at error, got {line}"
    );
    assert!(
        line.contains(r#""candidates":[{"path":"#),
        "and the ranking is appended after them, got {line}"
    );
    assert!(
        line.contains(r#""kind":"links-renderer","#),
        "each reason carries a stable name, got {line}"
    );
    assert_eq!(
        text.lines().count(),
        1,
        "one line per directory, got:\n{text}"
    );
}

#[test]
fn a_directory_with_no_executable_in_it_fails_rather_than_printing_a_blank_ranking() {
    // The question asked was which executable here is the game, and there is no
    // executable to answer with. In `steam`, which sweeps a whole machine and
    // meets games that are mid-download, the same state costs nothing.
    let dir = TempDir::new("game-empty");
    dir.write("readme.txt", b"nothing to see");

    let out = dxray(["game", &dir.path().display().to_string()]);

    assert!(
        stdout_of(&out).contains("no executable found in this directory"),
        "got:\n{}",
        stdout_of(&out)
    );
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn a_file_passed_to_the_game_flag_is_inspected_as_a_file() {
    // A mixed argument list should still do the obvious thing with the files in
    // it rather than refusing the whole run.
    let dir = TempDir::new("game-file");
    let exe = dir.write("lone.exe", &Image::x64().importing(&["d3d12.dll"]).build());

    let out = dxray(["game", &exe.display().to_string()]);

    assert!(
        stdout_of(&out).contains("verdict   Direct3D 12"),
        "got:\n{}",
        stdout_of(&out)
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn game_rejects_recursive_scan_option() {
    // Recursive file walking belongs to inspect, not installation ranking.
    let out = dxray(["game", "--recursive", "."]);

    assert_eq!(
        out.status.code(),
        Some(2),
        "a bad command line is neither a clean run nor a failed scan"
    );
}

#[test]
fn a_walk_that_stopped_at_its_limit_moves_the_exit_code_as_well_as_printing_a_note() {
    // The note explaining that the real binary may be below the limit is on
    // stdout, where a script never reads it. The exit code has always answered
    // one question — was everything read — and a bounded walk that hit its
    // bound did not read everything.
    let dir = TempDir::new("game-deep");
    dir.write(
        "launcher.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );
    let mut buried = String::new();
    // Built from the constant, so the fixture stays out of reach when the bound
    // moves. The value itself is pinned in `dxray-core`, on purpose.
    for _ in 0..=dxray_core::install::MAX_DEPTH {
        buried.push_str("down/");
    }
    buried.push_str("real-game.exe");
    dir.write(&buried, &Image::x64().importing(&["d3d12.dll"]).build());

    let out = dxray(["game", &dir.path().display().to_string()]);
    let text = stdout_of(&out);

    assert!(
        text.contains("missing from this list entirely"),
        "got:\n{text}"
    );
    assert!(
        !text.contains("real-game.exe"),
        "the fixture has to actually be out of reach, got:\n{text}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a truncated walk is an incomplete read, got:\n{text}"
    );
}

#[test]
fn a_thin_but_complete_answer_still_exits_zero() {
    // The other side of the same line. Everything was read; the ranking rests
    // on structure because the binaries import no renderer, and that is a
    // finding rather than a failure.
    let dir = TempDir::new("game-thin");
    dir.write(
        "ProjectZomboid64.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );

    let out = dxray(["game", &dir.path().display().to_string()]);

    assert!(
        stdout_of(&out).contains("rests on directory structure alone"),
        "got:\n{}",
        stdout_of(&out)
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn a_tie_for_first_place_is_declared_rather_than_settled_by_the_alphabet() {
    // Importing a renderer is not a games-only signal: an Electron main process
    // does it as a matter of course, and so does at least one shipped security
    // product. Beside a game whose only evidence is its own import, such a
    // binary ties at 100 and the top slot goes to whichever sorts first. The
    // numbers alone read exactly as confident as a correct answer, so the
    // listing has to say that the order was not the evidence's doing.
    let dir = TempDir::new("game-tie");
    dir.write(
        "Fortress.exe",
        &Image::x64()
            .importing(&["KERNEL32.dll", "d3d9.dll", "dxgi.dll"])
            .build(),
    );
    dir.write(
        "hlgame.exe",
        &Image::x64()
            .importing(&["KERNEL32.dll", "d3d9.dll"])
            .build(),
    );

    let out = dxray(["game", &dir.path().display().to_string()]);
    let text = stdout_of(&out);
    // The note is wrapped to the terminal width, so a phrase is asserted
    // against the text with its line breaks folded back into spaces.
    let unwrapped = text.split_whitespace().collect::<Vec<_>>().join(" ");

    assert!(
        unwrapped.contains("2 executables share the top score of 100"),
        "the tie is on the page, got:\n{text}"
    );
    assert!(
        unwrapped.contains("presentation and not a finding"),
        "and so is the fact that the order was arbitrary, got:\n{text}"
    );
    assert!(
        position(text, "Fortress.exe") < position(text, "hlgame.exe"),
        "the order itself is still deterministic, got:\n{text}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "everything was read; the answer is weak, not missing"
    );
}

#[test]
fn a_ranking_the_evidence_actually_settled_carries_no_tie_note() {
    // The other half. A caveat that appears on every run is a caveat nobody
    // reads, and this one has to keep its meaning.
    let dir = TempDir::new("game-clear");
    unreal(&dir);

    let text = stdout_of(&dxray(["game", &dir.path().display().to_string()])).to_owned();

    assert!(
        !text.contains("share the top score"),
        "the layout separated them, got:\n{text}"
    );
}
