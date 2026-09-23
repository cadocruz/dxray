//! The `nvapi` listing: what one Proton build will do to a game's NVAPI, and
//! how much of its script was understood. The evidence is printed before the
//! answers, since an answer is only worth what was read to reach it.

use std::fmt::Write as _;
use std::path::Path;

use dxray_core::nvapi::{self, Policy, Reading};
use dxray_core::proton;

use crate::wrap;

/// Width of the label column, matching the `installed` listing so the two read as
/// one tool.
const LABEL: usize = 12;
/// Where a wrapped value continues, under the first word of the one above it.
const INDENT: usize = 2 + LABEL;

/// What one `nvapi` run produced.
pub struct Outcome {
    pub text: String,
    /// True when a question that was asked got no answer. Stricter than
    /// `installed`: here the policy is the question.
    pub failed: bool,
}

/// Reads the policy in `target` and says what it does to `appids`.
pub fn inspect(target: &Path, appids: &[String]) -> Outcome {
    let mut text = String::new();
    // The banner, written once, before anything that can fail.
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
        // A script that defeated the reader must not look like one with no policy.
        Err(error) => return failure(text, &error),
    };

    row(&mut text, "read", &reading.evidence());
    row(&mut text, "policy", &policy_line(&reading));
    sites(&mut text, &reading);

    // The same answer with or without an appid. A build with no per-game policy
    // is settled, not failed.
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

/// The sentence that says which way the policy runs, in plain words.
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
        // No direction: the build's own account, since each way of having
        // none is different news.
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

/// Prints a condition verbatim; the reader knows their machine, and this tool
/// does not look.
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
        // A path that finds nothing is not a build with no policy.
        let outcome = inspect(Path::new("/definitely/not/here"), &[]);

        assert!(outcome.failed);
        assert!(outcome.text.contains("unreadable"), "got {}", outcome.text);
    }

    #[test]
    fn the_policy_line_spells_out_which_way_round_the_lists_run() {
        // Said in plain words, not this crate's vocabulary.
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
