//! What the listing promises: a trailer that cannot disagree with the exit
//! code, a row per game that says where the game came from, and one
//! implementation serving both flags.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use dxray_core::{Game, Identity, Launcher};

use super::{Cause, Listing, nothing_found, nothing_found_json, render_games, scan};

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
        incomplete: (0..directories).map(|i| format!("dir {i}")).collect(),
        ..listing(games, 0)
    }
}

#[test]
fn the_trailer_says_when_the_count_it_prints_is_incomplete() {
    // "41 games" under a block that also reported two unreadable manifests
    // is the line a person quotes, and alone it is not true: those two are
    // games too. The caveat has to sit in the same line as the number, or
    // it is the half that gets lost on the way to somebody else.
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
    // A note means an index declared nothing, so the library count — and
    // the games count that rides on it — may be smaller than the machine
    // really has. Same argument as the unreadable-manifest caveat: the
    // number is what gets quoted, so the doubt travels in the same line.
    assert_eq!(
        with_notes(3, 1).trailer(),
        "3 games in 1 library across 1 install; \
         1 index declared no libraries, so there may be more"
    );
}

#[test]
fn a_record_that_could_not_be_used_is_not_reported_as_an_index_that_declared_nothing() {
    // The defect this split exists for, measured on a real machine: a Heroic
    // cache entry that lost its install_path for a game another cache still
    // supplied produced "1 index declared no libraries, so there may be more".
    // Every word of that is wrong about it — no index was involved, every
    // library was read, and nothing is missing. The trailer is the sentence
    // people quote, and a sentence that names the wrong file sends whoever
    // reads it to go and inspect a file that is fine.
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
    // One clause per cause. Adding them would produce "3 indexes declared no
    // libraries" on a machine with one index note and two stale records, which
    // is a bigger doubt than the machine has and about the wrong thing; a
    // single clause naming both would have to be worded for whichever it was
    // mostly about. The two counts are separate because the two doubts are
    // different sizes: one says whole libraries may be hidden, the other says
    // nothing is.
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
    // They are different things — one file could not be read, another was
    // read and said less than expected — and collapsing them into one
    // phrase would lose whichever was mentioned second.
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
    // A bounded walk that ran out of budget did not look at every
    // executable in that directory, so the one named above it may not be
    // the best one there. The number is what gets quoted, so the doubt
    // travels in the same line as the number.
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
    // The rule this whole third list exists for: a human reading the
    // trailer and a script reading the status must not come away with
    // different stories about one run. Asserted in both directions,
    // because a test for one of them passes on a build that always fails.
    assert!(
        with_incomplete(3, 1).incomplete_scan(),
        "a truncated walk is a run that did not read everything"
    );
    assert!(
        listing(3, 1).incomplete_scan(),
        "so is a file that could not be read"
    );

    // And the caveat that does not claim anything was missed does not move
    // it. `NoRendererImported` never reaches this list at all — it says
    // everything was read and says little — and an index that declared no
    // libraries is the same kind of statement.
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
    // The trailer has to mean the same thing on a machine with one launcher
    // on it as on a machine with two. An installation with nothing in it is
    // one install with one library and no games — the sentence an empty
    // Steam library has always earned — and not an absence.
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

/// Renders `games` against a library that is not on this disk, which is the
/// state every game on this machine is in: no prefix, so no Proton build,
/// so no NVAPI answer. Exactly what most of a real library looks like too.
fn render(out: &mut String, games: &[Game]) {
    render_named(out, games, false);
}

fn render_named(out: &mut String, games: &[Game], name_origins: bool) {
    out.push_str(&render_both(games, name_origins).text);
}

/// Both renderings of the same games, from the one call that produces them.
///
/// Tests take the pair rather than one at a time on purpose: what is worth
/// asserting here is mostly that the two agree.
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

    assert!(out.contains("570      Dota 2"), "got {out:?}");
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
    // The whole point of listing every launcher at once: a row that does not
    // say where its game came from turns two libraries into one undifferentiated
    // pile, and the label is the only thing that tells a reader which launcher
    // to go and look in.
    let mut out = String::new();
    render_named(&mut out, &[dota(), hades()], true);

    assert!(
        out.contains("origin  Steam"),
        "the Steam game names Steam, got {out:?}"
    );
    assert!(
        out.contains("origin  Heroic / GOG"),
        "and the Heroic game names its backend, not just Heroic: {out:?}"
    );
}

#[test]
fn the_narrower_listing_does_not_repeat_the_one_launcher_it_was_asked_about() {
    // `steam` already said Steam. A column with the same value in every row
    // costs a line per game and tells nobody anything, and printing it would
    // change output that people have captured.
    let mut out = String::new();
    render(&mut out, &[dota()]);

    assert!(
        !out.contains("origin"),
        "a single-launcher listing names no origins, got {out:?}"
    );
}

#[test]
fn a_game_with_no_proton_prefix_still_gets_a_row_saying_so() {
    // The row is printed for every game, answer or not. A game with nothing
    // said about its NVAPI reads as a game with nothing wrong with it, and
    // "this has never been launched under Proton, so there is no build to
    // read" is a different statement from "Proton leaves it alone".
    let mut out = String::new();
    render(&mut out, &[dota()]);

    assert!(out.contains("nvapi"), "got {out:?}");
    assert!(
        out.contains("not determined"),
        "the row must not read as an answer, got {out:?}"
    );
    // Compared against the text with its wrapping taken out, because the
    // sentence is long enough to be folded across two lines and the test is
    // about what it says, not about where it breaks.
    let unwrapped = out.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        unwrapped.contains("has not been run under Proton"),
        "and it must say why there is none, got {out:?}"
    );
}

#[test]
fn a_game_with_no_steam_appid_is_refused_the_proton_question_rather_than_answered() {
    // The capability difference, spent where a reader can see it. "Not
    // determined" would mean nobody looked; this game cannot be looked up at
    // all, and the row has to say which of the two it is.
    let mut out = String::new();
    render_named(&mut out, &[hades()], true);

    let unwrapped = out.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(unwrapped.contains("not applicable"), "got {out:?}");
    assert!(
        unwrapped.contains("Heroic / GOG"),
        "the refusal names the launcher it is refusing for, so the next \
         launcher added does not inherit a sentence that is false about it: {out:?}"
    );
}

#[test]
fn a_game_with_no_nvapi_answer_does_not_move_the_exit_code() {
    // Most of a real library has never been launched under Proton. Counting
    // that as a scan that came up short would report every healthy machine
    // as a failure, and it is not what the status answers: the status says
    // whether every game the launcher declared was read.
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
    // Otherwise the message is unactionable: a person with Steam plainly
    // installed has no way to tell whether the tool is broken or simply
    // does not know about their layout.
    let message = nothing_found(super::STEAM_ONLY);

    assert!(message.contains("no Steam installation found"), "{message}");
    assert!(
        message.lines().count() > 2,
        "the candidate paths must be listed, got {message}"
    );
}

#[test]
fn finding_nothing_across_every_launcher_says_which_paths_belonged_to_which() {
    // A flat run of a dozen paths does not tell anybody which of them was a
    // Heroic that is not there. The headings are what make the message
    // actionable when more than one launcher was asked about.
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
    /// Notes raised by the index that says where the libraries are, which is
    /// what [`Libraries`](dxray_core::Libraries) carries. A separate field
    /// because the walk delivers them through a separate callback, and telling
    /// the two apart is the whole of what the trailer's two note clauses rest
    /// on.
    index_notes: Vec<String>,
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
            ..dxray_core::Libraries::default()
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

#[test]
fn a_library_nobody_could_look_inside_is_not_reported_as_an_empty_one() {
    // The two states are not the same state, and only one of them is a
    // statement about what the user owns. "(no games installed)" under a
    // library that could not be listed is a claim about a directory nobody
    // managed to open, and it is exactly the reassuring-looking output this
    // project refuses everywhere else.
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
    // The state `installed` turns into an exit code and a message naming where
    // it looked. An empty listing that exited 0 would be the same output a
    // working scan of an empty machine produces, and the two are not the same
    // state: one means "you own no games", the other means "this tool did not
    // find your launcher".
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
    // The flag is the launcher set and nothing else. This is what lets
    // `steam` and `installed` share one implementation instead of being two
    // renderers that drift.
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

/// A directory tree that removes itself, so that a test can point a game at a
/// directory that is really there. The ordering is decided by what is inside
/// one, which is the one thing a path to nowhere cannot exercise.
struct Tree {
    path: PathBuf,
}

impl Tree {
    fn new(tag: &str) -> Self {
        // Process id and a counter: tests run in parallel threads, and two of
        // them sharing a directory is a failure that only appears on a busy
        // machine.
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("dxray-listing-{}-{tag}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }

    /// An install directory holding one executable, returned as a game.
    ///
    /// The executable is two bytes, which is enough: what decides the
    /// ordering is whether anything in the directory argues it is a game, and
    /// on these fixtures the only argument available is the executable's name
    /// against the title. An image that parses would add a renderer to the
    /// reasons and change nothing about the grouping.
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
    // Four rows in nine of a measured library are Proton builds and Steam
    // runtimes. Nothing is hidden — both are here, and the trailer counts both
    // — but the reader's eye gets the games first.
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
    let game_at = out.find("Dota 2\n").expect("both are listed");
    assert!(game_at < runtime_at, "the evidence goes first, got:\n{out}");
    assert!(
        out.contains("nothing here carries evidence of being a game"),
        "and the demoted row says why it is down there, got:\n{out}"
    );
}

#[test]
fn an_install_that_argues_nothing_is_demoted_on_both_surfaces_or_neither() {
    // One question — does anything in this install argue it is a game — asked
    // once, of `dxray-core`, and spent on the order of both renderings and on
    // the field a consumer reads. Breaking
    // `dxray_core::inspect::lacks_evidence` fails this test, the ordering test
    // above it, the browser's marker and core's own; that is what says the
    // decision is in one place.
    let tree = Tree::new("order-json");
    let runtime = tree.install(
        100,
        "Proton - Experimental",
        "Proton Experimental",
        "filelock.exe",
    );
    let game = tree.install(570, "dota 2 beta", "Dota 2", "Dota 2.exe");

    let rendered = render_both(&[runtime, game], false);

    let text_game = rendered.text.find("Dota 2\n").expect("both are listed");
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
    // Demoting is not a shorter rendering. A Proton build whose directory was
    // not searched in full has to say so wherever it is printed, or the listing
    // has found a new way to drop a caveat.
    let tree = Tree::new("rows");
    let runtime = tree.install(
        100,
        "Proton - Experimental",
        "Proton Experimental",
        "filelock.exe",
    );
    let mut out = String::new();

    render_named(&mut out, &[runtime], true);

    assert!(out.contains("origin  Steam"), "got:\n{out}");
    assert!(out.contains("nvapi"), "got:\n{out}");
}

#[test]
fn a_directory_that_could_not_be_read_keeps_its_place_among_the_games() {
    // An absent answer and an answer of "nothing" must not look alike. A game
    // that is mid-download points at a directory that is not there yet, and
    // burying it under the runtimes would hide the one row worth reading.
    let tree = Tree::new("unread");
    let runtime = tree.install(
        100,
        "Proton - Experimental",
        "Proton Experimental",
        "filelock.exe",
    );
    let mut out = String::new();

    render(&mut out, &[runtime, dota()]);

    let missing_at = out.find("Dota 2\n").expect("both are listed");
    let runtime_at = out.find("Proton Experimental").expect("both are listed");
    assert!(
        missing_at < runtime_at,
        "an unanswered question is not an answer of nothing, got:\n{out}"
    );
}

#[test]
fn an_unread_directory_is_null_evidence_in_the_json_rather_than_false() {
    // The same rule, asserted where a program reads it, and in its own test so
    // that breaking the rule fails on the JSON surface too rather than being
    // hidden behind the failure above. `null` is an unanswered question; `false`
    // is an answer of "nothing in here argues it is a game", and a consumer
    // that cannot tell them apart will file a game that is mid-download under
    // the redistributables.
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

    let dota_at = out.find("Dota 2\n").expect("both are listed");
    let hades_at = out.find("Hades\n").expect("both are listed");
    assert!(dota_at < hades_at, "got:\n{out}");
}

#[test]
fn a_demoted_game_does_not_lose_the_note_that_moves_the_exit_code() {
    // The list the exit code is drawn from is filled before anything is
    // printed, so a game moved under the others cannot drop its caveat on the
    // way down. This is also the listing declining to charge truncation only to
    // the entries that look like games: a truncated walk is itself an
    // explanation for finding no evidence.
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
            .any(|note| note.contains("Proton - Experimental")),
        "the demoted game's truncation is still collected, got {incomplete:?}"
    );
    // Compared against the text with its wrapping taken out: the sentence is
    // long enough to be folded, and this test is about what it says, not about
    // where it breaks.
    let unwrapped = out.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        unwrapped.contains("1 directory was not searched"),
        "and printed where the game was printed, got:\n{out}"
    );
    // And the same caveat reaches a program on the game it belongs to, not
    // only in the summary's count. A consumer looking at one row must be able
    // to see that this row is the doubtful one.
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

/// Every byte the terminal listing prints for [`golden_scan`].
///
/// A golden, and deliberately unforgiving. `--json` had to be added without
/// moving a single character of what a person already sees, and "almost
/// unchanged" is not a thing this file can assert. Anything that reaches the
/// listing — a label width, a wrap point, the wording of a row that comes out
/// of `dxray-core` — fails this test, and that is the whole job: the diff is
/// what tells whoever made the change that a human-facing output moved, so that
/// they say so rather than discover it in a bug report.
///
/// The `No such file or directory (os error 2)` fragment is this platform's own
/// wording for a missing directory, and the wrap point after `(os` follows from
/// its length. On a platform that words it differently this test fails and
/// names the difference, which is better than a test that quietly stopped
/// checking the wrapping.
const GOLDEN_TEXT: &str = concat!(
    "/dxray-golden\n",
    "  note        its index declared no libraries\n",
    "  library     /dxray-golden\n",
    "    570      Dota 2\n",
    "             /dxray-golden/dota 2 beta\n",
    "             origin  Steam\n",
    "             best    (directory could not be read: No such file or directory (os\n",
    "                     error 2))\n",
    "             nvapi   not determined: /dxray-golden/steamapps/compatdata/570: no\n",
    "                     compatibility prefix, so this game has not been run under\n",
    "                     Proton and there is no build to read a policy out of\n",
    "  note        a cache record has no install_path\n",
    "  unreadable  appmanifest_999.acf: unreadable\n",
);

/// The same walk as [`GOLDEN_TEXT`], as the objects a program reads.
///
/// Line for line against the listing above: the root, the library, the game
/// with every row printed under it, the note, the unreadable record, and the
/// summary that the trailer is the prose form of. Nothing appears here that is
/// not up there, and nothing up there is missing from here — which is the claim
/// `--json` on a listing has to be able to make.
///
/// Compared as an exact string rather than parsed. There is no JSON parser in
/// this workspace and adding one to a dependency-light binary to check its own
/// output would be a poor trade, but that is not the main reason: a parser
/// accepts a line that is well-formed and wrong, and every interesting mistake
/// available here — a dropped row, a `null` where a value belongs, a value
/// where `null` belongs, a number rendered as a string — is exactly that kind
/// of mistake. The literal below is JSON by inspection and by every consumer
/// that has read it; what it is *for* is catching the wrong-and-well-formed.
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
    // The founding rule, spent on the thing most likely to break it. Both
    // renderings come out of the same pass over the same values, so a change
    // that adds a row, drops one, or reorders them shows up in both strings at
    // once — and if it ever shows up in only one of them, exactly one of these
    // two assertions fails and names which surface was told a different story.
    let listing = golden_scan();

    assert_eq!(listing.text, GOLDEN_TEXT, "the human listing moved");
    assert_eq!(listing.json(), GOLDEN_JSON, "the JSON listing moved");
}

#[test]
fn a_listing_object_cannot_be_mistaken_for_a_file_record() {
    // What makes two shapes behind one flag safe rather than reckless. The
    // record line is a frozen positional contract about a PE file and must
    // never gain a `kind`; every object the listing emits leads with one. A
    // reader tells them apart from the first characters of a line.
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
    // A consumer that reads games and never reads the summary is a consumer
    // that can be told a partial scan was a whole one. The rows are not
    // reachable without it: there is one function, and it appends this.
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
    // Three surfaces, one answer to "did everything get read". The pairing is
    // asserted rather than the three halves separately, because a test for one
    // of them passes on a build where the other two always disagree.
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
    // The half of the machine-readable answer that did not exist before: which
    // games are installed, and where. The install and the library travel with
    // the game, so a consumer that filters the stream down to games still has
    // the tree it came out of.
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
    // Zero is a real appid. The capability difference `Identity` exists to keep
    // has to survive the trip into JSON, or a consumer will happily go looking
    // for `compatdata/0`.
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
    // The rows are decided once and rendered twice: the terminal wraps them to
    // its width and the JSON carries the sentence entire. A consumer must never
    // have to unfold prose to read a caveat.
    let rendered = render_both(&[dota()], true);
    let sentence = "no compatibility prefix, so this game has not been run under Proton";

    assert!(
        rendered.json.contains(sentence),
        "the JSON carries the whole sentence, got {}",
        rendered.json
    );
    assert!(
        !rendered.text.contains(sentence),
        "and the terminal is the one that had to fold it, got:\n{}",
        rendered.text
    );
    for label in ["origin", "best", "nvapi"] {
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
    // What the flattening costs, and why the tree is kept. One object per game
    // would lose an installed launcher that holds nothing — which is a fact
    // about the machine, and the exact fact this listing already refuses to
    // hide from a person.
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
    // Nothing was scanned, so there are no counts and no summary — but a
    // stream with no lines in it reads exactly like a machine that owns no
    // games, and the two are not the same state.
    let line = nothing_found_json(super::STEAM_ONLY);

    assert!(line.starts_with(r#"{"kind":"problem","#), "got {line}");
    assert!(!line.contains('\n'), "one object, one line, got {line}");
    assert!(line.contains("no Steam installation found"), "got {line}");
    assert!(
        line.contains("\\n"),
        "the candidate paths come with it, escaped into the sentence: {line}"
    );
}

/// Everything a title or a path can hold that JSON cannot: a quote and the
/// makings of a second object behind it, a backslash, a newline, a tab, a
/// control character with no short escape, and a codepoint outside the BMP.
///
/// Not a paranoid fixture. Game names are read out of `appmanifest_*.acf` and
/// Heroic's caches — files this project does not write and does not get to
/// vet — and a Windows path carries backslashes on any ordinary day.
const HOSTILE: &str = "\",\"kind\":\"summary\" \\ \n \t \u{1} \u{1f600}";

/// [`HOSTILE`] as RFC 8259 requires it to appear between quotes.
///
/// Written out by hand rather than computed from [`HOSTILE`], because a test
/// that escaped its own expectation with the function under test would agree
/// with any escaper, including one that escapes nothing. The emoji is not
/// escaped at all: it is a valid UTF-8 string and `\u` surrogate pairs are a
/// thing this project does not need to emit.
const ESCAPED: &str = r#"\",\"kind\":\"summary\" \\ \n \t \u0001 😀"#;

#[test]
fn a_hostile_name_reaches_the_json_escaped_and_cannot_start_a_second_object() {
    // The listing surface has its own writers — the install row, the library
    // row, the game object, the worded rows — and every one of them has to go
    // through the one escaper. A field that interpolated a string by hand
    // would look right on every title anybody tests with and would break the
    // stream on the first game with a quote in its name, which is a name a
    // launcher is perfectly happy to store.
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

    // The install, the library, the game, the note, the problem and the
    // summary: six objects, six lines. A newline that survived unescaped puts
    // a seventh here and desynchronises every consumer reading line by line.
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
        // The injection the payload is shaped for: an unescaped quote in a
        // name would close the string and let the rest of the title write
        // keys of its own into the object it was a value in.
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

/// The `cause` a note object carries, read from that key and from nothing
/// else on the line.
///
/// String surgery rather than a parser, like every JSON assertion in this
/// file. It finds the key by name, so no key order is being relied on, and it
/// deliberately never looks at `says` or `library`: the test that uses it is
/// about whether a program has anything *else* to branch on.
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
    // Same words on both notes, on purpose: `says` cannot tell them apart, and
    // the README promises nobody that it could. `library` happens to differ —
    // an index note has none — but that is a fact about where the note sits
    // and not about what raised it, and a consumer branching on it breaks the
    // day a third root-level cause appears. The one field a program is
    // promised is `cause`, and this reads that field alone.
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
    // Pairwise across every cause there is, not just the two above, so that a
    // third cause spelled the same as an existing one fails here and not in a
    // consumer.
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
    // A consumer that wanted the number had only a caveat sentence to parse,
    // and the README promises that sentence may be reworded. The count is the
    // same one the clause is worded from, read through one function, so the
    // sentence and the number cannot drift apart.
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

    // Zero is written rather than the key omitted: a key absent because nothing
    // happened and a key absent because this build never heard of the cause
    // must not look alike to the program reading it.
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
