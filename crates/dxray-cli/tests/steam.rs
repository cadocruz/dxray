//! End to end tests for `steam`: the real binary, a fake Steam tree, real
//! exit codes.
//!
//! The tree is built here rather than found on the machine because there is no
//! Steam on the machine — none, on any drive. What these tests prove is that
//! the discovery path handles the layout `dxray-core::steam` believes in, and
//! not that the belief matches a real install.
//!
//! Something else in this repository does answer that now, which this comment
//! denied for as long as it was true. A real Steam has been found through the
//! candidate paths on one Linux machine, twice, with `libraryfolders.vdf`
//! where this layout expects it and the manifest keys spelled as expected; the
//! README section on what has met a real machine says how far that goes and
//! where it stops. These fixtures stay synthetic anyway, because a test that
//! needs a Steam install is a test that only runs on one machine.

mod common;

use common::{TempDir, dxray_with_home, stderr_of, stdout_of};
use std::path::Path;

/// Builds a Steam install under `home`, at the first place discovery looks.
///
/// `~/.steam/steam` rather than `~/.local/share/Steam` because it is first in
/// the candidate list, so a test using it also proves that the ordering is
/// being honoured.
fn fake_steam(home: &TempDir) -> std::path::PathBuf {
    let root = home.path().join(".steam/steam");
    let steamapps = root.join("steamapps");
    std::fs::create_dir_all(steamapps.join("common/dota 2 beta")).expect("tree");

    std::fs::write(
        steamapps.join("libraryfolders.vdf"),
        format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
            root.display()
        ),
    )
    .expect("index");

    std::fs::write(
        steamapps.join("appmanifest_570.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"570\"\n\t\"name\"\t\t\"Dota 2\"\n\
         \t\"installdir\"\t\t\"dota 2 beta\"\n}\n",
    )
    .expect("manifest");

    root
}

#[test]
fn a_steam_install_is_listed_with_its_library_and_the_games_in_it() {
    // The whole path in one go: roots, the library index, the manifests, and
    // the join that turns a bare installdir into a directory somebody can cd
    // into.
    let home = TempDir::new("steam-home");
    let root = fake_steam(&home);

    let out = dxray_with_home(home.path(), ["steam"]);
    let text = stdout_of(&out);

    assert!(
        text.contains(&root.display().to_string()),
        "the install root is named, got:\n{text}"
    );
    assert!(text.contains("570"), "the appid is listed, got:\n{text}");
    assert!(text.contains("Dota 2"), "the title is listed, got:\n{text}");
    assert!(
        text.contains("Path: ./steamapps/common/dota 2 beta"),
        "the install directory is resolved relative to the named library, got:\n{text}"
    );
    assert_eq!(out.status.code(), Some(0), "a clean scan exits 0");
}

#[test]
fn the_root_listed_inside_its_own_index_is_not_reported_as_two_libraries() {
    // The current schema names the root as entry "0". Without the dedup every
    // game in the main library is printed twice, which reads as two copies
    // installed rather than as one counted twice.
    let home = TempDir::new("steam-dedup");
    fake_steam(&home);

    let text = stdout_of(&dxray_with_home(home.path(), ["steam"])).to_owned();

    assert_eq!(
        text.matches("Dota 2").count(),
        1,
        "one game, listed once, got:\n{text}"
    );
    assert!(
        text.contains("1 game in 1 library across 1 install"),
        "the trailer counts each thing once, got:\n{text}"
    );
}

#[test]
fn a_machine_with_no_steam_says_where_it_looked_and_exits_1() {
    // An empty list and a 0 would be byte for byte what a working scan of a
    // machine with no games produces, and the two are not the same state. The
    // candidate paths are printed because "no Steam found" alone gives a
    // person with Steam plainly installed nothing to act on.
    if Path::new("/usr/local/games/Steam").is_dir() {
        // The one candidate that is not under HOME. If a real one exists here,
        // this test cannot create a Steam-less machine and would be asserting
        // something about the host rather than about the code.
        return;
    }
    let home = TempDir::new("steam-none");

    let out = dxray_with_home(home.path(), ["steam"]);
    let text = stderr_of(&out);

    assert!(text.contains("no Steam installation found"), "got:\n{text}");
    assert!(
        text.contains(".steam/steam"),
        "the candidates are named, got:\n{text}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "finding nothing is a failure, not an empty success"
    );
}

#[test]
fn a_corrupt_library_index_fails_loudly_rather_than_reporting_an_empty_machine() {
    // A truncated index parsed leniently says "one library, no games", which
    // is indistinguishable from a fresh install. The exit code has to carry
    // the difference for anything scripting this.
    let home = TempDir::new("steam-corrupt");
    let root = fake_steam(&home);
    std::fs::write(
        root.join("steamapps/libraryfolders.vdf"),
        "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\"/mnt/g\"\n",
    )
    .expect("truncate the index");

    let out = dxray_with_home(home.path(), ["steam"]);

    assert_eq!(out.status.code(), Some(1), "an unreadable index is not a 0");
    assert!(
        stderr_of(&out).contains("libraryfolders.vdf"),
        "the failing file is named, got:\n{}",
        stderr_of(&out)
    );
    // A root that prints its own path and then nothing underneath reads as an
    // install with no games. The line saying why must not live only on the
    // stream that gets dropped when the output is pasted somewhere.
    assert!(
        stdout_of(&out).contains("libraryfolders.vdf"),
        "and named in the listing itself, got:\n{}",
        stdout_of(&out)
    );
}

#[test]
fn an_index_declaring_no_entries_is_not_byte_identical_to_a_healthy_one() {
    // The defect this pair holds shut, at the level it was actually reported:
    // end to end, both of these used to print the same bytes and exit 0, so a
    // current-schema index truncated just past its bookkeeping keys lost every
    // library on every other drive and looked like perfect health.
    //
    // Both directions are asserted. A test for the note alone would pass on an
    // implementation that noted every file, which is useless the other way
    // round — it would train a reader to skip the line.
    let bookkeeping_only = TempDir::new("steam-note");
    let root = fake_steam(&bookkeeping_only);
    std::fs::write(
        root.join("steamapps/libraryfolders.vdf"),
        "\"libraryfolders\" { \"contentstatsid\" \"-123456789\" }\n",
    )
    .expect("an index with bookkeeping and nothing else");

    let healthy = TempDir::new("steam-healthy");
    fake_steam(&healthy);

    let noted = dxray_with_home(bookkeeping_only.path(), ["steam"]);
    let clean = dxray_with_home(healthy.path(), ["steam"]);
    let noted_text = stdout_of(&noted);
    let clean_text = stdout_of(&clean);

    assert!(
        noted_text.contains("declared no library entries"),
        "the ambiguity must be surfaced, got:\n{noted_text}"
    );
    assert!(
        noted_text.contains("only the Steam root is being scanned"),
        "and the consequence stated, got:\n{noted_text}"
    );
    assert!(
        !clean_text.contains("declared no library entries"),
        "a healthy index must NOT be noted, or the note means nothing:\n{clean_text}"
    );

    // The note reaches stdout, not only stderr: the listing is what gets pasted
    // somewhere, and a caveat on the other stream is the half that gets lost.
    assert!(
        stdout_of(&noted).contains("declared no library entries"),
        "the note belongs on stdout"
    );

    // And it reaches the count line, which is the sentence people quote. The
    // row above is a line in a block; "2 games in 1 library" on its own is what
    // gets repeated, and on this install it is a number with doubt attached.
    // Asserted end to end because the row and the trailer are fed by two
    // separate statements in the renderer: the row can print perfectly while
    // the trailer forgets, and every other assertion here would still pass.
    assert!(
        noted_text.contains("declared no libraries, so there may be more"),
        "the doubt has to ride in the same line as the number, got:\n{noted_text}"
    );
    assert!(
        !clean_text.contains("so there may be more"),
        "and a healthy index must leave the count line alone:\n{clean_text}"
    );
    // Still a caveat, not a failure. An old single-library install is healthy,
    // and exiting 1 on every one of them for ever would teach people to ignore
    // the code — which is the one thing an exit code cannot survive.
    assert_eq!(noted.status.code(), Some(0), "a caveat is not a failure");
    assert_eq!(clean.status.code(), Some(0));

    // The games still come back. The note qualifies the answer; it does not
    // replace it.
    assert!(noted_text.contains("Dota 2"), "got:\n{noted_text}");
}

#[test]
fn a_corrupt_manifest_costs_its_own_game_and_no_others() {
    // The behaviour the whole `Scan` shape exists for. A scan of 7000 files
    // does not abort on the first unreadable one, and one bad manifest must not
    // cost a user every other game in the library. The bad file is still named
    // and the exit code still says the scan was incomplete — showing the good
    // games is not the same as pretending nothing went wrong.
    let home = TempDir::new("steam-partial");
    let root = fake_steam(&home);
    std::fs::write(
        root.join("steamapps/appmanifest_999.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\"999\"\n\t\"name\"\t\"Trunc",
    )
    .expect("a truncated manifest beside a good one");

    let out = dxray_with_home(home.path(), ["steam"]);
    let text = stdout_of(&out);

    assert!(
        text.contains("Dota 2"),
        "the readable game survives its neighbour, got:\n{text}"
    );
    assert!(
        text.contains("appmanifest_999.acf"),
        "the unreadable one is named rather than dropped, got:\n{text}"
    );
    assert!(
        stderr_of(&out).contains("appmanifest_999.acf"),
        "and named on stderr, so a piped run still sees it, got:\n{}",
        stderr_of(&out)
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "an incomplete scan is not a clean one"
    );
}

#[test]
fn steam_needs_no_paths_and_answers_the_same_question_in_json() {
    // Inventory commands discover their roots and share a dedicated JSONL schema.
    let home = TempDir::new("steam-flags");
    let root = fake_steam(&home);

    assert_eq!(
        dxray_with_home(home.path(), ["steam"]).status.code(),
        Some(0),
        "no PATH argument is needed"
    );

    let out = dxray_with_home(home.path(), ["steam", "--json"]);
    let json = stdout_of(&out);

    assert_eq!(out.status.code(), Some(0), "got:\n{}", stderr_of(&out));
    for line in json.lines() {
        assert!(
            line.starts_with("{\"kind\":\"") && line.ends_with('}'),
            "every line is one tagged object, got: {line}"
        );
    }
    assert!(
        json.contains(&format!(
            "{{\"kind\":\"install\",\"origin\":\"steam\",\"origin_label\":\"Steam\",\"path\":\"{}\"}}",
            root.display()
        )),
        "got:\n{json}"
    );
    assert!(
        json.contains("\"id\":\"570\",\"steam_appid\":570,\"name\":\"Dota 2\""),
        "the identity a program would take, as a number where there is one: {json}"
    );
    assert!(
        json.contains(
            "\"kind\":\"summary\",\"games\":1,\"libraries\":1,\"installs\":1,\"complete\":true"
        ),
        "got:\n{json}"
    );
    // The narrower flag still does not spend a row on the launcher it already
    // named — and the field is there regardless, because a shape that changes
    // its keys with the flag is a shape every consumer has to special-case.
    assert!(
        !json.contains("{\"label\":\"origin\""),
        "the row a person did not get is not invented here either, got:\n{json}"
    );
    assert!(
        json.contains("\"kind\":\"game\",\"origin\":\"steam\""),
        "got:\n{json}"
    );
}

#[test]
fn the_steam_json_refuses_nothing_the_steam_listing_prints() {
    // A manifest that could not be read reaches both surfaces or neither. It
    // is the row most worth having in a machine-readable inventory: a game the
    // user owns that this tool could not account for.
    let home = TempDir::new("steam-json-partial");
    let root = fake_steam(&home);
    std::fs::write(
        root.join("steamapps/appmanifest_999.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\"999\"\n\t\"name\"\t\"Trunc",
    )
    .expect("a truncated manifest beside a good one");

    let human = dxray_with_home(home.path(), ["steam"]);
    let machine = dxray_with_home(home.path(), ["steam", "--json"]);
    let text = stdout_of(&human);
    let json = stdout_of(&machine);

    assert_eq!(
        human.status.code(),
        machine.status.code(),
        "one scan, one verdict on it"
    );
    assert_eq!(machine.status.code(), Some(1), "got:\n{json}");
    assert!(
        json.contains("\"kind\":\"problem\"") && json.contains("appmanifest_999.acf"),
        "got:\n{json}"
    );
    assert!(json.contains("\"complete\":false"), "got:\n{json}");
    let trailer = text
        .lines()
        .last()
        .expect("the human run ends in a trailer");
    assert!(
        json.contains(&format!("\"says\":\"{trailer}\"")),
        "the summary is the trailer a person read: {trailer:?}\n{json}"
    );
}

#[test]
fn a_path_given_alongside_steam_is_a_usage_error_rather_than_a_silently_ignored_argument() {
    // Inventory commands reject explicit paths rather than silently ignoring them.
    let home = TempDir::new("steam-path");
    fake_steam(&home);
    let file = home.path().join("game.exe");
    std::fs::write(
        &file,
        common::Image::x64().importing(&["d3d12.dll"]).build(),
    )
    .expect("write");

    let out = dxray_with_home(
        home.path(),
        [std::ffi::OsStr::new("steam"), file.as_os_str()],
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "a path that would not be looked at is refused, not accepted and dropped"
    );
    assert!(
        !stdout_of(&out).contains("game.exe"),
        "and nothing was scanned, got:\n{}",
        stdout_of(&out)
    );
}

#[test]
fn each_game_gets_the_executable_this_tool_would_analyse_named_under_its_directory() {
    // Discovery used to stop at the install directory. It now says which
    // executable in it is the game, with the reason attached, because a path to
    // a folder is not an answer to "what does this game link against".
    let home = TempDir::new("steam-best");
    let root = fake_steam(&home);
    let install = root.join("steamapps/common/dota 2 beta");
    std::fs::create_dir_all(install.join("game/bin/win64")).expect("tree");
    std::fs::write(
        install.join("game/bin/win64/dota2.exe"),
        common::Image::x64()
            .importing(&["KERNEL32.dll", "vulkan-1.dll"])
            .build(),
    )
    .expect("fixture");
    std::fs::write(
        install.join("launcher.exe"),
        common::Image::x64().importing(&["KERNEL32.dll"]).build(),
    )
    .expect("fixture");

    let text = stdout_of(&dxray_with_home(home.path(), ["steam"])).to_owned();

    assert!(text.contains("best"), "the row exists at all, got:\n{text}");
    assert!(
        text.contains("game/bin/win64/dota2.exe"),
        "and names the binary that links a renderer, got:\n{text}"
    );
    assert!(
        text.contains("links Vulkan (import)"),
        "with the reason it won, got:\n{text}"
    );
}

#[test]
fn a_redistributable_package_answers_for_itself_instead_of_being_filtered_by_name() {
    // `steam::games` refuses to filter manifests, on the grounds that any list
    // of names will one day hide a real game and that the executable ranking
    // one layer up has evidence instead. This is that promise being kept: a
    // folder of installers reports that nothing in it looks like a game.
    let home = TempDir::new("steam-redist");
    let root = fake_steam(&home);
    let steamapps = root.join("steamapps");
    std::fs::create_dir_all(steamapps.join("common/Steamworks Shared")).expect("tree");
    std::fs::write(
        steamapps.join("appmanifest_228980.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"228980\"\n\t\"name\"\t\t\"Steamworks Common Redistributables\"\n\
         \t\"installdir\"\t\t\"Steamworks Shared\"\n}\n",
    )
    .expect("manifest");
    std::fs::write(
        steamapps.join("common/Steamworks Shared/vcredist_x64.exe"),
        common::Image::x64().importing(&["KERNEL32.dll"]).build(),
    )
    .expect("fixture");

    let out = dxray_with_home(home.path(), ["steam"]);
    let text = stdout_of(&out);

    assert!(
        text.contains("nothing here carries evidence of being a game"),
        "got:\n{text}"
    );
    assert!(
        text.contains("Steamworks Common Redistributables"),
        "the manifest is still listed rather than hidden, got:\n{text}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "a folder with no game in it is a finding, not a failure"
    );
}

#[test]
fn a_truncated_walk_is_incomplete_in_the_json_too_though_nothing_failed_to_read() {
    // The one caveat with no unreadable file behind it: everything was read,
    // a bound stopped the walk, and the executable named for that install may
    // not be the right one. It moves the exit code, and the summary's
    // `complete` is that same predicate — so a consumer that reads the stream
    // and never the status cannot come away thinking this scan was whole.
    let home = TempDir::new("steam-json-truncated");
    let root = fake_steam(&home);
    let install = root.join("steamapps/common/dota 2 beta");
    let mut buried = install.clone();
    for _ in 0..=dxray_core::install::MAX_DEPTH {
        buried.push("down");
    }
    std::fs::create_dir_all(&buried).expect("a tree deeper than the walk goes");
    std::fs::write(
        buried.join("dota2.exe"),
        common::Image::x64().importing(&["vulkan-1.dll"]).build(),
    )
    .expect("fixture");
    std::fs::write(
        install.join("dota2.exe"),
        common::Image::x64().importing(&["d3d11.dll"]).build(),
    )
    .expect("fixture");

    let out = dxray_with_home(home.path(), ["steam", "--json"]);
    let json = stdout_of(&out);

    assert_eq!(out.status.code(), Some(1), "got:\n{json}");
    assert!(json.contains("\"complete\":false"), "got:\n{json}");
    assert!(
        json.contains("could not be searched in full"),
        "the caveat is data as well as prose, got:\n{json}"
    );
    assert!(
        json.contains(&format!("\"incomplete\":[\"{}", install.display())),
        "and it rides on the install it qualifies, got:\n{json}"
    );
    assert!(
        !json.contains("\"kind\":\"problem\""),
        "nothing failed to read here, and the stream must not say one did: {json}"
    );
}

#[test]
fn a_game_that_is_not_downloaded_yet_names_the_missing_directory_and_costs_no_exit_code() {
    // A manifest for a game mid-download points at a directory that is not
    // there. `steam::games` keeps the entry on purpose; the ranking row has to
    // say what it found rather than printing a blank or failing the run.
    let home = TempDir::new("steam-ghost");
    let root = fake_steam(&home);
    std::fs::write(
        root.join("steamapps/appmanifest_9999.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"9999\"\n\t\"name\"\t\t\"Ghost\"\n\
         \t\"installdir\"\t\t\"NotDownloadedYet\"\n}\n",
    )
    .expect("manifest");

    let out = dxray_with_home(home.path(), ["steam"]);
    let text = stdout_of(&out);

    assert!(text.contains("directory could not be read"), "got:\n{text}");
    assert_eq!(
        out.status.code(),
        Some(0),
        "a game that is still downloading is not a broken scan"
    );
}

#[test]
fn a_game_whose_directory_was_only_partly_searched_fails_the_scan_and_says_so_in_the_trailer() {
    // The rule the exit code is drawn from: if the trailer tells a human the
    // answer is incomplete, the status has to tell a script the same thing.
    // Before this, a `steam` run could print that a walk truncated and still
    // exit 0, so the two readers of one run came away with different stories.
    //
    // The install carries evidence of being a game — an executable named after
    // the title, importing a renderer. The test below it is the same fixture
    // with that evidence taken away, and it exits 1 too.
    let home = TempDir::new("steam-truncated");
    let root = fake_steam(&home);
    let install = root.join("steamapps/common/dota 2 beta");
    let mut buried = install.clone();
    // From the constant, so the fixture resizes with the bound rather than
    // quietly coming back into reach when it is raised.
    for _ in 0..=dxray_core::install::MAX_DEPTH {
        buried.push("down");
    }
    std::fs::create_dir_all(&buried).expect("tree");
    std::fs::write(
        buried.join("dota2.exe"),
        common::Image::x64().importing(&["vulkan-1.dll"]).build(),
    )
    .expect("fixture");
    std::fs::write(
        install.join("dota2.exe"),
        common::Image::x64().importing(&["d3d11.dll"]).build(),
    )
    .expect("fixture");

    let out = dxray_with_home(home.path(), ["steam"]);
    let text = stdout_of(&out);

    assert!(
        text.contains("could not be searched in full"),
        "the trailer admits the gap, got:\n{text}"
    );
    assert!(
        text.contains("may not be the right one"),
        "and says what the gap costs, got:\n{text}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "and the status agrees with the trailer, got:\n{text}"
    );
    assert!(
        stderr_of(&out).contains("missing from this list entirely"),
        "what moved the exit code is echoed where a piped run keeps it, got:\n{}",
        stderr_of(&out)
    );
}

#[test]
fn a_truncated_install_whose_only_reachable_binary_is_a_launcher_still_fails_the_scan() {
    // The exact shape a listing must never call clean, and the one an
    // evidence-gated status did call clean: a `launcher.exe` at the root with
    // nothing to say for itself, and the real shipping binary below the bound.
    // The listing then printed one game, no caveat and exit 0, while the row
    // beside it said nothing there carried evidence of being a game.
    //
    // That is the same wrong answer `dxray-core`'s depth bound exists to avoid
    // — a truncated walk that reports the launcher because it never reached the
    // real binary — moved one level up. The absence of evidence cannot excuse a
    // truncation, because the truncation is why the evidence is absent.
    let home = TempDir::new("steam-buried");
    let root = fake_steam(&home);
    let install = root.join("steamapps/common/dota 2 beta");
    let mut buried = install.clone();
    for _ in 0..=dxray_core::install::MAX_DEPTH {
        buried.push("down");
    }
    std::fs::create_dir_all(&buried).expect("tree");
    std::fs::write(
        buried.join("Game-Win64-Shipping.exe"),
        common::Image::x64().importing(&["d3d12.dll"]).build(),
    )
    .expect("fixture");
    // Reachable, and worth nothing: no renderer, no shipping suffix, no
    // resemblance to the title.
    std::fs::write(
        install.join("launcher.exe"),
        common::Image::x64().importing(&["KERNEL32.dll"]).build(),
    )
    .expect("fixture");

    let out = dxray_with_home(home.path(), ["steam"]);
    let text = stdout_of(&out);

    assert!(
        text.contains("could not be searched in full"),
        "the trailer has to admit the gap even here, got:\n{text}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "and a script has to hear it: this is the run most likely to be wrong, \
         got:\n{text}"
    );
    assert!(
        stderr_of(&out).contains("missing from this list entirely"),
        "with the reason kept where a piped run can still read it, got:\n{}",
        stderr_of(&out)
    );
}

#[test]
fn a_game_that_merely_imports_no_renderer_leaves_the_scan_clean() {
    // The other side of the same line, and the reason the distinction is worth
    // drawing at all. Everything in this directory was read; it simply has
    // nothing in its import tables, which is a finding rather than a gap. A
    // caveat that failed every scan would be a caveat nobody reads.
    let home = TempDir::new("steam-thin");
    let root = fake_steam(&home);
    let install = root.join("steamapps/common/dota 2 beta");
    std::fs::write(
        install.join("dota2.exe"),
        common::Image::x64().importing(&["KERNEL32.dll"]).build(),
    )
    .expect("fixture");

    let out = dxray_with_home(home.path(), ["steam"]);
    let text = stdout_of(&out);

    assert!(
        text.contains("nothing here imports a graphics API"),
        "the caveat is still printed, got:\n{text}"
    );
    assert!(
        !text.contains("could not be searched in full"),
        "but the trailer claims no gap, got:\n{text}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "and neither does the status, got:\n{text}"
    );
}

#[test]
fn an_install_that_argues_nothing_is_listed_under_the_ones_that_do() {
    // On the library this was measured against, four rows in nine are Proton
    // builds and Steam runtimes, and Steam hands them over sorted by
    // application id — so the tooling lands in the middle of the games. Nothing
    // is hidden: both appear, the trailer counts both, and the row under the
    // demoted one says why it is down there.
    let home = TempDir::new("steam-order");
    let root = fake_steam(&home);
    let steamapps = root.join("steamapps");

    // Appid 100, so the launcher declares it before Dota. Its one executable
    // imports nothing a game would: no renderer, and a name that resembles
    // neither the title nor anything else.
    std::fs::create_dir_all(steamapps.join("common/Proton - Experimental")).expect("tree");
    std::fs::write(
        steamapps.join("appmanifest_100.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"100\"\n\t\"name\"\t\t\"Proton Experimental\"\n\
         \t\"installdir\"\t\t\"Proton - Experimental\"\n}\n",
    )
    .expect("manifest");
    std::fs::write(
        steamapps.join("common/Proton - Experimental/filelock.exe"),
        common::Image::x64().importing(&["KERNEL32.dll"]).build(),
    )
    .expect("fixture");
    std::fs::write(
        steamapps.join("common/dota 2 beta/dota2.exe"),
        common::Image::x64().importing(&["d3d11.dll"]).build(),
    )
    .expect("fixture");

    let out = dxray_with_home(home.path(), ["steam"]);
    let text = stdout_of(&out);

    let game_at = text.find("dota 2 beta").expect("the game is listed");
    let runtime_at = text
        .find("Proton - Experimental")
        .expect("the runtime is listed too; nothing is filtered");
    assert!(
        game_at < runtime_at,
        "the evidence goes first, got:\n{text}"
    );
    assert!(
        text.contains("nothing here carries evidence of being a game"),
        "and the demoted row says what a marker would have said, got:\n{text}"
    );
    assert!(
        text.contains("2 games in 1 library across 1 install"),
        "the count includes both groups, got:\n{text}"
    );
    assert_eq!(out.status.code(), Some(0), "got:\n{text}");
}
