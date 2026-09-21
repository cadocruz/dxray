mod common;
use common::{Image, TempDir, dxray_with_home, stdout_of};

#[test]
fn subcommand_help_and_option_scopes() {
    let home = TempDir::new("view-args");
    let file = home.write("Game.exe", &Image::x64().build());
    let path = file.to_str().unwrap();
    for help in ["-h", "--help"] {
        let root = dxray_with_home(home.path(), [help]);
        assert_eq!(root.status.code(), Some(0));
        for command in ["inspect", "game", "installed", "steam", "nvapi"] {
            assert!(stdout_of(&root).contains(command));
            let out = dxray_with_home(home.path(), [command, help]);
            assert_eq!(out.status.code(), Some(0));
            let text = stdout_of(&out);
            assert_eq!(text.contains("--appid"), command == "nvapi");
            assert_eq!(text.contains("--json"), command != "nvapi");
            assert_eq!(text.contains("--recursive"), command == "inspect");
            assert_eq!(text.contains("--view"), command != "nvapi");
        }
    }
    let mut invalid = vec![
        vec![],
        vec![path],
        vec!["inspect"],
        vec!["game"],
        vec!["nvapi"],
        vec!["--json", "inspect", path],
        vec!["--view", "compact", "inspect", path],
        vec!["nvapi", path, "--json"],
    ];
    for command in ["inspect", "game"] {
        invalid.push(vec![command, "--appid", "1", path]);
        invalid.push(vec![command, "--view"]);
        invalid.push(vec![command, "--view", "unknown", path]);
        invalid.push(vec![command, "--view", "compact", "--view", "full", path]);
        for view in ["compact", "full"] {
            invalid.push(vec![command, "--view", view, "--json", path]);
        }
    }
    for command in ["installed", "steam", "nvapi"] {
        invalid.push(vec![command, "--view", "compact", path]);
        invalid.push(vec![command, "--recursive"]);
    }
    for command in ["installed", "steam"] {
        invalid.push(vec![command, "--appid", "1"]);
        invalid.push(vec![command, path]);
    }
    invalid.push(vec!["game", "--recursive", path]);
    for args in invalid {
        let out = dxray_with_home(home.path(), &args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert!(!out.stderr.is_empty());
    }
    let out = dxray_with_home(home.path(), ["nvapi", path, "--appid", "1", "--appid", "2"]);
    assert_eq!(out.status.code(), Some(1));
}
#[test]
fn views_keep_identity_full_evidence_and_recursive_results() {
    let home = TempDir::new("view-files");
    let bytes = Image::x64()
        .importing(&["d3d11.dll", "KERNEL32.dll"])
        .delay_loading(&["d3d12.dll"])
        .versioned([1, 2, 3, 4], [5, 6, 7, 8])
        .build();
    let a = home.write("A/Game.exe", &bytes);
    let b = home.write("B/Game.exe", &bytes);
    home.write("A/nvngx_dlss.dll", &Image::x64().build());
    for view in ["compact", "full"] {
        let out = dxray_with_home(
            home.path(),
            [
                "inspect",
                "--view",
                view,
                a.to_str().unwrap(),
                b.to_str().unwrap(),
            ],
        );
        assert_eq!(out.status.code(), Some(0));
        assert!(out.stderr.is_empty());
        let text = stdout_of(&out);
        for path in [&a, &b] {
            assert_eq!(
                text.matches(&format!("File: {}", path.display())).count(),
                1
            );
        }
        for token in [
            "DLSS",
            "static Proton policy not assessed",
            "does not establish runtime",
        ] {
            assert!(text.contains(token));
        }
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains("Complete static evidence"));
        assert_eq!(text.contains("KERNEL32.dll"), view == "full");
        if view == "full" {
            assert_eq!(text.matches("1.2.3.4").count(), 2);
            assert!(text.contains("5.6.7.8"));
            assert!(text.contains("delay-import"));
        }
        let recursive = dxray_with_home(
            home.path(),
            [
                "inspect",
                "--view",
                view,
                "--recursive",
                home.path().to_str().unwrap(),
            ],
        );
        assert_eq!(recursive.status.code(), Some(0));
        for path in [&a, &b] {
            assert!(stdout_of(&recursive).contains(&format!("File: {}", path.display())));
        }
    }
}

#[test]
fn game_views_preserve_caveats_ranking_and_failure_codes() {
    let home = TempDir::new("view-game");
    let file = home.write("Game/Game.exe", &Image::x64().build());
    let dir = file.parent().unwrap().to_str().unwrap();
    let bad = home.write("bad.exe", b"invalid");
    let empty = home.write("Empty/readme.txt", b"empty");
    for view in ["compact", "full"] {
        let out = dxray_with_home(home.path(), ["game", "--view", view, dir]);
        assert_eq!(out.status.code(), Some(0));
        let text = stdout_of(&out);
        assert!(text.contains(&format!("Install: {dir}")));
        assert!(text.contains("No graphics API determined statically"));
        assert!(text.contains("import table"));
        assert_eq!(text.matches("File: ").count(), 1);
        if view == "full" {
            for token in ["selected", "score", "ranked"] {
                assert!(text.contains(token));
            }
        }
        for args in [
            vec!["inspect", "--view", view, bad.to_str().unwrap()],
            vec![
                "game",
                "--view",
                view,
                empty.parent().unwrap().to_str().unwrap(),
            ],
            vec![
                "game",
                "--view",
                view,
                dir,
                file.to_str().unwrap(),
                bad.to_str().unwrap(),
            ],
        ] {
            let failed = dxray_with_home(home.path(), args);
            assert_eq!(failed.status.code(), Some(1));
            assert!(failed.stderr.is_empty());
        }
    }
}

#[test]
fn default_output_and_json_preserve_analysis() {
    let home = TempDir::new("view-default");
    let bad = home.write("bad.exe", b"invalid");
    let file = home.write("Game/Game.exe", &Image::x64().build());
    let out = dxray_with_home(home.path(), ["inspect", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout_of(&out);
    assert_eq!(
        text,
        format!(
            "{}\n  machine   x86-64 (64-bit)\n  version   none\n  verdict   no graphics API determined\n  imports   none\n\n",
            file.display()
        )
    );
    let game = dxray_with_home(
        home.path(),
        ["game", file.parent().unwrap().to_str().unwrap()],
    );
    assert_eq!(game.status.code(), Some(0));
    let game_text = stdout_of(&game);
    assert!(
        game_text.contains("ranked"),
        "game keeps the standard ranking: {game_text}"
    );
    assert!(
        game_text.contains("verdict   no graphics API determined"),
        "game keeps the standard report: {game_text}"
    );
    assert!(
        !game_text.contains("Install: "),
        "game did not inherit the inventory compact view: {game_text}"
    );
    for (args, code) in [
        (
            vec![
                "inspect",
                "--json",
                file.to_str().unwrap(),
                bad.to_str().unwrap(),
            ],
            1,
        ),
        (
            vec!["game", "--json", file.parent().unwrap().to_str().unwrap()],
            0,
        ),
    ] {
        let json = dxray_with_home(home.path(), args);
        assert_eq!(json.status.code(), Some(code));
        assert!(json.stderr.is_empty());
        for line in stdout_of(&json).lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(value.get("path").is_some());
            assert!(value.get("view").is_none());
        }
    }
}

#[test]
fn positional_paths_and_relative_full_install_identity() {
    let home = TempDir::new("command-paths");
    home.write("Game/Game.exe", &Image::x64().build());
    home.write("steam", &Image::x64().build());
    home.write("-game.exe", &Image::x64().build());
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_dxray"))
            .current_dir(home.path())
            .args(args)
            .output()
            .unwrap()
    };
    let before = run(&["game", "--view", "full", "Game"]);
    let after = run(&["game", "Game", "--view", "full"]);
    assert_eq!(before.status.code(), Some(0));
    assert_eq!(after.status.code(), Some(0));
    assert_eq!(before.stdout, after.stdout);
    assert!(stdout_of(&before).starts_with(&format!(
        "Install: {}\n",
        home.path().join("Game").display()
    )));
    let files = run(&["inspect", "--json", "--", "steam", "-game.exe"]);
    assert_eq!(files.status.code(), Some(0));
    let rows: Vec<serde_json::Value> = stdout_of(&files)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["path"], "steam");
    assert_eq!(rows[1]["path"], "-game.exe");
}
