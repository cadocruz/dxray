//! What Proton's launcher script says it will do to a game's NVAPI. Pure: no
//! paths, no filesystem.
//!
//! Proton withholds NVAPI from some games and hands it to others, and which is
//! which is not configuration and not a list anybody publishes — it is written
//! in the `proton` launcher script, a Python file inside every Proton install.
//! Reading that script says what Proton will do to a game before the game has
//! ever been run.
//!
//! # There is no Python parser here, and that changes the risk
//!
//! `ast.parse` fails loudly on a file it cannot handle. A hand-rolled scanner
//! mis-scans quietly, which is much the worse failure for a tool whose whole
//! claim is that it does not guess. So this reader is built to notice when it
//! is out of its depth:
//!
//! * The lexer tokenises the **whole file** — strings, comments, bracket
//!   continuations and all — and returns [`Error`] rather than a partial answer
//!   if it meets something it cannot resolve. A docstring holding the text this
//!   scanner looks for cannot fool it, because the docstring is one token.
//! * Only two statement shapes are understood: `if appid in [...]` (or a tuple
//!   or a set, or `appid == "..."`) and `ret.add("flag")`.
//! * Every mention of an NVAPI flag **not** explained by one of those two
//!   shapes becomes a [`Refusal`]. That is the net under the whole design: if
//!   Proton moves the flag into a loop, or sets it through `ret.update(...)`,
//!   or guards it with a compound test, the flag string is still in the file,
//!   this reader still sees it, cannot account for it, and says so.
//! * Every **block** between the function body and a policy site becomes a
//!   [`Guard`]. Not the ones this reader can read — it can read none of them —
//!   but every one of them, quoted back. See below.
//!
//! # Nothing between the list and the flag is ever waved through
//!
//! Proton 10.0 split the policy in two. One block sets the flag outright; the
//! other holds the same kind of appid list and then:
//!
//! ```text
//! try:
//!     with open('/proc/modules') as f:
//!         drivers = set([line.partition(' ')[0] for line in f.read().splitlines()])
//!         if not drivers.intersection({'nvidia', 'nouveau', 'nova'}):
//!             ret.add("disablenvapi")
//! except OSError:
//!     ret.add("disablenvapi")
//! ```
//!
//! Read the sense of it: for those games NVAPI is disabled only when **no**
//! NVIDIA driver is loaded. On the machines this tool is for, they keep it.
//!
//! An earlier version of this module treated that as *the* conditional shape
//! and everything else as no condition at all, which is backwards. A guard this
//! reader cannot state is a stronger reason to hedge than one it can quote,
//! because there is nothing to hand the reader instead. So the rule is
//! structural and has no list of shapes in it: **any block standing between the
//! function body and a `ret.add` makes that site conditional**, and every such
//! block is quoted verbatim with the literals it names pulled out. The
//! `/proc/modules` case is not special. It is simply the one that happens to
//! name four legible things.
//!
//! The single exception is the block that *binds* the application id —
//! `if "SteamAppId" in os.environ:` with `appid = os.environ[...]` inside it,
//! which wraps the whole policy in every release. It is not a condition on the
//! policy; it is the condition under which there is an appid to ask about at
//! all, so a caller asking about one has already assumed it. Nothing else is
//! exempt, and if that binding ever changes shape every site becomes
//! conditional and says so loudly — which is the safe direction to fail in.
//!
//! Conditions are never evaluated. Evaluating one would also make the answer
//! depend on where `dxray` happens to be running rather than on what Proton
//! will do to that game.
//!
//! # An empty answer is never a confident "no", and never a shrug either
//!
//! The reference implementation this replaces returns an empty set both when a
//! game is absent from the policy and when the script could not be read, then
//! reports both as "NVAPI is fine". That turns a release whose shape changed
//! into "NVAPI is fine for every game, forever".
//!
//! The opposite mistake is just as bad and is easier to make: reporting a
//! genuine true negative — Proton 7.0 has the function, has five real appid
//! lists, and has no NVAPI policy in it — with the same sentence as a script
//! whose policy was written in a shape this reader could not read. One is a
//! finding and the other is a failure. [`Unknown`] separates them, and so does
//! the exit code a caller draws from [`settled`].
//!
//! # The polarity flipped twice, so the flag name is not a constant
//!
//! Proton 5.0 and earlier have no such mechanism. Proton 6.3 through 8.0 spell
//! it `enablenvapi` with the **opposite** meaning — NVAPI is off and an appid
//! list turns it on, 136 games in 8.0. Proton 9.0 flips back to `disablenvapi`
//! as an opt-out. A reader that knows only `disablenvapi`, finds none in an 8.0
//! script and answers "not in the list, so NVAPI is on" gets every game on that
//! release exactly backwards.
//!
//! This module supports both directions rather than only noticing the other
//! one, because supporting it costs one enum and refusing it would throw away a
//! true answer for 136 games. The direction is read off the flag the lists are
//! written under — see [`Policy`] — and a script that lists appids under
//! neither of those two flags has a policy whose *direction* cannot be told,
//! which is not the same thing as having no policy. See
//! [`Unknown::PolarityUnknown`].
//!
//! # What is believed rather than proven
//!
//! The direction is **inferred**, on the reasoning that no release spends a
//! list enabling something already on. The line that actually decides it,
//! `use_nvapi = ...`, is a Python expression this reader does not parse. Two
//! things make the inference checkable rather than blind: every answer names
//! the flag it was drawn from, and [`Reading::elsewhere`] records which flags
//! the rest of the script names, so a build whose policy and whose environment
//! switches disagree about the direction is refused instead of answered.
//!
//! Version numbers are not used anywhere here and must not be: `GE-Proton10-1`
//! ships the old single-block shape under a version that suggests the new one.
//! Only shapes are recognised.
//!
//! # The lists are only the default
//!
//! [`decide`] describes the script's lists. A launch can override them:
//! `check_environment` calls run afterwards and add or remove each flag, so
//! `PROTON_FORCE_NVAPI=1` in a game's launch options changes the answer for
//! that game. [`resolve`] applies a launch's [`Environment`] the way the script
//! does.

mod environment;
#[cfg(test)]
mod tests;

pub use environment::{
    Applied, Environment, Formula, Resolution, SetBy, Switch, UserSettings, resolve, user_settings,
};

use std::fmt;
use std::fmt::Write as _;

/// The name of the function Proton keeps its per-game workarounds in.
///
/// Present from Proton 7.0 onwards. Absent from 6.3 and earlier, which is not a
/// failure and not a corrupt file: those releases have no such function.
const FUNCTION: &str = "default_compat_config";

/// The local name Proton binds the application id to inside that function.
const APPID: &str = "appid";

/// The set the function accumulates flags into.
const FLAGS: &str = "ret";

/// How wide a tab is taken to be when measuring indentation, matching `CPython`.
const TAB: usize = 8;

/// How deeply the walk will follow nested blocks before it stops.
///
/// The number is arbitrary; the bound is not. Proton nests four levels at the
/// most. Without a cap, a file of nothing but ever-deeper blocks would recurse
/// until the stack ran out, and a stack overflow aborts the process — it is not
/// an error a caller can catch, so one hostile file would kill a whole library
/// scan instead of producing one refusal.
const MAX_DEPTH: usize = 64;

/// An application id no Proton script lists, used to ask about a build rather
/// than about a game.
///
/// The empty string, which is not a decimal number and so cannot collide with a
/// real entry. If some script ever did list `""`, the only consequence is that
/// [`settled`] would report that build as settled, which it would be.
const NO_GAME: &str = "";

/// The NVAPI-related flags Proton's compat config knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flag {
    /// `disablenvapi`: the games listed under it do not get NVAPI. Proton 9.0
    /// onwards.
    Disable,
    /// `enablenvapi`: the games listed under it are the only ones that do.
    /// Proton 6.3 through 8.0.
    Enable,
    /// `forcenvapi`: overrides a `disablenvapi` for the games listed under it.
    Force,
}

/// Every flag this module knows, in the order answers prefer to mention them.
const FLAGS_KNOWN: [Flag; 3] = [Flag::Disable, Flag::Enable, Flag::Force];

impl Flag {
    /// The flag as the script spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disablenvapi",
            Self::Enable => "enablenvapi",
            Self::Force => "forcenvapi",
        }
    }

    /// The flag a string literal names, if it names one.
    fn lookup(text: &str) -> Option<Self> {
        FLAGS_KNOWN.into_iter().find(|flag| flag.as_str() == text)
    }
}

impl fmt::Display for Flag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which way round a script's NVAPI policy runs.
///
/// Inferred from the flag the lists are written under, not from the line that
/// computes the default. See the module docs for why that inference is drawn
/// and what it costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Every game gets NVAPI except the ones in the lists. Proton 9.0 onwards.
    DenyList,
    /// No game gets NVAPI except the ones in the lists. Proton 6.3 to 8.0.
    AllowList,
}

impl Policy {
    /// The flag whose lists express this policy.
    #[must_use]
    pub fn flag(self) -> Flag {
        match self {
            Self::DenyList => Flag::Disable,
            Self::AllowList => Flag::Enable,
        }
    }
}

/// One block standing between the function body and a flag being set.
///
/// Quoted back rather than summarised, and never evaluated. This reader
/// understands none of them — not the `/proc/modules` one either — so the only
/// honest thing it can do is print the text and name the literals in it, and
/// let whoever is reading apply it to their own machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guard {
    /// Whether it wraps the appid list itself, rather than sitting between that
    /// list and the flag.
    ///
    /// An enclosing guard is quoted as its header alone, because its body is
    /// the whole appid list and reprinting eight hundred application ids under
    /// a caveat helps nobody.
    pub enclosing: bool,
    /// 1-based first physical line of the quoted text.
    pub first: usize,
    /// 1-based last physical line of the quoted text.
    pub last: usize,
    /// The text, verbatim, with the indentation all of it shares removed.
    pub source: Vec<String>,
    /// The plain string literals it names, deduplicated and in order, with the
    /// NVAPI flag names themselves left out.
    ///
    /// The nearest thing to "what it depends on" that can be had without
    /// interpreting anything: the words come straight out of the file.
    pub mentions: Vec<String>,
}

impl Guard {
    /// One clause naming where it is and what it reads.
    #[must_use]
    pub fn summary(&self) -> String {
        let Self {
            first,
            last,
            mentions,
            enclosing,
            ..
        } = self;
        let where_ = if *enclosing {
            format!("a test at line {first}")
        } else if first == last {
            format!("a block at line {first}")
        } else {
            format!("a block at lines {first}-{last}")
        };
        if mentions.is_empty() {
            return format!("{where_} whose terms this reader cannot name");
        }
        let quoted: Vec<String> = mentions.iter().map(|m| format!("{m:?}")).collect();
        format!("{where_} that turns on {}", quoted.join(", "))
    }
}

/// Everything standing between an appid list and the flag it would set.
///
/// Outermost first. More than one is ordinary: a site can sit inside a guard
/// this reader did not recognise *and* reach its flag through a further block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub guards: Vec<Guard>,
}

impl Condition {
    /// One sentence naming every guard, outermost first.
    #[must_use]
    pub fn summary(&self) -> String {
        let n = self.guards.len();
        let parts: Vec<String> = self.guards.iter().map(Guard::summary).collect();
        format!(
            "{n} test{} this reader does not evaluate and does not claim to understand: {}",
            if n == 1 { "" } else { "s" },
            parts.join("; and inside it, "),
        )
    }

    /// Every guard's text, outermost first, for a report with room to print it.
    #[must_use]
    pub fn source(&self) -> Vec<String> {
        self.guards
            .iter()
            .flat_map(|guard| guard.source.iter().cloned())
            .collect()
    }
}

/// One `if appid in [...]` block whose body sets an NVAPI flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    /// The flag the block sets.
    pub flag: Flag,
    /// 1-based line of the `if` that opens the block.
    pub line: usize,
    /// The application ids the list holds, as the script spells them.
    pub appids: Vec<String>,
    /// What stands between the list and the flag, when anything does.
    ///
    /// `None` means nothing does: the block sets the flag outright for every id
    /// in the list, and no block this reader could not read wraps it.
    pub condition: Option<Condition>,
}

/// Something inside the policy that this reader met and would not guess at.
///
/// A refusal is not a parse failure: the rest of the scan still happened. What
/// it means is that an answer drawn from an *absence* in this reading is not
/// safe, because the appid being asked about could be in the part that was
/// refused. [`decide`] enforces exactly that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// 1-based line the refusal is about.
    pub line: usize,
    pub kind: RefusalKind,
}

/// The shapes a refusal comes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalKind {
    /// An NVAPI flag is named somewhere that is not a `ret.add("flag")` inside
    /// a recognised appid list.
    ///
    /// A flag set from a loop, set unconditionally for everything, or set
    /// through `ret.update({...})` lands here rather than being read as an
    /// empty policy.
    Unaccounted,
    /// A recognised appid list that sets an NVAPI flag holds an entry that is
    /// not a plain string literal, so the list read out of it is short.
    UnreadableEntries {
        /// How many entries could not be read.
        count: usize,
    },
    /// The policy's direction and the rest of the script disagree.
    ///
    /// The lists are written under one flag and the script's environment
    /// switches name only the other. Never seen on a real release, and exactly
    /// what a change of direction would look like on the day it happens, so it
    /// is refused instead of answered.
    PolarityDisagrees {
        /// The flag the lists use.
        listed: Flag,
        /// The flag the rest of the script uses.
        used: Flag,
    },
    /// Blocks nested past `MAX_DEPTH`, so the walk stopped before the bottom.
    TooDeep,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            RefusalKind::Unaccounted => write!(
                f,
                "line {}: an NVAPI flag is set by something this reader does not recognise \
                 as a list of application ids",
                self.line
            ),
            RefusalKind::UnreadableEntries { count } => write!(
                f,
                "line {}: {count} {} in this NVAPI list {} not a plain string",
                self.line,
                if count == 1 { "entry" } else { "entries" },
                if count == 1 { "is" } else { "are" },
            ),
            RefusalKind::PolarityDisagrees { listed, used } => write!(
                f,
                "line {}: the appid lists are written under {listed} while the rest of the \
                 script knows only {used}, so which way the policy runs is not settled",
                self.line
            ),
            RefusalKind::TooDeep => write!(
                f,
                "line {}: blocks nested past {MAX_DEPTH} levels, so this reader stopped \
                 before the bottom of them",
                self.line
            ),
        }
    }
}

/// Everything one `proton` script was found to say about NVAPI, and everything
/// about it that could not be determined.
///
/// The facts a caller needs in order to trust an answer are kept apart on
/// purpose: [`Reading::function_line`] says the function was found,
/// [`Reading::appid_lists`] says well-formed appid lists were read out of it,
/// [`Reading::sites`] says some of them touch NVAPI, [`Reading::refusals`] says
/// what was not understood, and [`Reading::elsewhere`] says what the rest of
/// the script knows about NVAPI. They are not the same fact, and a set of
/// appids alone reports none of them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reading {
    /// 1-based line of `default_compat_config`, when the script has one.
    pub function_line: Option<usize>,
    /// How many `if appid in [...]` blocks were read, whatever flag they set.
    ///
    /// The evidence that the scan understood the file's shape. A script with
    /// thirty of these and no NVAPI site is a build whose policy is absent or
    /// elsewhere; a script with none is a file this reader did not understand,
    /// and the two must not produce the same sentence.
    pub appid_lists: usize,
    /// The NVAPI sites, in the order the file spells them.
    pub sites: Vec<Site>,
    /// What was met inside the function and not understood.
    pub refusals: Vec<Refusal>,
    /// The NVAPI flags the script names outside the function, deduplicated and
    /// in file order.
    ///
    /// Two jobs. It corroborates the direction the lists imply, and it is what
    /// separates a build whose NVAPI is decided by an environment switch rather
    /// than by which game it is — Proton 6.3 and 7.0 — from one that says
    /// nothing about NVAPI anywhere.
    pub elsewhere: Vec<Flag>,
    /// How many string literals anywhere in the script hold the text `nvapi`,
    /// in any spelling.
    ///
    /// A far wider net than [`Reading::elsewhere`], and it is wide on purpose.
    /// It catches the DLL names and the environment variables as well as the
    /// flags, which makes it useless for deciding anything — and exactly right
    /// for one question: *is there any NVAPI machinery in this file at all?* A
    /// script with no NVAPI site, no refusal and no flag this reader knows is a
    /// true negative if this is zero and a build spelling its flags in some new
    /// way if it is not.
    pub nvapi_mentions: usize,
    /// The `check_environment` calls for NVAPI flags, in file order.
    pub switches: Vec<Switch>,
    /// NVAPI switches whose variable is not a plain string literal.
    pub switches_unread: usize,
    /// The `use_nvapi` expression, when it is one this reader recognizes.
    pub formula: Option<Formula>,
    /// 1-based line of `config_info` that records `use_nvapi`.
    pub recorded_line: Option<usize>,
}

impl Reading {
    /// Which way round the policy runs, if it can be told.
    ///
    /// `None` when no list is written under `disablenvapi` or `enablenvapi` —
    /// whether because the script has no NVAPI list at all or because its only
    /// lists are `forcenvapi` — and also when it has both kinds, a shape no
    /// release has had and one this reader will not pick a winner from.
    #[must_use]
    pub fn policy(&self) -> Option<Policy> {
        match (self.lists(Flag::Disable), self.lists(Flag::Enable)) {
            (true, false) => Some(Policy::DenyList),
            (false, true) => Some(Policy::AllowList),
            _ => None,
        }
    }

    /// Whether any site sets `flag`.
    #[must_use]
    pub fn lists(&self, flag: Flag) -> bool {
        self.sites.iter().any(|site| site.flag == flag)
    }

    /// How many application ids the sites for `flag` name in total.
    #[must_use]
    pub fn listed(&self, flag: Flag) -> usize {
        self.sites
            .iter()
            .filter(|site| site.flag == flag)
            .map(|site| site.appids.len())
            .sum()
    }

    /// One sentence about how much of the script was understood.
    ///
    /// Reported beside every answer, because "this game is not in the list" is
    /// worth one thing when thirty lists were read and nothing at all when the
    /// function was never found.
    #[must_use]
    pub fn evidence(&self) -> String {
        let Some(line) = self.function_line else {
            return match self.elsewhere.first() {
                Some(flag) => format!(
                    "no {FUNCTION} in this script, though it names {flag} elsewhere, \
                     so it has an NVAPI mechanism that is not an appid list"
                ),
                None => format!("no {FUNCTION} in this script, and no NVAPI flag anywhere in it"),
            };
        };
        let mut out = format!(
            "{FUNCTION} at line {line}; {} appid {} read",
            self.appid_lists,
            if self.appid_lists == 1 {
                "list"
            } else {
                "lists"
            },
        );
        if self.sites.is_empty() {
            out.push_str("; no NVAPI site among them");
        } else {
            let parts: Vec<String> = FLAGS_KNOWN
                .into_iter()
                .filter(|flag| self.lists(*flag))
                .map(|flag| {
                    let n = self.listed(flag);
                    let noun = if n == 1 { "appid" } else { "appids" };
                    format!("{n} {noun} under {flag}")
                })
                .collect();
            out.push_str("; ");
            out.push_str(&parts.join(", "));
        }
        let conditional = self
            .sites
            .iter()
            .filter(|site| site.condition.is_some())
            .count();
        if conditional > 0 {
            let _ = write!(
                out,
                "; {conditional} of the sites {} conditional",
                if conditional == 1 { "is" } else { "are" }
            );
        }
        if !self.refusals.is_empty() {
            let _ = write!(
                out,
                "; {} thing{} refused",
                self.refusals.len(),
                if self.refusals.len() == 1 { "" } else { "s" },
            );
        }
        out
    }
}

/// What the policy does to one application id.
///
/// More than the three states the reference implementation had, because the
/// real releases have more than three cases in them.
/// [`Decision::available`] collapses them back to the question most callers are
/// actually asking, and loses exactly the distinctions it says it loses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// In a `disablenvapi` list with nothing between the list and the flag:
    /// Proton withholds NVAPI from this game.
    ListedToDisable {
        /// 1-based line of the list.
        line: usize,
    },
    /// In a `forcenvapi` list, which beats a `disablenvapi` for the same id.
    ListedToForce {
        /// 1-based line of the list.
        line: usize,
    },
    /// A deny-list policy was read and this id is in none of its lists, so
    /// Proton leaves NVAPI alone for this game.
    NotListedToDisable,
    /// In an `enablenvapi` list: one of the games the release hands NVAPI to.
    ListedToEnable {
        /// 1-based line of the list.
        line: usize,
    },
    /// An allow-list policy was read and this id is not in it — so NVAPI is
    /// **withheld**. The same silence that means "fine" on Proton 9.0 means the
    /// opposite on Proton 8.0, which is why this is its own variant rather than
    /// being folded into [`Decision::NotListedToDisable`].
    NotListedToEnable,
    /// Listed, and the flag is set only when something holds that this reader
    /// will not evaluate.
    ///
    /// The condition is carried, not resolved. It is not a yes and not a no,
    /// and it is not the same thing as not knowing: the script says what it
    /// turns on, and that text is in here.
    Conditional {
        /// The flag the condition would set.
        flag: Flag,
        /// 1-based line of the list.
        line: usize,
        /// Everything standing between the list and the flag.
        condition: Condition,
    },
    /// The function was read in full, nothing in it was refused, and no appid
    /// list in it touches NVAPI.
    ///
    /// A finding, not a failure, and the difference matters: Proton 7.0 really
    /// is like this. Its NVAPI is decided by an environment switch rather than
    /// by which game is being launched, so nothing about *this* game changes
    /// what Proton does. [`Decision::available`] is still `None`, because what
    /// the switch defaults to is not something this reader parses.
    NotGameSpecific {
        /// The NVAPI flag the rest of the script names, when it names one.
        /// `None` means the script mentions no NVAPI flag at all.
        via: Option<Flag>,
    },
    /// Not determined, and why.
    Unknown(Unknown),
}

/// Why an answer could not be given.
///
/// Every variant is a different failure with a different fix, and the point of
/// having five of them is that none of them may be printed in place of
/// [`Decision::NotGameSpecific`], which is not a failure at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unknown {
    /// The script parsed and holds no `default_compat_config`.
    ///
    /// This is where a Proton 6.3 and a script truncated just above the
    /// function both land, and there is no way to tell them apart from one
    /// file. Neither is reported as a broken file and neither is reported as
    /// "no games are affected": the answer is that the function is not there.
    NoFunction {
        /// Whether the script names an NVAPI flag anywhere else.
        mechanism: bool,
    },
    /// NVAPI is set inside the function by shapes this reader does not model,
    /// and no site was read at all, so whether there is a per-game policy
    /// cannot be told.
    ///
    /// The variant that must never be confused with
    /// [`Decision::NotGameSpecific`]. One says the file was read and there is
    /// no policy; this one says the policy may be entirely inside the part that
    /// was not read.
    Unreadable {
        /// How many things were refused.
        refusals: usize,
    },
    /// Appid lists that touch NVAPI were read, and none of them is written
    /// under `disablenvapi` or `enablenvapi`.
    ///
    /// The direction of a policy is read off those two flags. A script with
    /// only `forcenvapi` lists has a policy whose direction cannot be told,
    /// which is emphatically not the same as having no policy — and saying "no
    /// list touches NVAPI" about it would be false against the lists printed
    /// beside it.
    PolarityUnknown {
        /// The flag the lists that were found use.
        listed: Flag,
        /// How many games those lists name.
        appids: usize,
    },
    /// The script lists appids under both `disablenvapi` and `enablenvapi`.
    Contradictory,
    /// A policy was read, this id is in none of it, and something was refused,
    /// so "not in the list" is not a safe reading.
    Refused {
        /// How many things were refused.
        refusals: usize,
    },
    /// No NVAPI site and nothing refused, but the script is full of `nvapi`
    /// strings and names no flag this reader knows.
    ///
    /// What a release that renamed its flags would look like. Reported rather
    /// than read as a true negative, because the wide net catching something is
    /// the only evidence that the narrow one missed anything.
    UnknownSpelling {
        /// How many `nvapi` strings are in the script.
        mentions: usize,
    },
}

impl Decision {
    /// Whether Proton will offer this game NVAPI: `Some(true)`, `Some(false)`,
    /// or `None` when it is not settled.
    ///
    /// A convenience, and a lossy one. [`Decision::Conditional`],
    /// [`Decision::NotGameSpecific`] and [`Decision::Unknown`] all answer
    /// `None` while meaning three very different things: a condition this crate
    /// declines to evaluate, a build with no per-game policy, and an absence of
    /// knowledge. A caller showing a person the result should render the
    /// [`Decision`] itself, which says which.
    #[must_use]
    pub fn available(&self) -> Option<bool> {
        match self {
            Self::ListedToDisable { .. } | Self::NotListedToEnable => Some(false),
            Self::ListedToForce { .. } | Self::NotListedToDisable | Self::ListedToEnable { .. } => {
                Some(true)
            }
            Self::Conditional { .. } | Self::NotGameSpecific { .. } | Self::Unknown(_) => None,
        }
    }

    /// Whether the answer is that there is no answer.
    ///
    /// False for [`Decision::Conditional`], which *is* an answer — the script
    /// says what it depends on and this carries that text. False too for
    /// [`Decision::NotGameSpecific`], which is a finding about the build.
    #[must_use]
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    /// What this game is under, when it is under anything.
    #[must_use]
    pub fn condition(&self) -> Option<&Condition> {
        match self {
            Self::Conditional { condition, .. } => Some(condition),
            _ => None,
        }
    }
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ListedToDisable { line } => write!(
                f,
                "NVAPI is withheld: this game is in the {} list at line {line}",
                Flag::Disable
            ),
            Self::ListedToForce { line } => write!(
                f,
                "NVAPI is forced on: this game is in the {} list at line {line}",
                Flag::Force
            ),
            Self::NotListedToDisable => write!(
                f,
                "NVAPI is left alone: this build withholds it only from the games in its \
                 {} lists, and this game is not in them",
                Flag::Disable
            ),
            Self::ListedToEnable { line } => write!(
                f,
                "NVAPI is offered: this game is in the {} list at line {line}",
                Flag::Enable
            ),
            Self::NotListedToEnable => write!(
                f,
                "NVAPI is withheld: this build offers it only to the games in its {} lists, \
                 and this game is not in them",
                Flag::Enable
            ),
            Self::Conditional {
                flag,
                line,
                condition,
            } => write!(
                f,
                "conditional: this game is in the list at line {line}, but {flag} is set \
                 there only under {} — the condition is reported, not resolved",
                condition.summary()
            ),
            Self::NotGameSpecific { via: Some(flag) } => write!(
                f,
                "not game-specific: {FUNCTION} was read in full and no appid list in it \
                 touches NVAPI. This build decides NVAPI by its {flag} switch rather than \
                 by which game is running, so being this game changes nothing"
            ),
            Self::NotGameSpecific { via: None } => write!(
                f,
                "not game-specific: {FUNCTION} was read in full, no appid list in it \
                 touches NVAPI, and no NVAPI flag appears anywhere in the script"
            ),
            Self::Unknown(why) => write!(f, "{why}"),
        }
    }
}

impl fmt::Display for Unknown {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFunction { mechanism: false } => write!(
                f,
                "not determined: this script contains no {FUNCTION}, and no NVAPI flag \
                 anywhere else either"
            ),
            Self::NoFunction { mechanism: true } => write!(
                f,
                "not determined: this script contains no {FUNCTION}, but it does name an \
                 NVAPI flag elsewhere, so it has a mechanism that is not a list of games"
            ),
            Self::Unreadable { refusals } => write!(
                f,
                "not determined: NVAPI is set inside {FUNCTION} by {refusals} thing{} this \
                 reader does not understand and no list it does, so whether this build has \
                 a per-game policy cannot be told from it",
                if *refusals == 1 { "" } else { "s" },
            ),
            Self::PolarityUnknown { listed, appids } => write!(
                f,
                "not determined: this build lists {appids} {} under {listed} and has no {} \
                 or {} list, so which way round its policy runs cannot be told",
                if *appids == 1 { "game" } else { "games" },
                Flag::Disable,
                Flag::Enable,
            ),
            Self::Contradictory => write!(
                f,
                "not determined: this script lists application ids under both {} and {}, \
                 which no release has done and this reader will not pick a winner from",
                Flag::Disable,
                Flag::Enable
            ),
            Self::Refused { refusals } => write!(
                f,
                "not determined: this game is in none of the lists that were read, but \
                 {refusals} thing{} in the NVAPI policy {} not understood, so \"not in the \
                 list\" is not a safe reading",
                if *refusals == 1 { "" } else { "s" },
                if *refusals == 1 { "was" } else { "were" },
            ),
            Self::UnknownSpelling { mentions } => write!(
                f,
                "not determined: no appid list touches an NVAPI flag this reader knows, yet \
                 {mentions} strings in this script mention nvapi — so it has machinery that \
                 is spelled in some way this reader does not recognise"
            ),
        }
    }
}

/// What the policy in `reading` does to `appid`.
///
/// `appid` is compared as the script spells it, which is a decimal string.
#[must_use]
pub fn decide(reading: &Reading, appid: &str) -> Decision {
    if reading.function_line.is_none() {
        return Decision::Unknown(Unknown::NoFunction {
            mechanism: !reading.elsewhere.is_empty(),
        });
    }
    // Nothing was found *and* something was not understood. The policy may be
    // entirely inside the part that was refused, so this is the one case that
    // must never be reported as a build with no per-game policy.
    if reading.sites.is_empty() && !reading.refusals.is_empty() {
        return Decision::Unknown(Unknown::Unreadable {
            refusals: reading.refusals.len(),
        });
    }

    let flat = |flag: Flag| find(reading, flag, appid, false).map(|site| site.line);
    let guarded = |flag: Flag| {
        find(reading, flag, appid, true).map(|site| Decision::Conditional {
            flag,
            line: site.line,
            condition: site
                .condition
                .clone()
                .unwrap_or(Condition { guards: vec![] }),
        })
    };
    let refused = || {
        (!reading.refusals.is_empty()).then_some(Decision::Unknown(Unknown::Refused {
            refusals: reading.refusals.len(),
        }))
    };

    match reading.policy() {
        // A `forcenvapi` beats a `disablenvapi` for the same id, because the
        // script's own test is `"disablenvapi" not in config or "forcenvapi" in
        // config`. Checked first for that reason and no other.
        Some(Policy::DenyList) => flat(Flag::Force)
            .map(|line| Decision::ListedToForce { line })
            .or_else(|| flat(Flag::Disable).map(|line| Decision::ListedToDisable { line }))
            .or_else(|| guarded(Flag::Force))
            .or_else(|| guarded(Flag::Disable))
            .or_else(refused)
            .unwrap_or(Decision::NotListedToDisable),
        Some(Policy::AllowList) => flat(Flag::Enable)
            .map(|line| Decision::ListedToEnable { line })
            .or_else(|| guarded(Flag::Enable))
            .or_else(refused)
            .unwrap_or(Decision::NotListedToEnable),
        None if reading.lists(Flag::Disable) && reading.lists(Flag::Enable) => {
            Decision::Unknown(Unknown::Contradictory)
        }
        // NVAPI lists were read, and none of them says which way round the
        // policy runs. A game named in one of them still has its answer — the
        // script's own expression makes `forcenvapi` sufficient on its own —
        // but a game outside them does not, because the direction is exactly
        // what is missing. Saying "no list touches NVAPI" here would be false
        // against the lists this same reading is about to print.
        None if !reading.sites.is_empty() => {
            let listed = reading.sites[0].flag;
            flat(listed)
                .map(|line| Decision::ListedToForce { line })
                .or_else(|| guarded(listed))
                .unwrap_or(Decision::Unknown(Unknown::PolarityUnknown {
                    listed,
                    appids: reading.listed(listed),
                }))
        }
        // No NVAPI site, and nothing refused. Either a genuine true negative or
        // a build whose flags are spelled in some way this reader has never
        // seen, and the wide `nvapi` net is what tells those apart.
        None if reading.elsewhere.is_empty() && reading.nvapi_mentions > 0 => {
            Decision::Unknown(Unknown::UnknownSpelling {
                mentions: reading.nvapi_mentions,
            })
        }
        None => Decision::NotGameSpecific {
            via: reading.elsewhere.first().copied(),
        },
    }
}

/// What this build does to a game it says nothing about in particular.
///
/// Answering the question "was this script understood well enough to answer
/// with" by asking [`decide`] about a game no script lists, so that a run which
/// names no application id and a run which names one cannot disagree about
/// whether the build was read.
#[must_use]
pub fn overall(reading: &Reading) -> Decision {
    decide(reading, NO_GAME)
}

/// Whether the build was read well enough for its answers to be worth having.
///
/// False when the function was not found, when something was refused, or when
/// the direction of the policy could not be told. **True** for a build with no
/// per-game NVAPI policy at all, which is a finding rather than a failure — see
/// [`Decision::NotGameSpecific`].
#[must_use]
pub fn settled(reading: &Reading) -> bool {
    !overall(reading).is_unknown()
}

/// The first site for `flag` that names `appid`, conditional or not as asked.
fn find<'a>(reading: &'a Reading, flag: Flag, appid: &str, conditional: bool) -> Option<&'a Site> {
    reading.sites.iter().find(|site| {
        site.flag == flag
            && site.condition.is_some() == conditional
            && site.appids.iter().any(|listed| listed == appid)
    })
}

/// What went wrong while reading the script, and where.
///
/// Every variant means the same thing to a caller — *this file was not
/// understood, so believe nothing about it* — and they are kept apart so the
/// message can say which shape defeated the reader rather than shrugging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    /// 1-based line the reader gave up on.
    pub line: usize,
}

/// The shapes a script this reader cannot handle comes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// A string literal ran to the end of the file, or a one-line string ran to
    /// the end of its line.
    UnterminatedString,
    /// A `)`, `]` or `}` with nothing open.
    UnmatchedBracket,
    /// The file ended inside a bracket.
    UnclosedBracket,
    /// A line inside the function is indented with a tab.
    ///
    /// Refused rather than guessed at. `CPython` rejects tabs mixed with spaces
    /// and Proton has only ever used spaces, so meeting one means a file that
    /// is not a Proton script or one that has been through something that
    /// rewrote it — and the block structure this reader depends on is exactly
    /// what a wrong tab width changes silently.
    TabIndent,
    /// The script defines the function more than once.
    ///
    /// Python would keep the last one. This reader will not assume which one
    /// Proton means, because the answer would be a policy read out of a
    /// function that never runs.
    DuplicateFunction {
        /// 1-based line of the first definition.
        first: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let line = self.line;
        match self.kind {
            ErrorKind::UnterminatedString => {
                write!(f, "unterminated string literal at line {line}")
            }
            ErrorKind::UnmatchedBracket => write!(f, "unmatched bracket at line {line}"),
            ErrorKind::UnclosedBracket => {
                write!(f, "the script ends inside a bracket opened at line {line}")
            }
            ErrorKind::TabIndent => write!(
                f,
                "line {line} is indented with a tab, and the block structure this reader \
                 depends on cannot be recovered from mixed indentation"
            ),
            ErrorKind::DuplicateFunction { first } => write!(
                f,
                "{FUNCTION} is defined twice, at lines {first} and {line}, and reading the \
                 wrong one would report a policy that never runs"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Reads one `proton` script's NVAPI policy out of its text.
///
/// A script with no `default_compat_config` is **not** an error: it comes back
/// as a [`Reading`] with no function line, which [`decide`] turns into
/// [`Unknown::NoFunction`]. Proton 6.3 and earlier genuinely have no such
/// function, and a script truncated just above it is indistinguishable from
/// one. Reporting either as a broken file would be as wrong as reporting it as
/// an empty policy, so neither happens.
///
/// # Errors
///
/// Returns [`Error`] when the file cannot be tokenised, or when it holds two
/// definitions of the function. Nothing partial comes back: if this returns an
/// error, nothing at all about the file has been established.
pub fn scan(script: &str) -> Result<Reading, Error> {
    let lines = lex(script)?;
    let raw: Vec<&str> = script.lines().collect();
    let (switches, switches_unread) = environment::switches(&lines);
    let mut reading = Reading {
        switches,
        switches_unread,
        formula: environment::formula(&lines),
        recorded_line: environment::recorded_line(&lines),
        ..Reading::default()
    };

    let header = function_line(&lines)?;
    let body = match header {
        Some(at) => {
            reading.function_line = Some(lines[at].line);
            let indent = lines[at].indent;
            let mut end = at + 1;
            while end < lines.len() && lines[end].indent > indent {
                end += 1;
            }
            at + 1..end
        }
        None => 0..0,
    };

    // Everything the script says about NVAPI that is not inside the function.
    // Gathered even when there is no function, because that is the only thing
    // separating a build with no mechanism from one whose mechanism is real and
    // is not a list of games.
    for (i, line) in lines.iter().enumerate() {
        reading.nvapi_mentions += nvapi_strings(&line.tokens);
        if body.contains(&i) || Some(i) == header {
            continue;
        }
        for flag in flags_named(&line.tokens) {
            if !reading.elsewhere.contains(&flag) {
                reading.elsewhere.push(flag);
            }
        }
    }

    let body = &lines[body];
    // Checked over the whole body before anything is read out of it, because a
    // block boundary computed from a tab of the wrong width is wrong silently.
    if let Some(tabbed) = body.iter().find(|line| line.tabbed) {
        return Err(Error {
            kind: ErrorKind::TabIndent,
            line: tabbed.line,
        });
    }

    walk(body, &raw, &mut Vec::new(), 0, &mut reading);
    check_polarity(&mut reading);
    Ok(reading)
}

/// Records a refusal when the lists and the rest of the script disagree about
/// which way round the policy runs.
///
/// Only when the rest of the script does not mention the lists' flag *at all*
/// and does mention the other one. A build that names both — every release
/// since 9.0 names `disablenvapi` and `forcenvapi` together — says nothing
/// about direction and is left alone.
fn check_polarity(reading: &mut Reading) {
    let Some(policy) = reading.policy() else {
        return;
    };
    let listed = policy.flag();
    if reading.elsewhere.contains(&listed) {
        return;
    }
    let opposite = match policy {
        Policy::DenyList => Flag::Enable,
        Policy::AllowList => Flag::Disable,
    };
    if !reading.elsewhere.contains(&opposite) {
        return;
    }
    reading.refusals.push(Refusal {
        line: reading.function_line.unwrap_or(1),
        kind: RefusalKind::PolarityDisagrees {
            listed,
            used: opposite,
        },
    });
}

/// The index of the function's `def`, or `None` if the script has no such
/// function.
fn function_line(lines: &[Logical]) -> Result<Option<usize>, Error> {
    let mut found: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let is_def = matches!(
            line.tokens.as_slice(),
            [Token::Name(keyword), Token::Name(name), Token::Punct('('), ..]
                if keyword == "def" && name == FUNCTION
        );
        if !is_def {
            continue;
        }
        if let Some(first) = found {
            return Err(Error {
                kind: ErrorKind::DuplicateFunction {
                    first: lines[first].line,
                },
                line: line.line,
            });
        }
        found = Some(i);
    }
    Ok(found)
}

/// Walks a block, consuming each recognised appid list whole and descending
/// through everything else with the tests it passed under recorded.
///
/// `guards` is the stack of blocks between the function body and here that this
/// reader could not state. It is what makes a wrapped policy site conditional
/// instead of invisible: an `if appid in [...]` block that sets a flag outright
/// is only *unconditional* if nothing on that stack stands over it.
///
/// Every line that is not part of a recognised appid list is still inspected
/// for NVAPI flag names, and each one found there becomes a refusal.
fn walk(body: &[Logical], raw: &[&str], guards: &mut Vec<Guard>, depth: usize, out: &mut Reading) {
    let mut i = 0;
    while i < body.len() {
        let line = &body[i];
        let mut end = i + 1;
        while end < body.len() && body[end].indent > line.indent {
            end += 1;
        }
        let inner = &body[i + 1..end];

        if let Some(test) = match_appid_test(&line.tokens) {
            out.appid_lists += 1;
            block(line, &test, inner, raw, guards, out);
            i = end;
            continue;
        }

        // Any flag named by a statement that is not a recognised list — whether
        // that statement opens a block or not — is a flag this reader cannot
        // account for.
        if !flags_named(&line.tokens).is_empty() {
            out.refusals.push(Refusal {
                line: line.line,
                kind: RefusalKind::Unaccounted,
            });
        }

        if !inner.is_empty() {
            if depth + 1 >= MAX_DEPTH {
                out.refusals.push(Refusal {
                    line: line.line,
                    kind: RefusalKind::TooDeep,
                });
                i = end;
                continue;
            }
            // The block that binds the application id is not a condition on the
            // policy — it is the condition under which there is an application
            // id at all, which a caller asking about one has already assumed.
            // Nothing else is exempt. See the module docs.
            let transparent = binds_appid(line, inner);
            if !transparent {
                guards.push(enclosing_guard(line, raw));
            }
            walk(inner, raw, guards, depth + 1, out);
            if !transparent {
                guards.pop();
            }
        }
        i = end;
    }
}

/// Reads one recognised `if appid in [...]` block.
fn block(
    header: &Logical,
    test: &Test,
    body: &[Logical],
    raw: &[&str],
    guards: &[Guard],
    out: &mut Reading,
) {
    let inline = &header.tokens[test.colon + 1..];
    // A statement on the same line as the `if` is the block's whole body, and
    // Python forbids an indented block after one. If both somehow appear, the
    // indented part is treated as deeper, which is the cautious reading.
    let top = if inline.is_empty() {
        body.first().map(|line| line.indent)
    } else {
        None
    };

    let mut outright: Vec<Flag> = Vec::new();
    let mut nested: Vec<Flag> = Vec::new();
    let mut unaccounted = false;

    if let Some(flag) = match_ret_add(inline).and_then(Flag::lookup) {
        outright.push(flag);
    } else if !flags_named(inline).is_empty() {
        unaccounted = true;
    }

    for line in body {
        match match_ret_add(&line.tokens).and_then(Flag::lookup) {
            Some(flag) if Some(line.indent) == top => outright.push(flag),
            Some(flag) => nested.push(flag),
            None if flags_named(&line.tokens).is_empty() => {}
            None => unaccounted = true,
        }
    }

    let mut added = false;
    for flag in FLAGS_KNOWN {
        // A flag set outright is still conditional when a block this reader
        // could not state wraps the list. That wrapper is invisible in the
        // block's own body, which is exactly why it is carried down here rather
        // than looked for from here.
        let condition = if outright.contains(&flag) {
            (!guards.is_empty()).then(|| Condition {
                guards: guards.to_vec(),
            })
        } else if nested.contains(&flag) {
            let mut all = guards.to_vec();
            all.push(inner_guard(body, raw));
            Some(Condition { guards: all })
        } else {
            continue;
        };
        added = true;
        out.sites.push(Site {
            flag,
            line: header.line,
            appids: test.appids.clone(),
            condition,
        });
    }

    if unaccounted {
        out.refusals.push(Refusal {
            line: header.line,
            kind: RefusalKind::Unaccounted,
        });
    }
    // Only worth saying when this block is one of the NVAPI ones. Half the
    // blocks in a Proton script set flags this module has no opinion about, and
    // a short list in one of those hides nothing.
    if added && test.opaque > 0 {
        out.refusals.push(Refusal {
            line: header.line,
            kind: RefusalKind::UnreadableEntries { count: test.opaque },
        });
    }
}

/// A block that wraps an appid list, quoted as its header alone.
///
/// The header and not the body, because the body is the list — quoting eight
/// hundred application ids under a caveat helps nobody, and the test is the
/// part a reader has to weigh.
fn enclosing_guard(header: &Logical, raw: &[&str]) -> Guard {
    Guard {
        enclosing: true,
        first: header.line,
        last: header.end,
        source: quote(raw, header.line, header.end),
        mentions: literals(std::slice::from_ref(header)),
    }
}

/// The body of an appid block that reaches its flag through something else.
fn inner_guard(body: &[Logical], raw: &[&str]) -> Guard {
    let first = body.first().map_or(1, |line| line.line);
    let last = body.last().map_or(first, |line| line.end);
    Guard {
        enclosing: false,
        first,
        last,
        source: quote(raw, first, last),
        mentions: literals(body),
    }
}

/// Physical lines `first` to `last`, with the indentation they share removed.
///
/// Stripped because the margin is an artefact of where the block sits in the
/// file, and keeping it pushes every quoted line off the right of a report.
fn quote(raw: &[&str], first: usize, last: usize) -> Vec<String> {
    let source: Vec<&str> = raw
        .iter()
        .skip(first.saturating_sub(1))
        .take(last.saturating_sub(first) + 1)
        .copied()
        .collect();
    let margin = source
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    source
        .iter()
        .map(|line| line.get(margin..).unwrap_or("").trim_end().to_owned())
        .collect()
}

/// The plain string literals some lines name, flags and blanks left out.
fn literals(lines: &[Logical]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        for token in &line.tokens {
            let Token::Text(text) = token else { continue };
            if text.trim().is_empty()
                || Flag::lookup(text).is_some()
                || out.iter().any(|seen| seen == text)
            {
                continue;
            }
            out.push(text.clone());
        }
    }
    out
}

/// Whether a block exists only to bind the application id.
///
/// `if "SteamAppId" in os.environ:` with an `appid = ...` at the top of its
/// body, which is how every release from 7.0 onwards wraps its whole policy —
/// twice, once for `SteamAppId` and once for `STEAM_COMPAT_APP_ID`. Recognised
/// so that it does not make every site in every script conditional; nothing
/// else is, so if this shape ever changes the answer degrades loudly to "all of
/// it is conditional, here is the test" rather than quietly to a wrong yes.
fn binds_appid(header: &Logical, body: &[Logical]) -> bool {
    let is_environ_test = matches!(
        header.tokens.as_slice(),
        [
            Token::Name(keyword),
            Token::Text(_),
            Token::Name(within),
            Token::Name(module),
            Token::Punct('.'),
            Token::Name(table),
            Token::Punct(':'),
        ] if keyword == "if" && within == "in" && module == "os" && table == "environ"
    );
    if !is_environ_test {
        return false;
    }
    let top = body.first().map(|line| line.indent);
    body.iter()
        .any(|line| Some(line.indent) == top && assigns_appid(&line.tokens))
}

/// Whether a statement binds `appid` to something.
///
/// Any right-hand side. What matters is that the name the lists compare against
/// is set here, not how — a release that switched to `os.environ.get(...)` is
/// still binding it, and refusing over that would be pedantry with a cost.
fn assigns_appid(tokens: &[Token]) -> bool {
    matches!(
        tokens,
        [Token::Name(name), Token::Punct('='), rest @ ..]
            if name == APPID && !matches!(rest.first(), Some(Token::Punct('=')))
    )
}

/// The NVAPI flags a line names in a string literal.
///
/// Matching the known names exactly — rather than anything containing `nvapi` —
/// keeps `"wine/nvapi/nvapi64.dll"` and the `dlloverrides` keys out of the
/// count, which would otherwise make every release look like it had a dozen
/// unmodelled mechanisms. The wide net lives in [`nvapi_strings`] and is used
/// for a different question.
fn flags_named(tokens: &[Token]) -> Vec<Flag> {
    let mut out = Vec::new();
    for token in tokens {
        if let Token::Text(text) = token
            && let Some(flag) = Flag::lookup(text)
            && !out.contains(&flag)
        {
            out.push(flag);
        }
    }
    out
}

/// How many string literals on a line mention `nvapi` in any spelling.
///
/// The wide net. Deliberately catches DLL paths and environment variable names
/// as well as flags, because the only question it answers is whether the script
/// has any NVAPI machinery in it at all.
fn nvapi_strings(tokens: &[Token]) -> usize {
    tokens
        .iter()
        .filter(|token| match token {
            Token::Text(text) => text.to_ascii_lowercase().contains("nvapi"),
            _ => false,
        })
        .count()
}

/// A recognised `if appid in [...]` test.
struct Test {
    /// The plain string entries, in file order.
    appids: Vec<String>,
    /// How many entries were something else.
    opaque: usize,
    /// Index of the `:` in the header's tokens.
    colon: usize,
}

/// Matches `if appid in [...]`, `if appid in (...)`, `if appid in {...}` and
/// `if appid == "..."`, and nothing else.
///
/// `elif` is deliberately not matched: its meaning depends on every test above
/// it in the chain, and a reader that treated it as an `if` would report a list
/// as unconditional when it is not. No release has set an NVAPI flag from one,
/// and if one starts, the flag name still lands in [`walk`] as a refusal.
fn match_appid_test(tokens: &[Token]) -> Option<Test> {
    match tokens.first()? {
        Token::Name(keyword) if keyword == "if" => {}
        _ => return None,
    }
    let colon = colon_at(tokens)?;
    let test = &tokens[1..colon];

    if let [
        Token::Name(name),
        Token::Punct('='),
        Token::Punct('='),
        Token::Text(value),
    ] = test
        && name == APPID
    {
        return Some(Test {
            appids: vec![value.clone()],
            opaque: 0,
            colon,
        });
    }

    let [
        Token::Name(name),
        Token::Name(keyword),
        Token::Punct(open),
        inner @ ..,
        Token::Punct(close),
    ] = test
    else {
        return None;
    };
    if name != APPID || keyword != "in" || !brackets_match(*open, *close) {
        return None;
    }

    let mut appids = Vec::new();
    let mut opaque = 0;
    for item in items(inner) {
        match item {
            [] => {}
            [Token::Text(value)] => appids.push(value.clone()),
            _ => opaque += 1,
        }
    }
    Some(Test {
        appids,
        opaque,
        colon,
    })
}

fn brackets_match(open: char, close: char) -> bool {
    matches!((open, close), ('[', ']') | ('(', ')') | ('{', '}'))
}

/// Splits a bracket's contents on the commas that belong to it.
fn items(tokens: &[Token]) -> Vec<&[Token]> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, token) in tokens.iter().enumerate() {
        if let Token::Punct(c) = token {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    out.push(&tokens[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
    }
    out.push(&tokens[start..]);
    out
}

/// The index of the `:` that ends a statement's header.
fn colon_at(tokens: &[Token]) -> Option<usize> {
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate() {
        if let Token::Punct(c) = token {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                ':' if depth == 0 => return Some(i),
                _ => {}
            }
        }
    }
    None
}

/// Matches exactly `ret.add("flag")` and returns the flag.
fn match_ret_add(tokens: &[Token]) -> Option<&str> {
    match tokens {
        [
            Token::Name(set),
            Token::Punct('.'),
            Token::Name(method),
            Token::Punct('('),
            Token::Text(flag),
            Token::Punct(')'),
        ] if set == FLAGS && method == "add" => Some(flag),
        _ => None,
    }
}

/// One logical line: everything Python joins into a single statement.
#[derive(Debug)]
struct Logical {
    /// Indentation of the first physical line, tabs counted to [`TAB`].
    indent: usize,
    /// Whether a tab was used to indent it.
    tabbed: bool,
    /// 1-based physical line the statement starts on.
    line: usize,
    /// 1-based physical line the statement ends on.
    end: usize,
    tokens: Vec<Token>,
}

/// As much of a Python token as this reader has any use for.
#[derive(Debug, PartialEq, Eq)]
enum Token {
    /// An identifier or keyword.
    Name(String),
    /// An unprefixed string literal with no escape in it, and its value.
    Text(String),
    /// A string literal whose value this reader will not claim to know: it
    /// carries a prefix (`f`, `r`, `b`) or an escape. Kept as a token rather
    /// than dropped, so an entry made of one still counts as an entry that
    /// could not be read.
    Opaque,
    /// A number. The value is never needed: Proton spells every application id
    /// as a string, and a bare number where one is expected is an entry this
    /// reader refuses rather than reads.
    Number,
    /// One punctuation character.
    Punct(char),
}

fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

fn is_name_part(byte: u8) -> bool {
    is_name_start(byte) || byte.is_ascii_digit()
}

/// Whether an identifier immediately before a quote is a string prefix.
fn is_string_prefix(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 2
        && name
            .bytes()
            .all(|b| matches!(b.to_ascii_lowercase(), b'r' | b'b' | b'u' | b'f'))
}

/// Turns the whole script into logical lines.
///
/// The whole script, not just the interesting part: a triple-quoted string
/// anywhere above the function can hold any text at all, and a scanner that
/// started reading in the middle would take that text for code.
fn lex(script: &str) -> Result<Vec<Logical>, Error> {
    Lexer::new(script).run()
}

/// Byte-oriented cursor over the script.
///
/// Scanning bytes is safe because every delimiter Python's grammar has is
/// ASCII, so a multi-byte character can only appear whole inside a token and is
/// copied through without being inspected.
struct Lexer<'a> {
    script: &'a str,
    bytes: &'a [u8],
    at: usize,
    line: usize,
    /// How many brackets are open. Non-zero means a newline joins lines rather
    /// than ending a statement, which is how every appid list in a Proton
    /// script is written.
    depth: usize,
    /// Where the outermost open bracket was, so an unclosed one is reported
    /// against the bracket rather than against the end of the file.
    open_line: usize,
    /// Whether the cursor is still in the indentation of a line.
    measuring: bool,
    indent: usize,
    tabbed: bool,
    /// First physical line of the statement being built.
    start: usize,
    tokens: Vec<Token>,
    out: Vec<Logical>,
}

impl<'a> Lexer<'a> {
    fn new(script: &'a str) -> Self {
        Self {
            script,
            bytes: script.as_bytes(),
            at: 0,
            line: 1,
            depth: 0,
            open_line: 1,
            measuring: true,
            indent: 0,
            tabbed: false,
            start: 1,
            tokens: Vec::new(),
            out: Vec::new(),
        }
    }

    fn run(mut self) -> Result<Vec<Logical>, Error> {
        while self.at < self.bytes.len() {
            if self.measuring {
                self.measure();
            } else {
                self.step()?;
            }
        }
        if self.depth != 0 {
            return Err(Error {
                kind: ErrorKind::UnclosedBracket,
                line: self.open_line,
            });
        }
        // A last statement with no newline after it.
        if !self.tokens.is_empty() {
            let end = self.line;
            self.finish_line(end);
        }
        Ok(self.out)
    }

    /// Consumes one byte of a line's indentation.
    fn measure(&mut self) {
        match self.bytes[self.at] {
            b' ' => self.indent += 1,
            b'\t' => {
                self.tabbed = true;
                self.indent = self.indent - self.indent % TAB + TAB;
            }
            b'\r' | 0x0c => {}
            b'\n' => {
                self.line += 1;
                self.indent = 0;
                self.tabbed = false;
            }
            // A comment-only line. Skipped whole, and its indentation is not a
            // block boundary — Python ignores it too.
            b'#' => {
                self.at = to_end_of_line(self.bytes, self.at);
                return;
            }
            _ => {
                self.measuring = false;
                self.start = self.line;
                return;
            }
        }
        self.at += 1;
    }

    /// Consumes one token, or one byte of whitespace, inside a statement.
    fn step(&mut self) -> Result<(), Error> {
        match self.bytes[self.at] {
            b'#' => self.at = to_end_of_line(self.bytes, self.at),
            b'\n' => {
                let ended = self.line;
                self.line += 1;
                self.at += 1;
                if self.depth == 0 {
                    self.finish_line(ended);
                }
            }
            b'\\' => self.continuation(),
            b' ' | b'\t' | b'\r' | 0x0c => self.at += 1,
            b'"' | b'\'' => {
                let (token, next, at) = read_string(self.script, self.at, self.line, false)?;
                self.tokens.push(token);
                self.at = next;
                self.line = at;
            }
            byte @ (b'(' | b'[' | b'{') => {
                if self.depth == 0 {
                    self.open_line = self.line;
                }
                self.depth += 1;
                self.tokens.push(Token::Punct(char::from(byte)));
                self.at += 1;
            }
            byte @ (b')' | b']' | b'}') => {
                if self.depth == 0 {
                    return Err(Error {
                        kind: ErrorKind::UnmatchedBracket,
                        line: self.line,
                    });
                }
                self.depth -= 1;
                self.tokens.push(Token::Punct(char::from(byte)));
                self.at += 1;
            }
            b'0'..=b'9' => self.number(),
            byte if is_name_start(byte) => self.name()?,
            byte => {
                self.tokens.push(Token::Punct(char::from(byte)));
                self.at += 1;
            }
        }
        Ok(())
    }

    /// A backslash: a line join when a newline follows it, punctuation if not.
    fn continuation(&mut self) {
        let mut next = self.at + 1;
        if self.bytes.get(next) == Some(&b'\r') {
            next += 1;
        }
        if self.bytes.get(next) == Some(&b'\n') {
            self.line += 1;
            self.at = next + 1;
        } else {
            self.tokens.push(Token::Punct('\\'));
            self.at += 1;
        }
    }

    fn number(&mut self) {
        while self.at < self.bytes.len()
            && (self.bytes[self.at].is_ascii_alphanumeric()
                || self.bytes[self.at] == b'.'
                || self.bytes[self.at] == b'_')
        {
            self.at += 1;
        }
        self.tokens.push(Token::Number);
    }

    /// An identifier, or the prefix of a string literal.
    fn name(&mut self) -> Result<(), Error> {
        let from = self.at;
        while self.at < self.bytes.len() && is_name_part(self.bytes[self.at]) {
            self.at += 1;
        }
        let name = &self.script[from..self.at];
        let quoted = matches!(self.bytes.get(self.at), Some(b'"' | b'\''));
        if quoted && is_string_prefix(name) {
            let (_, next, at) = read_string(self.script, self.at, self.line, true)?;
            self.tokens.push(Token::Opaque);
            self.at = next;
            self.line = at;
        } else {
            self.tokens.push(Token::Name(name.to_owned()));
        }
        Ok(())
    }

    fn finish_line(&mut self, ended: usize) {
        self.out.push(Logical {
            indent: self.indent,
            tabbed: self.tabbed,
            line: self.start,
            end: ended,
            tokens: std::mem::take(&mut self.tokens),
        });
        self.indent = 0;
        self.tabbed = false;
        self.measuring = true;
    }
}

fn to_end_of_line(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && bytes[at] != b'\n' {
        at += 1;
    }
    at
}

/// Reads one string literal, returning its token, the byte after it, and the
/// line the reader ended on.
///
/// `at` is the opening quote: both callers have already seen one there, the
/// prefixed form because [`Lexer::name`] steps over the prefix first. Checked
/// rather than assumed — this was a forward scan with no bound if there was
/// none.
///
/// `prefixed` makes the value opaque whatever the body says, because the value
/// of an `f`, `r` or `b` string is not its body.
///
/// **A known limit.** An f-string that nests the *same* quote character inside
/// its braces — legal only since Python 3.12 — ends early here. Nothing in any
/// Proton release uses one, and the damage is bounded: the tokens after it stop
/// matching the two shapes this reader knows, so the outcome is a refusal or a
/// bracket error rather than a policy read out of nonsense.
fn read_string(
    script: &str,
    at: usize,
    line: usize,
    prefixed: bool,
) -> Result<(Token, usize, usize), Error> {
    let bytes = script.as_bytes();
    let Some(&quote @ (b'"' | b'\'')) = bytes.get(at) else {
        return Err(Error {
            kind: ErrorKind::UnterminatedString,
            line,
        });
    };
    let triple = bytes.get(at + 1) == Some(&quote) && bytes.get(at + 2) == Some(&quote);
    let body = at + if triple { 3 } else { 1 };
    let opened = line;
    let mut line = line;
    let mut i = body;
    let mut escaped = false;

    loop {
        let Some(&byte) = bytes.get(i) else {
            return Err(Error {
                kind: ErrorKind::UnterminatedString,
                line: opened,
            });
        };
        if byte == b'\\' {
            escaped = true;
            if bytes.get(i + 1) == Some(&b'\n') {
                line += 1;
            }
            i += 2;
            continue;
        }
        if byte == b'\n' {
            if !triple {
                return Err(Error {
                    kind: ErrorKind::UnterminatedString,
                    line: opened,
                });
            }
            line += 1;
            i += 1;
            continue;
        }
        if byte == quote {
            if !triple {
                return Ok((finish(&script[body..i], escaped || prefixed), i + 1, line));
            }
            if bytes.get(i + 1) == Some(&quote) && bytes.get(i + 2) == Some(&quote) {
                return Ok((finish(&script[body..i], escaped || prefixed), i + 3, line));
            }
        }
        i += 1;
    }
}

fn finish(body: &str, opaque: bool) -> Token {
    if opaque {
        Token::Opaque
    } else {
        Token::Text(body.to_owned())
    }
}
