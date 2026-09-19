//! What was learned about one file, and how it turns into one line of JSON.
//!
//! A record exists for every input file, including the ones that failed. A
//! harness that reads the output has to be able to line results up with inputs
//! without counting, so a failure is a row with an `error` in it rather than a
//! row that is missing.

use std::fmt::Write as _;
use std::path::Path;

use dxray_core::LibraryCache;
use dxray_core::analysis::Finding;
use dxray_core::{Evidence, Verdict, analyse};
use dxray_pe::Pe;

/// Everything reported about a single file.
///
/// The optional fields are three-state in JSON terms — a version can be absent
/// because the image carries no version resource, which is not a failure, and a
/// machine can be absent only because nothing could be parsed at all.
pub struct Record {
    pub path: String,
    pub machine: Option<String>,
    pub bits: Option<u8>,
    pub imports: Vec<String>,
    pub delay_imports: Vec<String>,
    pub file_version: Option<String>,
    pub product_version: Option<String>,
    pub error: Option<String>,
    /// What `dxray-core` made of the imports and the directory. Empty for a
    /// file that failed, because a verdict drawn from no evidence would read as
    /// a finding about the file rather than about the scan.
    pub verdict: Verdict,
}

impl Record {
    /// A record for a file that could not be read or parsed.
    pub fn failed(path: &Path, error: &dyn std::fmt::Display) -> Self {
        Self {
            path: display_path(path),
            machine: None,
            bits: None,
            imports: Vec::new(),
            delay_imports: Vec::new(),
            file_version: None,
            product_version: None,
            error: Some(error.to_string()),
            verdict: Verdict::default(),
        }
    }

    /// Reads `path` and reports what the image says about itself.
    ///
    /// A single failing accessor sinks the whole record rather than being
    /// reported as an empty list, because "imports nothing" and "the import
    /// table could not be read" are different facts and only one of them is
    /// worth acting on.
    pub fn read(path: &Path) -> Self {
        Self::read_with(path, &mut LibraryCache::default())
    }

    /// The same, reusing what `cache` has already read of this directory.
    ///
    /// A scan of one directory asks the same questions of the same libraries
    /// once per file in it: an 803-file folder used to open each of its three
    /// DLSS runtimes 805 times, because every record listed the directory
    /// afresh and nothing was remembered between records. Handing one cache
    /// down the walk makes that one read each, and it also collapses the second
    /// multiplier — an imported library was read once for the link chase and
    /// again for its version, and now both answers come out of the one read.
    pub fn read_with(path: &Path, cache: &mut LibraryCache) -> Self {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => return Self::failed(path, &e),
        };
        match Self::from_image(path, &bytes, cache) {
            Ok(record) => record,
            Err(e) => Self::failed(path, &e),
        }
    }

    fn from_image(path: &Path, bytes: &[u8], cache: &mut LibraryCache) -> dxray_pe::Result<Self> {
        let pe = Pe::parse(bytes)?;
        let version = pe.version_info()?;
        // Assembled, analysed, then taken apart again: the record and the
        // evidence want the same two lists, and handing them over rather than
        // cloning keeps one copy of a table that can run to hundreds of names.
        let imports = pe.imports()?;
        let delay_imports = pe.delay_imports()?;
        // Listed through the cache too: one `read_dir` for a directory rather
        // than one per file in it.
        let neighbours = cache.neighbours(path);
        // The same one-link chase the ranking does, through the one function
        // that knows how to do it. A record that skipped it would call a Unity
        // stub "no graphics API determined" while `--game` called it Direct3D,
        // about the same file in the same run.
        let linked = cache.follow(path, &imports, &delay_imports, &neighbours);
        let evidence = Evidence {
            imports,
            delay_imports,
            neighbours,
            linked,
        };
        let mut verdict = analyse(&evidence);
        // The version of every library the verdict points at, read through the
        // one function that knows how. `nvngx_dlss.dll` is called that in every
        // build ever shipped, so the name alone answers nothing and the number
        // is the whole question a reader came with.
        cache.stamp(&mut verdict, path, &evidence.neighbours);
        Ok(Self {
            path: display_path(path),
            machine: Some(pe.machine().to_string()),
            bits: Some(if pe.is_64bit() { 64 } else { 32 }),
            imports: evidence.imports,
            delay_imports: evidence.delay_imports,
            file_version: version.map(|v| v.file.to_string()),
            product_version: version.map(|v| v.product.to_string()),
            error: None,
            verdict,
        })
    }

    /// One line of JSONL: always thirteen keys, always in this order.
    ///
    /// Written by hand so the order is a property of this function rather than
    /// of a derive macro's field order, since the order is part of what callers
    /// were promised.
    ///
    /// The first eight keys are frozen, in their original order, because a
    /// harness already reads them positionally. Everything the verdict adds is
    /// appended after `error` and never inserted among them.
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(256);
        out.push('{');
        push_string(&mut out, "path", &self.path);
        out.push(',');
        push_optional(&mut out, "machine", self.machine.as_deref());
        out.push(',');
        let _ = match self.bits {
            Some(bits) => write!(out, "\"bits\":{bits}"),
            None => write!(out, "\"bits\":null"),
        };
        out.push(',');
        push_array(&mut out, "imports", &self.imports);
        out.push(',');
        push_array(&mut out, "delay_imports", &self.delay_imports);
        out.push(',');
        push_optional(&mut out, "file_version", self.file_version.as_deref());
        out.push(',');
        push_optional(&mut out, "product_version", self.product_version.as_deref());
        out.push(',');
        push_optional(&mut out, "error", self.error.as_deref());
        out.push(',');
        push_string(&mut out, "verdict", &self.verdict.headline());
        out.push(',');
        push_findings(&mut out, "renderers", &self.verdict.renderers);
        out.push(',');
        push_findings(&mut out, "infrastructure", &self.verdict.infrastructure);
        out.push(',');
        push_findings(&mut out, "features", &self.verdict.features);
        out.push(',');
        push_findings(&mut out, "local_overrides", &self.verdict.local_overrides);
        out.push('}');
        out
    }
}

/// A list of findings, each carrying the observations behind it.
///
/// The provenance is nested rather than flattened into parallel arrays: which
/// library produced a finding, and whether it was imported, delay-loaded or
/// merely found in the directory, is part of the finding and stays attached to
/// it where a consumer cannot lose the pairing.
fn push_findings(out: &mut String, key: &str, findings: &[Finding]) {
    let _ = write!(out, "\"{key}\":[");
    for (i, finding) in findings.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('{');
        push_string(out, "name", &finding.name);
        let _ = write!(out, ",\"via\":[");
        for (j, signal) in finding.signals.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push('{');
            push_string(out, "library", &signal.library);
            out.push(',');
            push_string(out, "source", signal.source.as_str());
            out.push(',');
            push_version(out, signal.version);
            out.push('}');
        }
        out.push_str("]}");
    }
    out.push(']');
}

/// The version of the file a signal names, always the same three keys.
///
/// Rectangular on purpose. A consumer that wants the number reads `file`; one
/// that wants to know *why* there is no number reads `state`, which is the
/// difference between a DLL the game ships without a version resource, a name
/// the loader takes from `System32`, and a file that could not be read at all.
/// `0.0.0.0` is a value of `file` like any other and never a null.
fn push_version(out: &mut String, version: dxray_core::FileVersion) {
    let _ = write!(out, "\"version\":{{");
    push_string(out, "state", version.state());
    out.push(',');
    let info = version.info();
    push_optional(out, "file", info.map(|i| i.file.to_string()).as_deref());
    out.push(',');
    push_optional(
        out,
        "product",
        info.map(|i| i.product.to_string()).as_deref(),
    );
    out.push('}');
}

/// A path as text. Bytes that are not valid UTF-8 are replaced rather than
/// dropped, so the row still names something a human can match to a file.
///
/// Shared with [`listing`](crate::listing), which names paths in its own JSON
/// shape. One answer to "what does a path look like in JSON", so a directory
/// whose name is not valid UTF-8 cannot come out replaced on one surface and
/// dropped on the other.
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Shared with [`listing`](crate::listing) for the reason [`push_quoted`] is:
/// a second writer of `"key":"value"` is a second escaper the day somebody
/// forgets to call this one.
pub fn push_string(out: &mut String, key: &str, value: &str) {
    let _ = write!(out, "\"{key}\":");
    push_quoted(out, value);
}

/// A string, or a JSON `null` where there is no answer. Never an empty string:
/// "nothing was found" and "there was nothing to find" are different facts and
/// this project does not let them share a rendering.
pub fn push_optional(out: &mut String, key: &str, value: Option<&str>) {
    match value {
        Some(value) => push_string(out, key, value),
        None => {
            let _ = write!(out, "\"{key}\":null");
        }
    }
}

fn push_array(out: &mut String, key: &str, values: &[String]) {
    let _ = write!(out, "\"{key}\":[");
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_quoted(out, value);
    }
    out.push(']');
}

/// Escapes per RFC 8259. DLL names come out of an attacker-shaped file and a
/// Windows path can hold a backslash on any day of the week, so nothing here
/// assumes the input is already safe to paste between quotes.
///
/// Shared with [`game`](crate::game), which appends its own keys to a record's
/// line rather than growing a second escaper that would drift from this one.
pub fn push_quoted(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            // Everything below 0x20 has to be escaped, and the \u form is the
            // only one that covers the ones without a short name.
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::{Evidence, Record, analyse};

    fn sample() -> Record {
        let evidence = Evidence {
            imports: vec!["dxgi.dll".to_owned(), "KERNEL32.dll".to_owned()],
            delay_imports: vec!["d3d12.dll".to_owned()],
            neighbours: Vec::new(),
            ..Evidence::default()
        };
        let verdict = analyse(&evidence);
        Record {
            path: "/mnt/c/game.exe".to_owned(),
            machine: Some("x86-64".to_owned()),
            bits: Some(64),
            imports: evidence.imports,
            delay_imports: evidence.delay_imports,
            file_version: Some("1.2.3.4".to_owned()),
            product_version: Some("1.0.0.0".to_owned()),
            error: None,
            verdict,
        }
    }

    #[test]
    fn a_successful_record_emits_thirteen_keys_in_the_promised_order() {
        // The exact string, not a parsed comparison: key order is part of the
        // contract and a parser would happily accept it shuffled.
        assert_eq!(
            sample().to_json(),
            concat!(
                r#"{"path":"/mnt/c/game.exe","machine":"x86-64","bits":64,"imports":["dxgi.dll","KERNEL32.dll"],"delay_imports":["d3d12.dll"],"file_version":"1.2.3.4","product_version":"1.0.0.0","error":null,"#,
                r#""verdict":"Direct3D 12","renderers":[{"name":"Direct3D 12","via":[{"library":"d3d12.dll","source":"delay-import","version":{"state":"unread","file":null,"product":null}}]}],"#,
                r#""infrastructure":[{"name":"DXGI","via":[{"library":"dxgi.dll","source":"import","version":{"state":"unread","file":null,"product":null}}]}],"features":[],"local_overrides":[]}"#
            )
        );
    }

    #[test]
    fn the_original_eight_keys_keep_their_place_at_the_front_of_the_line() {
        // A harness reads them positionally, so the verdict may only append.
        // Asserting on the prefix is what turns that from a promise into a
        // thing that breaks the build when somebody inserts a key.
        let json = sample().to_json();

        assert!(
            json.starts_with(
                r#"{"path":"/mnt/c/game.exe","machine":"x86-64","bits":64,"imports":["dxgi.dll","KERNEL32.dll"],"delay_imports":["d3d12.dll"],"file_version":"1.2.3.4","product_version":"1.0.0.0","error":null,"#
            ),
            "the frozen prefix must survive verbatim, got {json}"
        );
    }

    #[test]
    fn a_failed_record_still_carries_every_key() {
        // A consumer indexes fields by name and would otherwise have to special
        // case failures; the shape stays rectangular so it does not have to.
        let path = std::path::Path::new("/mnt/c/notes.txt");
        let json = Record::failed(path, &"not a PE image: missing PE signature").to_json();

        assert_eq!(
            json,
            concat!(
                r#"{"path":"/mnt/c/notes.txt","machine":null,"bits":null,"imports":[],"delay_imports":[],"file_version":null,"product_version":null,"error":"not a PE image: missing PE signature","#,
                r#""verdict":"no graphics API determined","renderers":[],"infrastructure":[],"features":[],"local_overrides":[]}"#
            )
        );
    }

    #[test]
    fn a_verdict_keeps_the_provenance_of_every_signal_in_the_json() {
        // Flattening the libraries into one array and the sources into another
        // would let a consumer lose the pairing, and the pairing is the point:
        // an import and a delay-import are not the same fact.
        let evidence = Evidence {
            imports: Vec::new(),
            delay_imports: Vec::new(),
            neighbours: vec!["nvngx_dlss.dll".to_owned(), "winmm.dll".to_owned()],
            ..Evidence::default()
        };
        let mut record = sample();
        record.verdict = analyse(&evidence);
        let json = record.to_json();

        assert!(
            json.contains(
                r#""features":[{"name":"DLSS Super Resolution","via":[{"library":"nvngx_dlss.dll","source":"neighbour","version":{"state":"unread","file":null,"product":null}}]}]"#
            ),
            "a shipped DLSS file keeps its neighbour provenance, got {json}"
        );
        assert!(
            json.contains(
                r#""local_overrides":[{"name":"local copy of a system DLL","via":[{"library":"winmm.dll","source":"neighbour","version":{"state":"unread","file":null,"product":null}}]}]"#
            ),
            "the proxy DLL is reported apart from the renderers, got {json}"
        );
    }

    #[test]
    fn a_missing_version_resource_is_null_rather_than_an_error() {
        // Plenty of shipped executables carry no version resource at all.
        // Reporting that as a failure would make a clean scan look broken.
        let mut record = sample();
        record.file_version = None;
        record.product_version = None;
        let json = record.to_json();

        assert!(
            json.contains(r#""file_version":null,"product_version":null,"error":null"#),
            "versions must be null while error stays null, got {json}"
        );
    }

    #[test]
    fn a_json_line_is_a_single_line_with_no_pretty_printing() {
        // The output format is JSONL: a newline inside a record would split one
        // result into two rows and desynchronise every row after it.
        let json = sample().to_json();

        assert!(!json.contains('\n'), "record must not contain a newline");
        assert!(!json.contains(", "), "no separator padding, got {json}");
    }

    #[test]
    fn backslashes_and_quotes_in_a_path_are_escaped() {
        // Windows paths arrive with backslashes and the parser reports names
        // straight out of the file, so unescaped output is a matter of when,
        // not if.
        let mut record = sample();
        record.path = r#"C:\Games\my "game"\bin\app.exe"#.to_owned();
        record.imports = vec!["odd\tname.dll".to_owned()];

        assert!(
            record
                .to_json()
                .contains(r#""path":"C:\\Games\\my \"game\"\\bin\\app.exe""#),
            "path escaping, got {}",
            record.to_json()
        );
        assert!(
            record.to_json().contains(r#"["odd\tname.dll"]"#),
            "control characters escaping, got {}",
            record.to_json()
        );
    }

    #[test]
    fn an_empty_import_table_is_an_empty_array_not_a_null() {
        // An array that sometimes becomes null forces every consumer to write a
        // type check it should not need.
        let mut record = sample();
        record.imports.clear();
        record.delay_imports.clear();

        assert!(
            record
                .to_json()
                .contains(r#""imports":[],"delay_imports":[]"#),
            "empty tables stay arrays, got {}",
            record.to_json()
        );
    }
}
