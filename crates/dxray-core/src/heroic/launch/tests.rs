use std::path::PathBuf;

use super::{Launch, NoLaunch, launch};
use crate::heroic::Store;
use crate::testutil::TempDir;

const APP: &str = "e0fa47ae79514345823bff209ae29451";

fn pairs(values: &[(&str, &str)]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// A Heroic configuration whose game settings are `game`, a JSON object body.
fn configured(tag: &str, game: &str) -> TempDir {
    let root = TempDir::new(tag);
    root.write(
        &format!("GamesConfig/{APP}.json"),
        &format!(r#"{{"{APP}":{{{game}}},"version":"v0","explicit":false}}"#),
    );
    root
}

const PROTON: &str = r#""wineVersion":{"bin":"/opt/GE-Proton/proton","name":"GE-Proton","type":"proton"},"winePrefix":"/games/prefix""#;

#[test]
fn a_proton_game_hands_over_its_prefix_options_and_the_switch_heroic_sets() {
    let root = configured(
        "heroic-launch",
        &format!(
            r#"{PROTON},"autoInstallDxvkNvapi":false,"enviromentOptions":[{{"key":"PROTON_DISABLE_NVAPI","value":"0"}},{{"key":"WINEDLLOVERRIDES","value":"\"dxgi=n,b\""}}]"#
        ),
    );
    root.write(
        "store_cache/umu.json",
        &format!(r#"{{"legendary_{APP}":"umu-752590"}}"#),
    );

    assert_eq!(
        launch(root.path(), Store::Epic, APP),
        Ok(Launch {
            prefix: PathBuf::from("/games/prefix"),
            // Heroic's switch comes last, so it wins over the option.
            environment: pairs(&[
                ("PROTON_DISABLE_NVAPI", "0"),
                ("WINEDLLOVERRIDES", "dxgi=n,b"),
                ("PROTON_DISABLE_NVAPI", "1"),
            ]),
            appid: Ok("752590".to_owned()),
        })
    );
}

#[test]
fn a_setting_the_game_leaves_out_comes_from_the_defaults() {
    let root = configured("heroic-defaults", r#""winePrefix":"/games/prefix""#);
    root.write(
        "config.json",
        r#"{"defaultSettings":{"wineVersion":{"bin":"/p/proton","name":"P","type":"proton"},"autoInstallDxvkNvapi":false,"disableUMU":true},"version":"v0"}"#,
    );

    let launch = launch(root.path(), Store::Epic, APP).expect("a Proton launch");

    assert_eq!(launch.environment, pairs(&[("PROTON_DISABLE_NVAPI", "1")]));
    assert_eq!(
        launch.appid,
        Ok("0".to_owned()),
        "without umu Heroic passes 0"
    );
}

#[test]
fn with_no_nvapi_setting_anywhere_heroic_enables_it() {
    let root = configured("heroic-nvapi-default", PROTON);
    root.write(
        "store_cache/umu.json",
        &format!(r#"{{"legendary_{APP}":null}}"#),
    );

    let launch = launch(root.path(), Store::Epic, APP).expect("a Proton launch");

    assert_eq!(
        launch.environment,
        pairs(&[
            ("PROTON_ENABLE_NVAPI", "1"),
            ("DXVK_NVAPI_ALLOW_OTHER_DRIVERS", "1")
        ])
    );
    assert_eq!(
        launch.appid,
        Ok("0".to_owned()),
        "measured: with no umu id Heroic passes GAMEID=umu-0"
    );
}

#[test]
fn a_game_heroic_never_looked_up_in_umu_has_no_known_appid() {
    let root = configured("heroic-umu-unknown", PROTON);

    let launch = launch(root.path(), Store::Gog, APP).expect("a Proton launch");

    assert!(launch.appid.is_err(), "{:?}", launch.appid);
}

#[test]
fn another_runner_is_not_proton_and_missing_settings_are_not_guessed() {
    let wine = configured(
        "heroic-wine",
        r#""wineVersion":{"bin":"/usr/bin/wine","name":"Wine","type":"wine"},"winePrefix":"/p""#,
    );
    assert_eq!(
        launch(wine.path(), Store::Epic, APP),
        Err(NoLaunch::NotProton("wine".to_owned()))
    );

    let bare = TempDir::new("heroic-bare");
    assert!(matches!(
        launch(bare.path(), Store::Epic, APP),
        Err(NoLaunch::Unknown(_))
    ));
}

#[test]
fn a_prefix_under_a_tilde_starts_at_the_home_the_configuration_sits_in() {
    let home = TempDir::new("heroic-home");
    let root = home.dir(".config/heroic");
    std::fs::create_dir_all(root.join("GamesConfig")).expect("tree");
    std::fs::write(
        root.join(format!("GamesConfig/{APP}.json")),
        format!(
            r#"{{"{APP}":{{"wineVersion":{{"bin":"/p/proton","name":"P","type":"proton"}},"winePrefix":"~/Games/Heroic/Prefixes/Game","disableUMU":true}}}}"#
        ),
    )
    .expect("settings");

    let launch = launch(&root, Store::Epic, APP).expect("a Proton launch");

    assert_eq!(
        launch.prefix,
        home.path().join("Games/Heroic/Prefixes/Game")
    );
}
