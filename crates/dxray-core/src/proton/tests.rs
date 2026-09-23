//! `config_info` fixtures in the shape Proton writes; only the directory
//! lines matter, the rest proves they are ignored.

use super::{Error, compatdata, for_game, from_prefix, resolve, script_in};
use crate::testutil::TempDir;
use std::path::Path;

/// A Steam-and-Proton-shaped tree, the layout written down once.
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
    // `config_info` names the build that ran the game.
    let install = Install::new("proton-files");
    let root = install.proton("Proton 9.0", "files");
    install.prefix(570, &config_info(&root, "files"));

    let script = for_game(install.library(), 570).expect("the build is named and present");

    assert_eq!(script, Path::new(&root).join(SCRIPT));
}

#[test]
fn the_older_dist_spelling_resolves_just_as_the_current_files_one_does() {
    // Proton 8.0 and older unpack into `dist/`; 9.0 renamed it `files/`.
    let install = Install::new("proton-dist");
    let root = install.proton("Proton 8.0", "dist");
    install.prefix(570, &config_info(&root, "dist"));

    let script = for_game(install.library(), 570).expect("the old spelling still resolves");

    assert_eq!(script, Path::new(&root).join(SCRIPT));
}

#[test]
fn a_game_that_has_never_run_under_proton_is_not_reported_as_a_failure() {
    // A prefix never launched is its own state, not an error.
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
    // A deleted build is named, not reported as a parse failure.
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
    // Two builds in one prefix is refused.
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
    // The same root repeated is one build.
    let install = Install::new("proton-repeat");
    let root = install.proton("Proton 9.0", "files");
    install.prefix(570, &config_info(&root, "files"));

    assert!(for_game(install.library(), 570).is_ok());
}

#[test]
fn a_line_that_could_be_cut_in_two_places_is_settled_by_looking_for_the_script() {
    // Of two candidate roots, the one holding `proton` wins.
    let install = Install::new("proton-ambiguous-cut");
    let root = install.proton("files/Proton 9.0", "files");
    install.prefix(570, &config_info(&root, "files"));

    let script = for_game(install.library(), 570).expect("the cut with the script wins");

    assert_eq!(script, Path::new(&root).join(SCRIPT));
}

#[test]
fn a_directory_and_the_script_inside_it_are_both_accepted_as_a_proton() {
    // A build directory or its script both resolve.
    let install = Install::new("proton-resolve");
    let root = install.proton("Proton 9.0", "files");
    let script = Path::new(&root).join(SCRIPT);

    assert_eq!(resolve(Path::new(&root)).expect("a root"), script);
    assert_eq!(resolve(&script).expect("a script"), script);
}

#[test]
fn a_directory_with_no_launcher_in_it_is_not_a_proton_install() {
    // A folder with no `proton` is refused.
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
    // A half-built prefix names the file that is missing.
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
    // A game never launched under Proton is not an error.
    let answer = super::Builds::default().answer(None, Path::new("/definitely/not/here"), 440);

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
    // One answer for both binaries; only rendering differs.
    let answer = super::Builds::default().answer(None, Path::new("/definitely/not/here"), 440);

    assert!(
        answer.condition.is_empty(),
        "an answer that read nothing states no condition"
    );
}

/// A Steam root that is also the game's library, a 10.0-shaped build, the
/// game's launch options, and the prefix its last launch left.
struct Launched(TempDir);

const GOTG: u32 = 1_088_850;

impl Launched {
    fn new(tag: &str, options: &str, recorded: &str) -> Self {
        let dir = TempDir::new(tag);
        let build = "steamapps/common/Proton 10.0";
        dir.write(&format!("{build}/proton"), crate::testutil::PROTON_10);
        dir.dir(&format!("{build}/files/lib"));
        let root = dir.path().join(build).display().to_string();
        let config_info = [
            "10.0-200".to_owned(),
            format!("{root}/files/share/fonts/"),
            format!("{root}/files/lib/"),
            "/home/u/.steam/steam".to_owned(),
            String::new(),
            String::new(),
            String::new(),
            format!("{root}/files/share/default_pfx/"),
            String::new(),
            "False".to_owned(),
            "True".to_owned(),
            "d3dcompiler_*.dll".to_owned(),
            recorded.to_owned(),
            "False".to_owned(),
        ]
        .join("\n");
        dir.write(
            &format!("steamapps/compatdata/{GOTG}/config_info"),
            &config_info,
        );
        dir.write(
            "userdata/1/config/localconfig.vdf",
            &format!(
                "\"UserLocalConfigStore\" {{ \"Software\" {{ \"Valve\" {{ \"Steam\" {{ \"apps\" {{ \
                 \"{GOTG}\" {{ \"LaunchOptions\" \"{options}\" }} }} }} }} }} }}"
            ),
        );
        Self(dir)
    }

    fn settings(&self, text: &str) -> &Self {
        self.0
            .write("steamapps/common/Proton 10.0/user_settings.py", text);
        self
    }

    fn answer(&self) -> super::Answer {
        let root = self.0.path();
        super::Builds::default().answer(Some(root), root, GOTG)
    }
}

#[test]
fn a_launch_option_that_forces_nvapi_is_the_answer_and_the_last_launch_confirms_it() {
    let answer = Launched::new("forced", "PROTON_FORCE_NVAPI=1 %command%", "True").answer();

    assert_eq!(answer.available, Some(true), "{}", answer.verdict);
    for part in [
        "PROTON_FORCE_NVAPI=1 in launch options adds forcenvapi",
        "the script alone says: NVAPI is withheld",
        "the last launch recorded use_nvapi=True",
    ] {
        assert!(
            answer.verdict.contains(part),
            "{part:?} in {}",
            answer.verdict
        );
    }
    assert!(!answer.verdict.contains("disagrees"), "{}", answer.verdict);
}

#[test]
fn with_no_launch_option_the_default_stands_and_the_record_agrees() {
    let answer = Launched::new("default", "", "False").answer();

    assert_eq!(answer.available, Some(false), "{}", answer.verdict);
    assert!(
        answer.verdict.starts_with("NVAPI is withheld")
            && answer.verdict.contains("recorded use_nvapi=False"),
        "{}",
        answer.verdict
    );
}

#[test]
fn a_record_that_contradicts_the_prediction_leaves_the_answer_unsettled() {
    // The case this module used to get wrong: a launch the options no longer
    // describe, or a variable set somewhere this reader does not look.
    let answer = Launched::new("contradicted", "", "True").answer();

    assert_eq!(answer.available, None, "{}", answer.verdict);
    assert!(answer.verdict.contains("disagrees"), "{}", answer.verdict);
}

#[test]
fn user_settings_beside_the_build_are_read_as_a_source() {
    let launched = Launched::new("user-settings", "", "True");
    launched.settings("user_settings = {\n    \"PROTON_FORCE_NVAPI\": \"1\",\n}\n");

    let answer = launched.answer();

    assert_eq!(answer.available, Some(true), "{}", answer.verdict);
    assert!(
        answer
            .verdict
            .contains("in user_settings.py adds forcenvapi"),
        "{}",
        answer.verdict
    );
}

#[test]
fn without_a_steam_root_there_are_no_launch_options_to_apply() {
    let launched = Launched::new("no-root", "PROTON_FORCE_NVAPI=1 %command%", "False");
    let library = launched.0.path();

    let answer = super::Builds::default().answer(None, library, GOTG);

    assert_eq!(answer.available, Some(false), "{}", answer.verdict);
}
