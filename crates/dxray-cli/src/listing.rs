//! Inventory rendering for `installed`: one [`dxray_core::walk`]
//! produces both the text and the JSONL. Installs without game evidence are
//! grouped last, never filtered.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};

use dxray_core::proton::Builds;
use dxray_core::{Catalogue, Game, Launcher, Origin};

use crate::record::{Record, display_path, push_optional, push_quoted, push_string};

/// Rendered inventory and scan status. A failure is collected rather than
/// hiding the rest, and still makes the scan incomplete.
pub struct Listing {
    /// Human-readable inventory.
    pub text: String,
    /// JSONL counterpart built during the same walk as [`Self::text`].
    json: String,
    pub problems: Vec<String>,
    /// Non-fatal caveats, categorized for the summary and JSON output.
    pub notes: Vec<Cause>,
    /// One entry per game directory not searched in full, holding what was
    /// not read. Makes the scan incomplete.
    pub incomplete: Vec<Vec<String>>,
    pub roots: usize,
    pub libraries: usize,
    pub games: usize,
    /// Whether each game names its launcher: true when the scan asked about
    /// more than one launcher, whatever the machine has.
    pub name_origins: bool,
}

/// Category of a non-fatal scan note.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// A launcher index declared no libraries; more may exist.
    Index,
    /// One record could not be used, without excluding another discovered game.
    Record,
}

impl Cause {
    /// All causes, in JSON summary order.
    pub const ALL: [Self; 2] = [Self::Index, Self::Record];

    /// Stable key for JSON consumers.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Index => "index",
            Self::Record => "record",
        }
    }
}

/// Width of the label column, so `library`, `note` and `unreadable` all line up
/// and their values start in the same place.
const LABEL: usize = 12;
/// Where a wrapped note continues, under the first word of the one above it.
const INDENT: usize = 2 + LABEL;

impl Listing {
    fn new(name_origins: bool) -> Self {
        Self {
            text: String::new(),
            json: String::new(),
            problems: Vec::new(),
            notes: Vec::new(),
            incomplete: Vec::new(),
            roots: 0,
            libraries: 0,
            games: 0,
            name_origins,
        }
    }

    /// The one-line summary under the listing, with every caveat beside the
    /// counts. The JSON summary carries the same sentence, and the caveats that
    /// mean something was not read also move the exit code.
    pub fn trailer(&self) -> String {
        let summary = format!(
            "{} {} in {} {} across {} {}",
            self.games,
            plural(self.games, "game", "games"),
            self.libraries,
            plural(self.libraries, "library", "libraries"),
            self.roots,
            plural(self.roots, "install", "installs"),
        );
        let caveats = self.caveats();
        if caveats.is_empty() {
            return summary;
        }
        format!("{summary}; {}", caveats.join("; "))
    }

    /// Everything the trailer hedges the counts with, one sentence per
    /// [`Cause`]. Shared with the JSON summary so both carry the same words.
    fn caveats(&self) -> Vec<String> {
        let mut caveats = Vec::new();
        if !self.problems.is_empty() {
            // "files", not "manifests": a problem can also be an index or a directory.
            caveats.push(format!(
                "{} could not be read, so the count is incomplete",
                match self.problems.len() {
                    1 => "1 file".to_owned(),
                    n => format!("{n} files"),
                }
            ));
        }
        // An index that declared nothing means the library count may be short.
        match self.notes_about(Cause::Index) {
            0 => {}
            1 => caveats.push("1 index declared no libraries, so there may be more".to_owned()),
            n => caveats.push(format!(
                "{n} indexes declared no libraries, so there may be more"
            )),
        }
        // A record that cost nothing: the count is sound, and the row above says why.
        match self.notes_about(Cause::Record) {
            0 => {}
            1 => caveats.push(
                "1 launcher record could not be used, and nothing is missing because of it"
                    .to_owned(),
            ),
            n => caveats.push(format!(
                "{n} launcher records could not be used, and nothing is missing because of them"
            )),
        }
        // A truncated walk may have missed a better executable: not a failure,
        // but it moves the exit code like one.
        if !self.incomplete.is_empty() {
            caveats.push(format!(
                "{} searched in full, so the executable named for {} may not be the right one",
                match self.incomplete.len() {
                    1 => "1 game directory could not be".to_owned(),
                    n => format!("{n} game directories could not be"),
                },
                if self.incomplete.len() == 1 {
                    "it"
                } else {
                    "them"
                }
            ));
        }
        caveats
    }

    /// How many notes were raised by `cause`.
    fn notes_about(&self, cause: Cause) -> usize {
        self.notes.iter().filter(|note| **note == cause).count()
    }

    /// The whole listing as JSONL: one object per line, the summary last.
    ///
    /// Every object starts with `kind`: `install`, `library`, `game`, `note`,
    /// `problem` or `summary`, which the per-file record line never has. A scan
    /// that ran ends with one `summary`, whose `complete` is the exit status.
    /// Keys may be added and kinds may appear; key order and the wording of
    /// `says` and `rows` are not promised.
    pub fn json(&self) -> String {
        // The summary is never separable from the rows it counts.
        let mut out = self.json.clone();
        self.push_summary(&mut out);
        out
    }

    /// The last line: the counts, whether anything went unread (`complete`, the
    /// exit status), the notes per [`Cause`], and the caveats with the trailer's
    /// own sentence.
    fn push_summary(&self, out: &mut String) {
        out.push_str("{\"kind\":\"summary\",");
        let _ = write!(
            out,
            "\"games\":{},\"libraries\":{},\"installs\":{},\"complete\":{},",
            self.games,
            self.libraries,
            self.roots,
            !self.incomplete_scan()
        );
        out.push_str("\"notes\":{");
        for (i, cause) in Cause::ALL.into_iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let _ = write!(out, "\"{}\":{}", cause.key(), self.notes_about(cause));
        }
        out.push_str("},");
        out.push_str("\"caveats\":[");
        for (i, caveat) in self.caveats().iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            push_quoted(out, caveat);
        }
        out.push_str("],");
        push_string(out, "says", &self.trailer());
        out.push_str("}\n");
    }

    /// True when the trailer admits something was not read. The exit code and
    /// the JSON `complete` are drawn from this; notes do not count.
    pub fn incomplete_scan(&self) -> bool {
        !self.problems.is_empty() || !self.incomplete.is_empty()
    }
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

/// Walks every installation of every launcher in `launchers` and renders both
/// presentations in one pass.
#[cfg(test)]
pub fn scan(launchers: &[&dyn Launcher]) -> Listing {
    scan_with_view(launchers, crate::report::Presentation::Standard)
}

pub fn scan_with_view(
    launchers: &[&dyn Launcher],
    presentation: crate::report::Presentation,
) -> Listing {
    let mut render = Render {
        listing: Listing::new(launchers.len() > 1),
        // One reading per Proton build and per Steam root, not per game.
        builds: Builds::default(),
        install: None,
        presentation,
    };
    // This visitor never breaks, so the walk always completes.
    let _ = dxray_core::walk(launchers, &mut render);
    if presentation == crate::report::Presentation::Compact {
        render.listing.text.push_str("Static evidence only; runtime use and compatibility are not established. Unknown renderers may load dynamically.\n");
    }
    render.listing
}

/// Accumulates the listing as the walk finds things.
struct Render {
    presentation: crate::report::Presentation,
    listing: Listing,
    builds: Builds,
    /// The installation the walk is inside. `None` before the first root is
    /// written as `null`, never guessed.
    install: Option<PathBuf>,
}

/// What a worded row means for the exit code, and what a program calls it.
#[derive(Clone, Copy)]
enum Kind {
    /// Everything was read, and there may be less of it than expected.
    Note(Cause),
    /// Something could not be read.
    Problem,
}

impl Render {
    /// Files one worded item in all three places at once: the text row, the
    /// JSON object, and the list the exit code is drawn from. `label` is the
    /// row's word (`note`, `error`, `unreadable`); a note also carries its
    /// [`Cause`].
    fn record(
        &mut self,
        label: &'static str,
        kind: Kind,
        origin: Origin,
        library: Option<&Path>,
        text: String,
    ) {
        row(&mut self.listing.text, label, &text);

        let json = &mut self.listing.json;
        json.push_str(match kind {
            Kind::Note(_) => "{\"kind\":\"note\",",
            Kind::Problem => "{\"kind\":\"problem\",",
        });
        push_origin(json, origin);
        push_place(json, self.install.as_deref(), library);
        push_string(json, "label", label);
        json.push(',');
        if let Kind::Note(cause) = kind {
            push_string(json, "cause", cause.key());
            json.push(',');
        }
        push_string(json, "says", &text);
        json.push_str("}\n");

        match kind {
            Kind::Note(cause) => self.listing.notes.push(cause),
            Kind::Problem => self.listing.problems.push(text),
        }
    }
}

impl dxray_core::Visitor for Render {
    fn root(&mut self, origin: Origin, root: &Path) -> ControlFlow<()> {
        self.listing.roots += 1;
        self.install = Some(root.to_path_buf());
        let _ = writeln!(self.listing.text, "{} [{}]", origin.label(), root.display());

        let json = &mut self.listing.json;
        json.push_str("{\"kind\":\"install\",");
        push_origin(json, origin);
        push_string(json, "path", &display_path(root));
        json.push_str("}\n");
        ControlFlow::Continue(())
    }

    /// A launcher index note, printed above the libraries it qualifies and on
    /// stdout so it survives a paste. Always [`Cause::Index`].
    fn note(&mut self, origin: Origin, note: &str) -> ControlFlow<()> {
        self.record(
            "note",
            Kind::Note(Cause::Index),
            origin,
            None,
            note.to_owned(),
        );
        ControlFlow::Continue(())
    }

    /// A root whose index could not be read. Printed in the listing too, so the
    /// install does not read as one with no games.
    fn problem(&mut self, origin: Origin, problem: &str) -> ControlFlow<()> {
        // No library: the walk reports a root's own failures before it has
        // opened one, and the index that failed is what says where they are.
        self.record("error", Kind::Problem, origin, None, problem.to_owned());
        ControlFlow::Continue(())
    }

    fn library(&mut self, origin: Origin, library: &Path) -> ControlFlow<()> {
        self.listing.libraries += 1;

        let json = &mut self.listing.json;
        json.push_str("{\"kind\":\"library\",");
        push_origin(json, origin);
        push_optional(
            json,
            "install",
            self.install.as_deref().map(display_path).as_deref(),
        );
        json.push(',');
        push_string(json, "path", &display_path(library));
        json.push_str("}\n");
        ControlFlow::Continue(())
    }

    fn catalogue(
        &mut self,
        launcher: &dyn Launcher,
        library: &Path,
        catalogue: Catalogue,
    ) -> ControlFlow<()> {
        self.listing.games += catalogue.games.len();
        let label = library_label(library, self.install.as_deref(), launcher.origin());
        let _ = writeln!(
            self.listing.text,
            "  Library: {label} ({} {})",
            catalogue.games.len(),
            if catalogue.games.len() == 1 {
                "installation"
            } else {
                "installations"
            }
        );
        // Only failures is not an empty library: the rows below say what happened.
        let unreadable = catalogue.games.is_empty() && !catalogue.problems.is_empty();
        if !unreadable {
            render_games(
                &mut Sink {
                    text: &mut self.listing.text,
                    json: &mut self.listing.json,
                    incomplete: &mut self.listing.incomplete,
                },
                &catalogue.games,
                Place {
                    install: self.install.as_deref(),
                    library,
                },
                &mut self.builds,
                self.listing.name_origins,
                self.presentation,
            );
        }
        // Records that could not be used and cost nothing: notes, with the
        // reason. `Cause::Record`, since the library itself was read in full.
        for note in catalogue.notes {
            self.record(
                "note",
                Kind::Note(Cause::Record),
                launcher.origin(),
                Some(library),
                note,
            );
        }
        // Unreadable records after the games: each is still a game the user owns.
        for problem in catalogue.problems {
            self.record(
                "unreadable",
                Kind::Problem,
                launcher.origin(),
                Some(library),
                problem,
            );
        }
        ControlFlow::Continue(())
    }
}

/// One labelled row, wrapped at [`crate::wrap::WIDTH`] with continuations under
/// the value. A word longer than the line, such as a path, is not split.
fn row(out: &mut String, label: &str, value: &str) {
    let _ = write!(out, "  {label:<LABEL$}");
    crate::wrap::prose(out, INDENT, INDENT, value);
}

/// One entry per game: the install, the best executable and its strongest
/// reason, every caveat the survey raised, and the Proton/NVAPI rows.
///
/// Installs that were read and carry no evidence of being a game are printed
/// after the others, never hidden, in the launcher's order; the question is
/// [`Best::lacks_evidence`](crate::game::Best::lacks_evidence), shared with the
/// terminal browser. Each game is inspected and rendered as it comes; only the
/// rendered rows of demoted games wait until the library ends.
///
/// The text and the JSON are written in the same pass, from the same rows.
/// NVAPI answers never move the exit code: most games have never run under
/// Proton, which is not a failed scan.
fn render_games(
    sink: &mut Sink<'_>,
    games: &[Game],
    place: Place<'_>,
    builds: &mut Builds,
    name_origins: bool,
    presentation: crate::report::Presentation,
) {
    if games.is_empty() {
        // No JSON object: this is the absence of game rows, not a fact.
        let _ = writeln!(sink.text, "    (no games installed)");
        return;
    }
    // Demoted games' rendered rows, appended when the library ends.
    let mut ordinary = Vec::new();
    let mut demoted = Vec::new();
    for game in games {
        // One call, shared with the terminal browser, so that the facts
        // established about a game cannot depend on which surface asked.
        let facts = dxray_core::inspect(game, place.install, place.library, builds);
        let dxray_core::inspect::Inspection { survey, nvapi } = facts;
        let details = tree_facts(
            &survey,
            &game.install_dir,
            game.origin.label(),
            &nvapi.verdict,
            presentation,
        );
        let best = crate::game::best_rows(&game.install_dir, survey);
        let lacks_evidence = best.lacks_evidence();

        // The rows this game gets, decided once and rendered twice.
        let mut rows = Vec::new();
        if name_origins {
            rows.push(("origin", game.origin.label().to_owned()));
        }
        rows.extend(best.rows);
        // The build is named beside the verdict, so the verdict is checkable.
        if let Some(script) = &nvapi.script {
            rows.push(("proton", display_path(script)));
        }
        rows.push(("nvapi", nvapi.verdict.clone()));

        let mut one = Rendered {
            text: tree_entry(game, place, &nvapi, &details, &rows, presentation),
            json: String::new(),
        };
        push_game(
            &mut one.json,
            game,
            place,
            &rows,
            &best.incomplete,
            best.carries_evidence,
        );

        // In declared order, so a demoted game cannot lose its caveat.
        if !best.incomplete.is_empty() {
            sink.incomplete.push(best.incomplete);
        }

        // Both renderings move into the same group together.
        if lacks_evidence {
            demoted.push(one);
        } else {
            ordinary.push(one);
        }
    }
    let ordinary_len = ordinary.len();
    let has_demoted = !demoted.is_empty();
    for (index, one) in ordinary.into_iter().enumerate() {
        sink.text.push_str(&tree_branch(
            &one.text,
            "    ",
            index + 1 == ordinary_len && !has_demoted,
        ));
        sink.json.push_str(&one.json);
    }
    if !demoted.is_empty() {
        sink.text
            .push_str("    └─ Installations without game evidence\n");
        let demoted_len = demoted.len();
        for (index, one) in demoted.into_iter().enumerate() {
            sink.text
                .push_str(&tree_branch(&one.text, "      ", index + 1 == demoted_len));
            sink.json.push_str(&one.json);
        }
    }
}

/// Places a rendered game under its final sibling position once the library's
/// grouping is known.
fn tree_branch(entry: &str, prefix: &str, last: bool) -> String {
    let marker = if last { "└" } else { "├" };
    let continuation = if last { "   " } else { "│  " };
    entry
        .replacen("    ├", &format!("{prefix}{marker}"), 1)
        .replace("    │  ", &format!("{prefix}{continuation}"))
}

/// A main Steam library is already named by its launcher root; show its stable
/// `steamapps` component instead of repeating the absolute path.
fn library_label(library: &Path, root: Option<&Path>, origin: Origin) -> String {
    if root == Some(library) && origin.key() == "steam" {
        return "steamapps".to_owned();
    }
    display_path(library)
}

fn compact_install_path(install: &Path, library: &Path) -> String {
    if install.is_absolute()
        && library.is_absolute()
        && !install
            .components()
            .any(|part| part == Component::ParentDir)
        && !library
            .components()
            .any(|part| part == Component::ParentDir)
        && let Ok(relative) = install.strip_prefix(library)
    {
        let shown = if relative.as_os_str().is_empty() {
            ".".to_owned()
        } else {
            format!("./{}", relative.display())
        };
        if shown.len() < install.as_os_str().len() {
            return shown;
        }
    }
    display_path(install)
}

fn tree_entry(
    game: &Game,
    place: Place<'_>,
    nvapi: &dxray_core::proton::Answer,
    details: &TreeFacts,
    rows: &[(&str, String)],
    presentation: crate::report::Presentation,
) -> String {
    use crate::report::Presentation;

    let mut out = String::new();
    let _ = writeln!(
        out,
        "    ├─ {}  {}  [{}]",
        game.identity, game.name, details.result
    );

    let path = if presentation == Presentation::Full {
        display_path(&game.install_dir)
    } else {
        compact_install_path(&game.install_dir, place.library)
    };
    tree_row(&mut out, "Path", &path);

    if presentation != Presentation::Compact {
        for (label, value) in rows {
            if matches!(*label, "proton" | "nvapi") {
                continue;
            }
            if *label == "origin" && value == "Steam" {
                continue;
            }
            tree_row(&mut out, &tree_label(label), value);
        }
    }

    match &details.survey_error {
        None => {
            if details.result == "No executable" {
                tree_row(&mut out, "Status", "no executable found");
            }
            if presentation == Presentation::Compact && details.incomplete {
                tree_row(&mut out, "Status", "scan incomplete");
            }
            if presentation != Presentation::Full && !details.features.is_empty() {
                tree_row(&mut out, "Features", &details.features.join(", "));
            }
            if presentation == Presentation::Full {
                roots(&mut out, place);
                if let Some(report) = &details.full_report {
                    tree_block(&mut out, report);
                }
                if let Some(ranking) = &details.full_ranking {
                    tree_block(&mut out, ranking);
                }
            }
        }
        Some(error) => {
            tree_row(&mut out, "Status", error);
            if presentation == Presentation::Full {
                roots(&mut out, place);
            }
        }
    }

    if let Some(script) = &nvapi.script {
        let script = if presentation == Presentation::Full {
            display_path(script)
        } else {
            match (
                script.parent().and_then(Path::file_name),
                script.file_name(),
            ) {
                (Some(parent), Some(file)) => {
                    format!("{}/{}", parent.to_string_lossy(), file.to_string_lossy())
                }
                _ => display_path(script),
            }
        };
        if presentation == Presentation::Full {
            tree_row(&mut out, "Proton", &script);
        } else {
            tree_row(
                &mut out,
                "Proton",
                &format!("{script} [NVAPI: {}]", nvapi_brief(nvapi)),
            );
        }
    }
    if presentation != Presentation::Full
        && nvapi.script.is_none()
        && nvapi_brief(nvapi) != "not applicable"
    {
        tree_row(&mut out, "NVAPI", nvapi_brief(nvapi));
    }
    // Full prints the sentence inside the report; with no report, here.
    if presentation == Presentation::Full && details.full_report.is_none() {
        tree_row(&mut out, "NVAPI", &nvapi.verdict);
    }
    out
}

/// The library and launcher roots the full view names for every game.
fn roots(out: &mut String, place: Place<'_>) {
    tree_row(out, "Library", &display_path(place.library));
    if let Some(root) = place.install {
        tree_row(out, "Launcher root", &display_path(root));
    }
}

/// A scan-sized NVAPI summary. The complete policy sentence stays in `full`,
/// where it can be read without displacing the next game in the inventory.
fn nvapi_brief(answer: &dxray_core::proton::Answer) -> &'static str {
    match answer.available {
        Some(true) => "allowed",
        Some(false) => "withheld",
        None if answer.verdict.starts_with("not applicable") => "not applicable",
        None if answer.verdict.starts_with("not determined") => "not determined",
        None if answer.script.is_some() => "not determined",
        None => "not assessed",
    }
}

struct TreeFacts {
    result: String,
    features: Vec<String>,
    incomplete: bool,
    survey_error: Option<String>,
    full_report: Option<String>,
    full_ranking: Option<String>,
}

fn tree_facts(
    survey: &std::io::Result<dxray_core::Survey>,
    install_dir: &Path,
    origin: &str,
    nvapi: &str,
    presentation: crate::report::Presentation,
) -> TreeFacts {
    use crate::report::Presentation;

    let survey = match survey {
        Ok(survey) => survey,
        Err(error) => {
            return TreeFacts {
                result: "Evidence unavailable".to_owned(),
                features: Vec::new(),
                incomplete: false,
                survey_error: Some(format!("installation could not be read: {error}")),
                full_report: None,
                full_ranking: None,
            };
        }
    };
    let Some(candidate) = survey.best() else {
        return TreeFacts {
            result: "No executable".to_owned(),
            features: Vec::new(),
            incomplete: survey.is_incomplete(),
            survey_error: None,
            full_report: None,
            full_ranking: None,
        };
    };
    let record = Record::read(&candidate.path);
    let result = if record.error.is_some() {
        "Evidence unavailable".to_owned()
    } else if record.verdict.renderers.is_empty() {
        "No API determined".to_owned()
    } else {
        record.verdict.headline()
    };
    let features = record
        .verdict
        .features
        .iter()
        .map(|feature| feature.name.clone())
        .collect();
    let full_report = (presentation == Presentation::Full).then(|| {
        crate::report::present_with_context(&record, Presentation::Full, Some((origin, nvapi)))
    });
    let full_ranking = (presentation == Presentation::Full)
        .then(|| crate::game::ranking_paths(install_dir, survey, true));
    TreeFacts {
        result,
        features,
        incomplete: survey.is_incomplete(),
        survey_error: None,
        full_report,
        full_ranking,
    }
}

fn tree_label(label: &str) -> String {
    match label {
        "best" => "Exec".to_owned(),
        "origin" => "Source".to_owned(),
        "proton" => "Proton".to_owned(),
        "nvapi" => "NVAPI".to_owned(),
        "note" => "Note".to_owned(),
        "unread" => "Unread".to_owned(),
        other => other.to_owned(),
    }
}

fn tree_row(out: &mut String, label: &str, value: &str) {
    let _ = writeln!(out, "    │  {label}: {value}");
}

fn tree_block(out: &mut String, block: &str) {
    for line in block.lines().filter(|line| !line.is_empty()) {
        let _ = writeln!(out, "    │  {line}");
    }
}

/// The two renderings of one library's games, and the caveat list they share.
struct Sink<'a> {
    text: &'a mut String,
    json: &'a mut String,
    /// Game directories that were not searched in full. See
    /// [`Listing::incomplete`].
    incomplete: &'a mut Vec<Vec<String>>,
}

/// One thing, rendered both ways.
#[derive(Default)]
struct Rendered {
    text: String,
    json: String,
}

/// Where in the tree a library sits. `install` is never invented.
#[derive(Clone, Copy)]
struct Place<'a> {
    install: Option<&'a Path>,
    library: &'a Path,
}

/// One game as a JSON object: where it is, what its launcher calls it, and the
/// labelled prose rows printed under it. `directory` is spelled as `game --json`
/// spells it. `carries_evidence` is three-state: `null` means the install could
/// not be read, which is not `false`.
fn push_game(
    out: &mut String,
    game: &Game,
    place: Place<'_>,
    rows: &[(&'static str, String)],
    incomplete: &[String],
    carries_evidence: Option<bool>,
) {
    out.push_str("{\"kind\":\"game\",");
    push_origin(out, game.origin);
    push_place(out, place.install, Some(place.library));

    push_string(out, "id", &game.identity.to_string());
    // A Heroic game has no Steam application id: `null`, not a zero.
    match game.identity.steam_appid() {
        Some(appid) => {
            let _ = write!(out, ",\"steam_appid\":{appid},");
        }
        None => out.push_str(",\"steam_appid\":null,"),
    }
    push_string(out, "name", &game.name);
    out.push(',');
    push_string(out, "directory", &display_path(&game.install_dir));
    out.push(',');

    let _ = match carries_evidence {
        Some(carries) => write!(out, "\"carries_evidence\":{carries}"),
        None => write!(out, "\"carries_evidence\":null"),
    };

    out.push_str(",\"rows\":[");
    for (i, (label, value)) in rows.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('{');
        push_string(out, "label", label);
        out.push(',');
        push_string(out, "says", value);
        out.push('}');
    }
    out.push_str("],\"incomplete\":[");
    for (i, note) in incomplete.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_quoted(out, note);
    }
    out.push_str("]}\n");
}

/// The launcher a row came from: `key` for programs, `label` for people, and
/// the only one naming a Heroic backend. Leaves a trailing comma.
fn push_origin(out: &mut String, origin: Origin) {
    push_string(out, "origin", origin.key());
    out.push(',');
    push_string(out, "origin_label", origin.label());
    out.push(',');
}

/// Where in the tree a row sits: the installation, then the library. Either
/// may be `null`, and neither is guessed.
fn push_place(out: &mut String, install: Option<&Path>, library: Option<&Path>) {
    push_optional(out, "install", install.map(display_path).as_deref());
    out.push(',');
    push_optional(out, "library", library.map(display_path).as_deref());
    out.push(',');
}

/// The message for a machine where none of `launchers` was found, listing
/// where the search looked, grouped by launcher when there are several.
pub fn nothing_found(launchers: &[&dyn Launcher]) -> String {
    let single = match launchers {
        [only] => Some(only.origin().label()),
        _ => None,
    };
    let indent = if single.is_some() { 2 } else { 4 };
    let mut out = format!(
        "no {} installation found. Looked in:\n",
        single.unwrap_or("game launcher")
    );
    for launcher in launchers {
        if single.is_none() {
            let _ = writeln!(out, "  {}:", launcher.origin().label());
        }
        // Only repeats within one launcher are dropped.
        let mut seen = HashSet::new();
        for candidate in launcher.candidate_roots() {
            if seen.insert(candidate.clone()) {
                let _ = writeln!(out, "{:indent$}{}", "", candidate.display());
            }
        }
    }
    let _ = writeln!(
        out,
        "If {} is installed somewhere else, this build cannot find it; \
         pass the game directory to dxray directly.",
        single.unwrap_or("a launcher")
    );
    out
}

/// [`nothing_found`] as one JSON `problem` object. No `summary` follows:
/// nothing was scanned.
pub fn nothing_found_json(launchers: &[&dyn Launcher]) -> String {
    let mut out = String::from("{\"kind\":\"problem\",\"origin\":null,\"origin_label\":null,");
    push_place(&mut out, None, None);
    push_string(&mut out, "label", "error");
    out.push(',');
    push_string(&mut out, "says", nothing_found(launchers).trim_end());
    out.push('}');
    out
}

#[cfg(test)]
mod tests;
