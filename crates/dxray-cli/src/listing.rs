//! Inventory rendering for `installed` and `steam`.
//!
//! A single [`dxray_core::walk`] produces both human-readable text and JSONL,
//! keeping the two surfaces consistent. Launcher discovery, executable ranking
//! and static analysis remain in `dxray-core`; this module only presents them.
//!
//! Every launcher-declared installation is retained. Entries without static
//! game evidence are grouped after entries with evidence rather than filtered:
//! Steam metadata cannot reliably distinguish games from tools or runtimes.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::ops::ControlFlow;
use std::path::{Component, Path, PathBuf};

use dxray_core::proton::Builds;
use dxray_core::{Catalogue, Game, Launcher, Origin};

use crate::record::{Record, display_path, push_optional, push_quoted, push_string};

/// Rendered inventory and scan status.
///
/// Failures are collected so one unreadable library does not hide the rest of
/// the inventory; they still make the scan incomplete.
pub struct Listing {
    /// Human-readable inventory.
    pub text: String,
    /// JSONL counterpart built during the same walk as [`Self::text`].
    json: String,
    pub problems: Vec<String>,
    /// Non-fatal caveats, categorized for the summary and JSON output.
    pub notes: Vec<Cause>,
    /// Directories that reached the scan limit and therefore make the scan
    /// incomplete.
    pub incomplete: Vec<String>,
    pub roots: usize,
    pub libraries: usize,
    pub games: usize,
    /// Whether each game says which launcher it came from.
    ///
    /// True exactly when the scan was asked about more than one launcher, which
    /// is a fact about the question and not about the machine: `installed` names
    /// origins on a machine with only Heroic on it, because the flag could have
    /// found Steam and the reader has no way to know it did not. `steam`
    /// never names them, because the flag already did — a column with the same
    /// value in every row is noise, and it would change the bytes of an output
    /// people already have captured.
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

    /// The one-line summary under the listing.
    ///
    /// Says so when the count is incomplete. A bare "41 games" under a block
    /// that also reported two unreadable manifests is the line a person
    /// remembers and quotes, and on its own it is not true — the two failures
    /// are games as well, just ones this tool could not read. The count and the
    /// caveat have to travel together or the caveat is the part that gets lost.
    ///
    /// # It means the same thing for every launcher
    ///
    /// "In M libraries across K installs" counts what it always counted: the
    /// places games would be if the user had any. A Heroic installation is one
    /// install with one library — its configuration root, which is the
    /// directory its records were read out of — so a Heroic-only machine with
    /// three games reads `3 games in 1 library across 1 install`, and an
    /// installed Heroic with nothing in it reads `0 games in 1 library across
    /// 1 install` rather than disappearing. That is the same sentence an empty
    /// Steam library has always earned.
    ///
    /// # This line, the JSON summary and the exit code say the same thing
    ///
    /// Every caveat in [`Listing::caveats`] that means *something was not read*
    /// also moves the exit code — see [`Listing::incomplete_scan`]. A human
    /// reading this sentence and a script reading the status must not come away
    /// with different stories about one run.
    ///
    /// The `--json` summary carries this whole sentence as its `says` field and
    /// the list it is built from as its `caveats` field, both from
    /// [`Listing::caveats`]. There is one place where a scan decides it has
    /// something to hedge about, and all three surfaces read it.
    ///
    /// The caveats that do not move it are the notes, and that is not an
    /// exception to the rule but an instance of it: everything was read, and an
    /// old single-library install genuinely looks like that. They say "there
    /// may be more" and "nothing is missing because of it", never "some of this
    /// is missing" — and they say one or the other according to their
    /// [`Cause`], because the two are not the same doubt.
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

    /// Everything the trailer hedges the counts with, one sentence each.
    ///
    /// Separated from [`Listing::trailer`] so that the JSON summary can carry
    /// these sentences rather than word the same four conditions a second
    /// time. A caveat added here is printed to a person and handed to a program
    /// by the same edit; a caveat this list forgets is absent from both, which
    /// is a bug that shows up on every surface at once instead of on the one
    /// nobody was looking at.
    ///
    /// One clause per cause, counted separately, and that is the whole reason
    /// [`Cause`] exists. A clause has to describe the doubt it is actually
    /// about: a run whose only caveat was a stale launcher record used to read
    /// "1 index declared no libraries, so there may be more", which sends a
    /// reader to a file that was fine and reports doubt over libraries that
    /// were all read. Two causes, two sentences, two counts.
    fn caveats(&self) -> Vec<String> {
        let mut caveats = Vec::new();
        if !self.problems.is_empty() {
            // "files", not "manifests": a problem can also be an unreadable
            // library index or a directory that could not be listed, and the
            // trailer naming the wrong kind of file sends a reader looking in
            // the wrong place. The lines above it name each one exactly.
            caveats.push(format!(
                "{} could not be read, so the count is incomplete",
                match self.problems.len() {
                    1 => "1 file".to_owned(),
                    n => format!("{n} files"),
                }
            ));
        }
        // An index that declared nothing means the library count itself may be
        // short, which the games count inherits. Same argument as above: the
        // number is the part that gets quoted, so the doubt has to ride in the
        // same line as the number.
        match self.notes_about(Cause::Index) {
            0 => {}
            1 => caveats.push("1 index declared no libraries, so there may be more".to_owned()),
            n => caveats.push(format!(
                "{n} indexes declared no libraries, so there may be more"
            )),
        }
        // A record that could not be used is the opposite size of doubt, and
        // saying so is the point of printing it at all. Nothing is hidden —
        // that is what put it on the notes list rather than the problems list —
        // so the clause says the count is sound and leaves the reader with the
        // row above, which names the record and says why it cost nothing.
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
        // A bounded walk that ran out of budget did not look at every executable
        // in that game's directory, so the one named above it may not be the
        // best one there. Nothing failed to read, which is why this is not
        // phrased as a failure — but something was not read, which is why it
        // moves the exit code exactly as a failure does.
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
    ///
    /// The counting is here rather than at the two call sites so that the one
    /// list stays the one list: a second `Vec` per cause would count itself,
    /// and the day a third cause appeared, whoever added it would have to
    /// remember every place that iterates notes rather than adding a variant
    /// the compiler then asks about.
    fn notes_about(&self, cause: Cause) -> usize {
        self.notes.iter().filter(|note| **note == cause).count()
    }

    /// The whole listing as JSONL: one object per line, the summary last.
    ///
    /// # The shape, and what it is not
    ///
    /// It is deliberately **not** the record line `--json` emits for a PE
    /// file. That line is a frozen positional contract about one file on disk,
    /// with no field that could hold a game, and widening it to carry an
    /// install would break every harness already reading it. This is a second
    /// shape, behind a different question, and it says so in its own first
    /// key: every object here starts with `"kind"`, and no object a PE scan
    /// emits has that key at all. A reader can tell the two apart from the
    /// first eight characters of a line, which is the property that makes two
    /// shapes under one flag safe rather than reckless.
    ///
    /// # One object per row, not one per game
    ///
    /// The listing is a tree — install, library, games — and one object per
    /// game would flatten it. Flattening loses the installs and libraries that
    /// hold no games, and those are exactly the rows worth reading: an
    /// installed launcher with nothing in it, or a library whose index could
    /// not be read. So every row the human listing prints gets one object
    /// here, of kind `install`, `library`, `game`, `note` or `problem`, and
    /// each of them names the install and the library it sits under. A
    /// consumer that wants the tree groups by those two fields; a consumer
    /// that wants a flat list of games filters on `"kind":"game"` and has lost
    /// nothing, because the game rows carry their place with them.
    ///
    /// The one text row with no object of its own is `(no games installed)`,
    /// which is not a fact — it is the absence of game rows under a library
    /// row, and a reader that groups by library sees exactly that.
    ///
    /// # What is promised
    ///
    /// One object per line, no header and no trailer text. Every object has
    /// `kind` as its first key. A scan that ran ends with exactly one
    /// `summary` object, whose `complete` field is the exit status of the run
    /// and whose `notes` object counts the notes raised, one key per
    /// [`Cause`]. Every `note` object carries that same key as `cause`, and
    /// the spelling of a cause is stable. Keys may be **added** to any object
    /// and new kinds may appear, so a consumer must read by name and ignore
    /// what it does not know.
    ///
    /// What is **not** promised: the order of keys within an object — unlike
    /// the PE record line, nothing here may be read positionally — and the
    /// wording of any `says` field, which is prose written for a person and
    /// free to be reworded. Programs read `kind`, `cause`, `complete`, the
    /// counts and the paths; `says` and `rows` are for showing to a human.
    pub fn json(&self) -> String {
        // Cloned once per run rather than borrowed, because the summary must
        // not be separable from the rows it counts: a caller that could ask
        // for the body alone would be able to report games while dropping the
        // sentence that says the scan was partial.
        let mut out = self.json.clone();
        self.push_summary(&mut out);
        out
    }

    /// The last line: the counts, the caveats, and whether anything went
    /// unread.
    ///
    /// `says` is [`Listing::trailer`] verbatim — the same sentence printed
    /// under the human listing — and `caveats` is the list that sentence is
    /// built from. A program therefore reads the caveats without parsing the
    /// prose, and a person quoting the JSON and a person quoting the terminal
    /// quote the same words.
    ///
    /// `complete` is `!`[`Listing::incomplete_scan`], which is also the exit
    /// code. Not a fourth wording of "did everything get read": the status, the
    /// trailer and this field are one predicate, so a consumer that never looks
    /// at the exit status still cannot be told a partial scan was a whole one.
    ///
    /// Note that `complete` can be true while `caveats` is not empty. That is a
    /// note — either [`Cause`] of one — and it is deliberate: everything was
    /// read, and there may be less of it than the user expects.
    ///
    /// `notes` is how many notes of each [`Cause`] the scan raised, keyed by
    /// [`Cause::key`], every cause present and zero written as `0`. It is the
    /// same count [`Listing::caveats`] words a clause from, and it exists
    /// because that clause is prose: the README promises the sentences may be
    /// reworded, so a program that needed the number had to parse a sentence
    /// it was told not to rely on. The clause is still the sentence for a
    /// person, and this is the number for a program; they are one count read
    /// through [`Listing::notes_about`], not two vocabularies for it, and a
    /// count that moved in one and not the other would fail the golden.
    ///
    /// `caveats` itself stays a list of sentences and gains no key of its own.
    /// Every clause a program could want to branch on is already data beside
    /// it — `complete`, the counts, and now `notes` — and a clause tagged with
    /// the cause it words would be the row saying its cause a second time.
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

    /// True when the trailer admits something was not read.
    ///
    /// The exit code is drawn from exactly this, so that the sentence a person
    /// reads and the status a script reads cannot disagree — and so is the
    /// `complete` field of the JSON summary, so that a consumer who never looks
    /// at the status reads the same answer. [`Listing::notes`] is deliberately
    /// absent, whatever its [`Cause`]: everything was read and there may simply
    /// be less of it than expected, which is a caveat and not a failed scan.
    pub fn incomplete_scan(&self) -> bool {
        !self.problems.is_empty() || !self.incomplete.is_empty()
    }
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

/// Walks every installation of every launcher in `launchers` and renders what
/// is in it.
///
/// The traversal itself is [`dxray_core::walk`], shared with the terminal
/// browser. What is here is the presentation: the row layout, the order the
/// games and the unreadable records appear in, and the counters the trailer is
/// written from.
///
/// Both presentations, in one pass. The returned [`Listing`] holds the text
/// and the JSON of the same walk, whichever the caller ends up printing.
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
        // One reading per Proton build rather than one per game. A library
        // where a hundred games share one Proton would otherwise tokenise the
        // same two thousand lines a hundred times.
        builds: Builds::default(),
        install: None,
        presentation,
    };
    // Discarded, not ignored: this visitor never breaks, because it has nothing
    // to cancel and no channel that can go away. That is the whole of what a
    // caller with no cancellation has to write — there is no flag to pass and
    // no `Option` to unwrap, which is why the traversal takes neither.
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
    /// The installation the walk is currently inside, so every row below it can
    /// name where it came from.
    ///
    /// Remembered rather than passed, because [`dxray_core::walk`] announces a
    /// root before anything in it and never returns to one. `None` before the
    /// first root — a state no traversal produces today — is written as a JSON
    /// `null` rather than filled in with the library's own path, because a row
    /// that named the wrong install would be a wrong answer wearing a right
    /// one's clothes.
    install: Option<PathBuf>,
}

/// What a worded row means for the exit code, and what a program should call
/// it.
///
/// [`Listing`] already draws this line with two lists, because it is the whole
/// difference between a caveat and a failed scan. Naming it at the one call
/// site that fills them is what stops the row a person reads, the object a
/// program reads and the list the status is drawn from being given three
/// different answers about one sentence.
#[derive(Clone, Copy)]
enum Kind {
    /// Everything was read, and there may be less of it than expected. The
    /// [`Cause`] rides along because the caller is the only one who knows it:
    /// by the time the trailer counts a note, the sentence is worded and the
    /// part of the walk that raised it is gone.
    Note(Cause),
    /// Something could not be read.
    Problem,
}

impl Render {
    /// Files one worded item: the row a person reads, the object a program
    /// reads, and the list the exit code is drawn from.
    ///
    /// One call, three destinations, on purpose. A problem that reached the
    /// listing without reaching the JSON would be a scan telling a script it
    /// was clean while telling a person it was not, and the only way to make
    /// that unwritable is to leave no way of doing one without the others.
    ///
    /// `label` is the word the human row is filed under — `note`, `error` or
    /// `unreadable` — and travels into the JSON beside the kind, because it
    /// says something the kind does not: an `error` is the launcher's index and
    /// an `unreadable` is one record inside a library, and both are problems.
    ///
    /// A note also carries its [`Cause`] as a `cause` key, because for a note
    /// the label carries nothing: both causes are filed under `note`, which is
    /// the right word for a person and no word at all for a program. Written
    /// from [`Cause::key`], the same call the summary counts under, so the row
    /// and the count cannot name one note two ways.
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

    /// Printed before the libraries it qualifies, because it explains why there
    /// is only one of them. On stdout, not only on stderr: a caveat that lives
    /// on the other stream is the half that gets lost when the output is pasted
    /// somewhere.
    ///
    /// [`Cause::Index`], and this callback is the only place that can say so:
    /// [`dxray_core::walk`] raises it from the launcher's index, before a
    /// library has been opened, and that is exactly what the trailer's "there
    /// may be more" is about.
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

    /// A root whose index could not be read is still a launcher installation;
    /// what failed is the file that says where its other libraries are.
    ///
    /// Written into the listing as well as collected, for the same reason the
    /// notes are: a root that prints its own path and then nothing underneath
    /// reads as an install with no games, and the line saying why must not be
    /// on the stream that gets dropped when the output is pasted somewhere.
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
        // A library that produced nothing but failures is not an empty library,
        // and "(no games installed)" would be a claim about a directory nobody
        // managed to look inside. The rows below say what happened instead.
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
        // Records that could not be used but cost nothing: printed with the
        // reason they cost nothing, and counted as notes so the exit code stays
        // out of it. A Heroic cache entry that lost its install path for a game
        // another cache still supplies is the case this exists for.
        //
        // `Cause::Record`, because this is one entry inside a library that was
        // read in full — the same line the `error`/`unreadable` labels draw
        // between a launcher's index and a record inside it, drawn one list
        // over. It is the trailer's "nothing is missing because of it", and
        // wording it as an index note named a file nobody had touched.
        for note in catalogue.notes {
            self.record(
                "note",
                Kind::Note(Cause::Record),
                launcher.origin(),
                Some(library),
                note,
            );
        }
        // The games are shown first and the unreadable records named after
        // them. A manifest this tool cannot read is still a game the user owns,
        // so the count of what was found is not the whole story and the listing
        // must not let it look like it is.
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

/// One labelled row, wrapped at [`crate::wrap::WIDTH`] with continuations lined
/// up under the value rather than under the label.
///
/// Wrapped because a note is a sentence, not a path: three lines of prose
/// running off the right edge of a terminal is a caveat nobody finishes
/// reading, which defeats the point of printing it. A single word longer than
/// the remaining space — which is what a long path is — goes on its own line
/// and overflows instead of being split, because half a path is not findable.
fn row(out: &mut String, label: &str, value: &str) {
    let _ = write!(out, "  {label:<LABEL$}");
    crate::wrap::prose(out, INDENT, INDENT, value);
}

/// One line per game, with the install directory under it and the executable
/// this tool would analyse under that.
///
/// Several lines rather than one because these paths are long and the title is
/// what a person scans for. Putting them on one line pushes every title out of
/// alignment as soon as one library lives on a deeper mount than the others.
///
/// The best candidate only, with its strongest reason — not the whole ranking.
/// A library holds a hundred games and a full ranking for each would bury the
/// listing; `dxray game <path>` prints the rest, and the line here says the
/// score so a reader can see which answers are thin ones.
///
/// Every caveat the survey raised *is* printed, including the ones that say the
/// walk stopped early. Those are exactly the ones a short listing would most
/// like to drop, and dropping them is how a truncated answer starts looking
/// like a complete one.
///
/// # The order, and why there is no extra marker on it
///
/// Two groups, never fewer rows. The installs that were read and carry no
/// evidence of being a game are printed under the ones that do, rather than
/// hidden: on a measured library that is roughly four rows in nine — Proton
/// builds and Steam runtimes — and an honest inventory that cannot be read is
/// not much of an inventory. Erring by showing something extra beats erring by
/// hiding a game, so this demotes; it never filters, and the trailer counts
/// every game in both groups.
///
/// The question is [`Best::lacks_evidence`](crate::game::Best::lacks_evidence),
/// which is `dxray-core`'s, so this listing and the terminal browser demote the
/// same installs. An install that could not be read stays with the games: it
/// was never asked, and its row says so.
///
/// Within each group the launcher's own order is kept. There is no second sort
/// key, because reordering games against each other would be this listing
/// claiming a ranking between games that no evidence supports.
///
/// Nothing is appended to a demoted game's rows. The browser needs a ` no
/// evidence` marker because its list column shows a headline — a renderer
/// verdict — which on a Proton build reads as a confident answer about a game.
/// The column here is already the sentence: `best` reads "(nothing here carries
/// evidence of being a game; the highest ranked of N executables is X)", or
/// "(no executable in this directory)" where there was nothing to rank. Both
/// say what the marker would say and say more, and a marker repeating the row
/// it sits above is how a second vocabulary for one fact gets started.
///
/// # Partitioning without buffering the library
///
/// The order needs each game's answer before that game is printed, not before
/// the library is. So the inspections still run one game at a time, and the
/// rendered rows of a demoted game — not its facts — wait in a buffer that is
/// appended when the library ends. What is held at once is the rendering of
/// the demoted share of one library, plus the one game being written.
///
/// # Both renderings come out of one pass
///
/// Every row below is written to the terminal listing and to the JSON in the
/// same breath, out of the same values, and the rows themselves are decided
/// once into `rows` and rendered twice. `--json` does not select a renderer;
/// it selects which of two finished renderings is printed. That is not
/// economy — it is the only arrangement in which a launcher behaving oddly
/// cannot produce one story for a person and another for a program, which is
/// the failure this file has already paid for in other forms.
///
/// # The origin row
///
/// Printed per game rather than per root, because the root is not where the
/// answer lives: one Heroic configuration holds Epic, GOG and Amazon games
/// together, and a label on the directory above them would be wrong for two out
/// of three. It comes off the game's own [`Origin`], so a launcher added to the
/// registry names itself here and no branch in this file has to learn it.
///
/// # The NVAPI rows, and why they cost no exit code
///
/// Each game also gets what Proton will do to its NVAPI, read out of the
/// launcher script of the build its prefix says it last ran under. A row is
/// printed for every game including the ones with no answer, because a game
/// with nothing said about it reads as a game with nothing wrong with it. A
/// game from a launcher with no Steam application id is told in words that the
/// question does not apply to it, which is a different statement from silence.
///
/// None of it moves the exit code, and that is not an oversight. The status
/// this listing returns answers one question — was every game the launcher
/// declared actually read — and an NVAPI answer is not part of that count. Most
/// of a real library has never been launched under Proton and so has no prefix
/// and no build to read, which is the ordinary state of a healthy machine
/// rather than a scan that came up short. `dxray nvapi <build>` is the mode
/// where failing to read a policy *is* the failure, and it exits 1 for it.
fn render_games(
    sink: &mut Sink<'_>,
    games: &[Game],
    place: Place<'_>,
    builds: &mut Builds,
    name_origins: bool,
    presentation: crate::report::Presentation,
) {
    if games.is_empty() {
        // No JSON counterpart, and none is missing: this line is the *absence*
        // of game rows under a library, and a consumer grouping games by
        // library reads exactly that from the library row with nothing under
        // it. A row asserting emptiness would be the only object here that
        // states a fact nothing observed.
        let _ = writeln!(sink.text, "    (no games installed)");
        return;
    }
    // The second group, held back until the first is finished. Rendered rows,
    // not facts: each game is still inspected and rendered one at a time, in
    // the order the launcher declared it, and only the finished rows of a
    // demoted game wait here. A library holds tens of games and this holds the
    // rows of the ones that argue nothing — a few kilobytes — where
    // partitioning the games first would have made every inspection in the
    // library happen before the first line was printed.
    let mut ordinary = Vec::new();
    let mut demoted = Vec::new();
    for game in games {
        // One call, shared with the terminal browser, so that the facts
        // established about a game cannot depend on which surface asked.
        let facts = dxray_core::inspect(game, place.library, builds);
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

        // The rows this game gets, decided once and rendered twice. The origin
        // row is the listing's own — see above — and the two Proton rows come
        // from the inspection; everything between them is `best_rows`, so a row
        // added there appears on both surfaces without either learning its
        // name.
        let mut rows = Vec::new();
        if name_origins {
            rows.push(("origin", game.origin.label().to_owned()));
        }
        rows.extend(best.rows);
        // The build is named beside the verdict, never instead of it. Which
        // Proton ran a game decides the answer — the policy changed direction
        // twice across releases — so a verdict with no build behind it is not
        // checkable by the person reading it.
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

        // Collected in the order the launcher declared its games rather than
        // the order this prints them. The rows say it to a person and the
        // game's own `incomplete` array says it to a program; this is what says
        // it to the exit code, and a game moved to the bottom of the listing
        // must not be able to lose its caveat on the way down.
        sink.incomplete.extend(best.incomplete);

        // Both renderings of one game move together, into the same group. The
        // pair is what stops a demoted game from being demoted on one surface
        // and not on the other.
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
/// complete grouping is known. Analysis happens one game at a time; only the
/// small rendered records wait long enough for the tree to draw a truthful
/// final branch.
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

#[cfg(test)]
#[allow(dead_code)]
fn inventory_entry(
    game: &Game,
    place: Place<'_>,
    facts: &dxray_core::inspect::Inspection,
    presentation: crate::report::Presentation,
) -> String {
    use crate::report::Presentation;
    if presentation == Presentation::Compact {
        return compact_entry(game, place.library, facts);
    }
    let mut out = format!("\nEntry: {}\n", game.name);
    game_row(&mut out, "origin", game.origin.label());
    game_row(&mut out, "identity", &game.identity.to_string());
    if let Some(appid) = game.identity.steam_appid() {
        game_row(&mut out, "AppID", &appid.to_string());
    }
    game_row(&mut out, "install", &display_path(&game.install_dir));
    if presentation == Presentation::Full {
        game_row(&mut out, "library", &display_path(place.library));
        if let Some(root) = place.install {
            game_row(&mut out, "launcher root", &display_path(root));
        }
    }
    match &facts.survey {
        Ok(survey) => {
            game_row(
                &mut out,
                "search",
                if survey.is_incomplete() {
                    "incomplete"
                } else {
                    "complete within scan scope"
                },
            );
            if presentation == Presentation::Full {
                out.push_str(&crate::game::ranking_paths(&game.install_dir, survey, true));
            } else {
                for note in &survey.notes {
                    game_row(&mut out, "caveat", &note.to_string());
                }
            }
            if !survey.has_evidence() {
                game_row(
                    &mut out,
                    "caveat",
                    "No executable carries evidence of being the game; this does not classify the installation as a tool.",
                );
            }
            if let Some(best) = survey.best() {
                let record = crate::record::Record::read(&best.path);
                out.push_str(&crate::report::present_with_context(
                    &record,
                    presentation,
                    Some((game.origin.label(), &facts.nvapi.verdict)),
                ));
            } else {
                game_row(
                    &mut out,
                    "renderer",
                    "not determined statically: no executable found",
                );
                game_row(&mut out, "NVAPI", &facts.nvapi.verdict);
            }
        }
        Err(error) => {
            game_row(
                &mut out,
                "search",
                &format!("installation could not be read: {error}"),
            );
            game_row(
                &mut out,
                "renderer",
                "not determined statically: installation unreadable",
            );
            game_row(&mut out, "NVAPI", &facts.nvapi.verdict);
        }
    }
    if let Some(script) = &facts.nvapi.script {
        game_row(&mut out, "Proton script", &display_path(script));
    }
    game_row(
        &mut out,
        "policy scope",
        "NVAPI is a static Proton policy finding, not confirmation of runtime use.",
    );
    out
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

#[cfg(test)]
#[allow(dead_code)]
fn compact_entry(game: &Game, library: &Path, facts: &dxray_core::inspect::Inspection) -> String {
    let mut out = format!(
        "\nEntry: {} | {} {} {} | {}\n",
        game.name,
        game.origin.label(),
        if game.identity.steam_appid().is_some() {
            "AppID"
        } else {
            "ID"
        },
        game.identity,
        compact_install_path(&game.install_dir, library),
    );

    match &facts.survey {
        Ok(survey) => {
            if let Some(best) = survey.best() {
                let record = crate::record::Record::read(&best.path);
                if record.error.is_some() {
                    out.push_str("  renderer: evidence unavailable (executable unreadable)\n");
                } else {
                    let _ = write!(
                        out,
                        "  renderer: {} (static evidence)",
                        record.verdict.headline()
                    );
                    if !record.verdict.features.is_empty() {
                        let names: Vec<_> = record
                            .verdict
                            .features
                            .iter()
                            .map(|feature| feature.name.as_str())
                            .collect();
                        let _ = write!(out, " | features: {}", names.join(", "));
                    }
                    out.push('\n');
                }
            } else {
                out.push_str("  renderer: not determined statically (no executable found)\n");
            }

            let mut caveats = Vec::new();
            if !survey.has_evidence() {
                caveats.push("no static game evidence");
            }
            if survey.is_incomplete() {
                caveats.push("not searched in full");
            }
            if !caveats.is_empty() {
                let _ = writeln!(out, "  caveat: {}", caveats.join("; "));
            }
        }
        Err(_) => {
            out.push_str("  renderer: evidence unavailable (installation could not be read)\n");
        }
    }
    out
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
                tree_row(&mut out, "Library", &display_path(place.library));
                if let Some(root) = place.install {
                    tree_row(&mut out, "Launcher root", &display_path(root));
                }
                if let Some(report) = &details.full_report {
                    tree_block(&mut out, report);
                }
                if let Some(ranking) = &details.full_ranking {
                    tree_block(&mut out, ranking);
                }
            }
        }
        Some(error) => tree_row(&mut out, "Status", error),
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
    out
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
///
/// One struct rather than three arguments, so that a caller cannot hand over
/// somewhere to print and forget somewhere to report.
struct Sink<'a> {
    text: &'a mut String,
    json: &'a mut String,
    /// Game directories that were not searched in full. See
    /// [`Listing::incomplete`].
    incomplete: &'a mut Vec<String>,
}

/// One thing, rendered both ways.
#[derive(Default)]
struct Rendered {
    text: String,
    json: String,
}

/// Where in the tree a library sits, so that every row under it can say so.
///
/// `install` is an `Option` because nothing in this file may invent one: see
/// [`Render::install`].
#[derive(Clone, Copy)]
struct Place<'a> {
    install: Option<&'a Path>,
    library: &'a Path,
}

/// One game as a JSON object: where it is, what its launcher calls it, and
/// every row printed under it.
///
/// `directory` is spelled the way `game --json` spells it, because it is the
/// same thing and the two modes are meant to be used together: this answers
/// "what is installed and where", and that path handed to `dxray game <dir>
/// --json` answers "what is inside it", with the whole ranking, the scores and
/// the reasons. This shape deliberately does not repeat that ranking — a
/// hundred-game library would bury it — so `rows` carries what the terminal
/// listing shows and no more.
///
/// `rows` is prose, and labelled. A consumer reading it is reading what a
/// person would read, which is why the labels are there: `best`, `note`,
/// `unread`, `origin`, `proton`, `nvapi`. Nothing here should be matched on by
/// its wording.
///
/// `carries_evidence` is three-state and `null` is not `false`. `false` means
/// the install was read and nothing in it argues it is a game — a Proton build
/// or a redistributable — and `null` means the directory could not be read and
/// the question was never put. Collapsing the two would hide a failure under a
/// finding, which is the distinction
/// [`dxray_core::inspect::lacks_evidence`] exists to keep.
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
    // The one identifier in this project that opens a door, and the reason
    // `Identity` is an enum: a Heroic game has no Steam application id, so it
    // gets `null` rather than a zero that is also a real appid.
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

/// The launcher a row came from: the stable key first, the worded label after.
///
/// Both, because they answer different questions. `key` is what a program
/// filters on and never changes; `label` is written for a person, may be
/// reworded, and is the only one that names a launcher's *backend* — every
/// Heroic store shares the key `heroic`, so "which shop sold this" appears
/// here only as `Heroic / GOG` in the label. A consumer that needs the shop as
/// data is asking for something [`Origin`] does not yet carry, and this says so
/// rather than pretending the label is a key.
///
/// Leaves a trailing comma, like every field writer in this file.
fn push_origin(out: &mut String, origin: Origin) {
    push_string(out, "origin", origin.key());
    out.push(',');
    push_string(out, "origin_label", origin.label());
    out.push(',');
}

/// Where in the tree a row sits: the installation, then the library inside it.
///
/// Both may be `null`, and neither is ever guessed. A problem raised against a
/// launcher's own index has no library — the file that failed is the one that
/// would have said where the libraries are — and writing the root's path there
/// would invent a library nobody looked in.
fn push_place(out: &mut String, install: Option<&Path>, library: Option<&Path>) {
    push_optional(out, "install", install.map(display_path).as_deref());
    out.push(',');
    push_optional(out, "library", library.map(display_path).as_deref());
    out.push(',');
}

/// One labelled row under a game, wrapped under the label.
#[cfg(test)]
#[allow(dead_code)]
fn game_row(out: &mut String, label: &str, value: &str) {
    let _ = write!(out, "    {:<8} {label:<7} ", "");
    crate::wrap::prose(out, GAME_INDENT, GAME_INDENT, value);
}

/// Where a game's own rows start: four spaces, the eight-wide identity column,
/// a space, and a seven-wide label.
#[cfg(test)]
#[allow(dead_code)]
const GAME_INDENT: usize = 4 + 8 + 1 + 7 + 1;

/// The message for a machine where none of `launchers` was found.
///
/// It lists where the search looked. "No Steam installation found" on its own
/// is unactionable: the install may well be somewhere real that this tool does
/// not know to check, and only the list of candidates makes that visible.
///
/// Asked about one launcher it names that launcher, because the reader asked
/// about exactly it. Asked about several it groups the candidates under each,
/// because a flat run of a dozen paths does not tell anybody which of them was
/// a Heroic that is not there.
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
        // Two launchers can look in the same directory, and printing it under
        // both headings is the truth. Only repeats within one launcher are
        // dropped, which is what a candidate list built from overlapping
        // environment variables produces.
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

/// The same message as [`nothing_found`], as one JSON object.
///
/// A `problem` object and not a shape of its own, because that is what it is:
/// something the run could not do, worded. It names no install and no library
/// because there was none, and no `summary` follows it — nothing was scanned,
/// so there is nothing to count, and a summary reading `0 games` with
/// `complete` set would be the reassuring-looking lie this whole flag exists to
/// refuse. The exit code is 1, as it is without `--json`.
///
/// The sentence is [`nothing_found`]'s own, candidate paths and newlines
/// included, so the program reading stdout and the person reading stderr are
/// told the same thing.
pub fn nothing_found_json(launchers: &[&dyn Launcher]) -> String {
    let mut out = String::from("{\"kind\":\"problem\",\"origin\":null,\"origin_label\":null,");
    push_place(&mut out, None, None);
    push_string(&mut out, "label", "error");
    out.push(',');
    push_string(&mut out, "says", nothing_found(launchers).trim_end());
    out.push('}');
    out
}

/// Steam alone: the set of launchers `steam` asks about.
///
/// A constant slice so the caller can hand it to [`scan`] exactly where
/// `installed` hands it [`launcher::all`](dxray_core::launcher::all). The two
/// flags differ in this value and nowhere else, which is the entire reason
/// `steam` did not have to keep its own listing.
pub const STEAM_ONLY: &[&dyn Launcher] = &[&dxray_core::steam::STEAM];

#[cfg(test)]
mod tests;
