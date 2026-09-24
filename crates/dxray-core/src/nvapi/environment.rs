//! What Proton computes for `use_nvapi` once a launch's environment is known.
//!
//! The script's lists are only the default. `check_environment` calls run after
//! them and can add or remove each flag, so a launch option decides the answer
//! for one game. Everything here mirrors the script, read from its own lines:
//! the switches, the `use_nvapi` expression, and where `config_info` records it.

use super::{Flag, Logical, Policy, Reading, Token, find, items, lex};

/// A `check_environment("VARIABLE", "flag")` call for an NVAPI flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switch {
    pub variable: String,
    pub flag: Flag,
    /// 1-based line of the call.
    pub line: usize,
}

/// The `use_nvapi = ...` expressions this reader recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Formula {
    /// `'enablenvapi' in compat_config`: Proton 6.3 to 8.0.
    Enable,
    /// `'disablenvapi' not in compat_config or 'forcenvapi' in compat_config`:
    /// Proton 9.0 onwards. `arm_clause` is Experimental's extra
    /// `and host_pe_arch != "aarch64-windows"`.
    DisableUnlessForced { arm_clause: bool },
}

impl Formula {
    fn policy(self) -> Policy {
        match self {
            Self::Enable => Policy::AllowList,
            Self::DisableUnlessForced { .. } => Policy::DenyList,
        }
    }
}

/// Every NVAPI switch in the script, in order, and how many name their
/// variable in a way this reader cannot read.
pub(super) fn switches(lines: &[Logical]) -> (Vec<Switch>, usize) {
    let mut found = Vec::new();
    let mut unread = 0;
    for line in lines {
        let tokens = &line.tokens;
        for at in 0..tokens.len() {
            let [
                Token::Name(name),
                Token::Punct('('),
                variable,
                Token::Punct(','),
                Token::Text(flag),
                Token::Punct(')'),
                ..,
            ] = &tokens[at..]
            else {
                continue;
            };
            if name != "check_environment" {
                continue;
            }
            let Some(flag) = Flag::lookup(flag) else {
                continue;
            };
            match variable {
                Token::Text(variable) => found.push(Switch {
                    variable: variable.clone(),
                    flag,
                    line: line.line,
                }),
                _ => unread += 1,
            }
        }
    }
    (found, unread)
}

/// The recognized `use_nvapi` expression, if the script has one.
pub(super) fn formula(lines: &[Logical]) -> Option<Formula> {
    let rhs = lines.iter().find_map(|line| match line.tokens.as_slice() {
        [Token::Name(name), Token::Punct('='), rest @ ..]
            if name == "use_nvapi" && !matches!(rest.first(), Some(Token::Punct('='))) =>
        {
            Some(rest)
        }
        _ => None,
    })?;
    if shape(rhs, &ENABLE) {
        return Some(Formula::Enable);
    }
    if shape(rhs, &DISABLE_UNLESS_FORCED) {
        return Some(Formula::DisableUnlessForced { arm_clause: false });
    }
    // Experimental: `((<the 9.0 expression>) and <not an ARM host>)`.
    let outer = unwrap(rhs).filter(|inner| inner.len() + 2 == rhs.len())?;
    let core = unwrap(outer)?;
    let tail = &outer[core.len() + 2..];
    (shape(core, &DISABLE_UNLESS_FORCED) && shape(tail, &ARM_CLAUSE))
        .then_some(Formula::DisableUnlessForced { arm_clause: true })
}

/// 1-based line of `config_info` that holds `str(use_nvapi)`, read off the
/// `prefix_info` tuple: 14 up to 9.0, which still write `lib64_dir`, and 13
/// from 10.0.
pub(super) fn recorded_line(lines: &[Logical]) -> Option<usize> {
    let tokens = lines.iter().find_map(|line| match line.tokens.as_slice() {
        [Token::Name(name), Token::Punct('='), rest @ ..] if name == "prefix_info" => Some(rest),
        _ => None,
    })?;
    let join = tokens.windows(3).position(|w| {
        matches!(w, [Token::Name(name), Token::Punct('('), Token::Punct('(')] if name == "join")
    })?;
    let tuple = unwrap(&tokens[join + 2..])?;
    let recorded = [
        Token::Name("str".to_owned()),
        Token::Punct('('),
        Token::Name("use_nvapi".to_owned()),
        Token::Punct(')'),
    ];
    items(tuple)
        .iter()
        .position(|item| *item == recorded)
        .map(|at| at + 1)
}

/// The contents of `user_settings.py`, which fills in variables a launch did
/// not set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserSettings {
    /// No `user_settings.py` beside the script.
    Absent,
    Read(Vec<(String, String)>),
    /// Present, and not a plain dictionary of strings.
    Unreadable,
}

/// Reads `user_settings = {"KEY": "value", ...}`. Anything else is
/// [`UserSettings::Unreadable`]: Python may still build a dictionary from it.
#[must_use]
pub fn user_settings(text: &str) -> UserSettings {
    let Ok(lines) = lex(text) else {
        return UserSettings::Unreadable;
    };
    let body = lines.iter().find_map(|line| match line.tokens.as_slice() {
        [
            Token::Name(name),
            Token::Punct('='),
            Token::Punct('{'),
            inner @ ..,
            Token::Punct('}'),
        ] if name == "user_settings" => Some(inner),
        _ => None,
    });
    let Some(body) = body else {
        return UserSettings::Unreadable;
    };
    let mut values = Vec::new();
    for item in items(body) {
        match item {
            [] => {}
            [Token::Text(key), Token::Punct(':'), Token::Text(value)] => {
                values.push((key.clone(), value.clone()));
            }
            _ => return UserSettings::Unreadable,
        }
    }
    UserSettings::Read(values)
}

/// Where a variable came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SetBy {
    /// A Steam game's launch options.
    #[default]
    LaunchOptions,
    /// A Heroic game's environment options.
    Heroic,
    /// The variable Heroic sets from its DXVK-NVAPI setting
    /// (`autoInstallDxvkNvapi`), over the game's own options.
    HeroicSwitch,
    UserSettings,
}

impl SetBy {
    fn as_str(self) -> &'static str {
        match self {
            Self::LaunchOptions => "launch options",
            Self::Heroic => "Heroic's environment options",
            Self::HeroicSwitch => "Heroic's DXVK-NVAPI setting",
            Self::UserSettings => "user_settings.py",
        }
    }
}

/// The variables one launch hands Proton, as far as they can be seen.
///
/// Launch options win over `user_settings.py`, which only fills in what is
/// absent (`if key not in self.env` in every release since 6.3).
#[derive(Debug, Clone, Default)]
pub struct Environment {
    launch: Vec<(String, String, SetBy)>,
    settings: Vec<(String, String)>,
    launch_unseen: Option<String>,
    settings_unseen: Option<String>,
}

enum Lookup<'a> {
    Set(&'a str, SetBy),
    Unset,
    Unseen(&'a str),
}

impl Environment {
    /// `launch` is the parsed launch options, or why they could not be read.
    #[must_use]
    pub fn new(launch: Result<Vec<(String, String)>, String>, settings: &UserSettings) -> Self {
        let launch = launch.map(|values| {
            values
                .into_iter()
                .map(|(name, value)| (name, value, SetBy::LaunchOptions))
                .collect()
        });
        Self::sourced(launch, settings)
    }

    /// The same, with each launch variable naming where it came from.
    #[must_use]
    pub fn sourced(
        launch: Result<Vec<(String, String, SetBy)>, String>,
        settings: &UserSettings,
    ) -> Self {
        let (launch, launch_unseen) = match launch {
            Ok(values) => (values, None),
            Err(why) => (Vec::new(), Some(why)),
        };
        let (settings, settings_unseen) = match settings {
            UserSettings::Absent => (Vec::new(), None),
            UserSettings::Read(values) => (values.clone(), None),
            UserSettings::Unreadable => (
                Vec::new(),
                Some(
                    "the build's user_settings.py is not a dictionary this reader can read"
                        .to_owned(),
                ),
            ),
        };
        Self {
            launch,
            settings,
            launch_unseen,
            settings_unseen,
        }
    }

    fn get(&self, variable: &str) -> Lookup<'_> {
        // The last value given to `variable` wins, as the shell and a dict keep.
        if let Some((_, value, set_by)) =
            self.launch.iter().rev().find(|(name, ..)| name == variable)
        {
            return Lookup::Set(value, *set_by);
        }
        if let Some(why) = &self.launch_unseen {
            return Lookup::Unseen(why);
        }
        if let Some((_, value)) = self
            .settings
            .iter()
            .rev()
            .find(|(name, _)| name == variable)
        {
            return Lookup::Set(value, SetBy::UserSettings);
        }
        match &self.settings_unseen {
            Some(why) => Lookup::Unseen(why),
            None => Lookup::Unset,
        }
    }
}

/// One switch a launch sets, and what it does to its flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub variable: String,
    pub value: String,
    pub set_by: SetBy,
    pub flag: Flag,
    /// `false` when the value is `0` or empty, which removes the flag.
    pub added: bool,
}

impl Applied {
    /// `PROTON_FORCE_NVAPI=1 in launch options adds forcenvapi`.
    #[must_use]
    pub fn describe(&self) -> String {
        format!(
            "{}={} in {} {} {}",
            self.variable,
            self.value,
            self.set_by.as_str(),
            if self.added { "adds" } else { "removes" },
            self.flag
        )
    }
}

/// What the launch environment does to `use_nvapi`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Nothing in the environment reaches an NVAPI switch: the script's own
    /// policy stands, and [`decide`](super::decide) is the answer.
    Default,
    /// The value the script computes with these switches applied.
    Computed {
        use_nvapi: bool,
        applied: Vec<Applied>,
    },
    /// Why the value cannot be computed.
    Undetermined(String),
}

#[derive(Clone)]
enum State {
    Present,
    Absent,
    Uncertain(String),
}

/// Applies `environment` to the script's defaults for `appid` and evaluates
/// the `use_nvapi` expression, in the script's own order.
#[must_use]
pub fn resolve(reading: &Reading, appid: &str, environment: &Environment) -> Resolution {
    let mut disable = default_state(reading, Flag::Disable, appid);
    let mut enable = default_state(reading, Flag::Enable, appid);
    let mut force = default_state(reading, Flag::Force, appid);
    let mut applied = Vec::new();
    let mut touched = false;
    for switch in &reading.switches {
        let state = match environment.get(&switch.variable) {
            Lookup::Unset => continue,
            Lookup::Unseen(why) => State::Uncertain(why.to_owned()),
            Lookup::Set(value, set_by) => {
                let added = nonzero(value);
                applied.push(Applied {
                    variable: switch.variable.clone(),
                    value: value.to_owned(),
                    set_by,
                    flag: switch.flag,
                    added,
                });
                if added { State::Present } else { State::Absent }
            }
        };
        touched = true;
        match switch.flag {
            Flag::Disable => disable = state,
            Flag::Enable => enable = state,
            Flag::Force => force = state,
        }
    }
    if !touched {
        return Resolution::Default;
    }
    if reading.switches_unread > 0 {
        return Resolution::Undetermined(
            "an NVAPI switch in this script reads a variable this reader cannot name".to_owned(),
        );
    }
    let Some(formula) = reading.formula else {
        return Resolution::Undetermined(
            "the script's use_nvapi expression is not in a shape this reader knows".to_owned(),
        );
    };
    if reading
        .policy()
        .is_some_and(|policy| policy != formula.policy())
    {
        return Resolution::Undetermined(
            "the script's use_nvapi expression and its appid lists disagree about direction"
                .to_owned(),
        );
    }
    if matches!(formula, Formula::DisableUnlessForced { arm_clause: true })
        && !cfg!(target_arch = "x86_64")
    {
        return Resolution::Undetermined(
            "the script withholds NVAPI on an ARM host, and this build does not assume the host"
                .to_owned(),
        );
    }
    let value = match formula {
        Formula::Enable => known(&enable),
        Formula::DisableUnlessForced { .. } => match (&disable, &force) {
            (_, State::Present) | (State::Absent, _) => Ok(true),
            (State::Present, State::Absent) => Ok(false),
            (State::Uncertain(why), _) | (_, State::Uncertain(why)) => Err(why.clone()),
        },
    };
    match value {
        // Something was unseen, and the answer holds whatever it was.
        Ok(_) if applied.is_empty() => Resolution::Default,
        Ok(use_nvapi) => Resolution::Computed { use_nvapi, applied },
        Err(why) => Resolution::Undetermined(why),
    }
}

fn known(state: &State) -> Result<bool, String> {
    match state {
        State::Present => Ok(true),
        State::Absent => Ok(false),
        State::Uncertain(why) => Err(why.clone()),
    }
}

/// Whether the default lists set `flag` for `appid`.
fn default_state(reading: &Reading, flag: Flag, appid: &str) -> State {
    if find(reading, flag, appid, false).is_some() {
        return State::Present;
    }
    if let Some(site) = find(reading, flag, appid, true) {
        return State::Uncertain(format!(
            "the list at line {} sets {flag} only under a condition this reader does not evaluate",
            site.line
        ));
    }
    if reading.refusals.is_empty() {
        State::Absent
    } else {
        State::Uncertain("part of the NVAPI policy was not understood".to_owned())
    }
}

/// Proton's `nonzero`: any value but `""` and `"0"` sets the flag.
fn nonzero(value: &str) -> bool {
    !value.is_empty() && value != "0"
}

/// One element of a token pattern.
enum P {
    Name(&'static str),
    Text(&'static str),
    Punct(char),
    /// `<name>.compat_config`, as `g_session.` or `self.` spells it.
    Compat,
}

const ENABLE: [P; 3] = [P::Text("enablenvapi"), P::Name("in"), P::Compat];

const DISABLE_UNLESS_FORCED: [P; 8] = [
    P::Text("disablenvapi"),
    P::Name("not"),
    P::Name("in"),
    P::Compat,
    P::Name("or"),
    P::Text("forcenvapi"),
    P::Name("in"),
    P::Compat,
];

const ARM_CLAUSE: [P; 7] = [
    P::Name("and"),
    P::Name("g_proton"),
    P::Punct('.'),
    P::Name("host_pe_arch"),
    P::Punct('!'),
    P::Punct('='),
    P::Text("aarch64-windows"),
];

/// Whether `tokens` is exactly `pattern`.
fn shape(tokens: &[Token], pattern: &[P]) -> bool {
    let mut at = 0;
    for p in pattern {
        let rest = &tokens[at.min(tokens.len())..];
        let used = match (p, rest) {
            (P::Name(want), [Token::Name(got), ..]) if got == want => 1,
            (P::Text(want), [Token::Text(got), ..]) if got == want => 1,
            (P::Punct(want), [Token::Punct(got), ..]) if got == want => 1,
            (P::Compat, [Token::Name(_), Token::Punct('.'), Token::Name(got), ..])
                if got == "compat_config" =>
            {
                3
            }
            _ => return false,
        };
        at += used;
    }
    at == tokens.len()
}

/// The tokens inside a leading `(`, up to its matching `)`, when that `)`
/// closes the slice or is followed only by more closers of outer brackets.
fn unwrap(tokens: &[Token]) -> Option<&[Token]> {
    let [Token::Punct('('), rest @ ..] = tokens else {
        return None;
    };
    let mut depth = 0usize;
    for (at, token) in rest.iter().enumerate() {
        match token {
            Token::Punct('(' | '[' | '{') => depth += 1,
            Token::Punct(')') if depth == 0 => return Some(&rest[..at]),
            Token::Punct(')' | ']' | '}') => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests;
