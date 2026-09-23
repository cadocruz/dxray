//! Scripts cut down from real Proton releases, keeping the shapes that matter,
//! small enough for each test to break one thing.

use super::{Decision, Flag, Policy, Reading, RefusalKind, Unknown, decide, scan, settled};

/// A deny-list script: Proton 9.0 onwards, where an application id in the list
/// is one that does **not** get NVAPI.
const DENY: &str = r#"#!/usr/bin/env python3
import os

def default_compat_config():
    ret = set()
    if "SteamAppId" in os.environ:
        appid = os.environ["SteamAppId"]
        if appid in [
                "2630", #Call of Duty 2
                ]:
            ret.add("nofsync")
            ret.add("noesync")

        if appid in [
                # disable dxvknvapi for titles which dislike it
                "1088850", #Marvel's Guardians of the Galaxy
                "435150", #Divinity: Original Sin 2 - Definitive Edition
                ]:
            ret.add("disablenvapi")

        if appid in [
                "2395210", #Tony Hawk's Pro Skater 1 + 2
                ]:
            ret.add("forcenvapi")
    return ret
"#;

/// The same script with Proton 10.0's second block added: the same kind of
/// list, and a flag that is only set when no NVIDIA driver is loaded.
const GUARDED: &str = r#"import os

def default_compat_config():
    ret = set()
    if "SteamAppId" in os.environ:
        appid = os.environ["SteamAppId"]
        if appid in [
                "1088850", #Marvel's Guardians of the Galaxy
                ]:
            ret.add("disablenvapi")

        if appid in [
                "108710", #Alan Wake
                "44350", #GRID 2
                ]:
            try:
                with open('/proc/modules') as f:
                    drivers = set([line.partition(' ')[0] for line in f.read().splitlines()])
                    if not drivers.intersection({'nvidia', 'nouveau', 'nova'}):
                        ret.add("disablenvapi")
            except OSError:
                ret.add("disablenvapi")
    return ret
"#;

/// A flat block — nothing at all between the list and the flag — with one extra
/// test wrapped around it. Everything else is exactly [`DENY`].
const WRAPPED: &str = r#"import os

def default_compat_config():
    ret = set()
    if "SteamAppId" in os.environ:
        appid = os.environ["SteamAppId"]
        if not os.environ.get("PROTON_DISABLE_NVAPI_QUIRKS"):
            if appid in [
                    "1088850", #Marvel's Guardians of the Galaxy
                    "435150", #Divinity: Original Sin 2 - Definitive Edition
                    ]:
                ret.add("disablenvapi")
    return ret
"#;

/// An allow-list script: Proton 8.0, where the list holds the only games that
/// *do* get NVAPI and every other game goes without.
const ALLOW: &str = r#"import os

def default_compat_config():
    ret = set()
    if "SteamAppId" in os.environ:
        appid = os.environ["SteamAppId"]
        if appid in [
                # enable dxvknvapi for titles verified to benefit (e.g. working DLSS)
                "1182900", #A Plague Tale: Requiem
                "1086940", #Baldur's Gate 3
                ]:
            ret.add("enablenvapi")
    return ret
"#;

/// The function, real appid lists, and no NVAPI anywhere in the file. The
/// shape of a genuine true negative.
const NO_POLICY: &str = r#"import os

def default_compat_config():
    ret = set()
    if "SteamAppId" in os.environ:
        appid = os.environ["SteamAppId"]
        if appid in [
                "1621680",
                ]:
            ret.add("noforcelgadd")

        if appid in [
                "257420", #Serious Sam 4
                ]:
            ret.add("hidevggpu")
    return ret
"#;

/// Proton 7.0's shape: no per-game NVAPI policy, and an environment switch
/// that decides NVAPI for every game at once.
const SWITCH_ONLY: &str = r#"import os

def default_compat_config():
    ret = set()
    if appid in ["257420"]:
        ret.add("hidevggpu")
    return ret

class Session:
    def check(self):
        self.check_environment("PROTON_ENABLE_NVAPI", "enablenvapi")
"#;

/// Every NVAPI flag set by a shape this reader does not model, so nothing is
/// read. Must never pass for [`NO_POLICY`].
const UNREADABLE: &str = r#"import os

def default_compat_config():
    ret = set()
    if "SteamAppId" in os.environ:
        appid = os.environ["SteamAppId"]
        if appid in [
                "1088850",
                ]:
            ret.update({"disablenvapi"})
    return ret
"#;

/// The same script with the `forcenvapi` list kept and the `disablenvapi` one
/// taken out, so NVAPI lists exist and nothing says which way round they run.
const FORCE_ONLY: &str = r#"import os

def default_compat_config():
    ret = set()
    if "SteamAppId" in os.environ:
        appid = os.environ["SteamAppId"]
        if appid in [
                "257420",
                ]:
            ret.add("hidevggpu")

        if appid in [
                "2395210", #Tony Hawk's Pro Skater 1 + 2
                "1577120", #The Quarry
                ]:
            ret.add("forcenvapi")
    return ret
"#;

fn read(script: &str) -> Reading {
    scan(script).expect("this script is one the reader understands")
}

#[test]
fn a_game_in_a_flat_disable_list_is_reported_as_withheld() {
    // Listed under an outright flag: settled before the game ever runs.
    let decision = decide(&read(DENY), "1088850");

    assert_eq!(
        decision.available(),
        Some(false),
        "a game in the disable list does not get NVAPI, got {decision}"
    );
    assert!(
        matches!(decision, Decision::ListedToDisable { .. }),
        "and the reason says which list, got {decision:?}"
    );
}

#[test]
fn a_game_a_deny_list_does_not_name_keeps_nvapi() {
    // The other half of a determinate answer. This is only safe to say because
    // a policy was found and nothing in it was refused.
    let decision = decide(&read(DENY), "570");

    assert_eq!(
        decision.available(),
        Some(true),
        "a game outside the disable list keeps NVAPI, got {decision}"
    );
    assert_eq!(decision, Decision::NotListedToDisable);
}

#[test]
fn a_test_wrapped_around_a_flat_block_makes_it_conditional_rather_than_vanishing() {
    // An extra test around an outright block. It cannot be quoted, so it is
    // reported as a condition, never as a settled answer.
    let reading = read(WRAPPED);
    let decision = decide(&reading, "1088850");

    assert_eq!(
        decision.available(),
        None,
        "the wrapper must not be waved through, got {decision}"
    );
    let condition = decision.condition().expect("the wrapper is carried");
    assert_eq!(condition.guards.len(), 1, "got {:?}", condition.guards);
    assert!(condition.guards[0].enclosing, "it wraps the list itself");
    assert_eq!(
        condition.guards[0].mentions,
        vec!["PROTON_DISABLE_NVAPI_QUIRKS"],
        "and what it turns on comes out of the file"
    );
    assert!(
        condition.source().join("\n").contains("os.environ.get"),
        "the test is quoted back, got {:?}",
        condition.source()
    );
    assert!(
        reading.sites[0].condition.is_some(),
        "and the site itself carries it, so a listing cannot print it as flat"
    );
}

#[test]
fn the_block_that_only_binds_the_application_id_is_not_treated_as_a_condition() {
    // The `SteamAppId` guard around every release's policy is exempt: it is the
    // condition for having an appid at all.
    let reading = read(DENY);

    assert!(
        reading.sites.iter().all(|site| site.condition.is_none()),
        "no site in a plain deny-list script is conditional, got {:?}",
        reading.sites
    );
    assert_eq!(decide(&reading, "1088850").available(), Some(false));
}

#[test]
fn a_guarded_block_reports_the_condition_instead_of_deciding_it() {
    // Proton 10.0: this block disables NVAPI only when no NVIDIA driver is loaded.
    let decision = decide(&read(GUARDED), "108710");

    assert_eq!(
        decision.available(),
        None,
        "a condition is neither a yes nor a no, got {decision}"
    );
    assert!(
        !decision.is_unknown(),
        "but it is still an answer: the script says what it turns on"
    );
    let condition = decision.condition().expect("the condition is carried");
    assert_eq!(condition.guards.len(), 1);
    assert!(!condition.guards[0].enclosing, "it sits inside the block");
    assert_eq!(
        condition.guards[0].mentions,
        vec!["/proc/modules", "nvidia", "nouveau", "nova"],
        "what it turns on comes out of the file, not out of this crate"
    );
}

#[test]
fn the_condition_is_quoted_back_so_a_reader_can_apply_it() {
    // Quoted, not interpreted: the answer depends on the machine.
    let decision = decide(&read(GUARDED), "44350");
    let condition = decision.condition().expect("carried");
    let quoted = condition.source().join("\n");

    assert!(quoted.contains("/proc/modules"), "got:\n{quoted}");
    assert!(quoted.contains("except OSError"), "got:\n{quoted}");
    assert!(
        quoted.starts_with("try:"),
        "the shared indentation is stripped so the quote fits a report, got:\n{quoted}"
    );
    assert!(
        condition.guards[0].first < condition.guards[0].last,
        "and it says where in the script to look"
    );
}

#[test]
fn a_flat_block_and_a_guarded_one_in_the_same_script_stay_apart() {
    // Proton 10.0 and 11.0 keep settled and conditional lists apart.
    let reading = read(GUARDED);
    let flat = decide(&reading, "1088850");
    let guarded = decide(&reading, "108710");

    assert_eq!(flat.available(), Some(false));
    assert_eq!(guarded.available(), None);
    assert_eq!(
        reading.sites.len(),
        2,
        "two sites, not one merged list: {:?}",
        reading.sites
    );
    assert!(reading.sites[0].condition.is_none());
    assert!(reading.sites[1].condition.is_some());
}

#[test]
fn an_allow_list_script_inverts_what_a_missing_game_means() {
    // Proton 6.3 to 8.0 list the games that get NVAPI, via `enablenvapi`.
    let reading = read(ALLOW);

    assert_eq!(reading.policy(), Some(Policy::AllowList));
    assert_eq!(
        decide(&reading, "1182900").available(),
        Some(true),
        "a listed game is one of the few that does get it"
    );
    assert_eq!(
        decide(&reading, "570").available(),
        Some(false),
        "and an unlisted game does not — the same silence, the opposite meaning"
    );
    assert_eq!(decide(&reading, "570"), Decision::NotListedToEnable);
}

#[test]
fn a_forced_game_keeps_nvapi_even_though_it_is_also_listed_to_lose_it() {
    // `forcenvapi` wins; only an id in both lists can show it.
    let both = DENY.replace(r#""2395210", #Tony"#, r#""1088850", #Tony"#);
    let decision = decide(&read(&both), "1088850");

    assert_eq!(
        decision.available(),
        Some(true),
        "force beats disable, got {decision}"
    );
    assert!(matches!(decision, Decision::ListedToForce { .. }));
}

#[test]
fn a_script_read_in_full_with_no_nvapi_policy_is_a_finding_and_not_a_failure() {
    // Proton 7.0: lists parse and none touches NVAPI. A true negative, unlike a
    // refused script.
    let reading = read(SWITCH_ONLY);
    let decision = decide(&reading, "1088850");

    assert_eq!(
        decision,
        Decision::NotGameSpecific {
            via: Some(Flag::Enable)
        }
    );
    assert!(!decision.is_unknown(), "this is an answer, got {decision}");
    assert!(settled(&reading), "and the build was read well enough");
    assert_eq!(
        decision.available(),
        None,
        "though what the switch defaults to is not something this reader parses"
    );
    assert!(
        decision.to_string().contains("not game-specific"),
        "got {decision}"
    );
}

#[test]
fn a_policy_hidden_in_a_shape_this_reader_cannot_read_is_never_a_true_negative() {
    // Read in full versus partly refused: different sentence and exit code.
    let readable = read(SWITCH_ONLY);
    let hidden = read(UNREADABLE);

    assert!(hidden.sites.is_empty(), "no site was read out of either");
    assert!(readable.sites.is_empty());

    assert!(
        matches!(
            decide(&hidden, "1088850"),
            Decision::Unknown(Unknown::Unreadable { .. })
        ),
        "got {:?}",
        decide(&hidden, "1088850")
    );
    assert_ne!(
        decide(&hidden, "1088850").to_string(),
        decide(&readable, "1088850").to_string(),
        "the two must not say the same thing"
    );
    assert_ne!(
        settled(&hidden),
        settled(&readable),
        "and a caller drawing an exit code from this must not get the same one"
    );
}

#[test]
fn a_script_with_nvapi_lists_and_no_direction_does_not_claim_it_has_no_nvapi_lists() {
    // Only a `forcenvapi` list: no direction, but not "no list touches NVAPI"
    // either.
    let reading = read(FORCE_ONLY);
    let decision = decide(&reading, "1088850");

    assert!(reading.lists(Flag::Force), "there is an NVAPI list");
    assert_eq!(reading.policy(), None, "and no direction to be had from it");
    assert!(
        matches!(
            decision,
            Decision::Unknown(Unknown::PolarityUnknown {
                listed: Flag::Force,
                appids: 2
            })
        ),
        "got {decision:?}"
    );
    let said = decision.to_string();
    assert!(
        !said.contains("none of them touches NVAPI") && !said.contains("no NVAPI"),
        "it must not deny the lists it just read, got {said}"
    );
    assert!(
        said.contains("which way round its policy runs"),
        "got {said}"
    );
}

#[test]
fn a_game_named_in_the_only_list_there_is_still_gets_its_answer() {
    // `forcenvapi` alone hands NVAPI over, whatever the direction.
    let decision = decide(&read(FORCE_ONLY), "2395210");

    assert_eq!(decision.available(), Some(true), "got {decision}");
}

#[test]
fn the_answers_a_caller_can_act_on_are_told_apart_in_every_direction() {
    // Asserted together, because a test for any one of them passes on a build
    // that always gives that answer. These must never collapse into each other.
    let deny = read(DENY);
    let guarded = read(GUARDED);
    let switch = read(SWITCH_ONLY);
    let hidden = read(UNREADABLE);
    let force = read(FORCE_ONLY);

    assert_eq!(decide(&deny, "1088850").available(), Some(false));
    assert_eq!(decide(&deny, "570").available(), Some(true));
    assert_eq!(decide(&guarded, "108710").available(), None);
    assert_eq!(decide(&switch, "570").available(), None);
    assert_eq!(decide(&hidden, "570").available(), None);

    // Each `None` is told apart by its type and exit code.
    assert!(!decide(&guarded, "108710").is_unknown(), "a condition");
    assert!(!decide(&switch, "570").is_unknown(), "a true negative");
    assert!(decide(&hidden, "570").is_unknown(), "a refusal");
    assert!(decide(&force, "570").is_unknown(), "no direction");

    assert!(settled(&deny) && settled(&guarded) && settled(&switch));
    assert!(!settled(&hidden) && !settled(&force));
}

#[test]
fn a_script_that_cannot_be_read_never_becomes_an_empty_policy() {
    // A file that defeated the reader is an error, never an empty `Reading`.
    let error = scan("def default_compat_config():\n    ret = set(\n").expect_err("unclosed");

    assert!(
        error.to_string().contains("ends inside a bracket"),
        "the message has to say what defeated it, got {error}"
    );
}

#[test]
fn a_script_with_no_such_function_says_so_and_stops() {
    // No `default_compat_config` (6.3, or a truncated script): neither "no games
    // affected" nor a corrupt file.
    let reading = read("import os\n\ndef main():\n    pass\n");
    let decision = decide(&reading, "1088850");

    assert_eq!(reading.function_line, None);
    assert_eq!(
        decision,
        Decision::Unknown(Unknown::NoFunction { mechanism: false })
    );
    assert!(
        decision
            .to_string()
            .contains("contains no default_compat_config"),
        "got {decision}"
    );
}

#[test]
fn a_script_whose_nvapi_mechanism_is_not_a_list_of_games_is_told_apart_from_one_with_none() {
    // 6.3 has an environment switch and no function; 5.13 has neither.
    let mechanism =
        read("def main():\n    check_environment(\"PROTON_ENABLE_NVAPI\", \"enablenvapi\")\n");
    let neither = read("def main():\n    pass\n");

    assert_eq!(mechanism.elsewhere, vec![Flag::Enable]);
    assert!(neither.elsewhere.is_empty());
    assert_eq!(
        decide(&mechanism, "570"),
        Decision::Unknown(Unknown::NoFunction { mechanism: true })
    );
    assert!(
        decide(&mechanism, "570")
            .to_string()
            .contains("not a list of games"),
        "the sentence has to say a mechanism is there"
    );
}

#[test]
fn a_build_that_spells_its_flags_in_some_new_way_is_not_read_as_a_true_negative() {
    // Renamed flags are caught by the wide net, not mistaken for no policy.
    let renamed = read(
        "def default_compat_config():\n    ret = set()\n    if appid in [\"108710\"]:\n        \
         ret.add(\"nvapikill\")\n    return ret\n",
    );
    let decision = decide(&renamed, "570");

    assert!(renamed.sites.is_empty() && renamed.refusals.is_empty());
    assert!(renamed.nvapi_mentions > 0, "the wide net caught it");
    assert!(
        matches!(decision, Decision::Unknown(Unknown::UnknownSpelling { .. })),
        "got {decision:?}"
    );
    assert!(!settled(&renamed));
}

#[test]
fn a_script_with_no_nvapi_string_anywhere_is_a_plain_true_negative() {
    // The other side of that net. Proton 3.16 and 5.13 have no NVAPI in them at
    // all, and the answer for a build like that is a finding, not a hedge.
    let reading = read(NO_POLICY);

    assert_eq!(reading.nvapi_mentions, 0);
    assert_eq!(
        decide(&reading, "570"),
        Decision::NotGameSpecific { via: None }
    );
    assert!(settled(&reading));
}

#[test]
fn a_flag_set_by_a_shape_this_reader_does_not_model_is_refused_rather_than_ignored() {
    // Any unmodelled shape that sets a flag is refused, never dropped.
    let script = r#"def default_compat_config():
    ret = set()
    if "SteamAppId" in os.environ:
        appid = os.environ["SteamAppId"]
        if "WINE_SOMETHING" not in os.environ and appid in ["108710"]:
            ret.add("disablenvapi")
    return ret
"#;
    let reading = read(script);

    assert_eq!(reading.sites.len(), 0, "the shape was not understood");
    assert!(
        reading
            .refusals
            .iter()
            .any(|r| r.kind == RefusalKind::Unaccounted),
        "so it was refused, not dropped: {:?}",
        reading.refusals
    );
}

#[test]
fn a_refusal_blocks_a_negative_answer_without_blocking_a_positive_one() {
    // A refusal makes "not listed" unsafe, but not a positive find.
    let script = format!(
        "{}\n    for extra in others:\n        ret.add(\"disablenvapi\")\n",
        DENY.trim_end().trim_end_matches("    return ret")
    );
    let reading = read(&script);

    assert!(!reading.refusals.is_empty(), "the loop is refused");
    assert!(
        decide(&reading, "570").is_unknown(),
        "so an unlisted game can no longer be answered for"
    );
    assert_eq!(
        decide(&reading, "1088850").available(),
        Some(false),
        "but a game found in a list that was read still gets its answer"
    );
}

#[test]
fn an_appid_list_with_an_entry_that_is_not_a_plain_string_says_the_list_is_short() {
    // A list with a non-literal entry is incomplete, and says so.
    let script = DENY.replace(r#"                "435150","#, "                OTHER_ID,");
    let reading = read(&script);

    assert!(
        reading
            .refusals
            .iter()
            .any(|r| matches!(r.kind, RefusalKind::UnreadableEntries { count: 1 })),
        "got {:?}",
        reading.refusals
    );
    assert!(
        decide(&reading, "435150").is_unknown(),
        "and the id that was lost cannot be answered for"
    );
}

#[test]
fn a_comment_beside_an_application_id_never_becomes_an_application_id() {
    // Comments are stripped first: titles carry apostrophes.
    let reading = read(DENY);
    let site = reading
        .sites
        .iter()
        .find(|s| s.flag == Flag::Disable)
        .expect("the disable site");

    assert_eq!(site.appids, vec!["1088850", "435150"]);
}

#[test]
fn a_docstring_that_spells_out_the_policy_is_not_read_as_code() {
    // Matched on tokens, so a docstring is not a policy.
    let script = r#"DOC = """
def default_compat_config():
    if appid in ["108710"]:
        ret.add("disablenvapi")
"""

def default_compat_config():
    ret = set()
    if appid in ["1088850"]:
        ret.add("disablenvapi")
    return ret
"#;
    let reading = read(script);

    assert_eq!(reading.appid_lists, 1, "only the real one was read");
    assert_eq!(reading.sites[0].appids, vec!["1088850"]);
    assert_eq!(
        decide(&reading, "108710"),
        Decision::NotListedToDisable,
        "the id that only appears inside the docstring is not in the policy"
    );
}

#[test]
fn a_script_that_defines_the_function_twice_is_refused() {
    // Python keeps the last definition.
    let doubled = format!("{DENY}\n{DENY}");
    let error = scan(&doubled).expect_err("two definitions");

    assert!(error.to_string().contains("defined twice"), "got {error}");
}

#[test]
fn a_tab_in_the_function_is_refused_because_the_block_structure_rests_on_it() {
    // Blocks come from columns; a tab of the wrong width would move them.
    let tabbed = DENY.replace(
        "            ret.add(\"disablenvapi\")",
        "\t\t\tret.add(\"disablenvapi\")",
    );
    let error = scan(&tabbed).expect_err("a tab inside the function");

    assert!(
        error.to_string().contains("indented with a tab"),
        "got {error}"
    );
}

#[test]
fn a_single_id_compared_with_equality_is_read_as_a_list_of_one() {
    // `if appid == "...":` is read too.
    let script = r#"def default_compat_config():
    ret = set()
    if appid == "108710":
        ret.add("disablenvapi")
    return ret
"#;

    assert_eq!(decide(&read(script), "108710").available(), Some(false));
}

#[test]
fn a_body_on_the_same_line_as_the_test_is_still_a_flat_block() {
    // A one-line `if` is not a guarded block.
    let script = r#"def default_compat_config():
    ret = set()
    if appid in ["108710"]: ret.add("disablenvapi")
    return ret
"#;
    let decision = decide(&read(script), "108710");

    assert_eq!(decision.available(), Some(false), "got {decision}");
    assert!(matches!(decision, Decision::ListedToDisable { .. }));
}

#[test]
fn a_statement_split_across_lines_is_read_as_one_statement() {
    // Both of Python's ways of doing it: the brackets every appid list already
    // uses, and the backslash Proton uses elsewhere in the same file.
    let script = "def default_compat_config():\n    ret = set()\n    if appid in [\n        \"108710\",\n        ]:\n        ret.add(\"disable\" \\\n            )\n    return ret\n";
    let reading = read(script);

    assert_eq!(reading.appid_lists, 1);
    // The continued statement is not a `ret.add("<flag>")`, so it sets nothing
    // and is not mistaken for one.
    assert!(reading.sites.is_empty());
}

#[test]
fn an_unterminated_string_is_an_error_and_not_a_short_answer() {
    // An unterminated string is an error, not an empty policy.
    let error = scan("x = \"open\ny = 1\n").expect_err("unterminated");

    assert!(
        error.to_string().contains("unterminated string"),
        "got {error}"
    );
}

#[test]
fn nesting_past_the_cap_is_refused_rather_than_run_off_the_stack() {
    // Deep nesting is refused, not a stack overflow.
    let mut script = String::from("def default_compat_config():\n");
    for depth in 1..200 {
        script.push_str(&"    ".repeat(depth));
        script.push_str("if x:\n");
    }
    script.push_str(&"    ".repeat(200));
    script.push_str("ret.add(\"disablenvapi\")\n");
    let reading = read(&script);

    assert!(
        reading
            .refusals
            .iter()
            .any(|r| r.kind == RefusalKind::TooDeep),
        "got {:?}",
        reading.refusals
    );
}

#[test]
fn the_lists_and_the_rest_of_the_script_have_to_agree_about_which_way_the_policy_runs() {
    // Lists and switches disagreeing on direction is refused.
    let script = format!(
        "{DENY}\ndef check():\n    check_environment(\"PROTON_ENABLE_NVAPI\", \"enablenvapi\")\n"
    );
    let reading = read(&script);

    assert_eq!(reading.policy(), Some(Policy::DenyList));
    assert!(
        reading
            .refusals
            .iter()
            .any(|r| matches!(r.kind, RefusalKind::PolarityDisagrees { .. })),
        "got {:?}",
        reading.refusals
    );
    assert!(
        decide(&reading, "570").is_unknown(),
        "so no game can be cleared by absence"
    );
}

#[test]
fn a_release_that_names_both_its_own_flags_is_not_treated_as_a_disagreement() {
    // Every release since 9.0 names both switches; that is not a conflict.
    let script = format!(
        "{DENY}\ndef check():\n    check_environment(\"PROTON_DISABLE_NVAPI\", \"disablenvapi\")\n    \
         check_environment(\"PROTON_FORCE_NVAPI\", \"forcenvapi\")\n"
    );
    let reading = read(&script);

    assert!(reading.refusals.is_empty(), "got {:?}", reading.refusals);
    assert_eq!(decide(&reading, "570").available(), Some(true));
}

#[test]
fn the_evidence_line_says_which_of_the_facts_behind_an_answer_actually_held() {
    // The evidence sentence separates a thin answer from a solid one.
    let deny = read(DENY).evidence();
    let none = read(NO_POLICY).evidence();
    let missing = read("def main():\n    pass\n").evidence();

    assert!(deny.contains("default_compat_config at line 4"), "{deny}");
    assert!(deny.contains("3 appid lists read"), "{deny}");
    assert!(deny.contains("2 appids under disablenvapi"), "{deny}");

    assert!(none.contains("no NVAPI site among them"), "{none}");
    assert!(missing.contains("no default_compat_config"), "{missing}");
}

#[test]
fn the_evidence_line_counts_the_conditional_sites_separately() {
    // Twenty-two games in the lists, eight of which are settled and fourteen of
    // which are not, is a different statement from twenty-two games disabled.
    let evidence = read(GUARDED).evidence();

    assert!(
        evidence.contains("3 appids under disablenvapi"),
        "{evidence}"
    );
    assert!(
        evidence.contains("1 of the sites is conditional"),
        "{evidence}"
    );
}
