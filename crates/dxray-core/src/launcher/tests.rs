//! What the unification is allowed to cost, and what it is not.
//!
//! Most of the first half are about the third difference: Steam games can be
//! asked a Proton question and Heroic games cannot, and a model that loses
//! either half of that is worse than the two types it replaced.
//!
//! The second half are about [`walk`], which is the traversal both consumers
//! react to. What it promises is the order things are announced in and the
//! libraries it declines to visit twice — each of which used to be a copy in
//! each consumer, free to drift.

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
    let answer = builds.answer_for(Path::new("/config/heroic"), &heroic_game("hades-gog"));

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
    // The failure this enum exists to prevent. A GOG id is frequently a run of
    // digits, and a model that carried `appid: u32` with a zero for Heroic — or
    // that parsed the string — would hand this game a real Proton verdict read
    // off some unrelated Steam application's prefix.
    let game = heroic_game("1207658691");
    assert_eq!(
        game.identity.steam_appid(),
        None,
        "an all-digit launcher id is still not a Steam application id"
    );

    let mut builds = crate::proton::Builds::default();
    assert!(
        builds
            .answer_for(Path::new("/config/heroic"), &game)
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
    // A dedup key wants to know which scanner found a directory, because the
    // same install can be listed by two backends; a person wants to know which
    // shop it came from.
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
    // Difference one: Steam has a library level and Heroic does not. Kept
    // rather than flattened, because the library is what a compatdata prefix
    // sits beside — and Heroic's answer is the real directory its records came
    // out of, not a placeholder.
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
    // Difference two: Steam's wholesale failure has to land somewhere in a
    // shape that has no `Result`. It lands on `problems`, which is where both
    // consumers were already putting it.
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
    // The whole point: this loop names neither launcher, and gaining a third
    // one changes nothing in it. It is written here rather than only in a
    // consumer so the trait is proved object-safe and usable as one.
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

/// The one sequence everything the walk does lands in, shared by the visitor
/// and the launcher it walks.
///
/// Shared rather than owned by [`Transcript`], because half of what [`walk`]
/// promises is where a call that is *not* a visitor call sits among the ones
/// that are. A launcher that recorded its reads in a list of its own would
/// prove they happened and say nothing about when.
///
/// `Arc<Mutex<_>>` rather than `Rc<RefCell<_>>` because [`Launcher`] is `Sync`,
/// which is what lets [`all`] hand out trait objects a scan can share. A fake
/// that was not `Sync` could not implement the trait at all.
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

/// Records what the walk announced, in order, as one flat transcript.
///
/// Flat on purpose: the order the events arrive in is half of what [`walk`]
/// promises, and a test that sorted them into buckets would pass on a traversal
/// that announced a library after reading it.
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
    /// The transcript is required rather than optional. The read is the only
    /// expensive thing the walk does and the only call whose position the
    /// consumers depend on, and a fake that could be built without it would
    /// leave that position unobservable by default.
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

    /// Records the read into the transcript before answering.
    ///
    /// This is the disk access the walk's announcement order exists to let a
    /// consumer overlap with. It is written into the same sequence the visitor
    /// calls go into so a test can assert it happened *between* them; with the
    /// read invisible, folding the announcement into the read would leave the
    /// two visitor calls in the same order and every transcript unchanged.
    fn games(&self, library: &Path) -> Catalogue {
        self.log
            .push(format!("read {} {}", self.origin.key(), library.display()));
        Catalogue::default()
    }
}

#[test]
fn the_walk_announces_a_library_before_it_reads_it() {
    // Not cosmetic. Reading a library takes as long as the disk it is on, and a
    // consumer that fills a list as it goes has to be able to show the header
    // while the read is running. Announcing and cataloguing in one call would
    // have turned a progressive fill into a series of stalls, and nothing about
    // the output would have said so.
    //
    // The read is in the transcript for that reason. Asserting only that the
    // visitor was told "library" before "catalogue" would pass on a walk that
    // did the read first and announced afterwards, which is exactly the stall
    // this is about.
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
    // A caveat explaining why there is only one library has to be readable
    // above it. Printed after, it is a footnote to a list the reader has
    // already drawn conclusions from.
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
    // The rule each consumer used to carry its own copy of. A native and a
    // Flatpak Steam usually name the same library, and so can two launchers;
    // walking it twice prints every game in it twice, which reads as two copies
    // installed rather than as one counted twice.
    //
    // Roots are deliberately not deduplicated the same way: two installs that
    // share a library are still two installs, and their notes and failures
    // belong to each of them.
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
    // Cancellation is a visitor's business and not a parameter: a consumer that
    // can be cancelled polls its own flag and breaks, and one that cannot never
    // writes the word. That is what makes the second kind cost nothing, rather
    // than pass a flag that is always false.
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
    // The extension point, proved rather than asserted in a comment: a launcher
    // the traversal has never heard of is walked, and every message it produces
    // is prefixed from its own origin. A branch here would be right until
    // somebody added the third launcher.
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
    // Required rather than defaulted, because a launcher that answered nothing
    // would vanish from the "nothing found" message — the answer would be short
    // and nothing would say so, which is the defect one layer up.
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

    assert_eq!(inventory.entries.len(), 2);
    assert_eq!(inventory.entries[0].game.identity, Identity::SteamApp(570));
    assert_eq!(inventory.entries[0].game.origin, crate::steam::ORIGIN);
    assert_eq!(inventory.entries[0].library, steam_library);
    assert_eq!(inventory.entries[0].game.install_dir, steam_install);
    assert_eq!(
        inventory.entries[1].game.identity,
        Identity::Native("heroic-hades".to_owned())
    );
    assert_eq!(
        inventory.entries[1].game.origin,
        Origin::new("heroic", "Heroic / GOG")
    );
    assert_eq!(inventory.entries[1].library, heroic_library);
    assert_eq!(inventory.entries[1].game.install_dir, heroic_install);
}
