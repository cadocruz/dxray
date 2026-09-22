//! Fixtures are cut from Proton's upstream `proton` script: 10.0,
//! Experimental and 8.0. Only the lines this module reads are kept.

use super::{
    Applied, Environment, Formula, Resolution, SetBy, UserSettings, resolve, user_settings,
};
use crate::nvapi::{Flag, Reading, scan};
use crate::testutil::PROTON_10;

const GOTG: &str = "1088850";
const THPS: &str = "2395210";

/// Proton 8.0: an allow-list, and a tuple one entry longer.
const PROTON_8: &str = r#"import os

class CompatData:
    def setup_prefix(self):
            use_nvapi = 'enablenvapi' in g_session.compat_config
            prefix_info = '\n'.join((
                CURRENT_PREFIX_VERSION,
                g_proton.fonts_dir,
                g_proton.lib_dir,
                g_proton.lib64_dir,
                steamdir,
                getmtimestr(steamdir, 'legacycompat', 'steamclient.dll'),
                getmtimestr(steamdir, 'legacycompat', 'steamclient64.dll'),
                getmtimestr(steamdir, 'legacycompat', 'Steam.dll'),
                g_proton.default_pfx_dir,
                getmtimestr(g_proton.default_pfx_dir, 'system.reg'),
                str(use_wined3d),
                str(use_dxvk_dxgi),
                builtin_dll_copy,
                str(use_nvapi),
            ))

def default_compat_config():
    ret = set()
    if "SteamAppId" in os.environ:
        appid = os.environ["SteamAppId"]
        if appid in [
                "570", #Dota 2
                ]:
            ret.add("enablenvapi")
    return ret

class Session:
    def init_session(self):
        self.check_environment("PROTON_ENABLE_NVAPI", "enablenvapi")
"#;

/// Experimental's expression, which adds a host condition to 10.0's.
const EXPERIMENTAL_EXPRESSION: &str = r#"use_nvapi = (('disablenvapi' not in g_session.compat_config or 'forcenvapi' in g_session.compat_config) and
                        g_proton.host_pe_arch != "aarch64-windows")"#;

fn read(script: &str) -> Reading {
    scan(script).expect("the fixture is a script this reader handles")
}

fn line_of(script: &str, needle: &str) -> usize {
    script
        .lines()
        .position(|line| line.contains(needle))
        .expect("the fixture contains the needle")
        + 1
}

fn launch(values: &[(&str, &str)]) -> Environment {
    Environment::new(
        Ok(values
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()),
        &UserSettings::Absent,
    )
}

fn applied(variable: &str, value: &str, set_by: SetBy, flag: Flag, added: bool) -> Applied {
    Applied {
        variable: variable.to_owned(),
        value: value.to_owned(),
        set_by,
        flag,
        added,
    }
}

#[test]
fn only_the_nvapi_switches_are_read_and_in_file_order() {
    let reading = read(PROTON_10);

    assert_eq!(reading.switches_unread, 0);
    let got: Vec<(&str, Flag, usize)> = reading
        .switches
        .iter()
        .map(|s| (s.variable.as_str(), s.flag, s.line))
        .collect();
    assert_eq!(
        got,
        [
            (
                "PROTON_DISABLE_NVAPI",
                Flag::Disable,
                line_of(PROTON_10, "PROTON_DISABLE_NVAPI")
            ),
            (
                "PROTON_FORCE_NVAPI",
                Flag::Force,
                line_of(PROTON_10, "PROTON_FORCE_NVAPI")
            ),
        ],
        "wined3d is not an NVAPI flag"
    );
}

#[test]
fn the_recorded_line_is_read_off_the_tuple_rather_than_assumed() {
    // 8.0 carries lib64_dir, which moves use_nvapi one line down.
    assert_eq!(read(PROTON_10).recorded_line, Some(13));
    assert_eq!(read(PROTON_8).recorded_line, Some(14));
}

#[test]
fn every_known_use_nvapi_expression_is_recognized() {
    let experimental = PROTON_10.replace(
        "use_nvapi = 'disablenvapi' not in g_session.compat_config or 'forcenvapi' in g_session.compat_config",
        EXPERIMENTAL_EXPRESSION,
    );

    assert_eq!(
        read(PROTON_10).formula,
        Some(Formula::DisableUnlessForced { arm_clause: false })
    );
    assert_eq!(
        read(&experimental).formula,
        Some(Formula::DisableUnlessForced { arm_clause: true })
    );
    assert_eq!(read(PROTON_8).formula, Some(Formula::Enable));
}

#[test]
fn an_unknown_expression_is_not_guessed_at() {
    let script = PROTON_10.replace(
        "use_nvapi = 'disablenvapi' not in g_session.compat_config or 'forcenvapi' in g_session.compat_config",
        "use_nvapi = nvapi_wanted(g_session)",
    );
    let reading = read(&script);

    assert_eq!(reading.formula, None);
    assert!(matches!(
        resolve(&reading, GOTG, &launch(&[("PROTON_FORCE_NVAPI", "1")])),
        Resolution::Undetermined(_)
    ));
}

#[test]
fn with_no_switch_set_the_script_default_stands() {
    let reading = read(PROTON_10);

    assert_eq!(resolve(&reading, GOTG, &launch(&[])), Resolution::Default);
    assert_eq!(
        resolve(&reading, GOTG, &launch(&[("PROTON_LOG", "1")])),
        Resolution::Default,
        "a variable no switch reads changes nothing"
    );
}

#[test]
fn forcing_nvapi_overrides_the_list_that_withholds_it() {
    // The case measured on a real machine: listed under disablenvapi, forced
    // back on from the launch options.
    let got = resolve(
        &read(PROTON_10),
        GOTG,
        &launch(&[("PROTON_FORCE_NVAPI", "1")]),
    );

    assert_eq!(
        got,
        Resolution::Computed {
            use_nvapi: true,
            applied: vec![applied(
                "PROTON_FORCE_NVAPI",
                "1",
                SetBy::LaunchOptions,
                Flag::Force,
                true
            )],
        }
    );
}

#[test]
fn a_zero_removes_the_flag_rather_than_only_not_adding_it() {
    let reading = read(PROTON_10);

    let undisabled = resolve(&reading, GOTG, &launch(&[("PROTON_DISABLE_NVAPI", "0")]));
    assert!(
        matches!(
            undisabled,
            Resolution::Computed {
                use_nvapi: true,
                ..
            }
        ),
        "removing disablenvapi hands NVAPI back: {undisabled:?}"
    );

    let unforced = resolve(&reading, THPS, &launch(&[("PROTON_FORCE_NVAPI", "0")]));
    assert!(
        matches!(
            unforced,
            Resolution::Computed {
                use_nvapi: true,
                ..
            }
        ),
        "removing forcenvapi from a game that is not disabled leaves NVAPI on: {unforced:?}"
    );

    let both = resolve(&reading, GOTG, &launch(&[("PROTON_FORCE_NVAPI", "")]));
    assert!(
        matches!(
            both,
            Resolution::Computed {
                use_nvapi: false,
                ..
            }
        ),
        "an empty value is a zero too: {both:?}"
    );
}

#[test]
fn a_launch_option_wins_over_user_settings_and_user_settings_fills_the_rest() {
    let reading = read(PROTON_10);
    let settings = UserSettings::Read(vec![("PROTON_FORCE_NVAPI".to_owned(), "1".to_owned())]);

    let filled = resolve(&reading, GOTG, &Environment::new(Ok(Vec::new()), &settings));
    assert_eq!(
        filled,
        Resolution::Computed {
            use_nvapi: true,
            applied: vec![applied(
                "PROTON_FORCE_NVAPI",
                "1",
                SetBy::UserSettings,
                Flag::Force,
                true
            )],
        }
    );

    let overridden = resolve(
        &reading,
        GOTG,
        &Environment::new(
            Ok(vec![("PROTON_FORCE_NVAPI".to_owned(), "0".to_owned())]),
            &settings,
        ),
    );
    assert!(
        matches!(
            overridden,
            Resolution::Computed {
                use_nvapi: false,
                ..
            }
        ),
        "the launch option is the one Proton sees: {overridden:?}"
    );
}

#[test]
fn launch_options_that_could_not_be_read_leave_every_switch_open() {
    // Even a game forced on by the list: a hidden PROTON_FORCE_NVAPI=0 would
    // remove that.
    let reading = read(PROTON_10);
    let unseen = Environment::new(Err("unreadable".to_owned()), &UserSettings::Absent);

    for appid in [GOTG, THPS] {
        assert!(
            matches!(
                resolve(&reading, appid, &unseen),
                Resolution::Undetermined(_)
            ),
            "{appid}"
        );
    }
}

#[test]
fn unreadable_user_settings_do_not_hide_what_a_launch_option_settles() {
    let reading = read(PROTON_10);
    let environment = Environment::new(
        Ok(vec![
            ("PROTON_FORCE_NVAPI".to_owned(), "1".to_owned()),
            ("PROTON_DISABLE_NVAPI".to_owned(), "1".to_owned()),
        ]),
        &UserSettings::Unreadable,
    );

    assert!(matches!(
        resolve(&reading, GOTG, &environment),
        Resolution::Computed {
            use_nvapi: true,
            ..
        }
    ));
}

#[test]
fn a_switch_settles_a_default_the_reader_could_not() {
    let guarded = PROTON_10.replace(
        r#"            ret.add("disablenvapi")

        if appid in [
                "2395210","#,
        r#"            try:
                with open('/proc/modules') as f:
                    if not f.read():
                        ret.add("disablenvapi")
            except OSError:
                ret.add("disablenvapi")

        if appid in [
                "2395210","#,
    );
    let reading = read(&guarded);

    assert!(matches!(
        resolve(&reading, GOTG, &launch(&[("PROTON_DISABLE_NVAPI", "1")])),
        Resolution::Computed {
            use_nvapi: false,
            ..
        }
    ));
    assert!(matches!(
        resolve(&reading, GOTG, &launch(&[("PROTON_FORCE_NVAPI", "1")])),
        Resolution::Computed {
            use_nvapi: true,
            ..
        }
    ));
}

#[test]
fn the_allow_list_builds_resolve_the_other_way_round() {
    let reading = read(PROTON_8);

    assert!(matches!(
        resolve(&reading, "440", &launch(&[("PROTON_ENABLE_NVAPI", "1")])),
        Resolution::Computed {
            use_nvapi: true,
            ..
        }
    ));
    assert!(matches!(
        resolve(&reading, "570", &launch(&[("PROTON_ENABLE_NVAPI", "0")])),
        Resolution::Computed {
            use_nvapi: false,
            ..
        }
    ));
}

#[test]
fn an_expression_and_lists_that_disagree_are_refused() {
    let script = PROTON_10.replace(
        "use_nvapi = 'disablenvapi' not in g_session.compat_config or 'forcenvapi' in g_session.compat_config",
        "use_nvapi = 'enablenvapi' in g_session.compat_config",
    );

    assert!(matches!(
        resolve(
            &read(&script),
            GOTG,
            &launch(&[("PROTON_FORCE_NVAPI", "1")])
        ),
        Resolution::Undetermined(_)
    ));
}

#[test]
fn user_settings_are_read_from_a_plain_dictionary() {
    let text = r#"#!/usr/bin/env python3
user_settings = {
    #logs are saved to $HOME/steam-$STEAM_APP_ID.log
    "PROTON_LOG": "1",
#    "DXVK_HUD": "devinfo",
    "PROTON_FORCE_NVAPI": "1",
}
"#;

    assert_eq!(
        user_settings(text),
        UserSettings::Read(vec![
            ("PROTON_LOG".to_owned(), "1".to_owned()),
            ("PROTON_FORCE_NVAPI".to_owned(), "1".to_owned()),
        ])
    );
}

#[test]
fn user_settings_that_python_would_build_some_other_way_are_unreadable() {
    for text in [
        "user_settings = dict(PROTON_LOG='1')\n",
        "user_settings = {\"PROTON_LOG\": 1}\n",
        "user_settings = {KEY: \"1\"}\n",
        "import os\n",
    ] {
        assert_eq!(user_settings(text), UserSettings::Unreadable, "{text}");
    }
}

#[test]
fn an_applied_switch_describes_itself_in_one_clause() {
    assert_eq!(
        applied(
            "PROTON_FORCE_NVAPI",
            "1",
            SetBy::LaunchOptions,
            Flag::Force,
            true
        )
        .describe(),
        "PROTON_FORCE_NVAPI=1 in launch options adds forcenvapi"
    );
    assert_eq!(
        applied(
            "PROTON_DISABLE_NVAPI",
            "0",
            SetBy::UserSettings,
            Flag::Disable,
            false
        )
        .describe(),
        "PROTON_DISABLE_NVAPI=0 in user_settings.py removes disablenvapi"
    );
}
