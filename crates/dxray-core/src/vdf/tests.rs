//! Shapes from real Steam files, or from corrupt ones; each test says what
//! reading it wrong would cost.

use super::{Error, ErrorKind, Value, parse};

/// Parses and unwraps to the root object, for the many cases that only care
/// about what came out.
fn root(input: &str) -> super::Object {
    match parse(input) {
        Ok(Value::Object(o)) => o,
        Ok(Value::String(s)) => panic!("root must be an object, got the string {s:?}"),
        Err(e) => panic!("expected a parse, got {e}"),
    }
}

fn error(input: &str) -> Error {
    match parse(input) {
        Err(e) => e,
        Ok(v) => panic!("expected a failure, got {v:?}"),
    }
}

#[test]
fn a_nested_document_keeps_the_order_its_keys_were_written_in() {
    // Entries keep file order, so reports are stable.
    let object = root(
        r#"
        "libraryfolders"
        {
            "0" { "path" "/one" }
            "1" { "path" "/two" }
            "2" { "path" "/three" }
        }
        "#,
    );

    let folders = object
        .get_object("libraryfolders")
        .expect("the root block is an object");
    let keys: Vec<&str> = folders.iter().map(|(k, _)| k).collect();

    assert_eq!(keys, ["0", "1", "2"], "insertion order must survive");
}

#[test]
fn keys_are_matched_without_regard_to_case() {
    // Keys match without case: older installs spell `LibraryFolders`.
    let object = root(r#" "LibraryFolders" { "TimeNextStatsReport" "0" } "#);

    assert!(
        object.get("libraryfolders").is_some(),
        "the lowercase spelling must find the capitalised key"
    );
    assert_eq!(
        object
            .get_object("LIBRARYFOLDERS")
            .and_then(|o| o.get_str("timenextstatsreport")),
        Some("0"),
        "nested keys match the same way"
    );
}

#[test]
fn a_utf8_byte_order_mark_is_not_part_of_the_first_key() {
    // A BOM added by an editor or sync tool is skipped.
    let object = root("\u{feff}\"libraryfolders\"\n{\n\"0\" \"/one\"\n}\n");

    assert_eq!(
        object
            .get_object("libraryfolders")
            .and_then(|o| o.get_str("0")),
        Some("/one"),
        "the mark must be skipped, not absorbed"
    );
}

#[test]
fn crlf_line_endings_parse_the_same_as_lf() {
    // CRLF, from Steam on Windows read through `/mnt/c`.
    let object = root("\"AppState\"\r\n{\r\n\t\"appid\"\t\t\"570\"\r\n}\r\n");

    assert_eq!(
        object
            .get_object("AppState")
            .and_then(|o| o.get_str("appid")),
        Some("570"),
        "the value must not carry a trailing carriage return"
    );
}

#[test]
fn an_escaped_quote_and_an_escaped_backslash_come_back_as_one_character_each() {
    // Escaped backslashes and quotes, from Windows paths and titles.
    let object = root(r#" "path" "C:\\Steam\\Library"  "name" "The \"Best\" Game" "#);

    assert_eq!(object.get_str("path"), Some(r"C:\Steam\Library"));
    assert_eq!(object.get_str("name"), Some(r#"The "Best" Game"#));
}

#[test]
fn the_n_and_t_escapes_become_the_characters_they_name() {
    // Valve's writer emits these, so a reader that passes them through as two
    // characters produces a value that no longer equals what was stored.
    let object = root(r#" "a" "one\ttwo"  "b" "line\nline" "#);

    assert_eq!(object.get_str("a"), Some("one\ttwo"));
    assert_eq!(object.get_str("b"), Some("line\nline"));
}

#[test]
fn tokens_that_touch_with_no_whitespace_between_them_still_separate() {
    // Whitespace is not a separator in this format, the quotes are. A parser
    // that requires a gap rejects a minified file that Valve's reader accepts.
    let object = root("\"a\"\"b\"\"c\"{\"d\"\"e\"}");

    assert_eq!(object.get_str("a"), Some("b"));
    assert_eq!(
        object.get_object("c").and_then(|o| o.get_str("d")),
        Some("e")
    );
}

#[test]
fn an_escape_nobody_defined_keeps_its_backslash() {
    // An undefined escape keeps its backslash, so the path still resolves.
    let object = root(r#" "path" "C:\Games\Steam" "#);

    assert_eq!(
        object.get_str("path"),
        Some(r"C:\Games\Steam"),
        "an unknown escape is passed through rather than eaten"
    );
}

#[test]
fn an_undefined_escape_before_a_multi_byte_character_keeps_the_character_whole() {
    // The same with non-ASCII after the backslash, which used to panic.
    let object =
        root("\"a\" \"D:\\Übergame\"  \"b\" \"\\大神\"  \"c\" \"\\🎮 Game\"  \"d\" \"\\é\"");

    assert_eq!(
        object.get_str("a"),
        Some(r"D:\Übergame"),
        "the backslash is kept and Ü survives as one character"
    );
    assert_eq!(object.get_str("b"), Some(r"\大神"));
    assert_eq!(object.get_str("c"), Some(r"\🎮 Game"));
    assert_eq!(object.get_str("d"), Some(r"\é"));
}

#[test]
fn a_backslash_at_the_end_of_a_line_is_still_an_unterminated_string() {
    // The newline after a backslash still ends the line, so the unclosed string
    // is refused.
    let err = error("\"path\" \"C:\\\n\"next\" \"value\"");

    assert_eq!(err.kind, ErrorKind::NewlineInString);
}

#[test]
fn a_line_comment_is_not_data() {
    // Commented-out entries are skipped.
    let object = root(
        r#"
        // this whole line is a note
        "libraryfolders" // and so is this tail
        {
            "0" "/one"   // including one after a value
        }
        "#,
    );

    let folders = object.get_object("libraryfolders").expect("root block");
    assert_eq!(folders.len(), 1, "only the real entry survives");
    assert_eq!(folders.get_str("0"), Some("/one"));
}

#[test]
fn a_double_slash_inside_a_quoted_string_is_a_path_and_not_a_comment() {
    // `//` inside a quoted value is not a comment.
    let object = root(r#" "path" "/home//user/.steam" "#);

    assert_eq!(object.get_str("path"), Some("/home//user/.steam"));
}

#[test]
fn unquoted_tokens_are_accepted_on_both_sides_of_a_pair() {
    // Some Valve-authored files omit the quotes. Rejecting them would refuse
    // a file the official client reads without complaint.
    let object = root("AppState\n{\n\tappid\t\t570\n\tname \"Dota 2\"\n}\n");

    let state = object.get_object("appstate").expect("bare key parsed");
    assert_eq!(state.get_str("appid"), Some("570"));
    assert_eq!(state.get_str("name"), Some("Dota 2"));
}

#[test]
fn a_truncated_file_is_an_error_and_names_the_brace_that_was_left_open() {
    // A truncated manifest is an error, not a game with no `installdir`.
    let error = error("\"AppState\"\n{\n\t\"appid\"\t\"570\"\n\t\"name\"\t\"Dota");

    assert!(
        matches!(error.kind, ErrorKind::UnclosedString),
        "the cut landed mid-string, got {error:?}"
    );

    let cut_at_a_brace = super::parse("\"AppState\"\n{\n\t\"appid\"\t\"570\"\n")
        .expect_err("a file cut after a complete pair is still truncated");

    assert_eq!(
        cut_at_a_brace.kind,
        ErrorKind::UnclosedObject { opened_line: 2 },
        "the message must point at the '{{' on line 2, got {cut_at_a_brace}"
    );
}

#[test]
fn a_key_whose_value_was_cut_off_is_reported_rather_than_stored_empty() {
    // `"installdir"` with nothing after it would otherwise land in the tree as
    // an empty string and resolve to the bare `steamapps/common` directory.
    let error = error("\"AppState\"\n{\n\t\"installdir\"\n}\n");

    assert_eq!(error.kind, ErrorKind::MissingValue);
    assert_eq!(error.line, 3, "reported where the value should have been");
}

#[test]
fn a_missing_closing_quote_does_not_swallow_the_rest_of_the_file() {
    // Reading on to the next quote would fold two lines into one value and
    // produce a tree that parses cleanly and says something untrue.
    let error = error("\"a\" \"1\"\n\"name\" \"Dota 2\n\"appid\" \"570\"\n");

    assert_eq!(error.kind, ErrorKind::NewlineInString);
    assert_eq!(error.line, 2, "the line the quote opened on");
    assert_eq!(
        error.column, 8,
        "the column of the opening quote, which is the character to go and fix \
         — not the end of the line, which is only where the parser noticed"
    );
}

#[test]
fn an_unterminated_string_at_the_end_of_input_names_the_line_the_quote_opened_on() {
    // The error points at the unclosed string's opening quote, not the end of
    // the file.
    let error = error("\"AppState\"\n{\n\t\"appid\"\t\"570\"\n\t\"name\"\t\"Dota 2");

    assert_eq!(error.kind, ErrorKind::UnclosedString);
    assert_eq!(error.line, 4, "the line the quote opened on");
    assert_eq!(
        error.column, 9,
        "the opening quote, not the end of the input six characters later"
    );
}

#[test]
fn a_platform_conditional_is_refused_rather_than_read_as_the_next_key() {
    // A `[$WIN32]` conditional is refused rather than shifting every pair.
    let after_a_value = error("\"a\" \"1\" [$WIN32]\n\"b\" \"2\"\n");
    assert_eq!(after_a_value.kind, ErrorKind::PlatformConditional);
    assert_eq!(after_a_value.line, 1, "where the bracket is");

    // The other place it occurs: between a key and its value.
    let before_a_value = error("\"a\" [$WIN32] \"1\"\n");
    assert_eq!(before_a_value.kind, ErrorKind::PlatformConditional);
}

#[test]
fn a_base_or_include_directive_is_refused_rather_than_stored_as_a_pair() {
    // `#base` and `#include` are refused: their data would go missing.
    for input in ["#base \"other.vdf\"\n\"a\" \"1\"\n", "#include \"x.vdf\"\n"] {
        let error = error(input);
        assert_eq!(
            error.kind,
            ErrorKind::Directive,
            "a directive must not pass for data, got {error}"
        );
    }
}

#[test]
fn a_quoted_key_that_merely_looks_like_a_directive_is_ordinary_data() {
    // The refusal is about the bare `#base` token, not about the characters.
    // A quoted key is a key, and rejecting it would invent a failure.
    let object = root("\"#base\" \"not a directive\"\n");

    assert_eq!(object.get_str("#base"), Some("not a directive"));
}

#[test]
fn a_closing_brace_with_nothing_open_is_an_error() {
    let error = error("\"a\" \"b\"\n}\n");

    assert_eq!(error.kind, ErrorKind::UnmatchedCloseBrace);
    assert_eq!(error.line, 2);
}

#[test]
fn a_block_with_no_name_is_an_error_rather_than_a_key_invented_for_it() {
    let error = error("{ \"a\" \"b\" }");

    assert_eq!(error.kind, ErrorKind::ExpectedKey);
}

#[test]
fn nesting_without_end_is_an_error_and_not_a_stack_overflow() {
    // Deep nesting is an error, not a stack overflow.
    let input = "\"k\" {".repeat(10_000);

    let error = error(&input);

    assert_eq!(
        error.kind,
        ErrorKind::TooDeep,
        "deep nesting must come back as an error, got {error}"
    );
}

#[test]
fn a_document_nested_exactly_to_the_bound_parses_and_one_level_past_it_does_not() {
    // The documented depth is allowed; built from the constant.
    use super::MAX_DEPTH;

    let nested = |levels: usize| format!("{}{}", "\"k\" {".repeat(levels), "}".repeat(levels));

    assert!(
        parse(&nested(MAX_DEPTH)).is_ok(),
        "a document nested exactly {MAX_DEPTH} deep is within the bound"
    );
    assert_eq!(
        error(&nested(MAX_DEPTH + 1)).kind,
        ErrorKind::TooDeep,
        "and one level past it is refused"
    );
}

#[test]
fn the_reported_column_counts_characters_rather_than_bytes() {
    // Columns count characters, not bytes.
    let error = error("\"名前\" \"値\" }");

    assert_eq!(error.kind, ErrorKind::UnmatchedCloseBrace);
    assert_eq!(
        error.column, 10,
        "one-based character column of the '}}', got {error}"
    );
}

#[test]
fn an_empty_document_is_an_empty_root_rather_than_a_failure() {
    // An empty file is empty, not truncated.
    assert!(root("").is_empty());
    assert!(root("// only a comment\n").is_empty());
}

#[test]
fn a_duplicated_key_keeps_both_entries_and_resolves_to_the_first() {
    // Duplicate keys are all kept, in order.
    let object = root(r#" "path" "/first"  "path" "/second" "#);

    assert_eq!(object.get_str("path"), Some("/first"));
    assert_eq!(object.len(), 2, "both entries are kept");
}

#[test]
fn an_empty_block_parses_as_an_empty_object_and_not_as_a_string() {
    // `"apps" {}` appears in a library that has had every game uninstalled.
    let object = root(r#" "0" { "path" "/one"  "apps" {} } "#);

    let entry = object.get_object("0").expect("the entry is a block");
    assert_eq!(
        entry.get_object("apps").map(super::Object::is_empty),
        Some(true),
        "an empty block is still a block"
    );
}
