//! End to end tests for `installed`: the real binary, a fake machine with two
//! launchers, real exit codes. No launcher is privileged or left out.

mod common;

#[test]
fn inventory_views_preserve_entries_diagnostics_and_status() {
    let home = TempDir::new("inventory-views");
    let steam = fake_steam(&home);
    let heroic = fake_heroic(&home, true);
    // Same display name across launchers must retain distinct identities.
    std::fs::write(
        steam.join("steamapps/common/dota 2 beta/dota2.exe"),
        Image::x64().importing(&["d3d11.dll"]).build(),
    )
    .unwrap();
    std::fs::write(
        steam.join("steamapps/common/dota 2 beta/nvngx_dlss.dll"),
        Image::x64().build(),
    )
    .unwrap();
    let manifest = steam.join("steamapps/appmanifest_570.acf");
    let contents = std::fs::read_to_string(&manifest)
        .unwrap()
        .replace("Dota 2", "Hades");
    std::fs::write(manifest, contents).unwrap();
    std::fs::write(
        heroic.join("store_cache/gog_library.json"),
        "{\"library\":[{\"app_name\":\"1207658691\",\"title\":\"Hades\",\"is_installed\":true}]}",
    )
    .unwrap();
    std::fs::write(steam.join("steamapps/appmanifest_999.acf"), "broken").unwrap();
    for (command, args) in [
        ("installed", &["installed"][..]),
        ("steam", &["installed", "--launcher", "steam"][..]),
    ] {
        let default = dxray_with_home(home.path(), args.to_vec());
        let json = dxray_with_home(home.path(), [args, &["--json"][..]].concat());
        assert!(stdout_of(&default).contains("Library:"));
        assert!(stdout_of(&default).contains("├─") || stdout_of(&default).contains("└─"));
        let compact = dxray_with_home(home.path(), [args, &["--view", "compact"][..]].concat());
        assert_eq!(
            compact.stdout, default.stdout,
            "{command} default must be compact"
        );
        assert_eq!(compact.stderr, default.stderr);
        for view in ["compact", "full"] {
            let out = dxray_with_home(home.path(), [args, &["--view", view][..]].concat());
            let text = stdout_of(&out);
            assert_eq!(out.status.code(), default.status.code());
            assert_eq!(out.stderr, default.stderr);
            assert!(text.contains("Direct3D 11"));
            assert!(text.contains("DLSS"));
            for token in [
                "570  Hades",
                "Steam",
                "570",
                "Direct3D 11",
                "unreadable",
                "dota 2 beta",
            ] {
                assert!(text.contains(token), "missing {token}: {text}");
            }
            if command == "installed" {
                assert_eq!(text.matches("Hades  [").count(), 2);
                for token in ["Heroic", "1207658691", "Games/Hades", "note"] {
                    assert!(text.contains(token), "missing {token}: {text}");
                }
            }
            if view == "full" {
                assert!(text.contains("ranked"));
                assert!(text.contains("Launcher root"));
            } else {
                assert_compact_inventory(text, stdout_of(&default), command);
            }
            let conflict = dxray_with_home(
                home.path(),
                [args, &["--view", view, "--json"][..]].concat(),
            );
            assert_eq!(conflict.status.code(), Some(2));
            assert!(conflict.stdout.is_empty());
            assert!(!stderr_of(&conflict).contains("broken"));
        }
        assert_eq!(
            dxray_with_home(home.path(), args.to_vec()).stdout,
            default.stdout
        );
        assert_eq!(
            dxray_with_home(home.path(), [args, &["--json"][..]].concat()).stdout,
            json.stdout
        );
    }
}

fn assert_compact_inventory(text: &str, default: &str, command: &str) {
    assert!(text.lines().count() <= default.lines().count());
    for full_only in [
        "launcher root",
        "ranked",
        "graphics  ",
        "version   ",
        "imports   ",
    ] {
        assert!(!text.contains(full_only), "unexpected {full_only}: {text}");
    }
    let entries: Vec<_> = text
        .lines()
        .filter(|line| line.contains("Hades  ["))
        .collect();
    assert_eq!(entries.len(), if command == "installed" { 2 } else { 1 });
    assert!(entries.iter().any(|line| line.contains("570  Hades")));
    if command == "installed" {
        assert!(
            entries
                .iter()
                .any(|line| { line.contains("1207658691  Hades") })
        );
        assert!(
            entries
                .iter()
                .any(|line| { line.contains("1207658691  Hades") })
        );
        assert!(text.contains("note"));
    }
    assert!(
        text.contains("Static evidence only; runtime use and compatibility are not established.")
    );
    assert!(text.contains("unreadable"));
}

#[test]
fn compact_steam_uses_each_library_header_for_relative_install_paths() {
    let home = TempDir::new("compact-multiple-libraries");
    let root = fake_steam(&home);
    let other = home.path().join("another/long/steam/library/location");
    let other_install = other.join("steamapps/common/dota 2 beta");
    std::fs::create_dir_all(&other_install).unwrap();
    std::fs::write(other_install.join("dota2.exe"), Image::x64().build()).unwrap();
    std::fs::write(
        other.join("steamapps/appmanifest_570.acf"),
        "\"AppState\" { \"appid\" \"570\" \"name\" \"Dota 2\" \"installdir\" \"dota 2 beta\" }",
    )
    .unwrap();
    std::fs::write(
        root.join("steamapps/libraryfolders.vdf"),
        format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
            root.display(),
            other.display()
        ),
    )
    .unwrap();

    let compact = dxray_with_home(
        home.path(),
        ["installed", "--launcher", "steam", "--view", "compact"],
    );
    let text = stdout_of(&compact);
    let entries: Vec<_> = text
        .lines()
        .filter(|line| line.contains("570  Dota 2"))
        .collect();
    assert_eq!(entries.len(), 2, "{text}");
    assert!(entries.iter().all(|line| line.contains("570  Dota 2")));
    let first_library = text.find(&root.display().to_string()).unwrap();
    let first_entry = text.find("570  Dota 2").unwrap();
    let second_library = text.find(&other.display().to_string()).unwrap();
    let second_entry = text.rfind("570  Dota 2").unwrap();
    assert!(first_library < first_entry && first_entry < second_library);
    assert!(second_library < second_entry, "{text}");
    assert_eq!(
        text.matches("Path: ./steamapps/common/dota 2 beta").count(),
        2
    );

    let default = dxray_with_home(home.path(), ["installed", "--launcher", "steam"]);
    let full = dxray_with_home(
        home.path(),
        ["installed", "--launcher", "steam", "--view", "full"],
    );
    let json = dxray_with_home(home.path(), ["installed", "--launcher", "steam", "--json"]);
    assert_eq!(compact.stdout, default.stdout);
    assert_eq!(compact.stderr, default.stderr);
    assert_eq!(full.status.code(), default.status.code());
    assert_eq!(json.status.code(), default.status.code());
}

use common::{Image, TempDir, dxray_with_home, stderr_of, stdout_of};
use std::path::{Path, PathBuf};

#[test]
fn inventory_views_distinguish_empty_and_unavailable_installs() {
    let home = TempDir::new("inventory-empty");
    let root = fake_steam_tool(&home);
    std::fs::remove_file(root.join("steamapps/common/dota 2 beta/dota2.exe")).unwrap();
    std::fs::remove_dir(root.join("steamapps/common/dota 2 beta")).unwrap();
    for args in [
        &["installed"][..],
        &["installed", "--launcher", "steam"][..],
    ] {
        let default = dxray_with_home(home.path(), args.to_vec());
        for view in ["compact", "full"] {
            let out = dxray_with_home(home.path(), [args, &["--view", view][..]].concat());
            assert_eq!(out.status.code(), default.status.code());
            let text = stdout_of(&out);
            assert!(text.contains("no executable found"));
            assert!(text.contains("installation could not be read"));
            assert!(text.contains("Proton Experimental"));
            assert!(text.contains("Dota 2"));
            assert!(text.contains("No executable") || text.contains("Evidence unavailable"));
            if view == "compact" {
                assert_eq!(out.stdout, default.stdout);
                assert!(text.contains("Installations without game evidence"));
                assert!(text.contains("Evidence unavailable"));
                assert!(!text.contains("Tools & Runtimes"));
                assert!(!text.contains("tool/runtime"));
            }
        }
    }
}

/// Builds a Steam install under `home` holding one game.
fn fake_steam(home: &TempDir) -> PathBuf {
    let root = home.path().join(".steam/steam");
    let steamapps = root.join("steamapps");
    let install = steamapps.join("common/dota 2 beta");
    std::fs::create_dir_all(&install).expect("tree");
    // A real image, so the ranking is a ranking rather than a caveat about a
    // file that could not be parsed.
    std::fs::write(install.join("dota2.exe"), Image::x64().build()).expect("executable");

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

/// Adds a Steam manifest for content Steam labels as a tool. `installed`
/// lists it: it answers what is installed, not what is launchable.
fn fake_steam_tool(home: &TempDir) -> PathBuf {
    let root = fake_steam(home);
    let steamapps = root.join("steamapps");
    let install = steamapps.join("common/Proton - Experimental");
    std::fs::create_dir_all(&install).expect("tool install directory");
    std::fs::write(
        steamapps.join("appmanifest_1493710.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"1493710\"\n\t\"name\"\t\t\"Proton Experimental\"\n\
         \t\"installdir\"\t\t\"Proton - Experimental\"\n\t\"DownloadType\"\t\t\"1\"\n}\n",
    )
    .expect("tool manifest");

    root
}

/// Builds a Heroic configuration under `home` holding one GOG game.
/// `store_cache` alone makes an installation, as when nobody has signed in.
fn fake_heroic(home: &TempDir, with_game: bool) -> PathBuf {
    let root = home.path().join(".config/heroic");
    std::fs::create_dir_all(root.join("store_cache")).expect("store_cache");
    if !with_game {
        return root;
    }

    let install = home.path().join("Games/Hades");
    std::fs::create_dir_all(&install).expect("install directory");
    std::fs::write(install.join("Hades.exe"), Image::x64().build()).expect("executable");
    std::fs::write(
        root.join("store_cache/gog_install_info.json"),
        format!(
            "{{\"1207658691\":{{\"title\":\"Hades\",\"install\":\
             {{\"install_path\":\"{}\"}}}}}}",
            install.display()
        ),
    )
    .expect("gog cache");

    root
}

#[test]
fn every_launcher_on_the_machine_is_listed_and_the_full_view_names_each_game_source() {
    // Both launchers' games are listed.
    let home = TempDir::new("games-both");
    let steam = fake_steam(&home);
    let heroic = fake_heroic(&home, true);

    let out = dxray_with_home(home.path(), ["installed"]);
    let text = stdout_of(&out);

    for root in [&steam, &heroic] {
        assert!(
            text.contains(&root.display().to_string()),
            "{} must be listed, got:\n{text}",
            root.display()
        );
    }
    assert!(text.contains("Dota 2"), "got:\n{text}");
    assert!(text.contains("Hades"), "got:\n{text}");
    assert!(
        text.contains("Steam ["),
        "the Steam block establishes the source for its games, got:\n{text}"
    );
    assert!(text.contains("Heroic ["), "got:\n{text}");
    assert!(
        text.contains(&format!("Library: {} (1 installation)", heroic.display())),
        "the Heroic configuration is not a Steam library, got:\n{text}"
    );

    let full = dxray_with_home(home.path(), ["installed", "--view", "full"]);
    assert!(
        stdout_of(&full).contains("Source: Heroic / GOG"),
        "the full view names the shop for each entry, got:\n{}",
        stdout_of(&full)
    );
}

#[test]
fn installed_keeps_the_complete_steam_inventory_including_proton() {
    // A `DownloadType=1` runtime is listed like any other install.
    let home = TempDir::new("installed-steam-game-and-proton");
    fake_steam_tool(&home);

    let out = dxray_with_home(home.path(), ["installed"]);
    let text = stdout_of(&out);

    for (appid, name) in [("570", "Dota 2"), ("1493710", "Proton Experimental")] {
        assert!(
            text.contains(appid),
            "missing Steam identity {appid}:\n{text}"
        );
        assert!(text.contains(name), "missing Steam record {name}:\n{text}");
    }
    assert!(
        text.contains("2 games in 1 library across 1 install"),
        "the complete inventory count must include the runtime, got:\n{text}"
    );
    assert_eq!(out.status.code(), Some(0), "got:\n{}", stderr_of(&out));
}

#[test]
fn one_trailer_counts_both_launchers_rather_than_one_per_launcher() {
    // The trailer counts across both launchers.
    let home = TempDir::new("games-trailer");
    fake_steam(&home);
    fake_heroic(&home, true);

    let text = stdout_of(&dxray_with_home(home.path(), ["installed"])).to_owned();

    assert!(
        text.contains("2 games in 2 libraries across 2 installs"),
        "got:\n{text}"
    );
    assert_eq!(
        text.matches("across").count(),
        1,
        "one summary for one run, not one per launcher, got:\n{text}"
    );
}

#[test]
fn a_machine_with_only_one_launcher_on_it_gets_an_honest_trailer() {
    // An absent launcher gets no zero line.
    let home = TempDir::new("games-heroic-only");
    fake_heroic(&home, true);

    let out = dxray_with_home(home.path(), ["installed"]);
    let text = stdout_of(&out);

    assert!(
        text.contains("1 game in 1 library across 1 install"),
        "got:\n{text}"
    );
    // No Steam root or library for a Steam that is not installed.
    assert!(
        !text.contains("/.steam/"),
        "a launcher that is not installed contributes no root and no library, \
         got:\n{text}"
    );
    assert_eq!(out.status.code(), Some(0), "got:\n{}", stderr_of(&out));
}

#[test]
fn an_installed_launcher_holding_nothing_is_counted_rather_than_hidden() {
    // An empty Heroic is shown, as an empty Steam library is.
    let home = TempDir::new("games-heroic-empty");
    let root = fake_heroic(&home, false);

    let out = dxray_with_home(home.path(), ["installed"]);
    let text = stdout_of(&out);

    assert!(
        text.contains(&root.display().to_string()),
        "the installation is named, got:\n{text}"
    );
    assert!(
        text.contains("(no games installed)"),
        "and said to hold nothing rather than printing a blank, got:\n{text}"
    );
    assert!(
        text.contains("0 games in 1 library across 1 install"),
        "got:\n{text}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "owning no games is not a failed scan"
    );
}

#[test]
fn a_launcher_that_cannot_be_read_does_not_hide_the_other_launcher_s_games() {
    // One broken launcher keeps the other's games, and exits 1.
    let home = TempDir::new("games-broken-steam");
    let steam = fake_steam(&home);
    fake_heroic(&home, true);
    std::fs::write(
        steam.join("steamapps/libraryfolders.vdf"),
        "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\"/mnt/g\"\n",
    )
    .expect("truncate the index");

    let out = dxray_with_home(home.path(), ["installed"]);
    let text = stdout_of(&out);

    assert!(
        text.contains("Hades"),
        "the working launcher survives its neighbour, got:\n{text}"
    );
    assert!(
        text.contains("libraryfolders.vdf"),
        "and the broken one is named in the listing, got:\n{text}"
    );
    assert!(
        stderr_of(&out).contains("libraryfolders.vdf"),
        "and on stderr, so a piped run still sees it, got:\n{}",
        stderr_of(&out)
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "an incomplete scan is not a clean one"
    );
}

#[test]
fn a_stale_heroic_record_for_a_game_that_was_found_anyway_leaves_the_scan_clean() {
    // A stale record for a game found elsewhere is a note, not a failure.
    let home = TempDir::new("games-stale-record");
    let root = fake_heroic(&home, true);
    std::fs::write(
        root.join("store_cache/gog_library.json"),
        "{\"library\":[{\"app_name\":\"1207658691\",\"title\":\"Hades\",         \"is_installed\":true}]}",
    )
    .expect("a record that lost its install path");

    let out = dxray_with_home(home.path(), ["installed"]);
    let text = stdout_of(&out);

    assert!(
        text.contains("Hades"),
        "the game is listed from the cache that still has it, got:
{text}"
    );
    assert!(
        text.contains("has no install_path"),
        "the bad record is still printed, got:
{text}"
    );
    assert!(
        text.contains("another source"),
        "and says why it cost nothing, got:
{text}"
    );
    // The trailer does not blame an index for a stale record.
    assert!(
        text.contains(
            "1 game in 1 library across 1 install; 1 launcher record could not be used, \
             and nothing is missing because of it"
        ),
        "the trailer has to describe the caveat it actually has, got:
{text}"
    );
    assert!(
        !text.contains("declared no libraries"),
        "and not the one it does not, got:
{text}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "a record that hid nothing costs no exit code, got:
{text}"
    );

    // Nor does `caveats`, the list the trailer is built from.
    let json = dxray_with_home(home.path(), ["installed", "--json"]);
    let stream = stdout_of(&json);

    assert!(
        stream.contains(
            "\"caveats\":[\"1 launcher record could not be used, \
             and nothing is missing because of it\"],"
        ),
        "got:
{stream}"
    );
    assert!(
        stream.contains("\"complete\":true"),
        "everything was read, and the summary has to say so, got:
{stream}"
    );
    assert_eq!(json.status.code(), Some(0), "got:\n{}", stderr_of(&json));
}

#[test]
fn a_machine_with_no_launcher_at_all_says_where_it_looked_under_each_name() {
    // Nothing found exits 1 and lists candidates per launcher.
    let home = TempDir::new("games-nothing");

    let out = dxray_with_home(home.path(), ["installed"]);
    let text = stderr_of(&out);

    assert!(
        text.contains("no game launcher installation found"),
        "got:\n{text}"
    );
    assert!(text.contains("Steam:"), "got:\n{text}");
    assert!(text.contains("Heroic:"), "got:\n{text}");
    assert!(
        text.contains(".config/heroic"),
        "the Heroic candidates are named too, got:\n{text}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "finding nothing is a failure, not an empty success"
    );
}

#[test]
fn the_narrower_flag_still_answers_only_the_narrower_question() {
    // `--launcher steam` shows only Steam, with no launcher label.
    let home = TempDir::new("games-steam-still-narrow");
    fake_steam(&home);
    fake_heroic(&home, true);

    let text = stdout_of(&dxray_with_home(
        home.path(),
        ["installed", "--launcher", "steam"],
    ))
    .to_owned();

    assert!(text.contains("Dota 2"), "got:\n{text}");
    assert!(
        !text.contains("Hades"),
        "the narrower question does not quietly widen, got:\n{text}"
    );
    assert!(
        !text.contains("origin"),
        "and does not repeat the launcher the flag named, got:\n{text}"
    );
    assert!(
        text.contains("1 game in 1 library across 1 install"),
        "got:\n{text}"
    );
}

#[test]
fn the_launcher_filter_takes_any_launcher_and_every_launcher_named() {
    let home = TempDir::new("games-launcher-filter");
    fake_steam(&home);
    fake_heroic(&home, true);

    let heroic = stdout_of(&dxray_with_home(
        home.path(),
        ["installed", "--launcher", "heroic"],
    ))
    .to_owned();
    assert!(heroic.contains("Hades"), "got:\n{heroic}");
    assert!(!heroic.contains("Dota 2"), "got:\n{heroic}");
    assert!(!heroic.contains("origin"), "got:\n{heroic}");

    let both = dxray_with_home(
        home.path(),
        ["installed", "--launcher", "heroic", "--launcher", "steam"],
    );
    let all = dxray_with_home(home.path(), ["installed"]);
    assert_eq!(
        stdout_of(&both),
        stdout_of(&all),
        "naming every launcher, in any order, is the default"
    );
    assert_eq!(both.status.code(), all.status.code());
}

#[test]
fn a_filtered_launcher_that_is_absent_is_named_alone() {
    let home = TempDir::new("games-launcher-absent");
    fake_steam(&home);

    let out = dxray_with_home(home.path(), ["installed", "--launcher", "heroic"]);

    assert_eq!(out.status.code(), Some(1));
    let message = stderr_of(&out);
    assert!(
        message.contains("no Heroic installation found"),
        "got:\n{message}"
    );
    assert!(!message.contains("Steam"), "got:\n{message}");
}

#[test]
fn installed_accepts_json_but_rejects_paths_and_recursive() {
    let home = TempDir::new("games-flags");
    fake_steam(&home);

    for accepted in [vec!["installed"], vec!["installed", "--json"]] {
        let out = dxray_with_home(home.path(), accepted.clone());
        assert_eq!(
            out.status.code(),
            Some(0),
            "{accepted:?} needs no PATH argument, got:\n{}",
            stderr_of(&out)
        );
    }
    for refused in [
        vec!["installed", "steam"],
        vec!["installed", "--recursive"],
        vec!["installed", "game"],
    ] {
        let out = dxray_with_home(home.path(), refused.clone());
        assert_eq!(
            out.status.code(),
            Some(2),
            "{refused:?} is a usage error, told apart from a failed scan"
        );
    }
}

/// A game, a tool, a stale Heroic record, an unreadable manifest and a
/// truncated install, together, so none can hide another.
fn crowded(home: &TempDir) -> (PathBuf, PathBuf) {
    let steam = fake_steam_tool(home);
    // A manifest that stops mid-record: a game the user owns and this tool
    // cannot read.
    std::fs::write(
        steam.join("steamapps/appmanifest_999.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\"999\"\n\t\"name\"\t\"Trunc",
    )
    .expect("a truncated manifest beside the good ones");
    // Past the depth bound, from the constant.
    let mut buried = steam.join("steamapps/common/Proton - Experimental");
    for _ in 0..=dxray_core::install::MAX_DEPTH {
        buried.push("down");
    }
    std::fs::create_dir_all(&buried).expect("a tree deeper than the walk goes");
    std::fs::write(buried.join("proton.exe"), Image::x64().build()).expect("buried executable");

    let heroic = fake_heroic(home, true);
    std::fs::write(
        heroic.join("store_cache/gog_library.json"),
        "{\"library\":[{\"app_name\":\"1207658691\",\"title\":\"Hades\",\"is_installed\":true}]}",
    )
    .expect("a record that lost its install path");

    (steam, heroic)
}

#[test]
fn the_json_listing_carries_everything_the_terminal_listing_carries() {
    // Every state in the fixture is in the stream.
    let home = TempDir::new("games-json");
    let (steam, heroic) = crowded(&home);

    let human = dxray_with_home(home.path(), ["installed"]);
    let machine = dxray_with_home(home.path(), ["installed", "--json"]);
    let text = stdout_of(&human);
    let json = stdout_of(&machine);

    for line in json.lines() {
        assert!(
            line.starts_with("{\"kind\":\"") && line.ends_with('}'),
            "every line is one tagged object, got: {line}"
        );
    }
    for kind in ["install", "library", "game", "note", "problem", "summary"] {
        assert!(
            json.contains(&format!("{{\"kind\":\"{kind}\",")),
            "the listing holds a {kind} and the stream must too, got:\n{json}"
        );
    }
    // The installs, named exactly, with the key a program filters on beside the
    // label a person reads.
    assert!(
        json.contains(&format!(
            "{{\"kind\":\"install\",\"origin\":\"steam\",\"origin_label\":\"Steam\",\"path\":\"{}\"}}",
            steam.display()
        )),
        "got:\n{json}"
    );
    assert!(
        json.contains(&format!(
            "{{\"kind\":\"install\",\"origin\":\"heroic\",\"origin_label\":\"Heroic\",\"path\":\"{}\"}}",
            heroic.display()
        )),
        "got:\n{json}"
    );
    // The game, the tooling entry, the stale record, the unreadable manifest
    // and the truncation, in that order.
    assert!(
        json.contains("\"name\":\"Dota 2\",\"directory\":"),
        "got:\n{json}"
    );
    assert!(
        json.contains("\"name\":\"Proton Experimental\""),
        "nothing is filtered out of the stream either, got:\n{json}"
    );
    assert!(
        json.contains("\"carries_evidence\":false"),
        "and the entry that argues nothing says so as data, got:\n{json}"
    );
    assert!(
        json.contains("has no install_path") && json.contains("\"kind\":\"note\""),
        "the stale Heroic record is a note, got:\n{json}"
    );
    assert!(
        json.contains("appmanifest_999.acf") && json.contains("\"label\":\"unreadable\""),
        "the manifest nobody could read is a problem, got:\n{json}"
    );
    assert!(
        json.contains("\"incomplete\":[\"") && json.contains("Proton - Experimental"),
        "and the truncation rides on the install it qualifies, got:\n{json}"
    );
    // The sentence under the human listing, verbatim, in the summary.
    let trailer = text
        .lines()
        .last()
        .expect("the human run ends in a trailer");
    assert!(
        json.contains(&format!("\"says\":\"{trailer}\"")),
        "the summary must be the trailer a person read: {trailer:?}\n{json}"
    );
    assert!(json.contains("\"complete\":false"), "got:\n{json}");
}

#[test]
fn asking_for_json_changes_the_rendering_and_nothing_a_run_calls_a_failure() {
    // `--json` never changes the exit code, even when nothing is found.
    let crowded_home = TempDir::new("games-json-status");
    crowded(&crowded_home);
    let clean_home = TempDir::new("games-json-clean");
    fake_steam(&clean_home);
    let empty_home = TempDir::new("games-json-empty");

    for (home, expected) in [
        (&crowded_home, Some(1)),
        (&clean_home, Some(0)),
        (&empty_home, Some(1)),
    ] {
        let human = dxray_with_home(home.path(), ["installed"]);
        let machine = dxray_with_home(home.path(), ["installed", "--json"]);

        assert_eq!(human.status.code(), expected);
        assert_eq!(
            machine.status.code(),
            expected,
            "--json moved the exit code, got:\n{}",
            stderr_of(&machine)
        );
        assert_eq!(
            stderr_of(&human),
            stderr_of(&machine),
            "and everything that moves it is still echoed the same way"
        );
    }
}

#[test]
fn a_machine_with_no_launcher_answers_json_with_a_sentence_rather_than_nothing() {
    // Nothing found is one object, not an empty stream.
    let home = TempDir::new("games-json-nothing");

    let out = dxray_with_home(home.path(), ["installed", "--json"]);
    let json = stdout_of(&out);

    assert_eq!(json.lines().count(), 1, "got:\n{json}");
    assert!(json.starts_with("{\"kind\":\"problem\","), "got:\n{json}");
    assert!(
        json.contains("no game launcher installation found"),
        "got:\n{json}"
    );
    assert!(
        !json.contains("\"kind\":\"summary\""),
        "counting nothing as a complete scan is the lie this refuses, got:\n{json}"
    );
    assert_eq!(out.status.code(), Some(1), "got:\n{}", stderr_of(&out));
}

#[test]
fn a_path_given_alongside_games_is_a_usage_error_rather_than_a_silently_ignored_argument() {
    // Explicit paths are not part of the installed command's argument schema.
    let home = TempDir::new("games-path");
    fake_steam(&home);
    let file = home.path().join("image.exe");
    std::fs::write(&file, Image::x64().build()).expect("a file to be ignored");

    let out = dxray_with_home(home.path(), [Path::new("installed"), file.as_path()]);

    assert_eq!(out.status.code(), Some(2), "got:\n{}", stderr_of(&out));
}

#[test]
fn the_help_names_the_flag_and_what_its_exit_codes_mean() {
    // `--help` states the exit codes of every mode.
    let out = dxray_with_home(TempDir::new("games-help").path(), ["--help"]);
    let text = stdout_of(&out);

    assert!(text.contains("installed"), "got:\n{text}");
    assert!(
        text.contains("installed found no install"),
        "the 1 has to cover every launcher filter, got:\n{text}"
    );
}

#[test]
fn the_demoted_half_of_one_library_does_not_sink_past_the_next_library() {
    // Grouping happens within each library.
    let home = TempDir::new("games-order");
    let root = fake_steam_tool(&home);
    let heroic = fake_heroic(&home, true);
    // Appid 100 sorts before Dota's 570, so Steam declares the runtime first
    // and only the ordering here can put the game above it.
    std::fs::create_dir_all(root.join("steamapps/common/Steamworks Shared")).expect("tree");
    std::fs::write(
        root.join("steamapps/appmanifest_100.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\t\"100\"\n\t\"name\"\t\t\"Steamworks Common Redistributables\"\n\
         \t\"installdir\"\t\t\"Steamworks Shared\"\n}\n",
    )
    .expect("manifest");
    std::fs::write(
        root.join("steamapps/common/Steamworks Shared/installscript.exe"),
        Image::x64().build(),
    )
    .expect("executable");

    let out = dxray_with_home(home.path(), ["installed"]);
    let text = stdout_of(&out);

    let game = text.find("dota 2 beta").expect("the Steam game is listed");
    let shared = text
        .find("Steamworks Shared")
        .expect("and so is the runtime; nothing is filtered");
    let library = text
        .find(&heroic.display().to_string())
        .expect("the Heroic library heads its own block");
    assert!(game < shared, "the evidence goes first, got:\n{text}");
    assert!(
        shared < library,
        "but only within its own library, got:\n{text}"
    );
    assert!(
        text.contains("4 games in 2 libraries across 2 installs"),
        "and every row is still counted, got:\n{text}"
    );
    assert_eq!(out.status.code(), Some(0), "got:\n{}", stderr_of(&out));
}

#[test]
fn a_failure_in_a_launchers_own_index_names_no_library_in_the_json() {
    // A failed launcher index touches no library, so `library` is `null`,
    // not the install path.
    let home = TempDir::new("games-broken-steam-json");
    let steam = fake_steam(&home);
    fake_heroic(&home, true);
    std::fs::write(
        steam.join("steamapps/libraryfolders.vdf"),
        "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\"/mnt/g\"\n",
    )
    .expect("truncate the index");

    let out = dxray_with_home(home.path(), ["installed", "--json"]);
    let json = stdout_of(&out);

    let failure = json
        .lines()
        .find(|line| {
            line.starts_with("{\"kind\":\"problem\",") && line.contains("libraryfolders.vdf")
        })
        .unwrap_or_else(|| panic!("the broken index is a problem object, got:\n{json}"));
    assert!(
        failure.contains(&format!(
            "\"install\":\"{}\",\"library\":null,",
            steam.display()
        )),
        "the install is named and no library is invented, got: {failure}"
    );
    assert!(
        failure.contains("\"label\":\"error\""),
        "and it is the launcher's index rather than one record inside a library, got: {failure}"
    );
    assert!(
        json.contains("\"name\":\"Hades\""),
        "the working launcher still reaches the stream, got:\n{json}"
    );
    assert!(json.contains("\"complete\":false"), "got:\n{json}");
    assert_eq!(
        out.status.code(),
        Some(1),
        "an incomplete scan is not a clean one, got:\n{}",
        stderr_of(&out)
    );
}
