//! The firewall: `dxray` does not gain a terminal library.
//!
//! This slice takes the project from one dependency to sixty-eight. `dxray` is
//! the binary that goes in other people's scripts, and the argument for it has
//! always partly been that there is almost nothing behind it. A terminal
//! interface is worth having and is not worth spending that on, which is why it
//! is a second binary — and an intention nobody checks is an intention that
//! gets edited away by the next person who finds a `--tui` flag convenient.
//!
//! # Why the lockfile rather than `cargo tree`
//!
//! Shelling out to `cargo tree` in a test makes the test depend on a network,
//! on a registry index, and on the exit code of a subprocess. `Cargo.lock` is
//! checked in, is the resolver's own answer, and is a file. Parsing the few
//! lines of it that matter costs less than handling the ways a subprocess
//! fails, and it fails in the right direction: if the lockfile is missing or
//! unparseable the test fails rather than passing vacuously.
//!
//! # What it is strict about, on purpose
//!
//! A lockfile entry does not distinguish a normal dependency from a
//! development one, so a `dev-dependency` on ratatui in `dxray-cli` would fail
//! this test even though it would never be linked into the shipped binary.
//! That is the wanted behaviour and not a limitation worked around: the point
//! is that building and testing `dxray` stays cheap, and a dev-dependency costs
//! exactly as much to compile as a real one.
//!
//! # Why it lives here
//!
//! Beside the crate it protects, and not beside `dxray-tui`, so that `cargo
//! test -p dxray-cli` runs the guarantee and deleting the terminal interface
//! does not delete the thing guarding the binary that goes in scripts. It was
//! parked in `crates/dxray-tui/tests/` while that crate was excluded from the
//! workspace; it is a member now, and the reason expired with the exclusion.

use std::collections::{HashMap, HashSet};

/// The workspace lockfile, read from this crate's own manifest directory so the
/// test does not care what the working directory is when it runs.
fn lockfile() -> String {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock");
    std::fs::read_to_string(root)
        .unwrap_or_else(|error| panic!("the workspace lockfile must be readable: {root}: {error}"))
}

/// Package name to the names of everything it depends on.
///
/// Cargo writes a bare name when only one version of it is in the lock and
/// `"name version"` when there are several, so only the first word of an entry
/// is the name. Versions are dropped rather than kept: the question here is
/// whether a crate is reachable at all, and two versions of it would both be
/// two answers of yes.
fn graph(lock: &str) -> HashMap<String, Vec<String>> {
    let mut packages = HashMap::new();
    let mut name: Option<String> = None;
    let mut dependencies: Vec<String> = Vec::new();
    let mut in_dependencies = false;

    for line in lock.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            if let Some(name) = name.take() {
                packages.insert(name, std::mem::take(&mut dependencies));
            }
            in_dependencies = false;
        } else if let Some(value) = trimmed.strip_prefix("name = ") {
            name = Some(unquote(value).to_owned());
        } else if trimmed.starts_with("dependencies = [") {
            in_dependencies = true;
        } else if in_dependencies {
            if trimmed == "]" {
                in_dependencies = false;
            } else if let Some(first) = unquote(trimmed.trim_end_matches(',')).split(' ').next() {
                dependencies.push(first.to_owned());
            }
        }
    }
    if let Some(name) = name {
        packages.insert(name, dependencies);
    }
    packages
}

/// Strips one layer of double quotes, leaving anything unquoted alone.
fn unquote(value: &str) -> &str {
    value.trim().trim_matches('"')
}

/// Every package reachable from `from`, including `from` itself.
fn reachable(packages: &HashMap<String, Vec<String>>, from: &str) -> HashSet<String> {
    let mut seen = HashSet::new();
    let mut queue = vec![from.to_owned()];
    while let Some(package) = queue.pop() {
        if !seen.insert(package.clone()) {
            continue;
        }
        // A name with no entry of its own is a package the lock names as a
        // dependency and does not describe, which cannot happen in a lockfile
        // cargo wrote. It is skipped rather than panicked over so that a
        // hand-edited lock produces a dependency answer instead of a crash.
        if let Some(dependencies) = packages.get(&package) {
            queue.extend(dependencies.iter().cloned());
        }
    }
    seen
}

#[test]
fn dxray_cannot_reach_a_terminal_library() {
    // The whole argument for `dxray-tui` being a second binary. If this fails,
    // somebody has put ratatui behind a flag in the tool people script against,
    // and every scripted run now compiles sixty-eight crates to not use them.
    let lock = lockfile();
    let packages = graph(&lock);
    assert!(
        packages.contains_key("dxray-cli"),
        "the lockfile must describe dxray-cli, or this test proves nothing"
    );

    let reached = reachable(&packages, "dxray-cli");
    let terminal: Vec<&String> = reached
        .iter()
        .filter(|name| name.starts_with("ratatui") || *name == "crossterm")
        .collect();

    assert!(
        terminal.is_empty(),
        "dxray must not depend on a terminal library, directly or through \
         anything else; found {terminal:?} reachable from dxray-cli"
    );
}

#[test]
fn dxray_still_depends_on_almost_nothing() {
    // The stronger half, and the one that catches the case the test above
    // cannot: a dependency that is not ratatui and is not wanted either. The
    // list is spelled out so that adding to it is a deliberate edit somebody
    // has to justify in a diff, rather than a number quietly going up.
    let lock = lockfile();
    let reached = reachable(&graph(&lock), "dxray-cli");

    // clap, workspace crates, and serde_json used only by contract tests.
    let allowed: HashSet<&str> = [
        "dxray-cli",
        "dxray-core",
        "dxray-pe",
        "serde_json",
        "serde",
        "serde_core",
        "serde_derive",
        "itoa",
        "memchr",
        "zmij",
        "clap",
        "clap_builder",
        "clap_derive",
        "clap_lex",
        "anstream",
        "anstyle",
        "anstyle-parse",
        "anstyle-query",
        "anstyle-wincon",
        "colorchoice",
        "heck",
        "is_terminal_polyfill",
        "once_cell_polyfill",
        "proc-macro2",
        "quote",
        "strsim",
        "syn",
        "unicode-ident",
        "utf8parse",
        "windows-sys",
        "windows-targets",
        "windows_aarch64_gnullvm",
        "windows_aarch64_msvc",
        "windows_i686_gnu",
        "windows_i686_gnullvm",
        "windows_i686_msvc",
        "windows_x86_64_gnu",
        "windows_x86_64_gnullvm",
        "windows_x86_64_msvc",
        "windows-link",
    ]
    .into_iter()
    .collect();

    let unexpected: Vec<&String> = reached
        .iter()
        .filter(|name| !allowed.contains(name.as_str()))
        .collect();

    assert!(
        unexpected.is_empty(),
        "dxray gained a dependency nobody wrote down: {unexpected:?}. If it is \
         wanted, add it to this list and say why in the commit; if it is not, \
         this test just caught it."
    );
}

#[test]
fn the_walk_would_notice_a_terminal_library_if_there_were_one() {
    // A test that can only pass is not a test. This drives the same parser and
    // the same walk over a lockfile written here, where the dependency it is
    // looking for is definitely present and two links away rather than one —
    // so it also covers the case this exists to catch, which is ratatui
    // arriving through something else rather than being added directly.
    let lock = "\
[[package]]
name = \"pretend-cli\"
version = \"0.0.0\"
dependencies = [
 \"pretend-helper\",
]

[[package]]
name = \"pretend-helper\"
version = \"0.0.0\"
dependencies = [
 \"ratatui 0.30.2\",
]

[[package]]
name = \"ratatui\"
version = \"0.30.2\"
";
    let reached = reachable(&graph(lock), "pretend-cli");

    assert!(
        reached.contains("ratatui"),
        "the walk must follow a dependency through an intermediate crate, or the \
         real test above passes for the wrong reason"
    );
    assert!(
        reached.contains("pretend-helper"),
        "and must report the crate that brought it in, which is what a failure \
         message has to name to be actionable"
    );
}
