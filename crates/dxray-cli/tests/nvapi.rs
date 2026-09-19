//! End to end tests for `--nvapi`, and for the NVAPI rows `--steam` grew: the
//! real binary, a fake Proton install, real exit codes.
//!
//! The scripts below are cut down from real Proton releases and keep the shapes
//! that matter — the two levels of nesting the policy lives at, the comments
//! Valve writes beside every application id, and the `/proc/modules` block
//! Proton 10.0 introduced. There is no Proton on the machine these were written
//! on, so what they prove is that the reader handles those shapes, not that a
//! real install is laid out the way the resolver believes.

mod common;

use common::{TempDir, dxray, dxray_with_home, stdout_of};
use std::path::{Path, PathBuf};

/// Proton 9.0 and later: a game in the list is one that loses NVAPI.
const DENY: &str = r#"import os

def default_compat_config():
    ret = set()
    if "SteamAppId" in os.environ:
        appid = os.environ["SteamAppId"]
        if appid in [
                "1088850", #Marvel's Guardians of the Galaxy
                ]:
            ret.add("disablenvapi")

        if appid in [
                "108710", #Alan Wake
                ]:
            try:
                with open('/proc/modules') as f:
                    drivers = set([line.partition(' ')[0] for line in f.read().splitlines()])
                    if not drivers.intersection({'nvidia', 'nouveau', 'nova'}):
                        ret.add("disablenvapi")
            except OSError:
                ret.add("disablenvapi")
    return ret
"#;

/// Proton 8.0: a game in the list is one of the few that *gets* NVAPI.
const ALLOW: &str = r#"import os

def default_compat_config():
    ret = set()
    if appid in [
            "1182900", #A Plague Tale: Requiem
            ]:
        ret.add("enablenvapi")
    return ret
"#;

/// Proton 7.0: the function is there and nothing in it touches NVAPI. A true
/// negative about the build, not a failure to read it.
const NO_POLICY: &str = r#"def default_compat_config():
    ret = set()
    if appid in ["257420"]:
        ret.add("hidevggpu")
    return ret
"#;

/// The same absence for a different reason: the flag is set by a shape this
/// reader does not model, so the policy may be entirely in the part it could
/// not read.
const UNREADABLE: &str = r#"def default_compat_config():
    ret = set()
    if appid in ["1088850"]:
        ret.update({"disablenvapi"})
    return ret
"#;

/// A flat block with one extra test wrapped round it, and nothing else changed.
const WRAPPED: &str = r#"import os

def default_compat_config():
    ret = set()
    if "SteamAppId" in os.environ:
        appid = os.environ["SteamAppId"]
        if not os.environ.get("PROTON_DISABLE_NVAPI_QUIRKS"):
            if appid in [
                    "1088850", #Marvel's Guardians of the Galaxy
                    ]:
                ret.add("disablenvapi")
    return ret
"#;

/// NVAPI lists, and none of them says which way round the policy runs.
const FORCE_ONLY: &str = r#"def default_compat_config():
    ret = set()
    if appid in ["2395210", "1577120"]:
        ret.add("forcenvapi")
    return ret
"#;

/// Unpacks a Proton build under `at` and returns its root.
fn proton(at: &TempDir, name: &str, script: &str) -> PathBuf {
    let root = at.path().join("steamapps/common").join(name);
    std::fs::create_dir_all(root.join("files/lib")).expect("dist dir");
    std::fs::write(root.join("proton"), script).expect("script");
    root
}

/// Reads stdout with its wrapping taken out, so an assertion is about what the
/// output says rather than about where the terminal width folded it.
fn unwrapped(output: &std::process::Output) -> String {
    stdout_of(output)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn a_policy_is_reported_with_the_evidence_behind_it_and_not_just_a_list() {
    // The evidence comes first and the answers after it, in that order. "This
    // game is not in the list" is worth one thing when fifteen lists were read
    // and nothing at all when the function was never found, and a listing that
    // printed the answer alone would be hiding the only line that says which.
    let dir = TempDir::new("nvapi-deny");
    let root = proton(&dir, "Proton 9.0", DENY);

    let out = dxray([Path::new("--nvapi"), root.as_path()]);
    let text = unwrapped(&out);

    assert!(out.status.success(), "a readable policy exits 0: {text}");
    assert!(
        text.contains("default_compat_config at line 3"),
        "the function and where it is, got: {text}"
    );
    assert!(
        text.contains("2 appid lists read"),
        "how much of the shape was understood, got: {text}"
    );
    assert!(
        text.contains("opt-out: every game gets NVAPI except"),
        "and which way round it runs, in words, got: {text}"
    );
}

#[test]
fn a_listed_game_and_an_unlisted_one_get_opposite_answers_from_one_build() {
    let dir = TempDir::new("nvapi-two");
    let root = proton(&dir, "Proton 9.0", DENY);

    let out = dxray([
        "--nvapi",
        &root.display().to_string(),
        "--appid",
        "1088850",
        "--appid",
        "570",
    ]);
    let text = unwrapped(&out);

    assert!(out.status.success(), "both were answered: {text}");
    assert!(text.contains("1088850 NVAPI is withheld"), "got: {text}");
    assert!(text.contains("570 NVAPI is left alone"), "got: {text}");
}

#[test]
fn the_same_silence_means_the_opposite_thing_on_a_build_from_the_inverted_era() {
    // Proton 6.3 to 8.0 list the games that *get* NVAPI. A reader that knew
    // only `disablenvapi`, found none, and answered "not in the list, so NVAPI
    // is on" would get every game on those releases backwards. This is the
    // assertion that the two eras cannot collapse into each other.
    let dir = TempDir::new("nvapi-allow");
    let root = proton(&dir, "Proton 8.0", ALLOW);

    let out = dxray([
        "--nvapi",
        &root.display().to_string(),
        "--appid",
        "570",
        "--appid",
        "1182900",
    ]);
    let text = unwrapped(&out);

    assert!(
        text.contains("opt-in: no game gets NVAPI except"),
        "got: {text}"
    );
    assert!(
        text.contains("570 NVAPI is withheld"),
        "the unlisted game loses it here, got: {text}"
    );
    assert!(
        text.contains("1182900 NVAPI is offered"),
        "and the listed one keeps it, got: {text}"
    );
}

#[test]
fn a_conditional_block_prints_the_condition_rather_than_resolving_it() {
    // Proton 10.0 onwards disables NVAPI for these games only when no NVIDIA
    // driver is loaded — which on the machines this tool is for means they keep
    // it. Resolving that here would make the answer depend on where dxray runs
    // instead of on what Proton will do, so the block is quoted back and the
    // reader applies it.
    let dir = TempDir::new("nvapi-conditional");
    let root = proton(&dir, "Proton 10.0", DENY);

    let out = dxray(["--nvapi", &root.display().to_string(), "--appid", "108710"]);
    let text = stdout_of(&out);

    assert!(
        out.status.success(),
        "a condition is an answer, not a failure: {text}"
    );
    assert!(
        unwrapped(&out).contains("the condition is reported, not resolved"),
        "got:\n{text}"
    );
    assert!(
        text.contains("| except OSError:"),
        "the block is quoted verbatim, got:\n{text}"
    );
    assert!(
        unwrapped(&out).contains(r#"turns on "/proc/modules", "nvidia", "nouveau", "nova""#),
        "and what it turns on is named, got:\n{text}"
    );
}

#[test]
fn a_test_wrapped_around_a_flat_block_is_printed_rather_than_waved_through() {
    // The block here is the plain, outright shape with one extra test round it.
    // Reading it as unconditional prints a confident "NVAPI is withheld" for a
    // game whose fate is gated on something that was never looked at, with
    // nothing in the output to say the test exists.
    let dir = TempDir::new("nvapi-wrapped");
    let root = proton(&dir, "Proton 11.0", WRAPPED);

    let out = dxray(["--nvapi", &root.display().to_string(), "--appid", "1088850"]);
    let text = stdout_of(&out);
    let flat = unwrapped(&out);

    assert!(
        !flat.contains("NVAPI is withheld"),
        "the answer must not be flat, got: {flat}"
    );
    assert!(flat.contains("conditional:"), "got: {flat}");
    assert!(
        flat.contains(r#"a test at line 7 that turns on "PROTON_DISABLE_NVAPI_QUIRKS""#),
        "and it names what the test reads, got: {flat}"
    );
    assert!(
        text.contains("| if not os.environ.get"),
        "quoted verbatim, got:\n{text}"
    );
    assert!(
        flat.contains("does not claim to understand"),
        "and says plainly that it did not understand it, got: {flat}"
    );
}

#[test]
fn a_build_read_in_full_with_no_nvapi_policy_is_a_finding_and_exits_zero() {
    // Proton 7.0 really is like this. The function is there, real appid lists
    // parse out of it, and none of them touches NVAPI. That is an answer about
    // the build — being this game changes nothing — and a caller that exited 1
    // for it would be calling a complete reading a failed one.
    let dir = TempDir::new("nvapi-truenegative");
    let root = proton(&dir, "Proton 7.0", NO_POLICY);

    let out = dxray(["--nvapi", &root.display().to_string(), "--appid", "570"]);
    let text = unwrapped(&out);

    assert!(out.status.success(), "got a failure for: {text}");
    assert!(text.contains("no NVAPI site among them"), "got: {text}");
    assert!(text.contains("not game-specific"), "got: {text}");
}

#[test]
fn a_build_whose_policy_could_not_be_read_is_not_that_same_answer() {
    // The pair that matters. Both scripts yield no NVAPI site. One was read in
    // full; in the other the policy may be entirely inside the part that was
    // refused. They must differ in the sentence *and* in the exit code, or a
    // release whose policy moved reads as a release that has none.
    let dir = TempDir::new("nvapi-hidden");
    let root = proton(&dir, "Proton 11.0", UNREADABLE);

    let out = dxray(["--nvapi", &root.display().to_string(), "--appid", "1088850"]);
    let text = unwrapped(&out);

    assert!(!out.status.success(), "got 0 for: {text}");
    assert!(
        !text.contains("not game-specific"),
        "this is not a true negative, got: {text}"
    );
    assert!(
        text.contains("cannot be told from it"),
        "and it says the policy may be in the part it could not read, got: {text}"
    );
}

#[test]
fn a_build_with_nvapi_lists_and_no_direction_does_not_deny_the_lists_it_just_printed() {
    // A `forcenvapi` list and nothing else. Printing the list and then claiming
    // no list touches NVAPI is a report contradicting itself three lines apart,
    // which costs the whole output its credibility.
    let dir = TempDir::new("nvapi-forceonly");
    let root = proton(&dir, "Proton 9.0", FORCE_ONLY);

    let out = dxray(["--nvapi", &root.display().to_string(), "--appid", "1088850"]);
    let text = unwrapped(&out);

    assert!(!out.status.success(), "the direction was not settled");
    assert!(text.contains("2 appids under forcenvapi"), "got: {text}");
    assert!(
        !text.contains("none of them touches NVAPI"),
        "it must not deny what it printed, got: {text}"
    );
    assert!(
        text.contains("which way round its policy runs"),
        "got: {text}"
    );
}

#[test]
fn a_file_that_defeats_the_reader_is_named_once_and_not_twice() {
    // Cosmetic, and it looks like a bug in a tool whose whole claim is care
    // about what it prints.
    let dir = TempDir::new("nvapi-banner");
    let script = dir.path().join("proton");
    std::fs::write(&script, "ret = set(\n").expect("truncated script");

    let at = script.display().to_string();
    // Both forms, because the banner and the failure used to be written by two
    // different places and only one of them knew about the other.
    for argv in [
        vec!["--nvapi", &at],
        vec!["--nvapi", &at, "--appid", "1088850"],
    ] {
        let out = dxray(&argv);
        let text = stdout_of(&out);
        let banners = text.lines().filter(|line| line.trim() == at).count();

        assert_eq!(banners, 1, "for {argv:?}, got:\n{text}");
        assert!(text.contains("unreadable"), "for {argv:?}, got:\n{text}");
    }
}

#[test]
fn a_directory_that_is_not_a_proton_says_so_rather_than_reporting_an_empty_policy() {
    // Otherwise `--nvapi ~/Downloads` reports that the folder withholds NVAPI
    // from no games, which reads as a fact about Proton rather than about the
    // path that was typed.
    let dir = TempDir::new("nvapi-notproton");

    let out = dxray([Path::new("--nvapi"), dir.path()]);
    let text = unwrapped(&out);

    assert!(!out.status.success());
    assert!(text.contains("not a Proton install"), "got: {text}");
}

#[test]
fn nvapi_does_not_share_the_frozen_json_shape() {
    // The JSONL contract is positional and about PE files. A second, unrelated
    // shape behind the same flag would be the worse kind of breakage, so the
    // combination is refused at the command line rather than invented.
    let dir = TempDir::new("nvapi-json");
    let root = proton(&dir, "Proton 9.0", DENY);

    let out = dxray(["--nvapi", &root.display().to_string(), "--json"]);

    assert_eq!(out.status.code(), Some(2), "a usage error, not a run");
}

#[test]
fn asking_about_an_appid_without_saying_which_build_is_a_usage_error() {
    // The answer depends entirely on which Proton ran the game — the policy
    // changed direction twice across releases — so an appid with no build is
    // not a question that has an answer.
    let out = dxray(["--appid", "570"]);

    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn the_steam_listing_names_the_build_a_game_ran_under_and_what_it_does_to_nvapi() {
    // The whole chain in one go: a library, a manifest, the compatibility
    // prefix Proton left behind, the `config_info` inside it, the build that
    // file names, and the policy in that build's launcher script.
    let home = TempDir::new("nvapi-steam-home");
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
        steamapps.join("appmanifest_1088850.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"1088850\"\n\t\"name\"\t\t\"Guardians\"\n\
         \t\"installdir\"\t\t\"dota 2 beta\"\n}\n",
    )
    .expect("manifest");

    let build = root.join("steamapps/common/Proton 9.0");
    std::fs::create_dir_all(build.join("files/lib")).expect("dist");
    std::fs::write(build.join("proton"), DENY).expect("script");
    std::fs::create_dir_all(steamapps.join("compatdata/1088850")).expect("prefix");
    std::fs::write(
        steamapps.join("compatdata/1088850/config_info"),
        format!("4.11\n{}/files/lib/\nTrue\n", build.display()),
    )
    .expect("config_info");

    let out = dxray_with_home(home.path(), ["--steam"]);
    let text = unwrapped(&out);

    assert!(
        text.contains("proton ") && text.contains("Proton 9.0/proton"),
        "the build is named beside the verdict, got: {text}"
    );
    assert!(
        text.contains("nvapi NVAPI is withheld"),
        "and the verdict is drawn from that build's script, got: {text}"
    );
}

#[test]
fn a_game_that_has_never_run_under_proton_gets_a_row_and_costs_no_exit_code() {
    // Most of a real library is in this state. A game with nothing said about
    // its NVAPI reads as a game with nothing wrong with it, so the row is
    // always printed — and counting it as a scan that came up short would
    // report every healthy machine as a failure.
    let home = TempDir::new("nvapi-steam-bare");
    let root = home.path().join(".steam/steam");
    let steamapps = root.join("steamapps");
    std::fs::create_dir_all(steamapps.join("common/dota 2 beta")).expect("tree");
    std::fs::write(
        steamapps.join("appmanifest_570.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"570\"\n\t\"name\"\t\t\"Dota 2\"\n\
         \t\"installdir\"\t\t\"dota 2 beta\"\n}\n",
    )
    .expect("manifest");

    let out = dxray_with_home(home.path(), ["--steam"]);
    let text = unwrapped(&out);

    assert!(
        out.status.success(),
        "no answer is not a failed scan: {text}"
    );
    assert!(
        text.contains("nvapi not determined:"),
        "but it still says so, got: {text}"
    );
    assert!(
        text.contains("has not been run under Proton"),
        "and why, got: {text}"
    );
}
