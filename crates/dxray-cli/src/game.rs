//! The `game` listing: every executable ranked, then the analysis of the best.
//! The ranking comes first because it is what a reader must be able to
//! disagree with.

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
    /// True when something was not read: the directory, an image, or the rest
    /// of a truncated walk. A ranking with no import table behind it is not a
    /// failure.
    pub failed: bool,
}

/// Ranks the executables under `dir` and analyses the best of them.
pub fn inspect(dir: &Path, json: bool, presentation: report::Presentation) -> Outcome {
    let survey = match dxray_core::candidates(dir, None) {
        Ok(survey) => survey,
        Err(error) => return failure(dir, &error, json, presentation),
    };
    let Some(best) = survey.best() else {
        // No executable to answer with: a failure here, unlike in `installed`,
        // where a mid-download install is ordinary.
        return failure(
            dir,
            &"no executable found in this directory",
            json,
            presentation,
        );
    };

    let record = Record::read(&best.path);
    // A truncated walk fails the run; `NoRendererImported` does not.
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

/// A directory that could not be surveyed at all, still one row so results
/// line up with inputs.
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

    // The notes last, on stdout, so they survive a paste.
    if !survey.has_evidence() {
        // Said first and plainly: nothing argues any of these is the game.
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
        // Said out loud: a blank would read as truncated output.
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

/// One JSONL line: the best candidate's record with its keys untouched and in
/// order, then the survey appended.
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
            // `kind` is stable for programs; the sentence may be reworded.
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

/// What one game's directory produced for the listing: the rows, the caveats
/// the exit code is drawn from, and the evidence answer it orders by.
pub struct Best {
    pub rows: Vec<(&'static str, String)>,
    /// One entry per reason this game's directory was not searched in full,
    /// each naming the directory. Empty is the normal case.
    pub incomplete: Vec<String>,
    /// Whether anything in this install argues it is a game, as
    /// [`Survey::has_evidence`] answered, or `None` when it could not be read.
    pub carries_evidence: Option<bool>,
}

impl Best {
    /// True only when this install was read and argues nothing. The rule is
    /// [`dxray_core::inspect::lacks_evidence`], shared with the terminal browser.
    #[must_use]
    pub fn lacks_evidence(&self) -> bool {
        dxray_core::inspect::lacks_evidence(self.carries_evidence)
    }
}

/// The listing's view of one game: the best candidate and its strongest
/// reason, plus every caveat. The survey comes from
/// [`dxray_core::inspect`](dxray_core::inspect()), so both surfaces rank alike.
pub fn best_rows(install_dir: &Path, survey: io::Result<dxray_core::Survey>) -> Best {
    let survey = match survey {
        Ok(survey) => survey,
        // Named, not swallowed: a mid-download game points at no directory yet.
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
                // Never read: it keeps its place beside the games.
                carries_evidence: None,
            };
        }
    };
    // Asked once and carried: the row and the grouping are one answer.
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
        // The tie arrives as `Note::TiedAtTheTop`, as it does for `game`.
        rows.push(("best", format!("{shown}  ({}: {why})", best.score())));
    } else {
        // A zero-score row would read as an answer without this caveat.
        rows.push((
            "best",
            format!("(nothing here carries evidence of being a game; the highest ranked of {} executables is {shown})",
                survey.candidates.len()),
        ));
    }

    for note in &survey.notes {
        // Every note; only the one common across a library is shortened.
        let text = match note {
            Note::NoRendererImported { .. } => {
                "nothing here imports a graphics API, so this ranking rests on directory \
                 structure alone; dxray game on this path explains what that means"
                    .to_owned()
            }
            // Shortened, never dropped. `saturating_sub`: `count` is a public
            // field, so a hand-built zero must not wrap.
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

/// The notes that mean something in this game's directory was never looked
/// at, selected by [`Survey::incomplete_notes`] and worded here.
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
        // Naming the winner and hiding what it beat would be lying by omission.
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
        // Nothing observed anywhere: the analysis below must not read as an answer.
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
        // `count` is public, so a hand-built zero must not print a wrapped number.
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
        // Never asked is not "nothing": it keeps its place and says why.
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
        // No ` no evidence` marker here: the row is already the sentence.
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
