//! `dxray` — what does this binary actually link against?
//!
//! Reads PE images off disk and reports the machine, the version resource and
//! the import tables. It never loads or runs anything it looks at.

mod game;
mod listing;
mod nvapi;
mod record;
mod report;
mod walk;
mod wrap;

use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

use record::Record;
use walk::Target;

#[derive(Parser)]
#[command(
    name = "dxray",
    version,
    about = "Reports the machine, version and imports of PE images.",
    after_help = "EXIT CODES:\n  \
        0  every file was read and parsed\n  \
        1  at least one file could not be; a scan stopped at one of its limits, so\n     \
           a file was never looked at; or --installed or --steam found no install\n     \
           of any launcher it was asked about, or could not read one of its\n     \
           records\n  \
        2  the command line was wrong; nothing was scanned\n\n\
        --nvapi exits 1 when the question it was asked did not get an answer: the\n\
        script could not be read, or it holds no policy this build understands.\n\
        In --installed and --steam the same state costs nothing, because a game\n\
        that has never been launched has no Proton to read and most of a library\n\
        is in that state.\n\n\
        A --game ranking that rests on directory structure alone is a caveat, not\n\
        a failure: it is printed in full and the exit code stays 0.\n\n\
        A truncated walk is a failure on every surface, whatever the part that\n\
        was read turned out to look like. A directory that truncated and showed\n\
        nothing worth reporting is the answer most likely to be wrong, not the\n\
        least: the binary that would have argued otherwise may be the part that\n\
        was never reached."
)]
// Five flags, each of which is genuinely a switch the user either passed or
// did not. The lint's remedy is a state enum, and clap builds this struct from
// the command line by field: collapsing the flags into one would move the
// parsing out of the derive and into a hand-written match that could disagree
// with `--help`. The mutual exclusions are declared on the fields instead,
// where clap enforces them.
#[expect(clippy::struct_excessive_bools, reason = "clap parses flags by field")]
struct Cli {
    /// Files to inspect, or directories to scan for `*.exe` and `*.dll`.
    // Refused alongside `--installed`, `--steam` and `--nvapi`, which inspect no
    // binaries and return before the paths are ever read. Without the conflict
    // `dxray <path> --steam` opened nothing, said nothing about it, and exited
    // 0 — a run that looked at none of its arguments, wearing a clean run's
    // clothes. `--game` is not listed: it is the one flag that reads them.
    #[arg(
        required_unless_present_any = ["installed", "steam", "nvapi"],
        conflicts_with_all = ["installed", "steam", "nvapi"],
        value_name = "PATH"
    )]
    paths: Vec<PathBuf>,

    /// Emit JSON, one object per line.
    ///
    /// Two shapes, because there are two questions. Scanning files emits one
    /// record per file: thirteen keys in a frozen order, read positionally by
    /// harnesses that already exist, and it is why nothing may be inserted into
    /// it. `--installed` and `--steam` emit the listing instead — one object
    /// per row of the listing, each tagged with a `kind` key that a file record
    /// does not have at all, so no reader can mistake one for the other and no
    /// line means two things. See [`listing::Listing::json`] for what that
    /// shape promises.
    ///
    /// The flag says "in JSON" and never "and also do something else": every
    /// mode emits exactly what it would have printed, and the exit code is the
    /// same in both renderings.
    #[arg(long, conflicts_with = "nvapi")]
    json: bool,

    /// Descend into subdirectories when scanning a directory.
    #[arg(long, short, conflicts_with_all = ["installed", "steam", "game", "nvapi"])]
    recursive: bool,

    /// Treat each directory as a game install: rank the executables in it,
    /// explain the ranking, and analyse the best one.
    ///
    /// Not the default for a bare directory, because `dxray <dir>` already
    /// means "report every image in here" and a flag that silently changed what
    /// an existing invocation did would be the worse kind of breakage.
    ///
    /// A path that is not a directory is inspected as an ordinary file, so a
    /// mixed argument list still does something sensible with all of it.
    // `--rank` would be the better name. This flag does not take a game or
    // report one; it ranks the executables in a directory and says why, which
    // is what `--rank <DIR>` announces and what `--game <DIR>` leaves the user
    // to guess — the more so now that `--installed` sits two fields below and
    // the pair `--game` / `--games` was one letter apart until this week.
    //
    // Not renamed, because `--game` is already in `--help`, in the README, in
    // the exit-code block above and in the end-to-end tests, and the rename is
    // a break in the command line for every existing invocation. Paying that
    // buys a better word and nothing else, so it waits for a release that is
    // breaking something anyway. The cost then: this attribute and the field,
    // the two `conflicts_with_all` lists that name `game`, the `--help` and
    // exit-code prose here, `crates/dxray-cli/tests/game.rs` and
    // `crates/dxray-cli/src/game.rs`, and the README's `--game` section —
    // an afternoon, all of it found by grepping for the flag.
    #[arg(long, conflicts_with_all = ["installed", "steam"])]
    game: bool,

    /// List everything the launchers on this machine declare installed, with
    /// the launcher named against each one and the executable each install
    /// ranks best.
    ///
    /// "Everything", not "every game", because that is what a launcher can
    /// actually be asked. Steam's own Proton builds, its Linux runtimes and its
    /// redistributables are applications Steam declares installed, and nothing
    /// here filters them out: `DownloadType` cannot reliably tell a tool from a
    /// game, and a filter that got it wrong would hide a game. The installs
    /// that were read and carry no evidence of being a game are printed after
    /// the ones that do, and each says so in its own row — so expect a Proton
    /// build to appear, with its executables ranked and a sentence explaining
    /// why nothing there argues it is a game.
    ///
    /// Not a flag per launcher. `--heroic` beside `--steam` multiplies with
    /// every launcher added, and it would have the command line inventing its
    /// own notion of "which store" beside the one the code already has: a
    /// launcher is a value, and this flag passes the whole registry of them to
    /// one listing. A launcher added to that registry appears here with no flag
    /// of its own.
    ///
    /// `--steam` is kept beside this rather than replaced by it. It asks a
    /// narrower question — what does Steam have — and it is honest about
    /// answering exactly that; this one answers "what is installed", which is
    /// the question somebody auditing a library was asking all along and could
    /// not previously get a complete answer to.
    ///
    /// `--json` is accepted, and emits the listing rather than the PE record
    /// line. It was refused for a long time, and the argument for refusing it
    /// was right about the half it was about: the record line is a frozen
    /// positional contract about one file on disk, it has no field that could
    /// hold a game, and widening it would break every harness reading it. What
    /// that forbids is *reusing the record shape*, not a second shape existing.
    /// So nothing here pretends to be a file: every object the listing emits
    /// leads with a `kind` key that no record has, a reader can tell them apart
    /// from the first characters of a line, and one line never means two
    /// things. See [`listing::Listing::json`].
    // The exclusions are declared on the other fields, which each name `installed`
    // themselves.
    #[arg(long)]
    installed: bool,

    /// List everything Steam declares installed, and where.
    ///
    /// The same listing as `--installed`, asked about Steam alone: one
    /// implementation, given a different set of launchers. It lists and ranks
    /// the same way, tools included — see `--installed` for what that means.
    ///
    /// Refused together with `--installed`, because the two name different
    /// questions and a run that silently answered one of them would be guessing
    /// which.
    ///
    /// A flag rather than a subcommand: `dxray <path>` is the whole existing
    /// surface and a subcommand would move every current invocation behind a
    /// verb. `--json` emits the same listing as one object per line, in the
    /// shape `--installed` uses and for the same reasons — one listing, two
    /// renderings, and a `kind` key that keeps it clear of the record line's
    /// frozen contract.
    // Every other exclusion this flag takes part in is declared on the other
    // field: `--recursive`, `--game`, `--nvapi` and the positional paths each
    // name `steam` themselves — `--json` no longer does, because it is a
    // rendering of this listing and not a competing question. `--installed` is
    // the exception, because
    // it is a flag rather than a field with its own conflict list, so the pair
    // has to be declared on one of the two and this is it. `exclusive = false`
    // stood here and did nothing — false is the default.
    #[arg(long, conflicts_with = "installed")]
    steam: bool,

    /// Read one Proton build's NVAPI policy: which games it withholds NVAPI
    /// from, which it hands it to, and how much of the script was understood.
    ///
    /// Takes a Proton install directory or the `proton` script inside one.
    /// Inspects no binaries — this reads what Proton will do to a game before
    /// the game has ever been run.
    ///
    /// A flag rather than a subcommand, for the same reason `--steam` is one:
    /// `dxray <path>` is the whole existing surface and a verb would move every
    /// current invocation behind it.
    ///
    /// `--json` is still refused here, and the reason is no longer the one that
    /// used to be given. It is not that a flag may carry only one shape —
    /// `--installed` and `--steam` now prove otherwise — it is that nobody has
    /// designed a shape for a Proton policy. That output is a table of appid
    /// ranges, a direction the policy points in, and a statement of how much of
    /// the script was understood; deciding how a machine should read *that* is
    /// a piece of work, and inventing it in passing here would leave a shape
    /// this project has to keep. The refusal is a usage error rather than a
    /// silently ignored flag, so nobody gets a run that quietly was not JSON.
    #[arg(long, value_name = "PATH", conflicts_with_all = ["installed", "steam", "game"])]
    nvapi: Option<PathBuf>,

    /// Ask what the `--nvapi` policy does to this Steam application id. May be
    /// given more than once.
    ///
    /// Without it, `--nvapi` reports the policy and stops, which is the form
    /// that answers "has this build's policy changed shape" rather than "what
    /// happens to my game".
    #[arg(long, value_name = "ID", requires = "nvapi")]
    appid: Vec<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(target) = &cli.nvapi {
        return read_policy(target, &cli.appid);
    }
    if cli.installed {
        return list_games(dxray_core::launcher::all(), cli.json);
    }
    if cli.steam {
        return list_games(listing::STEAM_ONLY, cli.json);
    }
    if cli.game {
        return rank_installs(&cli.paths, cli.json);
    }
    let targets = walk::collect(&cli.paths, cli.recursive);

    // Locked and buffered once: a directory scan is thousands of small writes,
    // and `println!` takes the lock and flushes on every one of them.
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    let mut total = 0usize;
    let mut failed = 0usize;
    // One cache for the whole walk. It scopes itself to a directory and drops
    // everything when the walk moves on, so a recursive scan of a library holds
    // one folder's worth at a time rather than the tree's.
    let mut cache = dxray_core::LibraryCache::default();
    for target in targets {
        let record = match target {
            Target::File(path) => Record::read_with(&path, &mut cache),
            // The walk already knows this one failed; re-reading it would only
            // produce a second, less specific error.
            Target::Failed(record) => *record,
        };
        total += 1;
        if record.error.is_some() {
            failed += 1;
        }

        // Write failures are not the caller's business and must not change the
        // exit code, which answers one question only: did every file parse?
        let _ = if cli.json {
            writeln!(out, "{}", record.to_json())
        } else {
            write!(out, "{}", report::render(&record))
        };
    }

    if !cli.json
        && let Some(line) = report::summary(total, failed, "files")
    {
        let _ = writeln!(out, "{line}");
    }
    let _ = out.flush();

    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Ranks the executables in each directory and analyses the best of each.
///
/// The exit code answers the same question it always did: could everything that
/// was asked about be read? A walk that stopped at a limit, and a ranking with
/// no import table behind it, are notes rather than failures — they are printed
/// in full and the answer they qualify is still an answer.
fn rank_installs(paths: &[PathBuf], json: bool) -> ExitCode {
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    let mut failed = 0usize;
    for path in paths {
        let outcome = if path.is_dir() {
            game::inspect(path, json)
        } else {
            // Not an error. A mixed argument list should still do the obvious
            // thing with the files in it rather than refusing the whole run.
            let record = Record::read(path);
            let failed = record.error.is_some();
            let text = if json {
                format!("{}\n", record.to_json())
            } else {
                report::render(&record)
            };
            game::Outcome { text, failed }
        };
        if outcome.failed {
            failed += 1;
        }
        let _ = out.write_all(outcome.text.as_bytes());
    }

    if !json && let Some(line) = report::summary(paths.len(), failed, "paths") {
        let _ = writeln!(out, "{line}");
    }
    let _ = out.flush();

    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Prints one Proton build's NVAPI policy, and what it does to `appids`.
///
/// Exits 1 when a question that was asked did not get an answer — the script
/// could not be read, or it holds no policy this build understands. That is a
/// stricter rule than `--steam` applies, and deliberately so: here the policy
/// *is* what was asked about, so failing to read it is a failed run.
fn read_policy(target: &Path, appids: &[String]) -> ExitCode {
    let outcome = nvapi::inspect(target, appids);

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let _ = out.write_all(outcome.text.as_bytes());
    let _ = out.flush();

    if outcome.failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Prints what `launchers` have installed on this machine.
///
/// Both `--installed` and `--steam` are this function; they differ in the set of
/// launchers they hand it and in nothing else. That is what keeps the wider
/// flag from being a second implementation of the narrower one — the failure
/// this project has already paid for three times.
///
/// Finding no installation of any launcher asked about exits 1 with a message
/// naming the directories that were searched. An empty list and a 0 would be
/// the same output a working scan of an empty machine produces, and the two
/// states are not the same state: one means "you own no games", the other means
/// "this tool did not find your launcher".
///
/// One launcher failing does not stop another being listed: every failure
/// arrives as a worded problem beside the games, so a Steam whose index cannot
/// be read still leaves the Heroic games on the screen — and still moves the
/// exit code, because something was not read.
///
/// `json` chooses which of the two finished renderings is printed and changes
/// nothing else. One scan, one set of counts, one exit code: the flag cannot
/// alter what a run considers a failure, because the status is drawn from the
/// listing and not from what was printed.
fn list_games(launchers: &[&dyn dxray_core::Launcher], json: bool) -> ExitCode {
    let found = listing::scan(launchers);

    if found.roots == 0 {
        // Nothing was scanned, so there are no counts to report and no summary
        // is emitted — the shape says a scan that ran ends with one. A JSON
        // caller still gets a sentence rather than a blank stream: the same
        // message, as one `problem` object, because an empty stdout and a
        // machine with no games installed would otherwise look alike.
        eprint!("{}", listing::nothing_found(launchers));
        if json {
            // Through `write!` rather than `println!`, like every other write
            // here: a run piped into `head` must not turn a broken pipe into a
            // panic, and a write failure is not the caller's business and must
            // not change the exit code.
            let _ = writeln!(
                std::io::stdout(),
                "{}",
                listing::nothing_found_json(launchers)
            );
        }
        return ExitCode::FAILURE;
    }

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    if json {
        // One call, and the summary comes with it. There is no way to ask for
        // the rows alone, which is what stops a run reporting games while
        // dropping the sentence that says the scan was partial.
        let _ = write!(out, "{}", found.json());
    } else {
        let _ = write!(out, "{}", found.text);
        let _ = writeln!(out, "{}", found.trailer());
    }
    let _ = out.flush();

    // The problems are repeated on stderr so that a piped run still shows them
    // and an eyeballed one does not have to scroll back through the listing.
    // Everything that moves the exit code is echoed here, for that reason and
    // no other: a caveat that changes the status must be visible on the stream
    // a script is most likely to have kept.
    for problem in found.problems.iter().chain(&found.incomplete) {
        eprintln!("dxray: {problem}");
    }

    // Drawn from the same predicate the trailer is written from, so a human
    // reading "2 game directories could not be searched in full" and a script
    // reading the status come away with the same story about one run.
    if found.incomplete_scan() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
