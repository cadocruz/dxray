//! Keeps terminal dependencies, dev ones included, out of `dxray`, read from
//! the lockfile.

use std::collections::{HashMap, HashSet};

/// The workspace lockfile, read from this crate's own manifest directory so the
/// test does not care what the working directory is when it runs.
fn lockfile() -> String {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock");
    std::fs::read_to_string(root)
        .unwrap_or_else(|error| panic!("the workspace lockfile must be readable: {root}: {error}"))
}

/// Package name to the names of everything it depends on. An entry's first
/// word is the name; versions are dropped.
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
        // Undescribed in a hand-edited lock: skipped, not panicked over.
        if let Some(dependencies) = packages.get(&package) {
            queue.extend(dependencies.iter().cloned());
        }
    }
    seen
}

#[test]
fn dxray_cannot_reach_a_terminal_library() {
    // No ratatui in the scriptable binary.
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
    // The full allow-list, so any new dependency is a deliberate edit.
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
    // The walk finds a dependency two links away.
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
