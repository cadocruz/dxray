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

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Clone, Copy, ValueEnum)]
enum View {
    Compact,
    Full,
}

use record::Record;
use walk::Target;

#[derive(Parser)]
#[command(
    name = "dxray",
    version,
    about = "Statically inspect Windows games and launcher libraries.",
    after_help = "EXIT CODES:\n  0  analysis completed\n  1  unreadable input, incomplete scan or unanswered policy query\n  2  invalid command line; nothing scanned\n\ninstalled or steam found no install: exit 1.\nUnknown static renderer evidence alone is not a failure."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Inspect PE files or scan directories for executables and DLLs.
    Inspect(InspectArgs),
    /// Rank executables in game installations and inspect the best candidate.
    Game(GameArgs),
    /// List installations declared by all supported launchers.
    #[command(
        after_help = "Examples:\n  dxray installed --view compact\n  dxray installed --view full\n  dxray installed --json"
    )]
    Installed(InventoryArgs),
    /// List installations declared by Steam.
    #[command(
        after_help = "Examples:\n  dxray steam --view compact\n  dxray steam --view full\n  dxray steam --json"
    )]
    Steam(InventoryArgs),
    /// Read a Proton build's static NVAPI policy.
    Nvapi(NvapiArgs),
}

#[derive(Args)]
struct OutputArgs {
    /// Emit one JSON object per line.
    #[arg(long)]
    json: bool,
    /// Choose a human-readable presentation.
    #[arg(long, value_enum, conflicts_with = "json")]
    view: Option<View>,
}

impl OutputArgs {
    fn presentation(&self) -> report::Presentation {
        match self.view {
            Some(View::Compact) => report::Presentation::Compact,
            Some(View::Full) => report::Presentation::Full,
            None => report::Presentation::Standard,
        }
    }
}

#[derive(Args)]
struct InspectArgs {
    /// Files or directories to inspect.
    #[arg(required = true, value_name = "PATH")]
    paths: Vec<PathBuf>,
    #[command(flatten)]
    output: OutputArgs,
    /// Descend into subdirectories.
    #[arg(short, long)]
    recursive: bool,
}

#[derive(Args)]
struct GameArgs {
    /// Installation directories (individual files are also accepted).
    #[arg(required = true, value_name = "PATH")]
    paths: Vec<PathBuf>,
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Args)]
struct InventoryArgs {
    #[command(flatten)]
    output: OutputArgs,
}

#[derive(Args)]
struct NvapiArgs {
    /// Proton installation directory or proton script.
    #[arg(value_name = "PROTON_PATH")]
    proton_path: PathBuf,
    /// Query a Steam application ID; may be repeated.
    #[arg(long, value_name = "ID")]
    appid: Vec<String>,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Commands::Inspect(args) => inspect_files(&args),
        Commands::Game(args) => {
            rank_installs(&args.paths, args.output.json, args.output.presentation())
        }
        Commands::Installed(args) => list_games(dxray_core::launcher::all(), &args.output),
        Commands::Steam(args) => list_games(listing::STEAM_ONLY, &args.output),
        Commands::Nvapi(args) => read_policy(&args.proton_path, &args.appid),
    }
}

fn inspect_files(args: &InspectArgs) -> ExitCode {
    let presentation = args.output.presentation();
    let targets = walk::collect(&args.paths, args.recursive);

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
        let _ = if args.output.json {
            writeln!(out, "{}", record.to_json())
        } else {
            write!(out, "{}", report::present(&record, presentation))
        };
    }

    if !args.output.json
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
/// was asked about be read? A truncated walk fails; a ranking supported only
/// by directory structure carries a caveat without failing the scan.
fn rank_installs(paths: &[PathBuf], json: bool, presentation: report::Presentation) -> ExitCode {
    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    let mut failed = 0usize;
    for path in paths {
        let outcome = if path.is_dir() {
            game::inspect(path, json, presentation)
        } else {
            // Not an error. A mixed argument list should still do the obvious
            // thing with the files in it rather than refusing the whole run.
            let record = Record::read(path);
            let failed = record.error.is_some();
            let text = if json {
                format!("{}\n", record.to_json())
            } else {
                report::present(&record, presentation)
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
/// stricter rule than `steam` applies, and deliberately so: here the policy
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
/// Both `installed` and `steam` are this function; they differ in the set of
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
fn list_games(launchers: &[&dyn dxray_core::Launcher], output: &OutputArgs) -> ExitCode {
    let json = output.json;
    let found = listing::scan_with_view(launchers, output.presentation());

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
