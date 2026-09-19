//! Everything here is a layout written out by hand. No disk, no game.
//!
//! The assertions are about **order**, not membership. A ranking that contains
//! the right executable somewhere in it is not a ranking.

use super::{Candidate, NameFrom, Note, Observed, Reason, Resemblance, Survey, assess, weight};
use crate::analysis::{Evidence, Linked, Source, Verdict, analyse};
use std::path::PathBuf;

/// An executable at `relative`, with nothing else known about it yet.
fn at(relative: &str) -> Observed {
    Observed {
        path: PathBuf::from(relative),
        relative: PathBuf::from(relative),
        ..Observed::default()
    }
}

fn importing(names: &[&str]) -> Verdict {
    analyse(&Evidence {
        imports: names.iter().map(|&n| n.to_owned()).collect(),
        ..Evidence::default()
    })
}

fn delay_loading(names: &[&str]) -> Verdict {
    analyse(&Evidence {
        delay_imports: names.iter().map(|&n| n.to_owned()).collect(),
        ..Evidence::default()
    })
}

/// A verdict built from what libraries in the image's own directory import,
/// one link on. `direct` is what the image itself imports.
///
/// Built through `analyse` rather than assembled by hand, because the point of
/// the change this tests is that the ranking and the verdict read one set of
/// evidence. A fixture that bypassed `analyse` could not catch them drifting.
fn linking(direct: &[&str], through: &[(&str, &[&str])]) -> Verdict {
    analyse(&Evidence {
        imports: direct.iter().map(|&n| n.to_owned()).collect(),
        linked: through
            .iter()
            .map(|(library, imports)| Linked {
                library: (*library).to_owned(),
                imports: imports.iter().map(|&n| n.to_owned()).collect(),
                delay_imports: Vec::new(),
            })
            .collect(),
        ..Evidence::default()
    })
}

fn shipping(names: &[&str]) -> Verdict {
    analyse(&Evidence {
        neighbours: names.iter().map(|&n| n.to_owned()).collect(),
        ..Evidence::default()
    })
}

fn names(candidate: &Candidate) -> Vec<String> {
    candidate.reasons.iter().map(ToString::to_string).collect()
}

#[test]
fn the_score_is_exactly_the_arithmetic_of_the_reasons_printed_under_it() {
    // The number and the sentences are what a reader compares. If they can
    // disagree, the number is decoration and the explanation is a second
    // implementation of the ranking that nobody checks.
    let mut observed = at("Game/Binaries/Win64/Game-Win64-Shipping.exe");
    observed.own = importing(&["d3d12.dll"]);
    let candidate = assess(&observed, Some("Game"), None);

    let by_hand: i32 = candidate.reasons.iter().map(Reason::weight).sum();

    assert_eq!(
        candidate.score(),
        by_hand,
        "score must be the sum of {:?}",
        names(&candidate)
    );
    assert_eq!(
        candidate.score(),
        weight::RENDERER_IMPORT
            + weight::UNREAL_BINARIES
            + weight::SHIPPING_SUFFIX
            + weight::NAME_PARTIAL,
        "the four reasons that fire here, and no others"
    );
}

#[test]
fn a_renderer_dll_lying_in_the_directory_is_never_renderer_evidence_for_a_binary() {
    // Every executable in a folder shares its neighbours, so a `d3d12core.dll`
    // sitting there says exactly as much about the crash handler as about the
    // game. Treating it as an import would hand both the strongest signal this
    // module has and leave nothing to tell them apart.
    let mut observed = at("Game.exe");
    observed.own = shipping(&["d3d12core.dll"]);

    let candidate = assess(&observed, None, None);

    assert!(
        !candidate
            .reasons
            .iter()
            .any(|r| matches!(r, Reason::LinksRenderer { .. })),
        "a neighbouring file is not an import, got {:?}",
        names(&candidate)
    );
}

#[test]
fn a_load_time_import_outranks_the_same_renderer_delay_loaded() {
    // Both are real. One cannot be avoided; the other may never be reached.
    let mut eager = at("eager.exe");
    eager.own = importing(&["d3d11.dll"]);
    let mut lazy = at("lazy.exe");
    lazy.own = delay_loading(&["d3d11.dll"]);

    let eager = assess(&eager, None, None);
    let lazy = assess(&lazy, None, None);

    assert!(
        eager.score() > lazy.score(),
        "import {} must beat delay-import {}",
        eager.score(),
        lazy.score()
    );
}

#[test]
fn a_renderer_reached_twice_over_is_still_one_capability_and_scores_once() {
    // A binary that imports `d3d11.dll` *and* ships an engine DLL that imports
    // it too has one renderer, observed twice. Scoring both would let a
    // well-stocked directory outrank a game on the strength of one fact
    // counted again.
    let mut observed = at("Game.exe");
    observed.own = linking(&["d3d11.dll"], &[("Engine.dll", &["d3d11.dll"])]);

    let candidate = assess(&observed, None, None);

    assert_eq!(
        candidate.score(),
        weight::RENDERER_IMPORT,
        "one renderer reason only, got {:?}",
        names(&candidate)
    );
}

#[test]
fn a_stub_that_reaches_a_renderer_only_through_a_local_library_still_says_so() {
    // This is the Unity shape, and the reason the indirect signal exists at
    // all: `Game.exe` imports `UnityPlayer.dll` and nothing graphical, and it
    // is `UnityPlayer.dll` that imports `d3d11.dll`. Without this the strongest
    // signal in the module is blind to an entire engine.
    let mut observed = at("Game.exe");
    observed.own = linking(&[], &[("UnityPlayer.dll", &["d3d11.dll"])]);

    let candidate = assess(&observed, None, None);

    assert_eq!(candidate.score(), weight::RENDERER_INDIRECT);
    assert!(
        names(&candidate)[0].contains("through UnityPlayer.dll"),
        "the library that reached it is named, got {:?}",
        names(&candidate)
    );
}

#[test]
fn several_local_libraries_reaching_one_renderer_are_merged_into_one_reason() {
    // Three engine DLLs that each import `d3d11.dll` are one capability seen
    // three times. One reason per library would let the size of a directory
    // decide the ranking.
    let mut observed = at("Game.exe");
    observed.own = linking(&[], &[("b.dll", &["d3d11.dll"]), ("a.dll", &["d3d11.dll"])]);

    let candidate = assess(&observed, None, None);

    assert_eq!(candidate.reasons.len(), 1, "got {:?}", names(&candidate));
    assert_eq!(candidate.score(), weight::RENDERER_INDIRECT);
    assert!(
        names(&candidate)[0].contains("a.dll, b.dll"),
        "both libraries named, sorted, got {:?}",
        names(&candidate)
    );
}

#[test]
fn the_unity_data_directory_has_to_be_named_after_this_executable_and_no_other() {
    // `Game_Data` beside `UnityCrashHandler64.exe` is the game's data
    // directory, not the crash handler's. The pairing is the whole signal; a
    // match on any `_Data` directory in the folder would hand it to every
    // binary there and break the one tie it exists to break.
    let siblings = vec!["Game_Data".to_owned()];

    let mut game = at("Game.exe");
    game.sibling_directories.clone_from(&siblings);
    let mut handler = at("UnityCrashHandler64.exe");
    handler.sibling_directories = siblings;

    assert_eq!(assess(&game, None, None).score(), weight::UNITY_DATA);
    assert_eq!(
        assess(&handler, None, None).score(),
        0,
        "the handler owns none of it"
    );
}

#[test]
fn the_data_directory_is_matched_without_regard_to_case() {
    // The pairing survives a copy onto a case-preserving filesystem; a
    // case-sensitive match would quietly lose it there.
    let mut observed = at("MyGame.exe");
    observed.sibling_directories = vec!["mygame_data".to_owned()];

    assert_eq!(assess(&observed, None, None).score(), weight::UNITY_DATA);
}

#[test]
fn unreals_engine_tools_do_not_get_the_layout_reason_the_game_gets() {
    // `Engine/Binaries/Win64` holds the crash reporter and the rest of the
    // engine's own tooling. The game's `Binaries` is a sibling of `Engine`,
    // never a child of it, so the two are told apart structurally rather than
    // by knowing the names of Epic's executables.
    let game = at("FactoryGame/Binaries/Win64/FactoryGame-Win64-Shipping.exe");
    let tool = at("Engine/Binaries/Win64/CrashReportClient.exe");

    assert!(
        assess(&game, None, None)
            .reasons
            .iter()
            .any(|r| matches!(r, Reason::UnrealBinariesDirectory { .. })),
        "the game's own Binaries directory counts"
    );
    assert!(
        !assess(&tool, None, None)
            .reasons
            .iter()
            .any(|r| matches!(r, Reason::UnrealBinariesDirectory { .. })),
        "the engine's does not"
    );
}

#[test]
fn the_layout_rules_read_the_path_below_the_surveyed_directory_and_not_above_it() {
    // Pointing the tool at something that happens to live under a folder
    // called `Binaries` must not hand every executable in the tree a reason it
    // did not earn.
    let mut observed = at("thing.exe");
    observed.path = PathBuf::from("/opt/Binaries/Win64/thing.exe");

    assert_eq!(
        assess(&observed, None, None).score(),
        0,
        "only the relative path is layout evidence"
    );
}

#[test]
fn the_shipping_suffix_is_read_off_the_end_of_the_stem_only() {
    // `Shipping-Tool.exe` is not a shipping build, and a `contains` would say
    // it was.
    let yes = at("Game-Win64-Shipping.exe");
    let no = at("Shipping-Tool.exe");

    assert!(
        assess(&yes, None, None)
            .reasons
            .contains(&Reason::ShippingSuffix)
    );
    assert!(
        !assess(&no, None, None)
            .reasons
            .contains(&Reason::ShippingSuffix)
    );
}

#[test]
fn a_name_from_the_manifest_outranks_the_same_name_taken_off_the_directory() {
    // The manifest is evidence from somewhere else; the directory is the same
    // install talking about itself. They are worth different amounts, and a
    // reader should be able to see which one was used.
    let observed = at("Cyberpunk2077.exe");

    let supplied = assess(&observed, Some("Cyberpunk 2077"), Some("Cyberpunk 2077"));

    assert!(
        matches!(
            supplied.reasons.first(),
            Some(Reason::NameResembles {
                how: Resemblance::Exact,
                from: NameFrom::Supplied,
                ..
            })
        ),
        "the supplied name wins the tie, got {:?}",
        names(&supplied)
    );
}

#[test]
fn a_stem_and_a_title_that_are_the_same_words_match_through_the_punctuation() {
    // "Cyberpunk 2077" and `Cyberpunk2077.exe` are one name written twice. A
    // comparison that does not drop the space finds nothing.
    let exact = assess(&at("Cyberpunk2077.exe"), Some("Cyberpunk 2077"), None);
    let partial = assess(&at("Game-Win64-Shipping.exe"), Some("Game"), None);
    let neither = assess(&at("vcredist_x64.exe"), Some("Cyberpunk 2077"), None);
    // Found anywhere but the beginning, a short stem matches by accident:
    // `Skyrim Special Edition` ends with the word `edit`.
    let inside = assess(&at("edit.exe"), Some("Skyrim Special Edition"), None);
    assert!(
        !inside
            .reasons
            .iter()
            .any(|r| matches!(r, Reason::NameResembles { .. })),
        "an interior substring is a coincidence, got {:?}",
        names(&inside)
    );

    assert!(
        exact.reasons.contains(&Reason::NameResembles {
            name: "Cyberpunk 2077".to_owned(),
            how: Resemblance::Exact,
            from: NameFrom::Supplied,
        }),
        "got {:?}",
        names(&exact)
    );
    assert!(
        partial.reasons.iter().any(|r| matches!(
            r,
            Reason::NameResembles {
                how: Resemblance::Partial,
                ..
            }
        )),
        "a title inside a longer stem is a partial match, got {:?}",
        names(&partial)
    );
    assert!(
        !neither
            .reasons
            .iter()
            .any(|r| matches!(r, Reason::NameResembles { .. })),
        "nothing in common is nothing, got {:?}",
        names(&neither)
    );
}

#[test]
fn a_directory_full_of_upscalers_says_the_same_thing_about_everything_in_it() {
    // Which is why it is the weakest reason in the table. It cannot separate
    // two binaries in one folder, and the ranking must not let it try.
    let libraries = shipping(&["nvngx_dlss.dll", "sl.interposer.dll"]);

    let mut game = at("Game.exe");
    game.directory = libraries.clone();
    game.sibling_directories = vec!["Game_Data".to_owned()];
    let mut handler = at("UnityCrashHandler64.exe");
    handler.directory = libraries;

    let game = assess(&game, None, None);
    let handler = assess(&handler, None, None);

    assert_eq!(
        handler.score(),
        weight::SHIPS_GRAPHICS_LIBRARIES,
        "the handler gets the directory's credit too, got {:?}",
        names(&handler)
    );
    assert!(
        game.score() > handler.score(),
        "and it is nowhere near enough to matter: {} against {}",
        game.score(),
        handler.score()
    );
}

#[test]
fn a_mod_loader_in_the_directory_is_not_counted_as_a_graphics_library() {
    // A `dxgi.dll` next to a game is ReShade or SpecialK. Which executable it
    // was installed for is exactly the question it does not answer.
    let mut observed = at("Game.exe");
    observed.directory = shipping(&["dxgi.dll"]);

    assert_eq!(
        assess(&observed, None, None).score(),
        0,
        "a proxy DLL votes for nobody"
    );
}

#[test]
fn the_reasons_under_a_candidate_are_printed_strongest_first() {
    // The first line is the one that gets read. It should be the one that
    // decided the ranking, not whichever rule happened to run first.
    let mut observed = at("Game/Binaries/Win64/Game-Win64-Shipping.exe");
    observed.own = importing(&["d3d12.dll"]);
    observed.directory = shipping(&["nvngx_dlss.dll"]);

    let candidate = assess(&observed, Some("Game"), None);
    let weights: Vec<i32> = candidate.reasons.iter().map(Reason::weight).collect();

    assert!(
        weights.windows(2).all(|w| w[0] >= w[1]),
        "reasons must descend, got {weights:?} for {:?}",
        names(&candidate)
    );
}

#[test]
fn an_executable_with_nothing_to_say_scores_zero_and_admits_it() {
    // This is how `vcredist_x64.exe` sinks: on its own merits, with no list of
    // names anywhere deciding it. The empty explanation is the report.
    let candidate = assess(&at("vcredist_x64.exe"), Some("Dota 2"), Some("dota 2 beta"));

    assert_eq!(candidate.score(), 0);
    assert!(!candidate.has_evidence());
    assert!(candidate.reasons.is_empty(), "{:?}", names(&candidate));
}

#[test]
fn a_ranking_orders_by_score_and_breaks_every_tie_the_same_way_twice() {
    // Two scans of one install have to be diffable against each other, and
    // `read_dir` promises nothing about order.
    let one = Candidate {
        path: PathBuf::from("b.exe"),
        reasons: vec![Reason::ShippingSuffix],
    };
    let two = Candidate {
        path: PathBuf::from("a.exe"),
        reasons: vec![Reason::ShippingSuffix],
    };
    let three = Candidate {
        path: PathBuf::from("a/big.exe"),
        reasons: vec![Reason::LinksRenderer {
            apis: vec!["Direct3D 12".to_owned()],
            source: Source::Import,
        }],
    };

    let survey = Survey::ranked(vec![one, two.clone(), three], Vec::new());
    let order: Vec<String> = survey
        .candidates
        .iter()
        .map(|c| c.path.display().to_string())
        .collect();

    assert_eq!(
        order,
        ["a/big.exe", "a.exe", "b.exe"],
        "score first, then the shallower path, then the alphabetical one"
    );
    assert_eq!(
        Survey::ranked(vec![two], Vec::new())
            .best()
            .map(|c| &c.path),
        Some(&PathBuf::from("a.exe"))
    );
}

#[test]
fn candidates_the_evidence_cannot_separate_are_ordered_by_how_deep_they_sit() {
    // A tie-break, not a signal: it scores nothing and shows up in no
    // explanation. It exists because sorting on the path alone put a launcher
    // at the install root below a redistributable four directories down, for
    // no reason a reader could see in the output.
    let unevidenced = |path: &str| Candidate {
        path: PathBuf::from(path),
        reasons: Vec::new(),
    };
    let survey = Survey::ranked(
        vec![
            unevidenced("Engine/Binaries/Win64/CrashReportClient.exe"),
            unevidenced("_CommonRedist/vcredist/vcredist_x64.exe"),
            unevidenced("Launcher.exe"),
        ],
        Vec::new(),
    );

    assert_eq!(
        survey
            .candidates
            .first()
            .map(|c| c.path.display().to_string()),
        Some("Launcher.exe".to_owned()),
        "got {:?}",
        survey
            .candidates
            .iter()
            .map(|c| c.path.display().to_string())
            .collect::<Vec<_>>()
    );
    assert!(
        survey.candidates.iter().all(|c| c.score() == 0),
        "and none of them gained a point for it"
    );
}

#[test]
fn a_survey_of_nothing_but_redistributables_reports_that_it_found_no_evidence() {
    // Different from an empty directory, and the difference is what a caller
    // needs in order to say something true about it.
    let survey = Survey::ranked(
        vec![Candidate {
            path: PathBuf::from("vcredist_x64.exe"),
            reasons: Vec::new(),
        }],
        Vec::new(),
    );

    assert!(!survey.has_evidence());
    assert!(
        survey.best().is_some(),
        "the executable is still reported; it is simply not evidenced"
    );
}

#[test]
fn a_truncation_counts_whether_or_not_the_directory_looked_like_a_game() {
    // One question, not two, and this is the test that says why it cannot be
    // two. Narrowing a listing's status to entries that carry evidence was
    // tried, to stop Proton builds and container sysroots costing an exit code
    // on a healthy machine. It is unsound: the narrowing is only ever consulted
    // when the walk already truncated, and a truncated walk is itself an
    // explanation for finding no evidence — the shipping binary may be the part
    // that was not read. So the zero-evidence case below is the one most likely
    // to be a game this tool lost, and the last one to call complete.
    let truncation = || {
        vec![Note::DepthLimited {
            limit: 32,
            skipped: 3,
        }]
    };
    let evidenced = Survey::ranked(
        vec![Candidate {
            path: PathBuf::from("Game-Win64-Shipping.exe"),
            reasons: vec![Reason::ShippingSuffix],
        }],
        truncation(),
    );
    let unevidenced = Survey::ranked(
        vec![Candidate {
            path: PathBuf::from("launcher.exe"),
            reasons: Vec::new(),
        }],
        truncation(),
    );

    assert!(
        evidenced.is_incomplete(),
        "a game whose directory was only partly read is an incomplete answer"
    );
    assert!(
        unevidenced.is_incomplete(),
        "and so is a truncated walk that found nothing worth reporting, because          the truncation is why it found nothing"
    );
    assert_eq!(
        unevidenced.incomplete_notes().count(),
        1,
        "the note itself is what every surface prints from"
    );

    let read_in_full = Survey::ranked(
        vec![Candidate {
            path: PathBuf::from("launcher.exe"),
            reasons: Vec::new(),
        }],
        Vec::new(),
    );
    assert!(
        !read_in_full.is_incomplete(),
        "and a directory read in full is complete however little it carries"
    );
}

#[test]
fn a_truncated_walk_says_that_something_is_missing_rather_than_that_it_is_done() {
    // The wrong answer this whole module is shaped to avoid is a short list
    // that looks complete, so every limit has to render into a sentence a
    // reader cannot mistake for a clean result.
    let sentences = [
        Note::DepthLimited {
            limit: 8,
            skipped: 3,
        }
        .to_string(),
        Note::DirectoryLimited { limit: 4096 }.to_string(),
        Note::ExecutableLimited { limit: 512 }.to_string(),
    ];

    assert!(
        [
            Note::DepthLimited {
                limit: 8,
                skipped: 1
            },
            Note::DirectoryLimited { limit: 4096 },
            Note::ExecutableLimited { limit: 512 },
        ]
        .iter()
        .all(Note::is_incomplete),
        "a limit means files were not read, which is what an exit code answers"
    );
    assert!(
        !Note::NoRendererImported { executables: 3 }.is_incomplete(),
        "everything was read; the answer is thin, not missing"
    );

    for sentence in sentences {
        assert!(
            sentence.contains("not searched") || sentence.contains("not ranked"),
            "a limit has to say what was skipped, got {sentence:?}"
        );
        assert!(
            sentence.contains("missing from this list") || sentence.contains("may not be in this"),
            "and that the answer may be the one that is gone, got {sentence:?}"
        );
    }
}

#[test]
fn a_ranking_with_no_import_table_under_it_says_so_before_anybody_quotes_it() {
    // A Java, Electron or .NET game imports no graphics API at all: the runtime
    // loads the renderer with LoadLibrary after startup, where no import table
    // can see it. The structural evidence still ranks these, and the ranking is
    // still worth having, but it is weaker than one an import table backs and
    // must not be printed as though it were the same thing.
    let mut launcher = at("ProjectZomboid64.exe");
    launcher.sibling_directories = vec!["jre64".to_owned()];

    let survey = Survey::ranked(vec![assess(&launcher, None, None)], Vec::new());

    let note = survey
        .notes
        .iter()
        .find(|n| matches!(n, Note::NoRendererImported { .. }))
        .map(ToString::to_string)
        .expect("a structure-only ranking has to declare itself");
    assert!(
        note.contains("structure alone"),
        "the caveat has to be in the sentence, got {note:?}"
    );
    assert!(
        note.contains("Java, Electron or .NET"),
        "and so does what it usually means, which is the useful half, got {note:?}"
    );
}

#[test]
fn the_caveat_is_absent_the_moment_one_executable_reaches_a_renderer() {
    // The other half of the same promise. A note that appears on every survey
    // is a note nobody reads, and this one has to keep its meaning for the
    // directories where it is true.
    let mut game = at("Game.exe");
    game.own = importing(&["d3d11.dll"]);
    let quiet = at("UnityCrashHandler64.exe");

    let survey = Survey::ranked(
        vec![assess(&game, None, None), assess(&quiet, None, None)],
        Vec::new(),
    );

    assert!(
        !survey
            .notes
            .iter()
            .any(|n| matches!(n, Note::NoRendererImported { .. })),
        "one binary reaching a renderer is enough to make the ranking an ordinary one"
    );
}

#[test]
fn a_renderer_reached_only_through_a_local_library_still_counts_as_an_import_table() {
    // The caveat is about import tables being silent, and following one link
    // into a library beside the executable is still reading an import table.
    // Raising it here would call a Unity game a structure-only guess.
    let mut observed = at("Game.exe");
    observed.own = linking(&[], &[("UnityPlayer.dll", &["d3d11.dll"])]);

    let survey = Survey::ranked(vec![assess(&observed, None, None)], Vec::new());

    assert!(
        !survey
            .notes
            .iter()
            .any(|n| matches!(n, Note::NoRendererImported { .. })),
        "one link away is still evidence from a table, not from a layout"
    );
}

#[test]
fn two_builds_of_one_game_side_by_side_are_both_reported_rather_than_one_chosen() {
    // Project Zomboid ships a 32-bit and a 64-bit build in the same directory.
    // Both are real, both are launchable, and there is no evidence anywhere
    // that says which one the user wants. Picking would be a coin toss printed
    // as an answer.
    let mut wide = at("ProjectZomboid64.exe");
    wide.sibling_directories = vec!["jre64".to_owned(), "media".to_owned()];
    let mut narrow = at("ProjectZomboid32.exe");
    narrow
        .sibling_directories
        .clone_from(&wide.sibling_directories);

    let survey = Survey::ranked(
        vec![
            assess(&wide, Some("Project Zomboid"), None),
            assess(&narrow, Some("Project Zomboid"), None),
        ],
        Vec::new(),
    );

    let order: Vec<String> = survey
        .candidates
        .iter()
        .map(|c| c.path.display().to_string())
        .collect();
    assert_eq!(
        order,
        ["ProjectZomboid32.exe", "ProjectZomboid64.exe"],
        "both survive, and the tie breaks by path so it breaks the same way twice"
    );
    assert_eq!(
        survey.candidates[0].score(),
        survey.candidates[1].score(),
        "nothing observed separates them, and the scores must not pretend otherwise"
    );
    assert!(
        survey.candidates.iter().all(Candidate::has_evidence),
        "the name evidence is real for both, got {:?}",
        survey.candidates.iter().map(names).collect::<Vec<_>>()
    );
}

#[test]
fn two_candidates_sharing_the_top_score_are_reported_as_a_tie() {
    // Printing a list best-first is itself a claim. When the top two scores are
    // equal, whichever is printed first got there by the presentation rule, and
    // a reader has no way to tell that from a result the evidence separated.
    // This is the exact case where "rank, do not pick" degrades into picking.
    let mut one = at("alpha.exe");
    one.own = importing(&["d3d11.dll"]);
    let mut two = at("zulu.exe");
    two.own = importing(&["d3d11.dll"]);

    let survey = Survey::ranked(
        vec![assess(&one, None, None), assess(&two, None, None)],
        Vec::new(),
    );

    let note = survey
        .notes
        .iter()
        .find(|n| {
            matches!(
                n,
                Note::TiedAtTheTop {
                    count: 2,
                    score: 100
                }
            )
        })
        .map(ToString::to_string)
        .expect("a tie at the top has to be declared");
    assert!(
        note.contains("does not choose between them"),
        "the tie is stated, got {note:?}"
    );
    assert!(
        note.contains("presentation and not a finding"),
        "and so is the fact that the order was arbitrary, got {note:?}"
    );
}

#[test]
fn a_tie_is_reported_without_any_idea_of_what_caused_it() {
    // Two builds of one game tie because both are real; a game ties with an
    // Electron application or a security product because importing `d3d11.dll`
    // is ordinary behaviour for a great deal of software that is not a game. A
    // rule that had to tell those apart would one day meet a third cause it did
    // not recognise, so there is one rule and one sentence.
    let mut game = at("game.exe");
    game.own = importing(&["d3d11.dll"]);
    let mut stranger = at("SomeCorporateThing.exe");
    stranger.own = importing(&["d3d9.dll", "d3d11.dll", "dxgi.dll"]);

    let strangers = Survey::ranked(
        vec![assess(&game, None, None), assess(&stranger, None, None)],
        Vec::new(),
    );

    let mut wide = at("Build64.exe");
    wide.sibling_directories = vec!["jre64".to_owned()];
    let mut narrow = at("Build32.exe");
    narrow
        .sibling_directories
        .clone_from(&wide.sibling_directories);
    let builds = Survey::ranked(
        vec![
            assess(&wide, Some("Build"), None),
            assess(&narrow, Some("Build"), None),
        ],
        Vec::new(),
    );

    // Compared with the digits taken out, because the scores differ and
    // nothing else may: a tie between a game and a stranger and a tie between
    // two builds of one game have to read identically, or the wording is
    // leaking a judgement about the cause that nothing here can support.
    let without_numbers = |text: String| {
        let mut out = String::new();
        let mut digits = false;
        for c in text.chars() {
            if c.is_ascii_digit() {
                if !digits {
                    out.push('#');
                }
            } else {
                out.push(c);
            }
            digits = c.is_ascii_digit();
        }
        out
    };
    let shape = |survey: &Survey| {
        survey
            .notes
            .iter()
            .find(|n| matches!(n, Note::TiedAtTheTop { .. }))
            .map(|n| without_numbers(n.to_string()))
    };

    assert!(shape(&strangers).is_some(), "the tie is raised at all");
    assert_eq!(
        shape(&strangers),
        shape(&builds),
        "one tie, one sentence, whatever produced it"
    );
}

#[test]
fn a_clear_winner_is_not_accused_of_being_a_tie() {
    // A caveat printed on every run is a caveat nobody reads, and this one has
    // to keep its meaning for the rankings where it is true.
    let mut best = at("Game/Binaries/Win64/Game-Win64-Shipping.exe");
    best.own = importing(&["d3d11.dll"]);
    let mut other = at("other.exe");
    other.own = importing(&["d3d11.dll"]);

    let survey = Survey::ranked(
        vec![assess(&best, None, None), assess(&other, None, None)],
        Vec::new(),
    );

    assert!(
        !survey
            .notes
            .iter()
            .any(|n| matches!(n, Note::TiedAtTheTop { .. })),
        "the layout separated them, got {:?}",
        survey
            .candidates
            .iter()
            .map(|c| (c.path.display().to_string(), c.score()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_directory_where_everything_ties_at_nothing_gets_the_better_sentence_instead() {
    // Every candidate in a folder of installers ties at zero. Saying "five
    // things tie at the top" there would be true and useless; `has_evidence`
    // already gives the caller something worth printing.
    let survey = Survey::ranked(
        vec![
            Candidate {
                path: PathBuf::from("DXSETUP.exe"),
                reasons: Vec::new(),
            },
            Candidate {
                path: PathBuf::from("vcredist_x64.exe"),
                reasons: Vec::new(),
            },
        ],
        Vec::new(),
    );

    assert!(!survey.has_evidence());
    assert!(
        !survey
            .notes
            .iter()
            .any(|n| matches!(n, Note::TiedAtTheTop { .. })),
        "a tie at nothing is not the interesting fact about this directory"
    );
}

#[test]
fn a_tie_means_everything_was_read_and_does_not_fail_the_run() {
    // It is the same kind of statement as `NoRendererImported`: the answer is
    // weaker than it looks, not missing. Nothing went unread.
    assert!(
        !Note::TiedAtTheTop {
            count: 2,
            score: 100
        }
        .is_incomplete()
    );
}

#[test]
fn a_name_can_never_outweigh_where_a_file_sits() {
    // `PUBG: BATTLEGROUNDS` ships `TslGame.exe`. A ranking that let the title
    // carry much weight would rank a launcher called `PUBG.exe` above the game
    // itself, and the absence of the signal has to cost the real binary
    // nothing.
    const {
        assert!(
            weight::NAME_EXACT < weight::UNREAL_BINARIES,
            "layout beats a name"
        );
        assert!(
            weight::NAME_EXACT < weight::SHIPPING_SUFFIX,
            "so does a packaging convention"
        );
    }

    let mut real = at("TslGame/Binaries/Win64/TslGame-Win64-Shipping.exe");
    real.own = importing(&["d3d11.dll"]);
    let launcher = at("PUBG.exe");

    let survey = Survey::ranked(
        vec![
            assess(&real, Some("PUBG: BATTLEGROUNDS"), None),
            assess(&launcher, Some("PUBG: BATTLEGROUNDS"), None),
        ],
        Vec::new(),
    );

    assert_eq!(
        survey.best().map(|c| c.path.display().to_string()),
        Some("TslGame/Binaries/Win64/TslGame-Win64-Shipping.exe".to_owned()),
        "got {:?}",
        survey
            .candidates
            .iter()
            .map(|c| (c.path.display().to_string(), c.score()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_renderer_reason_names_every_api_rather_than_choosing_between_them() {
    // The same promise `Verdict::headline` makes: a binary that links both and
    // decides at run time is not a binary that links one.
    let mut observed = at("Game.exe");
    observed.own = importing(&["d3d12.dll", "vulkan-1.dll"]);

    let candidate = assess(&observed, None, None);

    assert_eq!(names(&candidate)[0], "links Direct3D 12 or Vulkan (import)");
    assert_eq!(
        candidate.score(),
        weight::RENDERER_IMPORT,
        "two APIs are one reason, not two"
    );
}

/// One of every shape of [`Reason`] that `assess` can emit.
///
/// The `match` at the end is the point of the function: a new variant stops
/// compiling here until somebody acknowledges it, and the acknowledgement is
/// next to the list it also has to be added to.
fn every_reason_assess_can_build() -> Vec<Reason> {
    let all = vec![
        // `Source::Linked` and `Source::Neighbour` are dropped by
        // `links_renderer`, which is what
        // `a_renderer_dll_lying_in_the_directory_is_never_renderer_evidence_for_a_binary`
        // holds down. These two are the sources that reach this variant.
        Reason::LinksRenderer {
            apis: vec!["Direct3D 12".to_owned()],
            source: Source::Import,
        },
        Reason::LinksRenderer {
            apis: vec!["Direct3D 12".to_owned()],
            source: Source::DelayImport,
        },
        Reason::LinksRendererThrough {
            libraries: vec!["UnityPlayer.dll".to_owned()],
            apis: vec!["Direct3D 11".to_owned()],
        },
        Reason::UnityDataDirectory {
            directory: "Game_Data".to_owned(),
        },
        Reason::UnrealBinariesDirectory {
            directory: "Binaries/Win64".to_owned(),
        },
        Reason::ShippingSuffix,
        Reason::NameResembles {
            name: "Game".to_owned(),
            how: Resemblance::Exact,
            from: NameFrom::Supplied,
        },
        Reason::NameResembles {
            name: "Game".to_owned(),
            how: Resemblance::Partial,
            from: NameFrom::Directory,
        },
        Reason::ShipsGraphicsLibraries {
            libraries: vec!["d3d12core.dll".to_owned()],
        },
    ];
    for reason in &all {
        match reason {
            Reason::LinksRenderer { .. }
            | Reason::LinksRendererThrough { .. }
            | Reason::UnityDataDirectory { .. }
            | Reason::UnrealBinariesDirectory { .. }
            | Reason::ShippingSuffix
            | Reason::NameResembles { .. }
            | Reason::ShipsGraphicsLibraries { .. } => {}
        }
    }
    all
}

#[test]
fn every_reason_assess_can_build_is_worth_more_than_nothing() {
    // The premise the `weight` module documents, and the one the TUI's two
    // separate evidence questions lean on: a score of zero and an empty list of
    // reasons have to be the same state. A reason worth nothing would make a
    // scored candidate that observed something look exactly like one that
    // observed nothing, and the row and the pane under it would then disagree
    // on screen.
    for reason in every_reason_assess_can_build() {
        let candidate = Candidate {
            path: PathBuf::from("Game.exe"),
            reasons: vec![reason.clone()],
        };
        assert!(
            candidate.has_evidence(),
            "{} is a reason, so it is evidence",
            reason.kind()
        );
        assert!(
            candidate.score() > 0,
            "{} is worth {}, so a candidate carrying only it scores zero while saying it observed something",
            reason.kind(),
            reason.weight()
        );
    }
}

#[test]
fn a_candidate_scores_zero_only_when_it_observed_nothing() {
    // The equivalence itself, over every combination of one reason and every
    // other: no pair cancels out either.
    let all = every_reason_assess_can_build();
    assert_eq!(
        Candidate {
            path: PathBuf::from("vcredist_x64.exe"),
            reasons: Vec::new(),
        }
        .score(),
        0,
        "nothing observed is the only way to score nothing"
    );
    for first in &all {
        for second in &all {
            let candidate = Candidate {
                path: PathBuf::from("Game.exe"),
                reasons: vec![first.clone(), second.clone()],
            };
            assert_eq!(
                candidate.score() == 0,
                !candidate.has_evidence(),
                "{} and {} together score {}",
                first.kind(),
                second.kind(),
                candidate.score()
            );
        }
    }
}
