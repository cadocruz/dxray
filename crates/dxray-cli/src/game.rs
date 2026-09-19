//! The `game` listing: which executable in an install directory is the game.
//!
//! The judgement is `dxray-core`'s; this module only lays it out. It prints the
//! whole ranking and then analyses the top of it, in that order, because the
//! ranking is the part a reader has to be able to disagree with. A tool that
//! printed the analysis alone would be naming one executable and hiding the
//! three it beat.
//!
//! **Nothing here has been run against a real game install.** There was none on
//! the machine it was written on. The layouts it handles correctly are the ones
//! in the tests, which were written from documented engine conventions; whether
//! those conventions hold across a library of shipped titles is exactly what
//! this module cannot tell you.

use std::fmt::Write as _;
use std::io;
use std::path::Path;

use dxray_core::game::{Candidate, Note, Survey};

use crate::record::{Record, push_quoted};
use crate::report;
use crate::wrap;

/// Width of the label column, matching the report block printed underneath so
/// the two read as one thing.
const LABEL: usize = 10;
/// Where a wrapped label's value continues.
const LABEL_INDENT: usize = 2 + LABEL;
/// Where a candidate's reasons start: under its path, not under its rank.
const REASON_INDENT: usize = 13;

/// What a directory produced, and whether it should move the exit code.
pub struct Outcome {
    pub text: String,
    /// True when something was not read: the directory would not list, an
    /// image would not parse, or the walk stopped at one of its bounds. A
    /// truncated walk **is** a failure here, because this command was pointed
    /// at one directory and a file in it went unlooked-at.
    ///
    /// A ranking with no import table behind it is not: everything was read and
    /// this is what it says, which is the same division `steam` draws between
    /// a problem and a caveat. Both commands ask
    /// [`Survey::is_incomplete`](dxray_core::game::Survey::is_incomplete) for
    /// the first half, so one directory cannot be complete on one surface and
    /// incomplete on the other.
    pub failed: bool,
}

/// Ranks the executables under `dir` and analyses the best of them.
pub fn inspect(dir: &Path, json: bool, presentation: report::Presentation) -> Outcome {
    let survey = match dxray_core::candidates(dir, None) {
        Ok(survey) => survey,
        Err(error) => return failure(dir, &error, json, presentation),
    };
    let Some(best) = survey.best() else {
        // The directory listed fine and holds no program at all. Treated as a
        // failure here, where the question asked was "which executable in this
        // directory is the game" and there is no executable to answer with. In
        // `steam`, which sweeps every install on the machine and meets games
        // that are mid-download, the same state is reported and costs nothing.
        return failure(
            dir,
            &"no executable found in this directory",
            json,
            presentation,
        );
    };

    let record = Record::read(&best.path);
    // A bounded walk that hit its bound did not read every file, and the exit
    // code has always answered exactly that question. It is also the only
    // signal a script gets: the note explaining that the real binary may be
    // below the limit is on stdout, where nothing reads it. The caveat that
    // everything was read and says little — `NoRendererImported` — is not a
    // failure and does not move it.
    let failed = record.error.is_some() || survey.is_incomplete();
    let text = if json {
        let mut line = json_line(dir, &survey, &record);
        line.push('\n');
        line
    } else if presentation != report::Presentation::Standard {
        let identity = if presentation == report::Presentation::Full {
            std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf())
        } else {
            dir.to_path_buf()
        };
        let mut text = format!("Install: {}\n", identity.display());
        if presentation == report::Presentation::Full {
            let absolute = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
            text.push_str(&ranking_paths(&absolute, &survey, true));
            let selected = std::path::absolute(&best.path).unwrap_or_else(|_| best.path.clone());
            labelled(&mut text, "selected", &selected.display().to_string());
            labelled(
                &mut text,
                "score",
                &format!("{}; {} candidates", best.score(), survey.candidates.len()),
            );
        } else {
            if !survey.has_evidence() {
                labelled(
                    &mut text,
                    "caveat",
                    "No executable carries evidence of being the game; selection is only the highest-ranked candidate.",
                );
            }
            for note in &survey.notes {
                labelled(&mut text, "caveat", &note.to_string());
            }
        }
        text.push_str(&report::present(&record, presentation));
        text
    } else {
        let mut text = ranking(dir, &survey);
        text.push('\n');
        text.push_str(&report::render(&record));
        text
    };
    Outcome { text, failed }
}

/// A directory that could not be surveyed at all, in whichever shape was asked
/// for. It stays a row rather than becoming a missing one, so a harness can
/// still line results up with inputs without counting.
fn failure(
    dir: &Path,
    error: &dyn std::fmt::Display,
    json: bool,
    presentation: report::Presentation,
) -> Outcome {
    let record = Record::failed(dir, error);
    let text = if json {
        let mut line = json_line(dir, &Survey::default(), &record);
        line.push('\n');
        line
    } else {
        report::present(&record, presentation)
    };
    Outcome { text, failed: true }
}

/// The ranked block: every executable, its score, and the sentences behind it.
fn ranking(dir: &Path, survey: &Survey) -> String {
    ranking_paths(dir, survey, false)
}

pub(crate) fn ranking_paths(dir: &Path, survey: &Survey, full_paths: bool) -> String {
    let mut out = String::with_capacity(512);
    let _ = writeln!(out, "{}", dir.display());
    labelled(
        &mut out,
        "ranked",
        &format!(
            "{} {}, best first; the block below analyses the first one",
            survey.candidates.len(),
            plural(survey.candidates.len(), "executable", "executables"),
        ),
    );

    for (i, candidate) in survey.candidates.iter().enumerate() {
        let shown = if full_paths {
            std::path::absolute(&candidate.path)
                .unwrap_or_else(|_| candidate.path.clone())
                .display()
                .to_string()
        } else {
            candidate
                .path
                .strip_prefix(dir)
                .unwrap_or(&candidate.path)
                .display()
                .to_string()
        };
        let rank = i + 1;
        let score = candidate.score();
        let _ = write!(out, "  {rank:>3} {score:>5}  ");
        wrap::prose(&mut out, REASON_INDENT, REASON_INDENT, &shown);
        reasons(&mut out, candidate);
    }

    // The notes last, under everything they qualify. Printed on stdout with the
    // listing rather than only on stderr, because a caveat that lives on the
    // other stream is the half that gets lost when the output is pasted
    // somewhere.
    if !survey.has_evidence() {
        // Said first and said plainly. Every line above it reads "nothing
        // observed", but the conclusion that follows from all of them together
        // — there is no game in this directory — is the one thing a reader
        // takes away, and the block printed underneath is an executable this
        // tool has no argument for. A directory of leftover installers looks
        // exactly like this, and so does a game this ranking cannot see.
        labelled(
            &mut out,
            "note",
            "no executable here carries any evidence of being the game. The block \
             below analyses the highest-ranked one because a path was asked about, \
             not because anything argues it is the game.",
        );
    }
    for note in &survey.notes {
        labelled(&mut out, "note", &note.to_string());
    }
    out
}

fn reasons(out: &mut String, candidate: &Candidate) {
    if candidate.reasons.is_empty() {
        // Said out loud rather than left as a blank. A score of zero under a
        // path with nothing under it reads as truncated output, and "nothing
        // observed" is the actual finding for a Visual C++ redistributable.
        for _ in 0..REASON_INDENT {
            out.push(' ');
        }
        out.push_str("nothing observed argues that this is the game\n");
        return;
    }
    for reason in &candidate.reasons {
        for _ in 0..REASON_INDENT {
            out.push(' ');
        }
        wrap::prose(out, REASON_INDENT, REASON_INDENT, &reason.to_string());
    }
}

fn labelled(out: &mut String, label: &str, value: &str) {
    let _ = write!(out, "  {label:<LABEL$}");
    wrap::prose(out, LABEL_INDENT, LABEL_INDENT, value);
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

/// One JSONL line: the best candidate's record, then the survey appended.
///
/// The thirteen keys of a record come through untouched and in their order —
/// the first eight of them are a frozen positional contract — and everything
/// this mode adds goes after them. A harness that knows nothing about `game`
/// reads such a line exactly as it reads any other.
fn json_line(dir: &Path, survey: &Survey, record: &Record) -> String {
    let mut out = record.to_json();
    // Reopen the object. `to_json` always ends in `}` and never pretty-prints.
    out.pop();

    out.push_str(",\"directory\":");
    push_quoted(&mut out, &dir.to_string_lossy());

    out.push_str(",\"candidates\":[");
    for (i, candidate) in survey.candidates.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"path\":");
        push_quoted(&mut out, &candidate.path.to_string_lossy());
        let _ = write!(out, ",\"score\":{},\"reasons\":[", candidate.score());
        for (j, reason) in candidate.reasons.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            // The kind is for a program and is stable; the sentence is for a
            // person and is free to be reworded. Both, so neither consumer has
            // to parse the other's.
            out.push_str("{\"kind\":");
            push_quoted(&mut out, reason.kind());
            let _ = write!(out, ",\"weight\":{},\"says\":", reason.weight());
            push_quoted(&mut out, &reason.to_string());
            out.push('}');
        }
        out.push_str("]}");
    }
    out.push(']');

    out.push_str(",\"survey_notes\":[");
    for (i, note) in survey.notes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_quoted(&mut out, &note.to_string());
    }
    out.push_str("]}");
    out
}

/// What one game's directory produced for the `installed` and `steam` listing.
///
/// The rows are what gets printed; `incomplete` is what the exit code is drawn
/// from; `carries_evidence` is what the listing orders by. They come back
/// together so that a caller cannot render the caveat and forget the status, or
/// the other way round — which is the exact divergence this type exists to
/// prevent.
pub struct Best {
    pub rows: Vec<(&'static str, String)>,
    /// One entry per reason this game's directory was not searched in full,
    /// each naming the directory. Empty is the normal case.
    pub incomplete: Vec<String>,
    /// Whether anything in this install argues it is a game, exactly as
    /// [`Survey::has_evidence`] answered for it, or `None` when the install
    /// directory could not be read and the question was never put.
    ///
    /// Carried out rather than consumed privately. The rows below already turn
    /// on this answer — one of them says in words that nothing here carries
    /// evidence — and the listing needs the same answer to decide where the
    /// game goes. Computing it once and handing it over is what stops the
    /// listing re-deriving it from the wording of a row, which is a second
    /// place to be wrong about one question.
    pub carries_evidence: Option<bool>,
}

impl Best {
    /// True only when this install was read and nothing in it argues it is a
    /// game, so the listing puts it under the ones that argue something.
    ///
    /// The rule is [`dxray_core::inspect::lacks_evidence`], shared with the
    /// terminal browser's `no evidence` marker, so the two surfaces cannot end
    /// up demoting different sets of installs.
    #[must_use]
    pub fn lacks_evidence(&self) -> bool {
        dxray_core::inspect::lacks_evidence(self.carries_evidence)
    }
}

/// The listing's view of one game: the best candidate on one line, and any
/// caveat that would otherwise be lost.
///
/// Deliberately short. A library can hold a hundred games and the full ranking
/// for each of them would bury the listing, so this shows the answer and the
/// single strongest reason for it. `dxray game <path>` shows the rest.
///
/// The ranking arrives already done, from
/// [`dxray_core::inspect`](dxray_core::inspect()), rather than being asked for
/// here. This function used to ask for it itself and asked without the game's
/// title, while the terminal browser asked with it — so the two surfaces could
/// name different executables as the game in one directory. Taking the survey
/// as an argument is what makes that impossible rather than merely fixed.
///
/// The evidence answer leaves with the rows, in [`Best::carries_evidence`]. It
/// is needed here anyway — one of the rows below says in words that nothing in
/// this directory carries any — and the listing that orders by it would
/// otherwise have to ask a second time or read it back out of a sentence.
pub fn best_rows(install_dir: &Path, survey: io::Result<dxray_core::Survey>) -> Best {
    let survey = match survey {
        Ok(survey) => survey,
        // Named, not swallowed. A manifest for a game that is mid-download or
        // mid-removal points at a directory that is not there yet, and that is
        // a fact about the install worth printing.
        Err(error) => {
            let row = ("best", format!("(directory could not be read: {error})"));
            // Share the launcher's root-error policy with the terminal browser
            // while retaining the diagnostic even for a missing directory.
            let incomplete = if dxray_core::inspect::is_incomplete_error(&error) {
                vec![format!("{}: {error}", install_dir.display())]
            } else {
                Vec::new()
            };
            return Best {
                rows: vec![row],
                incomplete,
                // Nothing was read, so nothing can be said either way, and the
                // listing must not shuffle this down among the directories it
                // did read and found empty.
                carries_evidence: None,
            };
        }
    };
    // Asked once, of core, and then carried. Every use below is this value:
    // the sentence a person reads and the group the game is printed in are one
    // answer to one question, not two questions that happen to agree today.
    let carries_evidence = survey.has_evidence();
    let Some(best) = survey.best() else {
        return Best {
            rows: vec![("best", "(no executable in this directory)".to_owned())],
            incomplete: incomplete_notes(install_dir, &survey),
            carries_evidence: Some(carries_evidence),
        };
    };

    let shown = best
        .path
        .strip_prefix(install_dir)
        .unwrap_or(&best.path)
        .display();
    let mut rows = Vec::new();
    if carries_evidence {
        let why = best
            .reasons
            .first()
            .map_or_else(String::new, ToString::to_string);
        // The tie is not computed here. It arrives as `Note::TiedAtTheTop`,
        // from the same place `game` gets it, so the two surfaces cannot end
        // up disagreeing about whether one run had a tie in it.
        rows.push(("best", format!("{shown}  ({}: {why})", best.score())));
    } else {
        // The caveat cannot be left to the `game` view. A row reading
        // "vcredist_x64.exe (0: )" under a Steam title reads as an answer, and
        // the number is not what anybody takes away from it.
        rows.push((
            "best",
            format!("(nothing here carries evidence of being a game; the highest ranked of {} executables is {shown})",
                survey.candidates.len()),
        ));
    }

    for note in &survey.notes {
        // Every note, not a chosen few: the ones that say the walk stopped
        // early are the ones a short listing would most like to drop. Only the
        // one that fires on a large share of a library is shortened, and it is
        // shortened rather than dropped, with a pointer at the full sentence.
        let text = match note {
            Note::NoRendererImported { .. } => {
                "nothing here imports a graphics API, so this ranking rests on directory \
                 structure alone; dxray game on this path explains what that means"
                    .to_owned()
            }
            // Shortened for the same reason and never dropped. This is the one
            // line in a hundred-game listing that says the name beside it was
            // not chosen by the evidence, and a summary that omits it is the
            // omission the whole slice was built to refuse.
            // `saturating_sub`, not `- 1`. `count` is a public field of a
            // public enum, so its "always at least two" invariant is enforced
            // by `tie_at_the_top` and by nothing the compiler knows about. A
            // caller constructing the note by hand with a zero would panic in a
            // debug build and print 18446744073709551615 in a release one, and
            // the release half of that is exactly the shape this project
            // refuses: a wrong answer wearing the clothes of a right one.
            // Nothing reaches it today; this is what it costs to keep it that
            // way, and the field cannot be made private while the variant is.
            Note::TiedAtTheTop { count, .. } => format!(
                "tied with {} other{}, which game lists; the evidence does not choose \
                 between them",
                count.saturating_sub(1),
                if *count == 2 { "" } else { "s" }
            ),
            other => other.to_string(),
        };
        rows.push((label_for(note), text));
    }
    Best {
        incomplete: incomplete_notes(install_dir, &survey),
        rows,
        carries_evidence: Some(carries_evidence),
    }
}

/// The notes that mean something in this game's directory was never looked at.
///
/// The selection is [`Survey::incomplete_notes`], so that `game`, `steam`
/// and the terminal browser draw the line in the same place and a person cannot
/// get one story from one surface and a different one from the other. This
/// function only words them; it decides nothing.
fn incomplete_notes(install_dir: &Path, survey: &Survey) -> Vec<String> {
    survey
        .incomplete_notes()
        .map(|note| format!("{}: {note}", install_dir.display()))
        .collect()
}

fn label_for(note: &Note) -> &'static str {
    match note {
        Note::Unreadable { .. } | Note::Unparsed { .. } => "unread",
        _ => "note",
    }
}

#[cfg(test)]
mod tests {
    use super::{best_rows, json_line, ranking};
    use crate::record::Record;
    use dxray_core::game::{Candidate, Note, Reason, Resemblance, Survey};
    use dxray_core::{Source, analyse};
    use std::io;
    use std::path::{Path, PathBuf};

    fn survey() -> Survey {
        Survey::ranked(
            vec![
                Candidate {
                    path: PathBuf::from("/games/App/Binaries/Win64/App-Win64-Shipping.exe"),
                    reasons: vec![
                        Reason::LinksRenderer {
                            apis: vec!["Direct3D 12".to_owned()],
                            source: Source::Import,
                        },
                        Reason::ShippingSuffix,
                    ],
                },
                Candidate {
                    path: PathBuf::from("/games/App/vcredist_x64.exe"),
                    reasons: Vec::new(),
                },
            ],
            Vec::new(),
        )
    }

    fn record() -> Record {
        Record {
            path: "/games/App/Binaries/Win64/App-Win64-Shipping.exe".to_owned(),
            machine: Some("x86-64".to_owned()),
            bits: Some(64),
            imports: vec!["d3d12.dll".to_owned()],
            delay_imports: Vec::new(),
            file_version: None,
            product_version: None,
            error: None,
            verdict: analyse(&dxray_core::Evidence {
                imports: vec!["d3d12.dll".to_owned()],
                ..dxray_core::Evidence::default()
            }),
        }
    }

    #[test]
    fn the_listing_shows_every_candidate_and_not_only_the_one_it_would_analyse() {
        // The whole principle of the slice. Naming the winner and hiding the
        // executables it beat is lying by omission, and a reader has to be able
        // to see the one this tool got wrong.
        let text = ranking(Path::new("/games/App"), &survey());

        assert!(text.contains("App-Win64-Shipping.exe"), "got:\n{text}");
        assert!(
            text.contains("vcredist_x64.exe"),
            "the one that lost is printed too, got:\n{text}"
        );
    }

    #[test]
    fn every_score_is_printed_beside_the_sentences_that_add_up_to_it() {
        // A number with no explanation is something to believe rather than
        // something to check, which is the failure this project refuses.
        let text = ranking(Path::new("/games/App"), &survey());

        assert!(text.contains("150"), "the score is shown, got:\n{text}");
        assert!(
            text.contains("links Direct3D 12 (import)"),
            "and so is the evidence, got:\n{text}"
        );
        assert!(
            text.contains("its name ends \"-Shipping\""),
            "all of it, got:\n{text}"
        );
    }

    #[test]
    fn a_candidate_with_no_evidence_says_so_rather_than_printing_a_blank_line() {
        // An empty run of lines under a path reads as truncated output. "There
        // is nothing here that argues for this file" is the actual finding.
        let text = ranking(Path::new("/games/App"), &survey());

        assert!(
            text.contains("nothing observed argues that this is the game"),
            "got:\n{text}"
        );
    }

    #[test]
    fn the_ranking_is_printed_in_order_with_the_best_first() {
        // Order is the output. A listing that holds the right answer somewhere
        // in it is not a ranking.
        let text = ranking(Path::new("/games/App"), &survey());
        let shipping = text.find("App-Win64-Shipping.exe").expect("present");
        let redist = text.find("vcredist_x64.exe").expect("present");

        assert!(shipping < redist, "got:\n{text}");
    }

    #[test]
    fn a_ranking_with_no_import_table_behind_it_carries_the_caveat_on_stdout() {
        // Not only on stderr: a caveat on the other stream is the half that is
        // lost when somebody pastes the output into a bug report.
        let quiet = Survey::ranked(
            vec![Candidate {
                path: PathBuf::from("/games/App/ProjectZomboid64.exe"),
                reasons: vec![Reason::NameResembles {
                    name: "Project Zomboid".to_owned(),
                    how: Resemblance::Partial,
                    from: dxray_core::game::NameFrom::Supplied,
                }],
            }],
            Vec::new(),
        );

        let text = ranking(Path::new("/games/App"), &quiet);

        assert!(
            text.contains("structure alone"),
            "the caveat is in the listing, got:\n{text}"
        );
        assert!(
            text.contains("Java, Electron or .NET"),
            "including the half that is useful in its own right, got:\n{text}"
        );
    }

    #[test]
    fn a_directory_with_no_game_in_it_says_so_instead_of_leaving_it_to_be_inferred() {
        // Every candidate line reads "nothing observed", but the conclusion
        // that follows from all of them together is the part a reader takes
        // away — and the analysis block printed underneath would otherwise look
        // like an answer.
        let redist = Survey::ranked(
            vec![Candidate {
                path: PathBuf::from("/games/App/vcredist_x64.exe"),
                reasons: Vec::new(),
            }],
            Vec::new(),
        );

        let text = ranking(Path::new("/games/App"), &redist);

        assert!(
            text.contains("no executable here carries any evidence of being the game"),
            "got:\n{text}"
        );
        assert!(
            text.contains("not because anything argues it is the game"),
            "and it has to say what the block underneath is, got:\n{text}"
        );
    }

    #[test]
    fn the_no_game_caveat_stays_away_from_a_directory_that_has_one() {
        // A caveat printed on every run is a caveat nobody reads.
        let text = ranking(Path::new("/games/App"), &survey());

        assert!(
            !text.contains("no executable here carries any evidence"),
            "got:\n{text}"
        );
    }

    #[test]
    fn a_truncated_walk_is_reported_under_the_listing_it_qualifies() {
        let truncated = Survey::ranked(
            vec![Candidate {
                path: PathBuf::from("/games/App/top.exe"),
                reasons: vec![Reason::ShippingSuffix],
            }],
            vec![Note::DepthLimited {
                limit: 8,
                skipped: 4,
            }],
        );

        let text = ranking(Path::new("/games/App"), &truncated);

        assert!(text.contains("note"), "got:\n{text}");
        assert!(text.contains("not searched"), "got:\n{text}");
    }

    #[test]
    fn no_line_of_the_listing_runs_past_the_wrap_width() {
        // Unwrapped output is what turns a scan into something that has to be
        // piped through `less -S` to be read at all.
        let text = ranking(Path::new("/games/App"), &survey());

        assert!(text.lines().all(|l| l.len() <= 80), "got:\n{text}");
    }

    #[test]
    fn the_listing_gets_the_evidence_answer_back_instead_of_having_to_re_derive_it() {
        // It was computed here to word the row and then dropped, and the
        // listing orders by the same question. One answer, handed over.
        let found = best_rows(Path::new("/games/App"), Ok(survey()));
        let nothing = best_rows(
            Path::new("/games/App"),
            Ok(Survey::ranked(
                vec![Candidate {
                    path: PathBuf::from("/games/App/vcredist_x64.exe"),
                    reasons: Vec::new(),
                }],
                Vec::new(),
            )),
        );

        assert_eq!(found.carries_evidence, Some(true));
        assert!(!found.lacks_evidence(), "a game stays with the games");
        assert_eq!(nothing.carries_evidence, Some(false));
        assert!(
            nothing.lacks_evidence(),
            "and a directory of installers does not"
        );
    }

    #[test]
    fn a_tie_count_below_two_is_worded_oddly_rather_than_catastrophically() {
        // `Note::TiedAtTheTop::count` is a public field of a public enum, so
        // its "always at least two" invariant is a sentence in `dxray-core` and
        // nothing the compiler enforces. `count - 1` on a zero panics in a
        // debug build and prints 18446744073709551615 in a release one, and the
        // release half of that is the shape this project refuses: a wrong
        // answer dressed as a right one, in the row that exists to say the
        // evidence did not choose.
        //
        // Nothing constructs a zero today. This is what keeps that from being
        // the only thing standing between a reader and that number.
        let odd = best_rows(
            Path::new("/games/App"),
            Ok(Survey::ranked(
                vec![Candidate {
                    path: PathBuf::from("/games/App/App-Win64-Shipping.exe"),
                    reasons: vec![Reason::ShippingSuffix],
                }],
                vec![Note::TiedAtTheTop { count: 0, score: 1 }],
            )),
        );
        let rows = format!("{odd:?}", odd = odd.rows);

        assert!(
            rows.contains("tied with 0 other"),
            "an impossible count is worded plainly, got {rows}"
        );
        assert!(
            !rows.contains("18446744073709551615"),
            "and never as a number nobody can read, got {rows}"
        );
    }

    #[test]
    fn a_directory_that_could_not_be_read_is_not_a_directory_that_argued_nothing() {
        // An absent answer and an answer of "nothing" must not look alike. This
        // one was never asked, so it keeps its place beside the games and the
        // row says what happened to it.
        let unread = best_rows(
            Path::new("/games/Gone"),
            Err(io::Error::new(io::ErrorKind::NotFound, "no such directory")),
        );

        assert_eq!(unread.carries_evidence, None);
        assert!(
            !unread.lacks_evidence(),
            "an unanswered question is not an answer of nothing"
        );
        assert!(
            unread.rows[0].1.contains("could not be read"),
            "and it says so in the listing, got {:?}",
            unread.rows
        );
    }

    #[test]
    fn a_demoted_game_says_in_its_own_row_what_a_marker_would_have_said() {
        // Why this listing appends no ` no evidence` marker: the browser's list
        // column shows a headline, so it needs one; the column here is already
        // the sentence, and repeating it would be a second wording for one fact.
        let nothing = best_rows(
            Path::new("/games/App"),
            Ok(Survey::ranked(
                vec![Candidate {
                    path: PathBuf::from("/games/App/vcredist_x64.exe"),
                    reasons: Vec::new(),
                }],
                Vec::new(),
            )),
        );
        let empty = best_rows(
            Path::new("/games/App"),
            Ok(Survey::ranked(Vec::new(), Vec::new())),
        );

        assert!(nothing.lacks_evidence());
        assert!(
            nothing.rows[0]
                .1
                .contains("nothing here carries evidence of being a game"),
            "got {:?}",
            nothing.rows
        );
        assert!(
            empty.lacks_evidence(),
            "nothing to rank is nothing observed"
        );
        assert!(
            empty.rows[0].1.contains("no executable in this directory"),
            "and that shape has its own sentence, got {:?}",
            empty.rows
        );
    }

    #[test]
    fn the_json_line_keeps_the_frozen_eight_keys_at_the_front_and_appends_after_them() {
        // A harness reads them positionally. `game` may extend a record; it
        // may not reshape one.
        let line = json_line(Path::new("/games/App"), &survey(), &record());

        assert!(
            line.starts_with(
                r#"{"path":"/games/App/Binaries/Win64/App-Win64-Shipping.exe","machine":"x86-64","bits":64,"imports":["d3d12.dll"],"delay_imports":[],"file_version":null,"product_version":null,"error":null,"#
            ),
            "got {line}"
        );
        assert!(
            line.contains(r#""verdict":"Direct3D 12""#),
            "and the verdict keys after them, got {line}"
        );
    }

    #[test]
    fn the_json_line_carries_the_whole_ranking_with_a_stable_name_for_each_reason() {
        // The sentence is free to be reworded; a program needs something that
        // is not. Both travel, so neither consumer has to parse the other's.
        let line = json_line(Path::new("/games/App"), &survey(), &record());

        assert!(
            line.contains(
                r#""candidates":[{"path":"/games/App/Binaries/Win64/App-Win64-Shipping.exe","score":150,"reasons":[{"kind":"links-renderer","weight":100,"says":"links Direct3D 12 (import)"}"#
            ),
            "got {line}"
        );
        assert!(
            line.contains(r#"{"path":"/games/App/vcredist_x64.exe","score":0,"reasons":[]}"#),
            "the unevidenced candidate is a row too, got {line}"
        );
        assert!(line.ends_with(r#""survey_notes":[]}"#), "got {line}");
    }

    #[test]
    fn a_json_line_is_one_line_however_long_the_ranking_is() {
        // JSONL: a newline inside a record splits one result into two rows and
        // desynchronises every row after it.
        let line = json_line(Path::new("/games/App"), &survey(), &record());

        assert!(!line.contains('\n'), "got {line}");
    }
}
