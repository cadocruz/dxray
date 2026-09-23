//! Launch options from `localconfig.vdf`, as the environment they hand Proton.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::vdf::{self, Object};

/// The keys from the top of `localconfig.vdf` down to the per-app blocks.
/// Unverified against a real file.
const APPS: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];

/// Every account's launch options on one Steam installation.
#[derive(Debug, Default)]
pub struct Launches {
    accounts: Vec<HashMap<u32, String>>,
    problems: Vec<String>,
}

/// Reads `<root>/userdata/*/config/localconfig.vdf`, one per account.
#[must_use]
pub fn launches(root: &Path) -> Launches {
    let mut out = Launches::default();
    let entries = match fs::read_dir(root.join("userdata")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return out,
        Err(error) => {
            out.problems
                .push(format!("{}: {error}", root.join("userdata").display()));
            return out;
        }
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path().join("config").join("localconfig.vdf"))
        .filter(|file| file.is_file())
        .collect();
    files.sort();
    for file in files {
        let parsed = fs::read_to_string(&file)
            .map_err(|error| error.to_string())
            .and_then(|text| vdf::parse(&text).map_err(|error| error.to_string()));
        match parsed {
            Ok(value) => out
                .accounts
                .push(value.as_object().map(apps).unwrap_or_default()),
            Err(error) => out.problems.push(format!("{}: {error}", file.display())),
        }
    }
    out
}

fn apps(root: &Object) -> HashMap<u32, String> {
    let Some(apps) = APPS
        .iter()
        .try_fold(root, |object, key| object.get_object(key))
    else {
        return HashMap::new();
    };
    apps.iter()
        .filter_map(|(key, value)| {
            let appid = key.parse().ok()?;
            let options = value.as_object()?.get_str("LaunchOptions")?;
            Some((appid, options.to_owned()))
        })
        .collect()
}

impl Launches {
    /// The variables the launch options set for `appid`.
    ///
    /// # Errors
    ///
    /// When a `localconfig.vdf` could not be read, the options need a shell,
    /// or accounts set different options for the game.
    pub fn environment(&self, appid: u32) -> Result<Vec<(String, String)>, String> {
        if let Some(problem) = self.problems.first() {
            return Err(format!("launch options could not be read: {problem}"));
        }
        let mut options = self
            .accounts
            .iter()
            .map(|account| account.get(&appid).map_or("", String::as_str));
        let Some(first) = options.next() else {
            return Ok(Vec::new());
        };
        if options.any(|other| other != first) {
            return Err(
                "accounts on this Steam installation set different launch options for this game"
                    .to_owned(),
            );
        }
        environment(first)
    }
}

/// The variables `options` sets before `%command%`, even behind `env`.
/// Without `%command%` they are the game's arguments, so nothing is set.
///
/// # Errors
///
/// When that part needs a shell (`$`, a backtick, a pipe or a redirection) or
/// a quote is not closed.
pub fn environment(options: &str) -> Result<Vec<(String, String)>, String> {
    let words = words(options)?;
    let Some(command) = words
        .iter()
        .position(|word| matches!(word, Word::Plain(text) if text == "%command%"))
    else {
        return Ok(Vec::new());
    };
    let mut values = Vec::new();
    let mut collecting = true;
    for word in &words[..command] {
        let Word::Plain(text) = word else {
            return Err(format!(
                "the launch options {options:?} need a shell to evaluate before %command%"
            ));
        };
        if !collecting {
            continue;
        }
        if let Some(pair) = assignment(text) {
            values.push(pair);
        } else if text != "env" {
            // Anything after the first command word is its arguments.
            collecting = false;
        }
    }
    Ok(values)
}

#[derive(Debug, PartialEq, Eq)]
enum Word {
    Plain(String),
    /// Syntax a shell would evaluate.
    Shell,
}

#[derive(Default)]
struct Current {
    text: String,
    started: bool,
    /// The word expands something, so its value is not known.
    shell: bool,
}

impl Current {
    fn push(&mut self, c: char) {
        self.started = true;
        self.text.push(c);
    }

    fn flush(&mut self, words: &mut Vec<Word>) {
        let word = std::mem::take(self);
        if word.shell {
            words.push(Word::Shell);
        } else if word.started {
            words.push(Word::Plain(word.text));
        }
    }
}

fn words(options: &str) -> Result<Vec<Word>, String> {
    let unclosed = || Err(format!("the launch options {options:?} leave a quote open"));
    let mut words = Vec::new();
    let mut current = Current::default();
    let mut chars = options.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' | '\r' => current.flush(&mut words),
            '\'' => {
                current.started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => current.push(c),
                        None => return unclosed(),
                    }
                }
            }
            '"' => {
                current.started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('$' | '`') => current.shell = true,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => current.push(c),
                            Some(c) => {
                                current.push('\\');
                                current.push(c);
                            }
                            None => return unclosed(),
                        },
                        Some(c) => current.push(c),
                        None => return unclosed(),
                    }
                }
            }
            '\\' => {
                current.started = true;
                if let Some(c) = chars.next() {
                    current.push(c);
                }
            }
            '$' | '`' => {
                current.started = true;
                current.shell = true;
            }
            ';' | '|' | '&' | '<' | '>' | '(' | ')' => {
                current.flush(&mut words);
                words.push(Word::Shell);
            }
            c => current.push(c),
        }
    }
    current.flush(&mut words);
    Ok(words)
}

/// `NAME=value`, where `NAME` is a shell variable name.
fn assignment(word: &str) -> Option<(String, String)> {
    let (name, value) = word.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    let valid = (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then(|| (name.to_owned(), value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{environment, launches};
    use crate::testutil::TempDir;

    fn pairs(values: &[(&str, &str)]) -> Vec<(String, String)> {
        values
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn assignments_before_the_command_are_the_environment() {
        assert_eq!(
            environment("PROTON_FORCE_NVAPI=1 %command%"),
            Ok(pairs(&[("PROTON_FORCE_NVAPI", "1")]))
        );
        assert_eq!(
            environment(r#"WINEDLLOVERRIDES="dxgi=n,b" PROTON_LOG=1 %command% -dx12"#),
            Ok(pairs(&[
                ("WINEDLLOVERRIDES", "dxgi=n,b"),
                ("PROTON_LOG", "1")
            ]))
        );
        assert_eq!(
            environment("env PROTON_LOG=1 %command%"),
            Ok(pairs(&[("PROTON_LOG", "1")]))
        );
    }

    #[test]
    fn an_assignment_after_a_wrapper_is_the_wrapper_argument() {
        assert_eq!(
            environment("DXVK_HUD=fps gamemoderun PROTON_LOG=1 %command%"),
            Ok(pairs(&[("DXVK_HUD", "fps")]))
        );
    }

    #[test]
    fn without_the_command_placeholder_the_options_are_game_arguments() {
        assert_eq!(environment("PROTON_FORCE_NVAPI=1 -dx12"), Ok(Vec::new()));
        assert_eq!(environment(""), Ok(Vec::new()));
    }

    #[test]
    fn shell_syntax_before_the_command_is_refused_and_after_it_ignored() {
        assert!(environment("PROTON_LOG=$HOME %command%").is_err());
        assert!(environment(r#"A="$(cat f)" %command%"#).is_err());
        assert!(environment("A=1 %command%'").is_err(), "an unclosed quote");
        assert_eq!(
            environment("PROTON_LOG=1 %command% > /tmp/log 2>&1"),
            Ok(pairs(&[("PROTON_LOG", "1")]))
        );
    }

    #[test]
    fn a_word_that_only_looks_like_an_assignment_is_not_one() {
        assert_eq!(
            environment("1A=1 %command%"),
            Ok(Vec::new()),
            "not a variable name, so it is the command"
        );
    }

    fn localconfig(options: &str) -> String {
        format!(
            "\"UserLocalConfigStore\"\n{{\n\t\"Software\"\n\t{{\n\t\t\"Valve\"\n\t\t{{\n\
             \t\t\t\"Steam\"\n\t\t\t{{\n\t\t\t\t\"apps\"\n\t\t\t\t{{\n\
             \t\t\t\t\t\"1088850\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1758500000\"\n\
             \t\t\t\t\t\t\"LaunchOptions\"\t\t\"{options}\"\n\t\t\t\t\t}}\n\
             \t\t\t\t}}\n\t\t\t}}\n\t\t}}\n\t}}\n}}\n"
        )
    }

    #[test]
    fn the_options_are_read_from_each_account_localconfig() {
        let root = TempDir::new("launches");
        root.write(
            "userdata/12345/config/localconfig.vdf",
            &localconfig(r#"PROTON_FORCE_NVAPI=1 WINEDLLOVERRIDES=\"dxgi=n,b\" %command%"#),
        );

        let found = launches(root.path());

        assert_eq!(
            found.environment(1_088_850),
            Ok(pairs(&[
                ("PROTON_FORCE_NVAPI", "1"),
                ("WINEDLLOVERRIDES", "dxgi=n,b")
            ]))
        );
        assert_eq!(
            found.environment(570),
            Ok(Vec::new()),
            "no entry, no options"
        );
    }

    #[test]
    fn no_userdata_means_no_launch_options() {
        let root = TempDir::new("no-userdata");
        assert_eq!(launches(root.path()).environment(570), Ok(Vec::new()));
    }

    #[test]
    fn two_accounts_that_disagree_leave_the_options_unknown() {
        let root = TempDir::new("two-accounts");
        root.write(
            "userdata/1/config/localconfig.vdf",
            &localconfig("PROTON_FORCE_NVAPI=1 %command%"),
        );
        root.write("userdata/2/config/localconfig.vdf", &localconfig(""));

        assert!(launches(root.path()).environment(1_088_850).is_err());
    }

    #[test]
    fn a_localconfig_that_does_not_parse_leaves_the_options_unknown() {
        let root = TempDir::new("broken-localconfig");
        root.write(
            "userdata/1/config/localconfig.vdf",
            "\"UserLocalConfigStore\" {",
        );

        assert!(launches(root.path()).environment(1_088_850).is_err());
    }
}
