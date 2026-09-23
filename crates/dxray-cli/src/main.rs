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
    after_help = "EXIT CODES:\n  0  analysis completed\n  1  unreadable input, incomplete scan or unanswered policy query\n  2  invalid command line; nothing scanned\n\ninstalled found no install: exit 1.\nUnknown static renderer evidence alone is not a failure."
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
    /// List installations declared by the supported launchers.
    #[command(
        after_help = "The default view is compact. Use --view full for complete evidence.\n\nExamples:\n  dxray installed\n  dxray installed --launcher steam\n  dxray installed --view full\n  dxray installed --json"
    )]
    Installed(InventoryArgs),
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

    /// Inventories prioritize scanning a library, so their default favors the
    /// compact tree. File and directory analysis retain the standard report.
    fn inventory_presentation(&self) -> report::Presentation {
        self.view
            .map_or(report::Presentation::Compact, |view| match view {
                View::Compact => report::Presentation::Compact,
                View::Full => report::Presentation::Full,
            })
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
    /// Only this launcher; may be repeated. Every launcher by default.
    #[arg(long, value_name = "LAUNCHER", value_parser = launcher_keys())]
    launcher: Vec<String>,
    #[command(flatten)]
    output: OutputArgs,
}

impl InventoryArgs {
    /// The launchers asked about, in registry order.
    fn launchers(&self) -> Vec<&'static dyn dxray_core::Launcher> {
        dxray_core::launcher::all()
            .iter()
            .copied()
            .filter(|launcher| {
                self.launcher.is_empty()
                    || self
                        .launcher
                        .iter()
                        .any(|key| key == launcher.origin().key())
            })
            .collect()
    }
}

/// Every launcher's key, so a new launcher is accepted without editing this.
fn launcher_keys() -> clap::builder::PossibleValuesParser {
    dxray_core::launcher::all()
        .iter()
        .map(|launcher| launcher.origin().key())
        .collect::<Vec<_>>()
        .into()
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
        Commands::Installed(args) => list_games(&args.launchers(), &args.output),
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
    // One cache for the walk; it holds one directory's worth at a time.
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

/// Ranks the executables in each directory and analyses the best of each. A
/// truncated walk fails the run; a ranking on structure alone does not.
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

/// Prints one Proton build's NVAPI policy, and what it does to `appids`. Exits
/// 1 when the policy asked about could not be read.
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

/// Prints what `launchers` have installed. Finding no installation exits 1 and
/// names where it looked. `json` only chooses which rendering is printed.
fn list_games(launchers: &[&dyn dxray_core::Launcher], output: &OutputArgs) -> ExitCode {
    let json = output.json;
    let found = listing::scan_with_view(launchers, output.inventory_presentation());

    if found.roots == 0 {
        // Nothing scanned: no summary, but JSON callers still get a sentence.
        eprint!("{}", listing::nothing_found(launchers));
        if json {
            // `writeln!` rather than `println!`: a broken pipe must not panic.
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
        // The rows and their summary come together, never apart.
        let _ = write!(out, "{}", found.json());
    } else {
        let _ = write!(out, "{}", found.text);
        let _ = writeln!(out, "{}", found.trailer());
    }
    let _ = out.flush();

    // Everything that moves the exit code is echoed on stderr too.
    for problem in found
        .problems
        .iter()
        .chain(found.incomplete.iter().flatten())
    {
        eprintln!("dxray: {problem}");
    }

    // The same predicate the trailer is written from.
    if found.incomplete_scan() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
