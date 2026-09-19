//! The `config_info` contents below are the shape Proton's own source writes:
//! a version, then the directories of the build that ran the prefix, then a
//! handful of booleans. What matters to this module is only the directory
//! lines, so the fixtures keep those and enough of the rest to prove the others
//! are ignored rather than tripped over.

use super::{Error, compatdata, for_game, from_prefix, resolve, script_in};
use crate::testutil::TempDir;
use std::path::Path;

/// A Steam-and-Proton-shaped tree. Everything these tests believe about the
/// real layout is written here once, so a reader can check the assumption in
/// one place instead of in ten.
struct Install(TempDir);

impl Install {
    fn new(tag: &str) -> Self {
        Self(TempDir::new(tag))
    }

    fn library(&self) -> &Path {
        self.0.path()
    }

    /// Unpacks a Proton build, `dist` for the pre-9.0 spelling.
    fn proton(&self, name: &str, dist: &str) -> String {
        let root = format!("steamapps/common/{name}");
        self.0
            .write(&format!("{root}/{SCRIPT}"), "#!/usr/bin/env python3\n");
        self.0.dir(&format!("{root}/{dist}/lib"));
        format!(
            "{}/{name}",
            self.0.path().join("steamapps/common").display()
        )
    }

    /// Writes the prefix Proton would leave behind for `appid`.
    fn prefix(&self, appid: u32, config_info: &str) {
        self.0.write(
            &format!("steamapps/compatdata/{appid}/config_info"),
            config_info,
        );
    }
}

const SCRIPT: &str = "proton";

/// A `config_info` of the shape Proton writes, naming `root` as the build.
fn config_info(root: &str, dist: &str) -> String {
    format!(
        "4.11\n{root}/{dist}/share/fonts/\n{root}/{dist}/lib/\n{root}/{dist}/lib64/\n\
         /home/u/.steam/steam\n2024-01-01 00:00:00\n{root}/{dist}/share/default_pfx/\n\
         False\nTrue\nTrue\n",
    )
}

#[test]
fn the_build_a_prefix_last_ran_under_is_read_out_of_its_config_info() {
    // The whole point of the module: a game's appid is not enough to answer the
    // NVAPI question, because the answer depends on which Proton ran it, and
    // this file is where that is written down.
    let install = Install::new("proton-files");
    let root = install.proton("Proton 9.0", "files");
    install.prefix(570, &config_info(&root, "files"));

    let script = for_game(install.library(), 570).expect("the build is named and present");

    assert_eq!(script, Path::new(&root).join(SCRIPT));
}

#[test]
fn the_older_dist_spelling_resolves_just_as_the_current_files_one_does() {
    // Proton 8.0 and everything before it unpacked into `dist/`; 9.0 renamed it
    // to `files/`. A resolver that knows only the new name identifies no prefix
    // last run under 8.0 — and 8.0 is the release whose policy runs the *other*
    // way round, so those are the prefixes where failing to identify the build
    // costs the most.
    let install = Install::new("proton-dist");
    let root = install.proton("Proton 8.0", "dist");
    install.prefix(570, &config_info(&root, "dist"));

    let script = for_game(install.library(), 570).expect("the old spelling still resolves");

    assert_eq!(script, Path::new(&root).join(SCRIPT));
}

#[test]
fn a_game_that_has_never_run_under_proton_is_not_reported_as_a_failure() {
    // Most of a Steam library is in this state at any moment. Counting it as
    // something that went wrong would report a healthy machine as a broken
    // scan, so it gets its own variant and its own predicate.
    let install = Install::new("proton-none");
    install.proton("Proton 9.0", "files");

    let error = for_game(install.library(), 570).expect_err("no prefix");

    assert!(error.is_absent(), "got {error}");
    assert!(
        error.to_string().contains("has not been run under Proton"),
        "and the message says so rather than blaming the file, got {error}"
    );
}

#[test]
fn a_build_that_has_been_uninstalled_says_so_instead_of_shrugging() {
    // A prefix outlives the Proton that made it: a build that was deleted or
    // renamed leaves a `config_info` pointing at nothing. "The build could not
    // be identified" sends a person looking for a parsing bug; naming the
    // directory that is gone tells them what actually happened.
    let install = Install::new("proton-gone");
    install.prefix(
        570,
        &config_info("/steam/steamapps/common/Proton 7.0", "dist"),
    );

    let error = for_game(install.library(), 570).expect_err("the build is gone");

    assert!(!error.is_absent(), "this one is a real failure");
    assert!(matches!(error, Error::ProtonGone { .. }), "got {error:?}");
    assert!(error.to_string().contains("Proton 7.0"), "got {error}");
}

#[test]
fn a_config_info_that_names_no_proton_at_all_is_told_apart_from_one_that_names_a_missing_build() {
    // Different faults with different fixes. One says this file is not what
    // this code believes it is; the other says the build it named has gone.
    let install = Install::new("proton-nameless");
    install.prefix(570, "4.11\n/home/u/.steam/steam\nTrue\nFalse\n");

    let error = for_game(install.library(), 570).expect_err("nothing named");

    assert!(
        matches!(error, Error::NoProtonNamed { .. }),
        "got {error:?}"
    );
    assert!(
        error.to_string().contains("cannot be identified"),
        "got {error}"
    );
}

#[test]
fn a_prefix_that_names_two_different_protons_is_refused_rather_than_resolved() {
    // A prefix belongs to one build. Two means the file is not what this code
    // believes it is, and picking the first would report a policy the game
    // never ran under — a confident answer drawn from the wrong script.
    let install = Install::new("proton-two");
    let nine = install.proton("Proton 9.0", "files");
    let ten = install.proton("Proton 10.0", "files");
    install.prefix(570, &format!("4.11\n{nine}/files/lib/\n{ten}/files/lib/\n"));

    let error = for_game(install.library(), 570).expect_err("two builds");

    assert!(matches!(error, Error::Ambiguous { .. }), "got {error:?}");
    assert!(
        error.to_string().contains("names more than one Proton"),
        "got {error}"
    );
}

#[test]
fn one_build_named_by_several_lines_is_still_one_build() {
    // A real `config_info` names the same root three or four times over — its
    // fonts, its libraries, its template prefix. Treating those as different
    // builds would refuse every healthy prefix on every machine.
    let install = Install::new("proton-repeat");
    let root = install.proton("Proton 9.0", "files");
    install.prefix(570, &config_info(&root, "files"));

    assert!(for_game(install.library(), 570).is_ok());
}

#[test]
fn a_line_that_could_be_cut_in_two_places_is_settled_by_looking_for_the_script() {
    // A person whose directory is called `files` produces two candidate roots
    // from one line. Which cut is the real boundary is not decided by picking
    // an occurrence; it is decided by which candidate has a `proton` in it,
    // because that is evidence and the other is a guess.
    let install = Install::new("proton-ambiguous-cut");
    let root = install.proton("files/Proton 9.0", "files");
    install.prefix(570, &config_info(&root, "files"));

    let script = for_game(install.library(), 570).expect("the cut with the script wins");

    assert_eq!(script, Path::new(&root).join(SCRIPT));
}

#[test]
fn a_directory_and_the_script_inside_it_are_both_accepted_as_a_proton() {
    // The directory is what Steam shows in a library listing; the script is
    // what a path copied out of a `config_info` points at. Both are things a
    // person has in front of them.
    let install = Install::new("proton-resolve");
    let root = install.proton("Proton 9.0", "files");
    let script = Path::new(&root).join(SCRIPT);

    assert_eq!(resolve(Path::new(&root)).expect("a root"), script);
    assert_eq!(resolve(&script).expect("a script"), script);
}

#[test]
fn a_directory_with_no_launcher_in_it_is_not_a_proton_install() {
    // Otherwise `--nvapi <some folder>` would go on to report that the folder
    // has no policy, which reads as a fact about Proton rather than about the
    // path that was typed.
    let dir = TempDir::new("proton-empty");

    assert!(script_in(dir.path()).is_none());
    let error = resolve(dir.path()).expect_err("not a Proton");
    assert!(
        error.to_string().contains("not a Proton install"),
        "got {error}"
    );
}

#[test]
fn the_prefix_path_is_built_where_steam_puts_it() {
    // Written down in one place and asserted, because every other test in this
    // file depends on it and none of them has been run against a real Steam.
    let built = compatdata(Path::new("/lib"), 570);

    assert_eq!(built, Path::new("/lib/steamapps/compatdata/570"));
}

#[test]
fn a_prefix_directory_with_no_config_info_in_it_reports_the_file_it_wanted() {
    // A half-built prefix, or one from a Proton that failed early. Still "no
    // build recorded here", but the path in the message is the file rather than
    // the directory, so a person can see the difference.
    let install = Install::new("proton-halfbuilt");
    install.0.dir("steamapps/compatdata/570/pfx");

    let error = from_prefix(&compatdata(install.library(), 570)).expect_err("no config_info");

    assert!(error.is_absent(), "still not a failure, got {error}");
    assert!(
        error.path().ends_with("config_info"),
        "got {}",
        error.path().display()
    );
}

#[test]
fn a_game_with_no_prefix_is_a_sentence_rather_than_an_error() {
    // The ordinary state of most of a real library: owned, installed, never
    // launched under Proton. A row that said "error" about it would put a
    // hundred red lines on a screen where nothing is wrong.
    let answer = super::Builds::default().answer(Path::new("/definitely/not/here"), 440);

    assert!(
        answer.verdict.starts_with("not determined"),
        "expected a sentence, got {:?}",
        answer.verdict
    );
    assert!(
        answer.available.is_none(),
        "nothing was read, so nothing may be claimed either way"
    );
    assert!(
        answer.script.is_none(),
        "naming a script that was never found would send a reader to a file that is not there"
    );
}

#[test]
fn one_cache_serves_both_the_listing_and_the_detail_pane() {
    // The two binaries kept a copy of this each, and the copies were free to
    // drift: one carried the condition's source lines and the other did not.
    // They now share the answer and differ only in what they choose to print,
    // which is a rendering decision rather than a data one.
    let answer = super::Builds::default().answer(Path::new("/definitely/not/here"), 440);

    assert!(
        answer.condition.is_empty(),
        "an answer that read nothing states no condition"
    );
}
