//! The `--nvapi` listing: what one Proton build's launcher script will do to a
//! game's NVAPI, and how much of that script was actually understood.
//!
//! The judgement is `dxray-core`'s; this module lays it out and caches it. It
//! prints the evidence *before* the answers, in that order and never the other
//! way round, because "this game is not in the list" is worth one thing when
//! thirty lists were read and nothing at all when the function was never found.
//! A tool that printed the answer alone would be hiding the only thing that
//! says whether to believe it.
//!
//! **Nothing here has been run against a Proton on a real machine.** There is
//! none on the machine it was written on, and no Steam, no prefix and no NVIDIA
//! GPU either. It has been run against genuine upstream `proton` scripts, which
//! settles how the reader behaves on real source and settles nothing at all
//! about how a real install is laid out around it.

use std::fmt::Write as _;
use std::path::Path;

use dxray_core::nvapi::{self, Policy, Reading};
use dxray_core::proton;

use crate::wrap;

/// Width of the label column, matching the `--steam` listing so the two read as
/// one tool.
const LABEL: usize = 12;
/// Where a wrapped value continues, under the first word of the one above it.
const INDENT: usize = 2 + LABEL;

/// What one `--nvapi` run produced.
pub struct Outcome {
    pub text: String,
    /// True when a question that was asked did not get an answer.
    ///
    /// A stricter rule than `--steam` uses, and deliberately: here the policy
    /// *is* the question, so failing to read it is a failed run. In a listing
    /// that sweeps a whole machine, a game with no prefix is the ordinary state
    /// of most of a library and costs nothing. Both rules are written down
    /// where they are applied.
    pub failed: bool,
}

/// Reads the policy in `target` and says what it does to `appids`.
pub fn inspect(target: &Path, appids: &[String]) -> Outcome {
    let mut text = String::new();
    // The banner is written exactly once, here, before anything that can fail
    // with a path of its own. It used to be written again by the failure path,
    // which printed the file name twice above its own error.
    let script = match proton::resolve(target) {
        Ok(script) => {
            let _ = writeln!(text, "{}", script.display());
            script
        }
        Err(error) => {
            let _ = writeln!(text, "{}", target.display());
            return failure(text, &error);
        }
    };

    let source = match proton::read(&script) {
        Ok(source) => source,
        Err(error) => return failure(text, &error),
    };
    let reading = match nvapi::scan(&source) {
        Ok(reading) => reading,
        // The whole point of the pure module's error type reaching this far: a
        // script that defeated the reader must not be able to produce the same
        // output as one that was read and holds no policy.
        Err(error) => return failure(text, &error),
    };

    row(&mut text, "read", &reading.evidence());
    row(&mut text, "policy", &policy_line(&reading));
    sites(&mut text, &reading);

    // Drawn from the pure module's own answer about a game no script lists, so
    // a run that names no application id and a run that names one cannot
    // disagree about whether this build was read well enough to answer with.
    // A build with no per-game policy at all is *settled*: that is a finding
    // about the build, not a failure to read it.
    let mut failed = !nvapi::settled(&reading);
    for appid in appids {
        let decision = nvapi::decide(&reading, appid);
        failed |= decision.is_unknown();
        let _ = write!(text, "  {appid:<LABEL$}");
        wrap::prose(&mut text, INDENT, INDENT, &decision.to_string());
        if let Some(condition) = decision.condition() {
            quote(&mut text, &condition.source());
        }
    }

    Outcome { text, failed }
}

/// A run that never got as far as a policy. Still a row rather than a missing
/// one, and still on stdout with everything else. The banner is already there.
fn failure(mut text: String, error: &dyn std::fmt::Display) -> Outcome {
    row(&mut text, "unreadable", &error.to_string());
    Outcome { text, failed: true }
}

/// The sentence that says which way round the policy runs.
///
/// Spelled out rather than named, because "deny-list" is a word this tool made
/// up and the thing it stands for is the difference between a game keeping
/// NVAPI and losing it.
fn policy_line(reading: &Reading) -> String {
    match reading.policy() {
        Some(Policy::DenyList) => format!(
            "opt-out: every game gets NVAPI except the ones named in this script's {} \
             lists. Proton 9.0 and later work this way.",
            Policy::DenyList.flag()
        ),
        Some(Policy::AllowList) => format!(
            "opt-in: no game gets NVAPI except the ones named in this script's {} lists, \
             which is the opposite of what later releases do. Proton 6.3 to 8.0 work this \
             way.",
            Policy::AllowList.flag()
        ),
        // No direction to report. What is printed instead is the pure module's
        // own account of the build, because there is more than one way to have
        // no direction and they are not the same news: a build with no per-game
        // policy at all, one whose only lists are `forcenvapi`, one whose flags
        // are spelled some new way, and one whose policy was refused all land
        // here and must not share a sentence.
        None => nvapi::overall(reading).to_string(),
    }
}

/// One row per NVAPI list, and the refusals under them.
fn sites(text: &mut String, reading: &Reading) {
    for site in &reading.sites {
        let head = format!(
            "line {}, {}, {} {}",
            site.line,
            site.flag,
            site.appids.len(),
            if site.appids.len() == 1 {
                "game"
            } else {
                "games"
            },
        );
        match &site.condition {
            None => row(text, "list", &head),
            Some(condition) => {
                row(
                    text,
                    "list",
                    &format!("{head}, set only under {}", condition.summary()),
                );
                quote(text, &condition.source());
            }
        }
    }
    for refusal in &reading.refusals {
        row(text, "refused", &refusal.to_string());
    }
}

/// Prints a condition back verbatim.
///
/// Quoted rather than summarised because the reader of the output is the one
/// who gets to apply it: they know whether their machine has an NVIDIA module
/// loaded and this tool has decided, on purpose, not to look.
fn quote(text: &mut String, source: &[String]) {
    for line in source {
        let _ = writeln!(text, "{:INDENT$}| {line}", "");
    }
}

/// One labelled row, wrapped so a sentence can be finished.
fn row(out: &mut String, label: &str, value: &str) {
    let _ = write!(out, "  {label:<LABEL$}");
    wrap::prose(out, INDENT, INDENT, value);
}

#[cfg(test)]
mod tests {
    use super::{inspect, policy_line};
    use dxray_core::nvapi::scan;
    use std::path::Path;

    #[test]
    fn a_path_that_is_not_a_proton_is_a_row_rather_than_a_silence() {
        // `--nvapi /some/folder` that finds nothing must not go on to report
        // that the folder has no NVAPI policy, which reads as a fact about
        // Proton rather than about the path that was typed.
        let outcome = inspect(Path::new("/definitely/not/here"), &[]);

        assert!(outcome.failed);
        assert!(outcome.text.contains("unreadable"), "got {}", outcome.text);
    }

    #[test]
    fn the_policy_line_spells_out_which_way_round_the_lists_run() {
        // "deny-list" is a phrase this tool invented. What it stands for is the
        // difference between a game keeping NVAPI and losing it, so the line
        // says it in words that do not need this crate's vocabulary.
        let deny = scan(
            "def default_compat_config():\n    if appid in [\"1\"]:\n        \
             ret.add(\"disablenvapi\")\n",
        )
        .expect("reads");
        let allow = scan(
            "def default_compat_config():\n    if appid in [\"1\"]:\n        \
             ret.add(\"enablenvapi\")\n",
        )
        .expect("reads");

        assert!(policy_line(&deny).contains("every game gets NVAPI except"));
        assert!(policy_line(&allow).contains("no game gets NVAPI except"));
        assert!(
            policy_line(&allow).contains("opposite"),
            "the inversion is the thing a reader must not miss"
        );
    }
}
