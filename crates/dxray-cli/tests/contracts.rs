mod common;

use common::{Image, TempDir, dxray_with_home, stderr_of, stdout_of};
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::process::Output;

const FILE_KEYS: &str = "path machine bits imports delay_imports file_version product_version error verdict renderers infrastructure features local_overrides";
const GAME_KEYS: &str = "kind origin origin_label install library id steam_appid name directory carries_evidence rows incomplete";

fn keys(value: &Value, expected: &str) {
    let mut expected: Vec<_> = expected.split_whitespace().collect();
    expected.sort_unstable();
    let mut actual: Vec<_> = value
        .as_object()
        .expect("JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    actual.sort_unstable();
    assert_eq!(actual, expected, "{value}");
}

fn plain(output: &Output) {
    for stream in [&output.stdout, &output.stderr] {
        assert!(
            !stream.contains(&0x1b),
            "unexpected ANSI escape: {stream:?}"
        );
    }
}

fn records(output: &Output, code: i32) -> Vec<Value> {
    assert_eq!(output.status.code(), Some(code), "{}", stderr_of(output));
    plain(output);
    let text = stdout_of(output);
    assert!(text.ends_with('\n'), "JSONL must end with a newline");
    text.lines()
        .map(|line| {
            let value: Value = serde_json::from_str(line).expect("each stdout line is pure JSON");
            assert!(value.is_object());
            value
        })
        .collect()
}

fn steam(home: &TempDir) {
    let root = home.path().join(".steam/steam");
    home.write(
        ".steam/steam/steamapps/libraryfolders.vdf",
        format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
            root.display()
        )
        .as_bytes(),
    );
    home.write(
        ".steam/steam/steamapps/appmanifest_42.acf",
        b"\"AppState\" { \"appid\" \"42\" \"name\" \"Sample\" \"installdir\" \"Sample\" }",
    );
    home.write(
        ".steam/steam/steamapps/common/Sample/Sample.exe",
        &Image::x64().build(),
    );
}

#[test]
fn file_jsonl_preserves_schema_and_keeps_failures_in_records() {
    let home = TempDir::new("contract-file");
    let good = home.write("good.exe", &Image::x64().importing(&["d3d11.dll"]).build());
    let bad = home.write("bad.exe", b"invalid image");
    let out = dxray_with_home(
        home.path(),
        [
            OsStr::new("inspect"),
            OsStr::new("--json"),
            good.as_os_str(),
            bad.as_os_str(),
        ],
    );
    let rows = records(&out, 1);
    assert_eq!(rows.len(), 2);
    assert!(out.stderr.is_empty());
    for row in &rows {
        keys(row, FILE_KEYS);
    }
    assert_eq!(rows[0]["path"], good.to_str().unwrap());
    assert_eq!(rows[0]["bits"], 64);
    assert_eq!(rows[0]["imports"], json!(["d3d11.dll"]));
    assert_eq!(rows[0]["error"], Value::Null);
    assert_eq!(rows[0]["verdict"], "Direct3D 11");
    keys(&rows[0]["renderers"][0], "name via");
    keys(&rows[0]["renderers"][0]["via"][0], "library source version");
    keys(
        &rows[0]["renderers"][0]["via"][0]["version"],
        "state file product",
    );
    assert_eq!(rows[1]["path"], bad.to_str().unwrap());
    assert!(rows[1]["error"].is_string());
    assert!(rows[1]["machine"].is_null());
    assert_eq!(rows[1]["imports"], json!([]));
    let clean = dxray_with_home(
        home.path(),
        [
            OsStr::new("inspect"),
            OsStr::new("--json"),
            good.as_os_str(),
        ],
    );
    assert_eq!(records(&clean, 0).len(), 1);
    assert!(clean.stderr.is_empty());
}

#[test]
fn game_schema_and_static_caveat_do_not_imply_a_failed_scan() {
    let home = TempDir::new("contract-game");
    let game = home.write("Game/Game.exe", &Image::x64().build());
    let directory = game.parent().unwrap();
    let out = dxray_with_home(
        home.path(),
        [
            OsStr::new("game"),
            OsStr::new("--json"),
            directory.as_os_str(),
        ],
    );
    let rows = records(&out, 0);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    keys(
        row,
        &format!("{FILE_KEYS} directory candidates survey_notes"),
    );
    assert_eq!(row["directory"], directory.to_str().unwrap());
    assert_eq!(row["path"], game.to_str().unwrap());
    keys(&row["candidates"][0], "path score reasons");
    assert!(row["candidates"][0]["score"].is_number());
    for reason in row["candidates"][0]["reasons"].as_array().unwrap() {
        keys(reason, "kind weight says");
    }
    assert_eq!(row["verdict"], "no graphics API determined");
    assert!(
        row["survey_notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().contains("run time"))
    );
    assert!(out.stderr.is_empty());
    let empty = home.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    let failed = dxray_with_home(
        home.path(),
        [OsStr::new("game"), OsStr::new("--json"), empty.as_os_str()],
    );
    let failure = records(&failed, 1);
    keys(
        &failure[0],
        &format!("{FILE_KEYS} directory candidates survey_notes"),
    );
    assert!(failure[0]["error"].is_string());
    assert_eq!(failure[0]["candidates"], json!([]));
    assert!(failed.stderr.is_empty());
}

#[test]
fn both_inventory_modes_have_tagged_schemas_and_stderr_problems() {
    let home = TempDir::new("contract-inventory");
    steam(&home);
    for mode in ["steam", "installed"] {
        let out = dxray_with_home(home.path(), [mode, "--json"]);
        let rows = records(&out, 0);
        assert_eq!(
            rows.iter()
                .map(|r| r["kind"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["install", "library", "game", "summary"]
        );
        keys(&rows[0], "kind origin origin_label path");
        keys(&rows[1], "kind origin origin_label install path");
        keys(&rows[2], GAME_KEYS);
        assert_eq!(rows[2]["steam_appid"], 42);
        assert_eq!(rows[2]["id"], "42");
        assert_eq!(rows[2]["origin"], "steam");
        assert_eq!(rows[2]["incomplete"], json!([]));
        for row in rows[2]["rows"].as_array().unwrap() {
            keys(row, "label says");
        }
        keys(
            &rows[3],
            "kind games libraries installs complete notes caveats says",
        );
        keys(&rows[3]["notes"], "index record");
        assert_eq!(rows[3]["complete"], true);
        assert_eq!(rows[3]["games"], 1);
        assert!(out.stderr.is_empty());
        let human = dxray_with_home(home.path(), [mode]);
        plain(&human);
        assert_eq!(human.status.code(), Some(0));
    }
    home.write(
        ".steam/steam/steamapps/appmanifest_99.acf",
        b"\"AppState\" {",
    );
    for mode in ["steam", "installed"] {
        let out = dxray_with_home(home.path(), [mode, "--json"]);
        let rows = records(&out, 1);
        let problem = rows.iter().find(|r| r["kind"] == "problem").unwrap();
        keys(
            problem,
            "kind origin origin_label install library label says",
        );
        assert!(stderr_of(&out).contains(problem["says"].as_str().unwrap()));
        assert_eq!(rows.last().unwrap()["complete"], false);
        assert!(rows.iter().any(|r| r["kind"] == "game"));
    }
}

#[test]
fn a_redundant_heroic_record_is_a_note_not_a_stderr_problem() {
    let home = TempDir::new("contract-note");
    let path = home.write("Games/Sample/Sample.exe", &Image::x64().build());
    home.write(
        ".config/heroic/store_cache/gog_install_info.json",
        json!({"42": {"title": "Sample", "install": {"install_path": path.parent().unwrap()}}})
            .to_string()
            .as_bytes(),
    );
    home.write(
        ".config/heroic/store_cache/gog_library.json",
        br#"{"library":[{"app_name":"42","title":"Sample","is_installed":true}]}"#,
    );
    let out = dxray_with_home(home.path(), ["installed", "--json"]);
    let rows = records(&out, 0);
    let note = rows.iter().find(|r| r["kind"] == "note").unwrap();
    keys(
        note,
        "kind origin origin_label install library label cause says",
    );
    assert_eq!(note["cause"], "record");
    assert_eq!(note["label"], "note");
    assert!(!rows.iter().any(|r| r["kind"] == "problem"));
    assert_eq!(rows.last().unwrap()["notes"]["record"], 1);
    assert_eq!(rows.last().unwrap()["complete"], true);
    assert!(out.stderr.is_empty());
}

#[test]
fn no_launcher_is_one_problem_without_a_summary() {
    let home = TempDir::new("contract-no-launcher");
    for mode in ["steam", "installed"] {
        let out = dxray_with_home(home.path(), [mode, "--json"]);
        let rows = records(&out, 1);
        assert_eq!(rows.len(), 1);
        keys(
            &rows[0],
            "kind origin origin_label install library label says",
        );
        assert_eq!(rows[0]["kind"], "problem");
        assert!(rows[0]["origin"].is_null());
        assert!(stderr_of(&out).contains(rows[0]["says"].as_str().unwrap()));
    }
}

#[test]
fn piped_human_file_and_game_output_is_plain_text() {
    let home = TempDir::new("contract-human");
    let file = home.write(
        "Game/Game.exe",
        &Image::x64().importing(&["d3d11.dll"]).build(),
    );
    for args in [
        vec![OsStr::new("inspect"), file.as_os_str()],
        vec![OsStr::new("game"), file.parent().unwrap().as_os_str()],
    ] {
        let out = dxray_with_home(home.path(), args);
        assert_eq!(out.status.code(), Some(0));
        assert!(stdout_of(&out).contains("Direct3D 11"));
        assert!(out.stderr.is_empty());
        plain(&out);
    }
}

#[test]
fn usage_errors_are_stderr_only_and_exit_two_even_with_json() {
    let home = TempDir::new("contract-usage");
    for args in [
        vec!["inspect", "--json", "--unknown-option"],
        vec!["game", "--json", "--recursive", "."],
        vec!["steam", "--json", "--appid", "1"],
    ] {
        let out = dxray_with_home(home.path(), args);
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.is_empty());
        assert!(stderr_of(&out).contains("error:"));
        plain(&out);
    }
}

#[cfg(unix)]
#[test]
fn hostile_names_and_paths_round_trip_without_splitting_jsonl() {
    let home = TempDir::new("contract-escaping");
    let name = "Quote\" slash\\ tab\t line\nreturn\r é \u{1b}[31m";
    let file = home.write(&format!("{name}/Game.exe"), &Image::x64().build());
    for (mode, target) in [
        (None, file.as_path()),
        (Some("game"), file.parent().unwrap()),
    ] {
        let mut args = vec![OsStr::new(mode.unwrap_or("inspect")), OsStr::new("--json")];
        args.push(target.as_os_str());
        let out = dxray_with_home(home.path(), args);
        let rows = records(&out, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["path"], file.to_str().unwrap());
        if mode.is_some() {
            assert_eq!(rows[0]["directory"], target.to_str().unwrap());
            assert_eq!(rows[0]["candidates"][0]["path"], file.to_str().unwrap());
        }
    }
    home.write(
        ".config/heroic/store_cache/gog_install_info.json",
        json!({"42": {"title": name, "install": {"install_path": file.parent().unwrap()}}})
            .to_string()
            .as_bytes(),
    );
    let out = dxray_with_home(home.path(), ["installed", "--json"]);
    let rows = records(&out, 0);
    let game = rows.iter().find(|r| r["kind"] == "game").unwrap();
    assert_eq!(game["name"], name);
    assert_eq!(game["directory"], file.parent().unwrap().to_str().unwrap());
    assert!(game["steam_appid"].is_null());
    keys(game, GAME_KEYS);
    assert!(out.stderr.is_empty());
}
