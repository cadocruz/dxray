//! What the launcher model must keep: only Steam games can be asked a Proton
//! question. Then [`walk`]: the order it announces things in, and the
//! libraries it will not visit twice.

use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::{Catalogue, Game, Identity, Launcher, Libraries, Origin, Visitor, all, walk};
use crate::heroic::{self, Heroic};
use crate::steam::{self, Steam};
use crate::testutil::TempDir;

fn steam_game(appid: u32) -> Game {
    Game {
        identity: Identity::SteamApp(appid),
        name: "Dota 2".to_owned(),
        install_dir: PathBuf::from("/steam/steamapps/common/dota 2 beta"),
        origin: steam::ORIGIN,
    }
}

fn heroic_game(id: &str) -> Game {
    Game {
        identity: Identity::Native(id.to_owned()),
        name: "Hades".to_owned(),
        install_dir: PathBuf::from("/games/Hades"),
        origin: heroic::Store::Gog.origin(),
    }
}

#[test]
fn a_steam_game_can_still_be_asked_for_a_proton_policy() {
    // The half of the capability that is easy to lose by accident: unify two
    // types by the fields they share and the appid is the field that goes.
    let game = steam_game(570);
    assert_eq!(
        game.identity.steam_appid(),
        Some(570),
        "the appid must survive the unification, because it is the only key to \
         a compatdata prefix and therefore to a truthful NVAPI verdict"
    );
    assert_eq!(
        crate::proton::compatdata(Path::new("/steam"), game.identity.steam_appid().unwrap()),
        PathBuf::from("/steam/steamapps/compatdata/570"),
        "the prefix path is reachable from the unified game and the library it \
         was found in, which is why the library level was kept"
    );
}

#[test]
fn a_heroic_game_is_told_the_proton_question_does_not_apply_rather_than_answered() {
    let mut builds = crate::proton::Builds::default();
    let answer = builds.answer_for(None, Path::new("/config/heroic"), &heroic_game("hades-gog"));

    assert_eq!(
        answer.available, None,
        "a launcher with no Steam AppID must not colour a word either way"
    );
    assert!(
        answer.verdict.starts_with("not applicable:"),
        "the refusal must be worded as a refusal, not as an absence of \
         knowledge, which is what \"not determined\" means: {}",
        answer.verdict
    );
    assert!(
        answer.verdict.contains("Heroic / GOG"),
        "the sentence names the launcher it is refusing for, so the next \
         launcher added does not inherit a sentence that is false about it: {}",
        answer.verdict
    );
    assert!(
        answer.script.is_none(),
        "there is no build behind a refusal, and quoting one would invite the \
         reader to check an answer that was never read off it"
    );
}

#[test]
fn a_heroic_id_cannot_become_a_steam_appid_even_when_it_is_all_digits() {
    // A numeric GOG id must never become a Steam appid.
    let game = heroic_game("1207658691");
    assert_eq!(
        game.identity.steam_appid(),
        None,
        "an all-digit launcher id is still not a Steam application id"
    );

    let mut builds = crate::proton::Builds::default();
    assert!(
        builds
            .answer_for(None, Path::new("/config/heroic"), &game)
            .verdict
            .starts_with("not applicable:"),
        "and asking anyway gets the refusal, not somebody else's policy"
    );
}

#[test]
fn zero_is_a_steam_appid_like_any_other_and_not_a_marker_for_absence() {
    // The browser used to carry `appid: 0` to mean "no Steam id". Zero is a
    // real number and the placeholder was the bug.
    assert_eq!(steam_game(0).identity.steam_appid(), Some(0));
    assert_eq!(heroic_game("0").identity.steam_appid(), None);
    assert_ne!(
        Identity::SteamApp(0),
        Identity::Native("0".to_owned()),
        "the two are different facts and must not compare equal"
    );
}

#[test]
fn every_heroic_backend_reports_one_launcher_key_and_its_own_label() {
    // Both the scanner that found a directory and the shop it came from.
    for store in [
        heroic::Store::Epic,
        heroic::Store::Gog,
        heroic::Store::Amazon,
    ] {
        assert_eq!(
            store.origin().key(),
            heroic::ORIGIN.key(),
            "{} must dedup as Heroic",
            store.name()
        );
    }
    assert_eq!(heroic::Store::Gog.origin().label(), "Heroic / GOG");
    assert_eq!(steam::ORIGIN.key(), "steam");
    assert_ne!(
        steam::ORIGIN.key(),
        heroic::ORIGIN.key(),
        "two launchers finding the same directory are two findings, not one"
    );
}

#[test]
fn the_registry_holds_every_launcher_compiled_in_and_names_them_apart() {
    let keys: Vec<&str> = all().iter().map(|l| l.origin().key()).collect();
    assert_eq!(
        keys,
        ["steam", "heroic"],
        "adding a launcher is one module and one line here"
    );
}

#[test]
fn a_heroic_root_reports_itself_as_its_one_library() {
    // Heroic's library is the real directory its records came from.
    let root = TempDir::new("launcher-heroic-library");
    let libraries = Heroic.libraries(root.path());
    assert_eq!(
        libraries,
        Libraries {
            paths: vec![root.path().to_path_buf()],
            notes: Vec::new(),
            problems: Vec::new(),
        },
        "one library, no notes, and nothing that can fail"
    );
}

#[test]
fn a_steam_root_that_cannot_be_read_yields_no_libraries_and_one_problem() {
    // A Steam index failure lands on `problems`.
    let root = TempDir::new("launcher-steam-bad-index");
    root.dir("steamapps");
    root.write("steamapps/libraryfolders.vdf", "\"libraryfolders\"\n{\n}\n");

    let libraries = Steam.libraries(root.path());
    assert!(
        libraries.paths.is_empty(),
        "an unusable index is not a machine with one library"
    );
    assert_eq!(libraries.problems.len(), 1, "{:?}", libraries.problems);
    assert!(
        libraries.problems[0].contains("library index is empty"),
        "the worded problem keeps the sentence the typed error rendered to: {}",
        libraries.problems[0]
    );
}

#[test]
fn a_library_that_is_not_there_is_a_problem_and_never_an_empty_catalogue() {
    // The one thing the collapse to `Catalogue` must not do: report an
    // unreadable library as a library holding no games.
    let missing = Path::new("/definitely/not/a/steam/library");
    let catalogue = Steam.games(missing);
    assert!(catalogue.games.is_empty());
    assert_eq!(
        catalogue.problems.len(),
        1,
        "the wholesale failure survives as a problem: {:?}",
        catalogue.problems
    );
}

#[test]
fn a_steam_library_comes_back_through_the_trait_with_its_appids_intact() {
    let install = TempDir::new("launcher-steam-scan");
    install.dir("steamapps");
    install.write(
        "steamapps/appmanifest_570.acf",
        "\"AppState\"\n{\n\t\"appid\"\t\"570\"\n\t\"name\"\t\"Dota 2\"\n\t\"installdir\"\t\"dota 2 beta\"\n}\n",
    );

    let catalogue = Steam.games(install.path());
    assert!(catalogue.problems.is_empty(), "{:?}", catalogue.problems);
    assert_eq!(catalogue.games.len(), 1);
    assert_eq!(
        catalogue.games[0].identity,
        Identity::SteamApp(570),
        "read through the trait, a Steam game is still a Steam application"
    );
    assert_eq!(catalogue.games[0].origin, steam::ORIGIN);
}

#[test]
fn a_scan_can_be_written_once_over_every_launcher() {
    // A loop that names neither launcher; the trait is object-safe.
    let root = TempDir::new("launcher-uniform-scan");

    let mut seen = Vec::new();
    for launcher in all() {
        let libraries = launcher.libraries(root.path());
        for library in libraries.paths {
            let catalogue = launcher.games(&library);
            for game in catalogue.games {
                // Everything the launcher declared, unfiltered: the trait
                // offers nothing to filter with, and that is the point.
                seen.push((launcher.origin().key(), game.identity));
            }
        }
    }

    assert!(
        seen.is_empty(),
        "an empty directory is neither a Steam nor a Heroic install"
    );
}

#[test]
fn an_origin_prints_its_label_and_an_identity_prints_what_the_launcher_calls_it() {
    assert_eq!(Origin::new("steam", "Steam").to_string(), "Steam");
    assert_eq!(Identity::SteamApp(570).to_string(), "570");
    assert_eq!(
        Identity::Native("hades-gog".to_owned()).to_string(),
        "hades-gog",
        "printed verbatim, because nothing in this crate knows what a \
         launcher's id scheme means and a parser that guessed would invent facts"
    );
}

/// The one sequence the visitor and the fake launcher both record into, so a
/// read's position among the visitor calls is observable. `Arc<Mutex<_>>`
/// because [`Launcher`] is `Sync`.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn push(&self, event: String) {
        self.0
            .lock()
            .expect("no test panics holding this")
            .push(event);
    }

    fn events(&self) -> Vec<String> {
        self.0.lock().expect("no test panics holding this").clone()
    }
}

/// Records what the walk announced, in order, flat: order is half of what
/// [`walk`] promises.
#[derive(Default)]
struct Transcript {
    log: Log,
    /// How many visits are left before the visitor asks the walk to stop.
    /// `None` never stops.
    budget: Option<usize>,
}

impl Transcript {
    fn record(&mut self, event: String) -> ControlFlow<()> {
        self.log.push(event);
        match &mut self.budget {
            None => ControlFlow::Continue(()),
            Some(0) => ControlFlow::Break(()),
            Some(left) => {
                *left -= 1;
                ControlFlow::Continue(())
            }
        }
    }

    /// Everything recorded so far, visitor calls and disk reads alike, in order.
    fn events(&self) -> Vec<String> {
        self.log.events()
    }
}

impl Visitor for Transcript {
    fn root(&mut self, origin: Origin, root: &Path) -> ControlFlow<()> {
        self.record(format!("root {} {}", origin.key(), root.display()))
    }

    fn note(&mut self, origin: Origin, note: &str) -> ControlFlow<()> {
        self.record(format!("note {} {note}", origin.key()))
    }

    fn problem(&mut self, origin: Origin, problem: &str) -> ControlFlow<()> {
        self.record(format!("problem {} {problem}", origin.key()))
    }

    fn library(&mut self, origin: Origin, library: &Path) -> ControlFlow<()> {
        self.record(format!("library {} {}", origin.key(), library.display()))
    }

    fn catalogue(
        &mut self,
        launcher: &dyn Launcher,
        _library: &Path,
        catalogue: Catalogue,
    ) -> ControlFlow<()> {
        self.record(format!(
            "catalogue {} {}",
            launcher.origin().key(),
            catalogue.games.len()
        ))
    }
}

/// A launcher whose whole machine is supplied, so a test can build the shapes
/// no real machine can be relied upon to have.
struct Fake {
    origin: Origin,
    roots: Vec<PathBuf>,
    libraries: Vec<PathBuf>,
    notes: Vec<String>,
    problems: Vec<String>,
    /// The transcript's own sequence, so that reading a library leaves a mark
    /// in it. See [`Log`].
    log: Log,
}

impl Fake {
    /// The transcript is required, so the read is always observable.
    fn new(key: &'static str, roots: &[&str], transcript: &Transcript) -> Self {
        Self {
            origin: Origin::new(key, key),
            roots: roots.iter().map(PathBuf::from).collect(),
            libraries: Vec::new(),
            notes: Vec::new(),
            problems: Vec::new(),
            log: transcript.log.clone(),
        }
    }

    fn holding(mut self, libraries: &[&str]) -> Self {
        self.libraries = libraries.iter().map(PathBuf::from).collect();
        self
    }
}

impl Launcher for Fake {
    fn origin(&self) -> Origin {
        self.origin
    }

    fn roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }

    fn candidate_roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }

    fn libraries(&self, root: &Path) -> Libraries {
        Libraries {
            paths: if self.libraries.is_empty() {
                vec![root.to_path_buf()]
            } else {
                self.libraries.clone()
            },
            notes: self.notes.clone(),
            problems: self.problems.clone(),
        }
    }

    /// Records the read into the transcript before answering, so a test can
    /// see it land between visitor calls.
    fn games(&self, library: &Path) -> Catalogue {
        self.log
            .push(format!("read {} {}", self.origin.key(), library.display()));
        Catalogue::default()
    }
}

#[test]
fn the_walk_announces_a_library_before_it_reads_it() {
    // The library is announced before it is read, so a consumer can show the
    // header while the disk works.
    let mut transcript = Transcript::default();
    let launcher = Fake::new("only", &["/dxray-walk-root"], &transcript);

    let outcome = walk(&[&launcher], &mut transcript);

    assert_eq!(outcome, ControlFlow::Continue(()));
    assert_eq!(
        transcript.events(),
        [
            "root only /dxray-walk-root",
            "library only /dxray-walk-root",
            "read only /dxray-walk-root",
            "catalogue only 0",
        ],
        "the library is named, then read, then handed over"
    );
}

#[test]
fn notes_and_problems_are_announced_before_the_libraries_they_qualify() {
    // Notes come before the library they explain.
    let mut transcript = Transcript::default();
    let mut launcher = Fake::new("only", &["/dxray-walk-noted"], &transcript);
    launcher
        .notes
        .push("its index declared no library".to_owned());
    launcher
        .problems
        .push("a second index could not be read".to_owned());

    assert_eq!(
        walk(&[&launcher], &mut transcript),
        ControlFlow::Continue(())
    );

    assert_eq!(
        transcript.events(),
        [
            "root only /dxray-walk-noted",
            "note only its index declared no library",
            "problem only a second index could not be read",
            "library only /dxray-walk-noted",
            "read only /dxray-walk-noted",
            "catalogue only 0",
        ]
    );
}

#[test]
fn one_library_named_by_two_installs_is_walked_once_and_both_installs_are_kept() {
    // A library named twice is walked once. Roots are not deduplicated: two
    // installs sharing a library are still two installs.
    let mut transcript = Transcript::default();
    let first =
        Fake::new("first", &["/dxray-walk-a"], &transcript).holding(&["/dxray-walk-shared"]);
    let second =
        Fake::new("second", &["/dxray-walk-b"], &transcript).holding(&["/dxray-walk-shared"]);

    assert_eq!(
        walk(&[&first, &second], &mut transcript),
        ControlFlow::Continue(())
    );

    let events = transcript.events();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.starts_with("library "))
            .count(),
        1,
        "the shared library is walked once: {events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.starts_with("read "))
            .count(),
        1,
        "and read once, which is the cost the deduplication is for: {events:?}"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.starts_with("root "))
            .count(),
        2,
        "and both installs are still announced: {events:?}"
    );
}

#[test]
fn a_visitor_that_breaks_stops_the_walk_where_it_asked_to() {
    // Cancellation is the visitor's: it breaks, and the walk stops.
    let mut transcript = Transcript {
        budget: Some(1),
        ..Transcript::default()
    };
    let first = Fake::new("first", &["/dxray-walk-first"], &transcript);
    let second = Fake::new("second", &["/dxray-walk-second"], &transcript);

    let outcome = walk(&[&first, &second], &mut transcript);

    assert_eq!(
        outcome,
        ControlFlow::Break(()),
        "the walk reports that it did not finish"
    );
    // No "read" line, and that is the point of breaking rather than filtering:
    // a visitor that has stopped caring must not still be paying for the disk.
    assert_eq!(
        transcript.events(),
        [
            "root first /dxray-walk-first",
            "library first /dxray-walk-first"
        ],
        "and nothing past the break was looked at: {:?}",
        transcript.events()
    );
}

#[test]
fn the_walk_names_no_launcher_and_gains_one_without_being_edited() {
    // An unknown launcher is walked, its messages prefixed from its own origin.
    let mut transcript = Transcript::default();
    let newcomer = Fake::new("newcomer", &["/dxray-walk-new"], &transcript);

    assert_eq!(
        walk(&[&newcomer], &mut transcript),
        ControlFlow::Continue(())
    );

    let events = transcript.events();
    assert!(
        events.iter().all(|event| event.contains("newcomer")),
        "every event carries the launcher that raised it: {events:?}"
    );
}

#[test]
fn every_launcher_in_the_registry_can_say_where_it_looked() {
    // Every launcher says where it looked.
    for launcher in all() {
        assert!(
            !launcher.candidate_roots().is_empty(),
            "{} must name somewhere it looked",
            launcher.origin().key()
        );
    }
}

/// Adapts a real launcher to a fixture root without teaching production
/// discovery about test-only paths.
struct AtRoot {
    launcher: &'static dyn Launcher,
    root: PathBuf,
}

impl Launcher for AtRoot {
    fn origin(&self) -> Origin {
        self.launcher.origin()
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![self.root.clone()]
    }

    fn candidate_roots(&self) -> Vec<PathBuf> {
        vec![self.root.clone()]
    }

    fn libraries(&self, root: &Path) -> Libraries {
        self.launcher.libraries(root)
    }

    fn games(&self, library: &Path) -> Catalogue {
        self.launcher.games(library)
    }
}

#[test]
fn inventory_collects_the_identity_origin_and_library_from_every_launcher() {
    let fixture = TempDir::new("inventory-contract");
    let steam_library = fixture.dir("steam");
    let steam_install = fixture.dir("steam/steamapps/common/dota 2 beta");
    fixture.write(
        "steam/steamapps/appmanifest_570.acf",
        "\"AppState\" { \"appid\" \"570\" \"name\" \"Dota 2\" \"installdir\" \"dota 2 beta\" }",
    );
    let heroic_library = fixture.dir("heroic");
    let heroic_install = fixture.dir("games/Hades");
    fixture.write(
        "heroic/store_cache/gog_library.json",
        &format!(
            r#"{{"library":[{{"app_name":"heroic-hades","title":"Hades","is_installed":true,"install":{{"install_path":"{}"}}}}]}}"#,
            heroic_install.display()
        ),
    );
    let steam = AtRoot {
        launcher: &crate::steam::STEAM,
        root: steam_library.clone(),
    };
    let heroic = AtRoot {
        launcher: &crate::heroic::HEROIC,
        root: heroic_library.clone(),
    };

    let inventory = super::Inventory::collect(&[&steam, &heroic]);

    assert_eq!(
        inventory.roots,
        [
            super::InventoryRoot {
                origin: crate::steam::ORIGIN,
                path: steam_library.clone(),
            },
            super::InventoryRoot {
                origin: crate::heroic::ORIGIN,
                path: heroic_library.clone(),
            },
        ]
    );
    assert_eq!(
        inventory.libraries,
        [
            super::InventoryLibrary {
                origin: crate::steam::ORIGIN,
                root: Some(steam_library.clone()),
                path: steam_library.clone(),
            },
            super::InventoryLibrary {
                origin: crate::heroic::ORIGIN,
                root: Some(heroic_library.clone()),
                path: heroic_library.clone(),
            },
        ]
    );
    assert!(inventory.notes.is_empty());
    assert!(inventory.problems.is_empty());
    assert_eq!(inventory.entries.len(), 2);
    assert_eq!(inventory.entries[0].root, Some(steam_library.clone()));
    assert_eq!(inventory.entries[0].game.identity, Identity::SteamApp(570));
    assert_eq!(inventory.entries[0].game.origin, crate::steam::ORIGIN);
    assert_eq!(inventory.entries[0].library, steam_library);
    assert_eq!(inventory.entries[0].game.install_dir, steam_install);
    assert_eq!(
        inventory.entries[1].game.identity,
        Identity::Native("heroic-hades".to_owned())
    );
    assert_eq!(inventory.entries[1].root, Some(heroic_library.clone()));
    assert_eq!(
        inventory.entries[1].game.origin,
        Origin::new("heroic", "Heroic / GOG")
    );
    assert_eq!(inventory.entries[1].library, heroic_library);
    assert_eq!(inventory.entries[1].game.install_dir, heroic_install);
}

struct ReportingFixture {
    root: PathBuf,
}

impl Launcher for ReportingFixture {
    fn origin(&self) -> Origin {
        Origin::new("fixture", "Fixture")
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![self.root.clone()]
    }

    fn candidate_roots(&self) -> Vec<PathBuf> {
        vec![self.root.clone()]
    }

    fn libraries(&self, root: &Path) -> Libraries {
        Libraries {
            paths: vec![root.join("library")],
            notes: vec!["index caveat".to_owned()],
            problems: vec!["index problem".to_owned()],
        }
    }

    fn games(&self, _library: &Path) -> Catalogue {
        Catalogue {
            notes: vec!["catalogue caveat".to_owned()],
            problems: vec!["catalogue problem".to_owned()],
            ..Catalogue::default()
        }
    }
}

#[test]
fn inventory_keeps_root_and_catalogue_diagnostics_at_their_real_locations() {
    let fixture = ReportingFixture {
        root: PathBuf::from("/dxray-inventory-diagnostics"),
    };
    let inventory = super::Inventory::collect(&[&fixture]);
    let library = fixture.root.join("library");

    assert_eq!(inventory.notes.len(), 2);
    assert_eq!(inventory.notes[0].root, Some(fixture.root.clone()));
    assert_eq!(inventory.notes[0].library, None);
    assert_eq!(inventory.notes[0].message, "index caveat");
    assert_eq!(inventory.notes[1].root, Some(fixture.root.clone()));
    assert_eq!(inventory.notes[1].library, Some(library.clone()));
    assert_eq!(inventory.notes[1].message, "catalogue caveat");

    assert_eq!(inventory.problems.len(), 2);
    assert_eq!(inventory.problems[0].root, Some(fixture.root.clone()));
    assert_eq!(inventory.problems[0].library, None);
    assert_eq!(inventory.problems[0].message, "index problem");
    assert_eq!(inventory.problems[1].root, Some(fixture.root));
    assert_eq!(inventory.problems[1].library, Some(library));
    assert_eq!(inventory.problems[1].message, "catalogue problem");
}
