//! The trailer agrees with the exit code, every row names its launcher, and
//! one implementation serves both flags.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use dxray_core::{Game, Identity, Launcher};

use super::{
    Cause, Listing, compact_install_path, nothing_found, nothing_found_json, render_games, scan,
};

#[test]
fn compact_paths_use_only_the_containing_library_as_context() {
    let library = Path::new("/very/long/steam/library/on/another/disk");
    let inside = library.join("steamapps/common/Dota 2");
    assert_eq!(
        compact_install_path(&inside, library),
        "./steamapps/common/Dota 2"
    );
    assert_eq!(compact_install_path(library, library), ".");

    let sibling = Path::new("/very/long/steam/library/on/another/disk-2/Dota 2");
    assert_eq!(
        compact_install_path(sibling, library),
        sibling.display().to_string()
    );

    let outside = Path::new("/Games/Hades");
    assert_eq!(
        compact_install_path(outside, library),
        outside.display().to_string()
    );

    let escaping = Path::new("/very/long/steam/library/on/another/disk/../elsewhere");
    assert_eq!(
        compact_install_path(escaping, library),
        escaping.display().to_string()
    );
    assert_eq!(
        compact_install_path(&inside, Path::new("relative/library")),
        inside.display().to_string()
    );
}

fn listing(games: usize, problems: usize) -> Listing {
    Listing {
        problems: (0..problems).map(|i| format!("problem {i}")).collect(),
        roots: 1,
        libraries: 1,
        games,
        ..Listing::new(false)
    }
}

/// A listing whose only caveat is `notes` indexes that declared no libraries.
fn with_notes(games: usize, notes: usize) -> Listing {
    Listing {
        notes: vec![Cause::Index; notes],
        ..listing(games, 0)
    }
}

/// The other cause: `notes` launcher records that could not be used and cost
/// this listing nothing.
fn with_records(games: usize, notes: usize) -> Listing {
    Listing {
        notes: vec![Cause::Record; notes],
        ..listing(games, 0)
    }
}

fn with_incomplete(games: usize, directories: usize) -> Listing {
    Listing {
        incomplete: (0..directories).map(|i| vec![format!("dir {i}")]).collect(),
        ..listing(games, 0)
    }
}

#[test]
fn the_trailer_says_when_the_count_it_prints_is_incomplete() {
    // The caveat sits in the same line as the number it qualifies.
    assert_eq!(
        listing(41, 2).trailer(),
        "41 games in 1 library across 1 install; \
         2 files could not be read, so the count is incomplete"
    );
}

#[test]
fn a_clean_scan_gets_a_plain_count_with_no_hedging_on_it() {
    // The other half of the same promise: if nothing failed, nothing in
    // the line should suggest otherwise.
    assert_eq!(
        listing(41, 0).trailer(),
        "41 games in 1 library across 1 install"
    );
}

#[test]
fn the_trailer_says_when_the_library_count_itself_may_be_short() {
    // An index that declared nothing may hide libraries: said beside the count.
    assert_eq!(
        with_notes(3, 1).trailer(),
        "3 games in 1 library across 1 install; \
         1 index declared no libraries, so there may be more"
    );
}

#[test]
fn a_record_that_could_not_be_used_is_not_reported_as_an_index_that_declared_nothing() {
    // A stale catalogue record is not an index that declared no libraries;
    // naming the wrong file sends the reader to one that is fine.
    assert_eq!(
        with_records(3, 1).trailer(),
        "3 games in 1 library across 1 install; \
         1 launcher record could not be used, and nothing is missing because of it"
    );
    assert!(
        !with_records(3, 1).trailer().contains("index"),
        "and it must not borrow the other note's noun"
    );
    assert!(
        with_records(3, 2).trailer().contains(
            "2 launcher records could not be used, and nothing is missing because of them"
        ),
        "and it counts in the plural too"
    );
}

#[test]
fn the_two_kinds_of_note_are_counted_apart_rather_than_added_together() {
    // One clause per cause: the two doubts are different sizes.
    let mut mixed = with_notes(3, 1);
    mixed.notes.push(Cause::Record);
    mixed.notes.push(Cause::Record);

    let trailer = mixed.trailer();

    assert!(
        trailer.contains("1 index declared no libraries, so there may be more"),
        "{trailer}"
    );
    assert!(
        trailer.contains("2 launcher records could not be used"),
        "{trailer}"
    );
    // And neither of them is a failure: three notes, one clean scan.
    assert!(!mixed.incomplete_scan(), "{trailer}");
}

#[test]
fn a_scan_with_both_a_failure_and_a_caveat_reports_both() {
    // Unreadable and incomplete are different, and both are named.
    let mut listing = listing(3, 1);
    listing.notes.push(Cause::Index);

    let trailer = listing.trailer();

    assert!(trailer.contains("1 file could not be read"), "{trailer}");
    assert!(
        trailer.contains("1 index declared no libraries"),
        "{trailer}"
    );
}

#[test]
fn the_trailer_says_when_a_game_directory_was_not_searched_in_full() {
    // A truncated walk may have missed the best executable.
    assert_eq!(
        with_incomplete(3, 1).trailer(),
        "3 games in 1 library across 1 install; \
         1 game directory could not be searched in full, so the executable named \
         for it may not be the right one"
    );
    assert!(
        with_incomplete(3, 2)
            .trailer()
            .contains("2 game directories could not be searched in full"),
        "and it counts in the plural too"
    );
}

#[test]
fn every_caveat_that_admits_something_was_missed_also_moves_the_exit_code() {
    // The trailer and the exit status tell one story; asserted both ways, since
    // one direction passes on a build that always fails.
    assert!(
        with_incomplete(3, 1).incomplete_scan(),
        "a truncated walk is a run that did not read everything"
    );
    assert!(
        listing(3, 1).incomplete_scan(),
        "so is a file that could not be read"
    );

    // A caveat that claims nothing was missed does not move it.
    assert!(
        !with_notes(3, 1).incomplete_scan(),
        "an index with no entries is a caveat, not a failed scan"
    );
    assert!(
        !with_records(3, 1).incomplete_scan(),
        "and neither is a record that could not be used and cost nothing"
    );
    assert!(!listing(3, 0).incomplete_scan(), "and a clean run is clean");

    // The pairing itself, rather than the two halves separately: if the
    // trailer hedges about something being missing, the status has to agree.
    for candidate in [
        listing(3, 1),
        with_incomplete(3, 1),
        with_notes(3, 1),
        with_records(3, 1),
    ] {
        let trailer = candidate.trailer();
        let admits_a_gap =
            trailer.contains("could not be read") || trailer.contains("could not be searched");
        assert_eq!(
            admits_a_gap,
            candidate.incomplete_scan(),
            "the trailer and the exit code disagree about {trailer:?}"
        );
    }
}

#[test]
fn a_note_is_wrapped_so_the_whole_sentence_can_be_read() {
    // Three lines of prose running off the right edge of a terminal is a
    // caveat nobody finishes reading, which defeats printing it at all.
    let mut out = String::new();
    super::row(
        &mut out,
        "note",
        "/home/user/.steam/steam/steamapps/libraryfolders.vdf declared no library \
         entries; only the Steam root is being scanned. On an old single-library \
         install this is normal; on a newer one it means the index lost its entries.",
    );

    assert!(
        out.lines().count() > 1,
        "a sentence this long has to wrap, got:\n{out}"
    );
    assert!(
        out.lines().skip(1).all(|l| l.starts_with("              ")),
        "continuations line up under the value, not the label, got:\n{out}"
    );
    assert!(
        out.lines().all(|l| l.len() <= 80),
        "no line may run past the wrap width, got:\n{out}"
    );
}

#[test]
fn the_trailer_counts_one_of_something_in_the_singular() {
    // "1 games in 1 libraries" reads as a bug in a tool whose whole claim
    // is that it is careful about what it says.
    assert_eq!(
        listing(1, 1).trailer(),
        "1 game in 1 library across 1 install; \
         1 file could not be read, so the count is incomplete"
    );
}

#[test]
fn a_launcher_that_is_installed_and_holds_nothing_is_counted_rather_than_hidden() {
    // An empty installation is one install with one library, not an absence.
    let empty = Listing {
        roots: 1,
        libraries: 1,
        games: 0,
        ..Listing::new(true)
    };

    assert_eq!(empty.trailer(), "0 games in 1 library across 1 install");
    assert!(
        !empty.incomplete_scan(),
        "owning no games is not a failed scan"
    );
}

#[test]
fn a_library_with_nothing_in_it_says_so_rather_than_printing_a_blank() {
    // An empty run of lines under a library header reads as truncated
    // output. A library that really has no games is a fact worth stating.
    let mut out = String::new();
    render(&mut out, &[]);

    assert!(out.contains("no games installed"), "got {out:?}");
}

/// Renders `games` against a library not on this disk: no prefix, so no
/// NVAPI answer.
fn render(out: &mut String, games: &[Game]) {
    render_named(out, games, false);
}

fn render_named(out: &mut String, games: &[Game], name_origins: bool) {
    out.push_str(&render_both(games, name_origins).text);
}

/// Both renderings of the same games, from the one call that produces them,
/// so tests can assert they agree.
fn render_both(games: &[Game], name_origins: bool) -> super::Rendered {
    render_view(games, name_origins, crate::report::Presentation::Standard)
}

fn render_view(
    games: &[Game],
    name_origins: bool,
    view: crate::report::Presentation,
) -> super::Rendered {
    let mut rendered = super::Rendered::default();
    render_games(
        &mut super::Sink {
            text: &mut rendered.text,
            json: &mut rendered.json,
            incomplete: &mut Vec::new(),
        },
        games,
        super::Place {
            install: Some(Path::new("/not/an/install")),
            library: Path::new("/not/a/library"),
        },
        &mut dxray_core::proton::Builds::default(),
        name_origins,
        view,
    );
    rendered
}

#[test]
fn every_view_prints_the_nvapi_answer_of_a_game_that_could_not_be_read() {
    for (name, view) in [
        ("compact", crate::report::Presentation::Compact),
        ("standard", crate::report::Presentation::Standard),
        ("full", crate::report::Presentation::Full),
    ] {
        let text = render_view(&[dota()], false, view).text;
        assert!(text.contains("NVAPI"), "{name} dropped it, got:\n{text}");
    }
}

#[test]
fn inventory_presentations_leave_json_bytes_unchanged() {
    let tree = Tree::new("view-json-bytes");
    let games = vec![tree.install(570, "entry", "Entry", "Entry.exe")];
    for origins in [false, true] {
        let legacy = render_both(&games, origins);
        for view in [
            crate::report::Presentation::Compact,
            crate::report::Presentation::Full,
        ] {
            let rendered = render_view(&games, origins, view);
            assert_eq!(rendered.json.as_bytes(), legacy.json.as_bytes());
            assert_ne!(rendered.text, legacy.text);
        }
    }
}

#[test]
fn a_game_is_printed_with_its_id_its_title_and_the_directory_it_lives_in() {
    // The id is what the next stage takes, the title is what a person
    // searches for, and the path is what makes the other two checkable.
    let mut out = String::new();
    render(&mut out, &[dota()]);

    assert!(out.contains("570  Dota 2  ["), "got {out:?}");
    assert!(
        out.contains("/steam/steamapps/common/dota 2 beta"),
        "got {out:?}"
    );
}

fn dota() -> Game {
    Game {
        identity: Identity::SteamApp(570),
        name: "Dota 2".to_owned(),
        install_dir: PathBuf::from("/steam/steamapps/common/dota 2 beta"),
        origin: dxray_core::steam::ORIGIN,
    }
}

fn hades() -> Game {
    Game {
        identity: Identity::Native("hades-gog".to_owned()),
        name: "Hades".to_owned(),
        install_dir: PathBuf::from("/games/Hades"),
        origin: dxray_core::heroic::Store::Gog.origin(),
    }
}

#[test]
fn a_game_says_which_launcher_supplied_it_when_more_than_one_could_have() {
    // Every row names the launcher its game came from.
    let mut out = String::new();
    render_named(&mut out, &[dota(), hades()], true);

    assert!(
        !out.contains("Source: Steam"),
        "the Steam block already establishes the source, got {out:?}"
    );
    assert!(
        out.contains("Source: Heroic / GOG"),
        "and the Heroic game names its backend, not just Heroic: {out:?}"
    );
}

#[test]
fn the_narrower_listing_does_not_repeat_the_one_launcher_it_was_asked_about() {
    // One launcher asked about needs no launcher label.
    let mut out = String::new();
    render(&mut out, &[dota()]);

    assert!(
        !out.contains("Source:"),
        "a single-launcher listing names no origins, got {out:?}"
    );
}

#[test]
fn a_game_with_no_proton_prefix_still_gets_a_row_saying_so() {
    // The NVAPI row is printed whether or not there is an answer.
    let mut out = String::new();
    render(&mut out, &[dota()]);

    assert!(out.contains("NVAPI:"), "got {out:?}");
    assert!(
        out.contains("not determined"),
        "the row must not read as an answer, got {out:?}"
    );
    assert!(
        out.contains("NVAPI: not determined"),
        "and it must retain the refusal, got {out:?}"
    );
}

#[test]
fn a_heroic_game_whose_settings_cannot_be_read_has_an_undetermined_nvapi_row() {
    // Its NVAPI comes from Heroic's settings, which this install does not have.
    let mut out = String::new();
    render_named(&mut out, &[hades()], true);

    let unwrapped = out.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(unwrapped.contains("NVAPI: not determined"), "got {out:?}");
    assert!(unwrapped.contains("Heroic / GOG"), "got {out:?}");
}

#[test]
fn a_game_with_no_nvapi_answer_does_not_move_the_exit_code() {
    // A game never launched under Proton is not a failed scan.
    let mut listing = listing(1, 0);
    render(&mut listing.text, &[dota()]);

    assert!(listing.text.contains("not determined"), "the row is there");
    assert!(
        !listing.incomplete_scan(),
        "and it costs nothing: {}",
        listing.trailer()
    );
}

#[test]
fn finding_nothing_names_the_places_that_were_searched() {
    // The message names where it looked, so it can be acted on.
    let message = nothing_found(&[&dxray_core::steam::STEAM]);

    assert!(message.contains("no Steam installation found"), "{message}");
    assert!(
        message.lines().count() > 2,
        "the candidate paths must be listed, got {message}"
    );
}

#[test]
fn finding_nothing_across_every_launcher_says_which_paths_belonged_to_which() {
    // Grouped by launcher when more than one was asked about.
    let message = nothing_found(dxray_core::launcher::all());

    assert!(
        message.contains("no game launcher installation found"),
        "one launcher is not named when several were asked about: {message}"
    );
    for label in ["Steam:", "Heroic:"] {
        assert!(
            message.contains(label),
            "every launcher heads its own candidates, got {message}"
        );
    }
    assert!(
        message.contains("If a launcher is installed somewhere else"),
        "and the closing sentence must not name one of them either: {message}"
    );
}

/// A launcher whose machine is supplied, so a test can build the states no real
/// machine can be relied upon to have. Installed nowhere unless given a root.
struct Fake {
    origin: dxray_core::Origin,
    roots: Vec<PathBuf>,
    games: Vec<Game>,
    /// Notes raised by one record inside a library, which is what a
    /// [`Catalogue`](dxray_core::Catalogue) carries.
    notes: Vec<String>,
    /// Notes from the index of libraries, delivered by their own callback and
    /// worded apart in the trailer.
    index_notes: Vec<String>,
    index_problems: Vec<String>,
    problems: Vec<String>,
}

impl Fake {
    fn new(key: &'static str) -> Self {
        Self {
            origin: dxray_core::Origin::new(key, key),
            roots: Vec::new(),
            games: Vec::new(),
            notes: Vec::new(),
            index_notes: Vec::new(),
            index_problems: Vec::new(),
            problems: Vec::new(),
        }
    }
}

impl Launcher for Fake {
    fn origin(&self) -> dxray_core::Origin {
        self.origin
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }

    fn candidate_roots(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("/dxray-cli-nowhere")]
    }

    fn libraries(&self, root: &Path) -> dxray_core::Libraries {
        dxray_core::Libraries {
            paths: vec![root.to_path_buf()],
            notes: self.index_notes.clone(),
            problems: self.index_problems.clone(),
        }
    }

    fn games(&self, _library: &Path) -> dxray_core::Catalogue {
        dxray_core::Catalogue {
            games: self.games.clone(),
            problems: self.problems.clone(),
            notes: self.notes.clone(),
        }
    }
}

fn inventory_json_rows(launchers: &[&dyn Launcher]) -> Vec<serde_json::Value> {
    scan(launchers)
        .json()
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSONL row"))
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
struct JsonDiagnostic {
    kind: String,
    origin: String,
    origin_label: String,
    install: Option<String>,
    library: Option<String>,
    cause: Option<String>,
    label: String,
    says: String,
}

fn expected_diagnostics(
    diagnostics: &[dxray_core::InventoryDiagnostic],
    kind: &str,
) -> Vec<JsonDiagnostic> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let is_catalogue = diagnostic.library.is_some();
            JsonDiagnostic {
                kind: kind.to_owned(),
                origin: diagnostic.origin.key().to_owned(),
                origin_label: diagnostic.origin.label().to_owned(),
                install: diagnostic
                    .root
                    .as_ref()
                    .map(|path| path.display().to_string()),
                library: diagnostic
                    .library
                    .as_ref()
                    .map(|path| path.display().to_string()),
                cause: match kind {
                    "note" if is_catalogue => Some("record".to_owned()),
                    "note" => Some("index".to_owned()),
                    _ => None,
                },
                label: match (kind, is_catalogue) {
                    ("problem", true) => "unreadable",
                    ("problem", false) => "error",
                    _ => "note",
                }
                .to_owned(),
                says: diagnostic.message.clone(),
            }
        })
        .collect()
}

fn json_diagnostics(rows: &[serde_json::Value], kind: &str) -> Vec<JsonDiagnostic> {
    rows.iter()
        .filter(|row| row["kind"] == kind)
        .map(|row| JsonDiagnostic {
            kind: row["kind"].as_str().expect("kind").to_owned(),
            origin: row["origin"].as_str().expect("origin").to_owned(),
            origin_label: row["origin_label"]
                .as_str()
                .expect("origin label")
                .to_owned(),
            install: row["install"].as_str().map(ToOwned::to_owned),
            library: row["library"].as_str().map(ToOwned::to_owned),
            cause: row["cause"].as_str().map(ToOwned::to_owned),
            label: row["label"].as_str().expect("label").to_owned(),
            says: row["says"].as_str().expect("diagnostic").to_owned(),
        })
        .collect()
}

fn assert_json_matches_inventory(rows: &[serde_json::Value], inventory: &dxray_core::Inventory) {
    let expected_entries: std::collections::HashSet<_> = inventory
        .entries
        .iter()
        .map(|entry| {
            (
                entry.game.identity.to_string(),
                entry.game.origin.key().to_owned(),
                entry.game.install_dir.display().to_string(),
                entry.library.display().to_string(),
            )
        })
        .collect();
    let expected_roots: std::collections::HashSet<_> = inventory
        .roots
        .iter()
        .map(|root| {
            (
                root.origin.key().to_owned(),
                root.path.display().to_string(),
            )
        })
        .collect();
    let expected_libraries: std::collections::HashSet<_> = inventory
        .libraries
        .iter()
        .map(|library| {
            (
                library.origin.key().to_owned(),
                library.root.as_ref().map(|path| path.display().to_string()),
                library.path.display().to_string(),
            )
        })
        .collect();
    let actual_entries: std::collections::HashSet<_> = rows
        .iter()
        .filter(|row| row["kind"] == "game")
        .map(|row| {
            (
                row["id"].as_str().expect("id").to_owned(),
                row["origin"].as_str().expect("origin").to_owned(),
                row["directory"].as_str().expect("directory").to_owned(),
                row["library"].as_str().expect("library").to_owned(),
            )
        })
        .collect();
    let actual_roots: std::collections::HashSet<_> = rows
        .iter()
        .filter(|row| row["kind"] == "install")
        .map(|row| {
            (
                row["origin"].as_str().expect("origin").to_owned(),
                row["path"].as_str().expect("path").to_owned(),
            )
        })
        .collect();
    let actual_libraries: std::collections::HashSet<_> = rows
        .iter()
        .filter(|row| row["kind"] == "library")
        .map(|row| {
            (
                row["origin"].as_str().expect("origin").to_owned(),
                row["install"].as_str().map(ToOwned::to_owned),
                row["path"].as_str().expect("path").to_owned(),
            )
        })
        .collect();

    assert_eq!(actual_entries, expected_entries);
    assert_eq!(actual_roots, expected_roots);
    assert_eq!(actual_libraries, expected_libraries);
    assert_eq!(
        json_diagnostics(rows, "note"),
        expected_diagnostics(&inventory.notes, "note")
    );
    assert_eq!(
        json_diagnostics(rows, "problem"),
        expected_diagnostics(&inventory.problems, "problem")
    );
}

#[test]
fn listing_json_preserves_the_core_inventory_contract() {
    let steam_library = PathBuf::from("/dxray-cli-contract/steam");
    let heroic_library = PathBuf::from("/dxray-cli-contract/heroic");
    let steam = Fake {
        origin: dxray_core::steam::ORIGIN,
        roots: vec![steam_library.clone()],
        games: vec![Game {
            identity: Identity::SteamApp(570),
            name: "Dota 2".to_owned(),
            install_dir: steam_library.join("steamapps/common/dota 2 beta"),
            origin: dxray_core::steam::ORIGIN,
        }],
        index_notes: vec!["steam index caveat".to_owned()],
        index_problems: vec!["steam index problem".to_owned()],
        notes: vec![
            "steam catalogue caveat".to_owned(),
            "steam catalogue caveat".to_owned(),
        ],
        ..Fake::new("steam")
    };
    let heroic = Fake {
        origin: dxray_core::heroic::ORIGIN,
        roots: vec![heroic_library.clone()],
        games: vec![Game {
            identity: Identity::Native("heroic-hades".to_owned()),
            name: "Hades".to_owned(),
            install_dir: PathBuf::from("/games/Hades"),
            origin: dxray_core::Origin::new("heroic", "Heroic / GOG"),
        }],
        problems: vec![
            "heroic catalogue problem".to_owned(),
            "heroic catalogue problem".to_owned(),
        ],
        ..Fake::new("heroic")
    };
    let launchers: [&dyn Launcher; 2] = [&steam, &heroic];

    let inventory = dxray_core::Inventory::collect(&launchers);
    let rows = inventory_json_rows(&launchers);
    assert_json_matches_inventory(&rows, &inventory);
}

#[test]
fn listing_json_matches_the_shared_steam_and_heroic_fixture() {
    let fixture = dxray_core::test_support::SteamHeroicFixture::new();
    let launchers = fixture.launchers();
    let inventory = fixture.inventory();

    assert!(
        inventory
            .entries
            .iter()
            .any(|entry| matches!(entry.game.identity, Identity::SteamApp(570)))
    );
    assert!(inventory.entries.iter().any(|entry| {
        entry.game.identity == Identity::Native("Hades".to_owned())
            && entry.game.origin == dxray_core::heroic::Store::Gog.origin()
    }));
    assert!(inventory.notes.iter().any(|diagnostic| {
        diagnostic.origin == dxray_core::heroic::ORIGIN && diagnostic.library.is_some()
    }));
    assert!(inventory.problems.iter().any(|diagnostic| {
        diagnostic.origin == dxray_core::steam::ORIGIN && diagnostic.library.is_some()
    }));

    assert_json_matches_inventory(&inventory_json_rows(&launchers), &inventory);
}

#[test]
fn a_library_nobody_could_look_inside_is_not_reported_as_an_empty_one() {
    // A library that could not be listed is not a library with no games.
    let broken = Fake {
        roots: vec![PathBuf::from("/dxray-cli-broken")],
        problems: vec!["/dxray-cli-broken/steamapps: No such file or directory".to_owned()],
        ..Fake::new("broken")
    };

    let listing = scan(&[&broken]);

    assert!(
        !listing.text.contains("no games installed"),
        "got {:?}",
        listing.text
    );
    assert!(
        listing.text.contains("unreadable"),
        "the row says what happened instead, got {:?}",
        listing.text
    );
    assert!(
        listing.incomplete_scan(),
        "and the status agrees with it: {}",
        listing.trailer()
    );
}

#[test]
fn a_scan_of_no_launchers_at_all_counts_nothing_and_blames_nobody() {
    // Nothing found is an exit code and a message, not an empty listing.
    let absent = Fake::new("absent");
    let listing = scan(&[&absent]);

    assert_eq!(listing.roots, 0, "nothing was found to walk");
    assert_eq!(listing.games, 0);
    assert!(listing.text.is_empty(), "got {:?}", listing.text);
    assert!(
        !listing.incomplete_scan(),
        "nothing failed to read; there was nothing to read"
    );
}

#[test]
fn the_listing_asks_the_launchers_it_was_given_and_names_no_others() {
    // The flag chooses the launcher set and nothing else.
    let one = Fake::new("first");
    let two = Fake::new("second");

    assert!(
        !scan(&[&one]).name_origins,
        "asked about one launcher, the listing does not label every row with it"
    );
    assert!(
        scan(&[&one, &two]).name_origins,
        "asked about two, it has to say which is which"
    );
}

/// A directory tree that removes itself, for games that really exist.
struct Tree {
    path: PathBuf,
}

impl Tree {
    fn new(tag: &str) -> Self {
        // Process id and a counter: tests run in parallel.
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("dxray-listing-{}-{tag}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }

    /// An install directory holding one executable, returned as a game. Two
    /// bytes suffice: only the name decides the grouping.
    fn install(&self, appid: u32, dir: &str, title: &str, exe: &str) -> Game {
        let install_dir = self.path.join(dir);
        std::fs::create_dir_all(&install_dir).expect("install dir");
        std::fs::write(install_dir.join(exe), b"MZ").expect("executable");
        Game {
            identity: Identity::SteamApp(appid),
            name: title.to_owned(),
            install_dir,
            origin: dxray_core::steam::ORIGIN,
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn an_install_that_argues_nothing_is_printed_under_the_ones_that_argue_something() {
    // Proton builds and runtimes are listed after the games, never hidden.
    let tree = Tree::new("order");
    let runtime = tree.install(
        100,
        "Proton - Experimental",
        "Proton Experimental",
        "filelock.exe",
    );
    let game = tree.install(570, "dota 2 beta", "Dota 2", "Dota 2.exe");
    let mut out = String::new();

    // Declared runtime first, which is the order a launcher sorting by
    // application id hands them over in.
    render(&mut out, &[runtime, game]);

    let runtime_at = out.find("Proton Experimental").expect("both are listed");
    let game_at = out.find("Dota 2  [").expect("both are listed");
    assert!(game_at < runtime_at, "the evidence goes first, got:\n{out}");
    assert!(
        out.contains("nothing here carries evidence of being a game"),
        "and the demoted row says why it is down there, got:\n{out}"
    );
}

#[test]
fn an_install_that_argues_nothing_is_demoted_on_both_surfaces_or_neither() {
    // One answer from `dxray-core`, spent on both renderings and on the JSON
    // field.
    let tree = Tree::new("order-json");
    let runtime = tree.install(
        100,
        "Proton - Experimental",
        "Proton Experimental",
        "filelock.exe",
    );
    let game = tree.install(570, "dota 2 beta", "Dota 2", "Dota 2.exe");

    let rendered = render_both(&[runtime, game], false);

    let text_game = rendered.text.find("Dota 2  [").expect("both are listed");
    let text_runtime = rendered
        .text
        .find("Proton Experimental")
        .expect("both are listed");
    let json_game = rendered
        .json
        .find(r#""name":"Dota 2""#)
        .expect("both are listed");
    let json_runtime = rendered
        .json
        .find(r#""name":"Proton Experimental""#)
        .expect("both are listed");

    assert!(text_game < text_runtime, "got:\n{}", rendered.text);
    assert_eq!(
        json_game < json_runtime,
        text_game < text_runtime,
        "the two renderings put the same install in different places, got:\n{}",
        rendered.json
    );
    // And the answer itself, so a consumer does not have to infer it from the
    // order it happened to arrive in.
    assert!(
        rendered.json.contains(r#""carries_evidence":false"#),
        "the demoted install says why it is down there, got:\n{}",
        rendered.json
    );
    assert!(
        rendered.json.contains(r#""carries_evidence":true"#),
        "and the game says why it is up there, got:\n{}",
        rendered.json
    );
}

#[test]
fn a_demoted_game_keeps_every_row_it_would_have_had_further_up() {
    // A demoted row keeps its caveats.
    let tree = Tree::new("rows");
    let runtime = tree.install(
        100,
        "Proton - Experimental",
        "Proton Experimental",
        "filelock.exe",
    );
    let mut out = String::new();

    render_named(&mut out, &[runtime], true);

    assert!(!out.contains("Source: Steam"), "got:\n{out}");
    assert!(out.contains("NVAPI: not determined"), "got:\n{out}");
}

#[test]
fn a_directory_that_could_not_be_read_keeps_its_place_among_the_games() {
    // Unread stays with the games: it is not an answer of "nothing".
    let tree = Tree::new("unread");
    let runtime = tree.install(
        100,
        "Proton - Experimental",
        "Proton Experimental",
        "filelock.exe",
    );
    let mut out = String::new();

    render(&mut out, &[runtime, dota()]);

    let missing_at = out.find("Dota 2  [").expect("both are listed");
    let runtime_at = out.find("Proton Experimental").expect("both are listed");
    assert!(
        missing_at < runtime_at,
        "an unanswered question is not an answer of nothing, got:\n{out}"
    );
}

#[test]
fn an_unread_directory_is_null_evidence_in_the_json_rather_than_false() {
    // The same rule in JSON: `null` is unanswered, `false` is "nothing".
    let tree = Tree::new("unread-json");
    let runtime = tree.install(
        100,
        "Proton - Experimental",
        "Proton Experimental",
        "filelock.exe",
    );

    let rendered = render_both(&[runtime, dota()], false);
    let missing_at = rendered
        .json
        .find(r#""name":"Dota 2""#)
        .expect("both are listed");
    let runtime_at = rendered
        .json
        .find(r#""name":"Proton Experimental""#)
        .expect("both are listed");

    assert!(
        missing_at < runtime_at,
        "the unread install keeps its place among the games, got:\n{}",
        rendered.json
    );
    assert!(
        rendered.json[missing_at..].starts_with(
            r#""name":"Dota 2","directory":"/steam/steamapps/common/dota 2 beta","carries_evidence":null"#
        ),
        "and says the question was never put, got:\n{}",
        rendered.json
    );
}

#[test]
fn the_games_of_one_library_keep_the_order_their_launcher_gave_them() {
    // No second sort key. Reordering games against each other would be this
    // listing claiming a ranking between them that no evidence supports.
    let tree = Tree::new("stable");
    let first = tree.install(570, "dota 2 beta", "Dota 2", "Dota 2.exe");
    let second = tree.install(1_145_360, "Hades", "Hades", "Hades.exe");
    let mut out = String::new();

    render(&mut out, &[first, second]);

    let dota_at = out.find("Dota 2  [").expect("both are listed");
    let hades_at = out.find("Hades  [").expect("both are listed");
    assert!(dota_at < hades_at, "got:\n{out}");
}

#[test]
fn a_demoted_game_does_not_lose_the_note_that_moves_the_exit_code() {
    // Caveats are collected before printing, and truncation is charged to
    // every entry: it may be why no evidence was found.
    let tree = Tree::new("deep");
    let runtime = tree.install(
        100,
        "Proton - Experimental",
        "Proton Experimental",
        "filelock.exe",
    );
    let mut under = runtime.install_dir.clone();
    for i in 0..=dxray_core::install::MAX_DEPTH {
        under.push(format!("d{i}"));
    }
    std::fs::create_dir_all(&under).expect("a tree deeper than the walk goes");
    let mut incomplete = Vec::new();
    let mut rendered = super::Rendered::default();

    render_games(
        &mut super::Sink {
            text: &mut rendered.text,
            json: &mut rendered.json,
            incomplete: &mut incomplete,
        },
        &[
            tree.install(570, "dota 2 beta", "Dota 2", "Dota 2.exe"),
            runtime,
        ],
        super::Place {
            install: Some(Path::new("/not/an/install")),
            library: Path::new("/not/a/library"),
        },
        &mut dxray_core::proton::Builds::default(),
        false,
        crate::report::Presentation::Standard,
    );
    let out = rendered.text;

    assert!(
        incomplete
            .iter()
            .flatten()
            .any(|note| note.contains("Proton - Experimental")),
        "the demoted game's truncation is still collected, got {incomplete:?}"
    );
    // Wrapping removed: this is about what the sentence says.
    let unwrapped = out.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        unwrapped.contains("1 directory was not searched"),
        "and printed where the game was printed, got:\n{out}"
    );
    // The caveat also reaches the row it belongs to in JSON.
    assert!(
        rendered.json.contains(r#""incomplete":["#),
        "got:\n{}",
        rendered.json
    );
    assert!(
        rendered
            .json
            .lines()
            .any(|line| line.contains("Proton - Experimental")
                && line.contains(r#""incomplete":[""#)),
        "the truncation rides on the game it qualifies, got:\n{}",
        rendered.json
    );
}

#[test]
fn a_directory_with_several_gaps_is_counted_once() {
    // Measured: one Proton build with six notes read as six directories.
    let tree = Tree::new("gaps");
    let runtime = tree.install(
        100,
        "Proton - Experimental",
        "Proton Experimental",
        "rundll.exe",
    );
    for exe in ["winhelp.exe", "krnl386.exe"] {
        std::fs::write(runtime.install_dir.join(exe), b"MZ").expect("executable");
    }
    let fake = Fake {
        roots: vec![tree.path.clone()],
        games: vec![runtime],
        ..Fake::new("gaps")
    };

    let listing = scan(&[&fake]);

    assert_eq!(
        listing.incomplete.iter().flatten().count(),
        3,
        "every gap is still kept, got {:?}",
        listing.incomplete
    );
    let trailer = listing.trailer();
    assert!(
        trailer.contains("1 game directory could not be searched in full"),
        "got: {trailer}"
    );
}

/// A launcher whose every string is fixed in this file, so that one whole
/// listing can be compared byte for byte.
fn golden_scan() -> Listing {
    let fake = Fake {
        roots: vec![PathBuf::from("/dxray-golden")],
        games: vec![Game {
            identity: Identity::SteamApp(570),
            name: "Dota 2".to_owned(),
            install_dir: PathBuf::from("/dxray-golden/dota 2 beta"),
            origin: dxray_core::steam::ORIGIN,
        }],
        notes: vec!["a cache record has no install_path".to_owned()],
        index_notes: vec!["its index declared no libraries".to_owned()],
        problems: vec!["appmanifest_999.acf: unreadable".to_owned()],
        ..Fake::new("golden")
    };
    scan(&[&fake, &Fake::new("absent")])
}

/// Every byte the terminal listing prints for [`golden_scan`]: any change to
/// human-facing output fails here. The `(os error 2)` wording, and so its wrap
/// point, is this platform's.
const GOLDEN_TEXT: &str = concat!(
    "golden [/dxray-golden]\n",
    "  note        its index declared no libraries\n",
    "  Library: /dxray-golden (1 installation)\n",
    "    └─ 570  Dota 2  [Evidence unavailable]\n",
    "       Path: ./dota 2 beta\n",
    "       Exec: (directory could not be read: No such file or directory (os error 2))\n",
    "       Status: installation could not be read: No such file or directory (os error 2)\n",
    "       NVAPI: not determined\n",
    "  note        a cache record has no install_path\n",
    "  unreadable  appmanifest_999.acf: unreadable\n",
);

/// The same walk as [`GOLDEN_TEXT`], as JSON, line for line. Compared as an
/// exact string: a parser would accept output that is well-formed and wrong.
const GOLDEN_JSON: &str = concat!(
    r#"{"kind":"install","origin":"golden","origin_label":"golden","path":"/dxray-golden"}"#,
    "\n",
    r#"{"kind":"note","origin":"golden","origin_label":"golden","install":"/dxray-golden","library":null,"label":"note","cause":"index","says":"its index declared no libraries"}"#,
    "\n",
    r#"{"kind":"library","origin":"golden","origin_label":"golden","install":"/dxray-golden","path":"/dxray-golden"}"#,
    "\n",
    r#"{"kind":"game","origin":"steam","origin_label":"Steam","install":"/dxray-golden","library":"/dxray-golden","#,
    r#""id":"570","steam_appid":570,"name":"Dota 2","directory":"/dxray-golden/dota 2 beta","carries_evidence":null,"#,
    r#""rows":[{"label":"origin","says":"Steam"},"#,
    r#"{"label":"best","says":"(directory could not be read: No such file or directory (os error 2))"},"#,
    r#"{"label":"nvapi","says":"not determined: /dxray-golden/steamapps/compatdata/570: no compatibility prefix, "#,
    r#"so this game has not been run under Proton and there is no build to read a policy out of"}],"incomplete":[]}"#,
    "\n",
    r#"{"kind":"note","origin":"golden","origin_label":"golden","install":"/dxray-golden","library":"/dxray-golden","label":"note","cause":"record","says":"a cache record has no install_path"}"#,
    "\n",
    r#"{"kind":"problem","origin":"golden","origin_label":"golden","install":"/dxray-golden","library":"/dxray-golden","label":"unreadable","says":"appmanifest_999.acf: unreadable"}"#,
    "\n",
    r#"{"kind":"summary","games":1,"libraries":1,"installs":1,"complete":false,"notes":{"index":1,"record":1},"#,
    r#""caveats":["1 file could not be read, so the count is incomplete","#,
    r#""1 index declared no libraries, so there may be more","#,
    r#""1 launcher record could not be used, and nothing is missing because of it"],"#,
    r#""says":"1 game in 1 library across 1 install; 1 file could not be read, so the count is incomplete; "#,
    r#"1 index declared no libraries, so there may be more; "#,
    r#"1 launcher record could not be used, and nothing is missing because of it"}"#,
    "\n",
);

#[test]
fn one_walk_produces_the_listing_and_the_json_and_neither_is_the_other() {
    // Both renderings come from one pass; a drift fails exactly one assertion.
    let listing = golden_scan();

    assert_eq!(listing.text, GOLDEN_TEXT, "the human listing moved");
    assert_eq!(listing.json(), GOLDEN_JSON, "the JSON listing moved");
}

#[test]
fn a_listing_object_cannot_be_mistaken_for_a_file_record() {
    // Record lines never carry `kind`; listing objects always lead with it.
    let record = crate::record::Record::failed(
        std::path::Path::new("/games/app.exe"),
        &"not a PE image: missing PE signature",
    )
    .to_json();

    assert!(
        !record.contains("\"kind\""),
        "the record line must stay free of the listing's tag, got {record}"
    );
    assert!(record.starts_with("{\"path\":"), "got {record}");
    for line in golden_scan().json().lines() {
        assert!(
            line.starts_with("{\"kind\":\""),
            "every listing object leads with its kind, got {line}"
        );
        assert!(line.ends_with('}'), "and is one object, got {line}");
    }
}

#[test]
fn the_summary_is_the_last_line_and_comes_with_the_rows_or_not_at_all() {
    // The rows cannot be emitted without the summary.
    let json = golden_scan().json();
    let mut lines = json.lines();

    assert!(
        lines
            .next_back()
            .is_some_and(|last| last.starts_with(r#"{"kind":"summary","#)),
        "the summary is last, got:\n{json}"
    );
    assert_eq!(
        json.matches(r#""kind":"summary""#).count(),
        1,
        "one summary for one run, got:\n{json}"
    );
    assert!(json.ends_with("}\n"), "and the stream ends in a whole line");
}

#[test]
fn the_summary_the_trailer_and_the_exit_code_are_one_predicate() {
    // Trailer, summary and exit code agree, asserted as a pair.
    for candidate in [
        listing(3, 0),
        listing(3, 1),
        with_notes(3, 1),
        with_records(3, 1),
        with_incomplete(3, 1),
    ] {
        let json = candidate.json();
        let says_complete = json.contains(r#""complete":true"#);

        assert_eq!(
            says_complete,
            !candidate.incomplete_scan(),
            "the JSON and the exit code disagree about {json}"
        );
        assert!(
            json.contains(&format!("\"says\":\"{}\"", candidate.trailer())),
            "the summary carries the trailer verbatim, got {json}"
        );
        for caveat in candidate.caveats() {
            assert!(
                json.contains(&format!("\"{caveat}\"")),
                "every caveat the trailer hedges with is data too, got {json}"
            );
        }
    }
}

#[test]
fn a_game_object_says_where_it_is_and_what_may_be_asked_about_it() {
    // Each game carries its install and library.
    let rendered = render_both(&[dota()], false);

    assert!(
        rendered.json.contains(concat!(
            r#"{"kind":"game","origin":"steam","origin_label":"Steam","#,
            r#""install":"/not/an/install","library":"/not/a/library","#,
            r#""id":"570","steam_appid":570,"name":"Dota 2","#,
            r#""directory":"/steam/steamapps/common/dota 2 beta","#,
        )),
        "got {}",
        rendered.json
    );
}

#[test]
fn a_game_with_no_steam_appid_gets_a_null_rather_than_a_zero() {
    // Appid zero is real; a native identity must not look like it.
    let rendered = render_both(&[hades()], true);

    assert!(
        rendered
            .json
            .contains(r#""id":"hades-gog","steam_appid":null,"#),
        "got {}",
        rendered.json
    );
    // The key is for machines and never changes; the label is for people and
    // names the shop. Both, because neither is the other's abbreviation.
    assert!(
        rendered
            .json
            .contains(r#""origin":"heroic","origin_label":"Heroic / GOG""#),
        "got {}",
        rendered.json
    );
}

#[test]
fn a_games_rows_reach_the_json_whole_where_the_terminal_had_to_fold_them() {
    // Terminal rows wrap; JSON carries each sentence whole.
    let rendered = render_both(&[dota()], true);
    let sentence = "no compatibility prefix, so this game has not been run under Proton";

    assert!(
        rendered.json.contains(sentence),
        "the JSON carries the whole sentence, got {}",
        rendered.json
    );
    assert!(
        !rendered.text.contains(sentence),
        "the inventory keeps long NVAPI prose in JSON and full view, got:\n{}",
        rendered.text
    );
    for label in ["best", "nvapi"] {
        assert!(
            rendered
                .json
                .contains(&format!("{{\"label\":\"{label}\",\"says\":")),
            "every labelled row a person sees is a row a program sees: {label}"
        );
    }
}

#[test]
fn a_library_with_nothing_in_it_is_a_library_row_with_no_games_under_it() {
    // The tree keeps an installed launcher that holds nothing.
    let empty = Fake {
        roots: vec![PathBuf::from("/dxray-empty")],
        ..Fake::new("empty")
    };

    let listing = scan(&[&empty]);
    let json = listing.json();

    assert!(listing.text.contains("(no games installed)"), "got {json}");
    assert!(
        !json.contains(r#""kind":"game""#),
        "nothing may be invented to stand for the absence, got {json}"
    );
    assert!(
        json.contains(concat!(
            r#"{"kind":"library","origin":"empty","origin_label":"empty","#,
            r#""install":"/dxray-empty","path":"/dxray-empty"}"#
        )),
        "but the library is there to hold the nothing, got {json}"
    );
    assert!(
        json.contains(r#""games":0,"libraries":1,"installs":1,"complete":true"#),
        "and the counts say it in the summary, got {json}"
    );
}

#[test]
fn finding_no_launcher_at_all_is_a_sentence_rather_than_an_empty_stream() {
    // Nothing scanned still yields a line, unlike an empty machine.
    let line = nothing_found_json(&[&dxray_core::steam::STEAM]);

    assert!(line.starts_with(r#"{"kind":"problem","#), "got {line}");
    assert!(!line.contains('\n'), "one object, one line, got {line}");
    assert!(line.contains("no Steam installation found"), "got {line}");
    assert!(
        line.contains("\\n"),
        "the candidate paths come with it, escaped into the sentence: {line}"
    );
}

/// Everything a title or path can hold that JSON cannot. Names come from files
/// this project does not write.
const HOSTILE: &str = "\",\"kind\":\"summary\" \\ \n \t \u{1} \u{1f600}";

/// [`HOSTILE`] escaped per RFC 8259, by hand so the test cannot agree with a
/// broken escaper. The emoji stays raw UTF-8.
const ESCAPED: &str = r#"\",\"kind\":\"summary\" \\ \n \t \u0001 😀"#;

#[test]
fn a_hostile_name_reaches_the_json_escaped_and_cannot_start_a_second_object() {
    // Every listing writer goes through the one escaper.
    let hostile = Fake {
        roots: vec![PathBuf::from(format!("/dxray-hostile{HOSTILE}"))],
        games: vec![Game {
            identity: Identity::Native(format!("id{HOSTILE}")),
            name: format!("Hades{HOSTILE}"),
            install_dir: PathBuf::from(format!("/dxray-hostile{HOSTILE}/Hades")),
            origin: dxray_core::heroic::Store::Gog.origin(),
        }],
        notes: vec![format!("a record{HOSTILE} has no install_path")],
        problems: vec![format!("appmanifest{HOSTILE}.acf: unreadable")],
        ..Fake::new("hostile")
    };

    let json = scan(&[&hostile]).json();

    // Six objects, six lines: an unescaped newline would add a seventh.
    assert_eq!(
        json.lines().count(),
        6,
        "one line per object, whatever the strings hold, got:\n{json}"
    );
    for line in json.lines() {
        assert!(
            line.starts_with("{\"kind\":\"") && line.ends_with('}'),
            "every line is still one whole object, got: {line}"
        );
        // An unescaped quote would let the title write keys of its own.
        assert_eq!(
            line.matches("\"kind\":").count(),
            1,
            "a value wrote a key of its own, got: {line}"
        );
    }
    // And the values arrive whole rather than merely harmless, on each of the
    // writers this surface has.
    assert!(
        json.contains(&format!("\"path\":\"/dxray-hostile{ESCAPED}\"")),
        "the install and library paths, got:\n{json}"
    );
    assert!(
        json.contains(&format!("\"id\":\"id{ESCAPED}\"")),
        "the identity, got:\n{json}"
    );
    assert!(
        json.contains(&format!("\"name\":\"Hades{ESCAPED}\"")),
        "the title, got:\n{json}"
    );
    assert!(
        json.contains(&format!("\"directory\":\"/dxray-hostile{ESCAPED}/Hades\"")),
        "the install directory, got:\n{json}"
    );
    assert!(
        json.contains(&format!(
            "\"says\":\"a record{ESCAPED} has no install_path\""
        )),
        "the note, got:\n{json}"
    );
    assert!(
        json.contains(&format!(
            "\"says\":\"appmanifest{ESCAPED}.acf: unreadable\""
        )),
        "the problem, got:\n{json}"
    );
}

/// The `cause` a note object carries, found by key and never from `says` or
/// `library`.
fn cause_of(line: &str) -> &str {
    const KEY: &str = r#""cause":""#;
    let start = line
        .find(KEY)
        .map_or_else(|| panic!("no cause key on {line}"), |at| at + KEY.len());
    let rest = &line[start..];
    let end = rest.find('"').expect("a cause value that closes");
    &rest[..end]
}

#[test]
fn a_note_says_which_cause_it_is_without_a_program_reading_the_prose() {
    // Same words, different causes: only `cause` tells them apart.
    let same = "the same sentence on both";
    let fake = Fake {
        roots: vec![PathBuf::from("/dxray-cause")],
        notes: vec![same.to_owned()],
        index_notes: vec![same.to_owned()],
        ..Fake::new("cause")
    };
    let json = scan(&[&fake]).json();

    let causes: Vec<&str> = json
        .lines()
        .filter(|line| line.contains(r#""kind":"note""#))
        .map(cause_of)
        .collect();
    assert_eq!(
        causes,
        [Cause::Index.key(), Cause::Record.key()],
        "the index note comes first and each says its own cause, got:\n{json}"
    );
    // Every pair of causes is spelled differently.
    for (i, a) in Cause::ALL.iter().enumerate() {
        for b in &Cause::ALL[i + 1..] {
            assert_ne!(
                a.key(),
                b.key(),
                "two causes, one word: a program reading `cause` cannot tell them apart"
            );
        }
    }
}

#[test]
fn the_summary_counts_the_notes_of_each_cause_as_data_and_writes_zero_as_zero() {
    // The counts, read through the function the clauses are worded from.
    let mut mixed = with_notes(3, 1);
    mixed.notes.push(Cause::Record);
    mixed.notes.push(Cause::Record);
    let json = mixed.json();

    assert!(
        json.contains(r#""notes":{"index":1,"record":2}"#),
        "one count per cause, keyed by the cause's own word, got {json}"
    );
    assert!(
        json.contains("1 index declared no libraries")
            && json.contains("2 launcher records could not be used"),
        "and the clauses are worded from those same counts, got {json}"
    );

    // Zero is written: an absent key could mean an unknown cause.
    let clean = listing(3, 0).json();
    assert!(
        clean.contains(r#""notes":{"index":0,"record":0}"#),
        "a clean scan says 0 of each, not nothing, got {clean}"
    );
    assert!(
        clean.contains(r#""caveats":[]"#),
        "and hedges nothing in prose either, got {clean}"
    );
}
