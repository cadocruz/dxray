//! The human-readable report.
//!
//! The judgement is `dxray-core`'s; this module only lays it out. Nothing here
//! decides what a library means, and the raw import lists stay below the
//! verdict so a reader who disagrees with it can check the evidence without
//! re-running anything.

use std::fmt::Write as _;

use dxray_core::Source;
use dxray_core::analysis::{Finding, is_graphics_related};

use crate::record::Record;

/// Width of the label column, chosen so the longest label plus one space fits.
const LABEL: usize = 10;
/// Where a wrapped value continues, lined up under the first one.
const INDENT: usize = 2 + LABEL;
/// Wrap width. Terminals narrower than this exist; output that is impossible to
/// scan on a normal one because it was built for them does not.
const WIDTH: usize = 80;

/// Renders one record as an indented block, ending in a blank line.
pub fn render(record: &Record) -> String {
    let mut out = String::with_capacity(512);
    let _ = writeln!(out, "{}", record.path);

    if let Some(error) = &record.error {
        row(&mut out, "error", error);
        out.push('\n');
        return out;
    }

    let machine = record.machine.as_deref().unwrap_or("unknown");
    match record.bits {
        Some(bits) => row(&mut out, "machine", &format!("{machine} ({bits}-bit)")),
        None => row(&mut out, "machine", machine),
    }
    row(&mut out, "version", &versions(record));

    // The verdict first, then the findings that produced it, then the raw
    // tables. A reader who trusts the tool stops at the first line; one who
    // does not can walk down to the evidence without leaving the block.
    let verdict = &record.verdict;
    row(&mut out, "verdict", &verdict.headline());
    findings(&mut out, "renderer", &verdict.renderers);
    findings(&mut out, "shared", &verdict.infrastructure);
    findings(&mut out, "feature", &verdict.features);
    findings(&mut out, "override", &verdict.local_overrides);

    // Delay-loaded entries are marked rather than listed apart: what matters to
    // a reader is that the dependency exists, and the fact that it resolves
    // late is a property of the dependency, not a second list.
    let mut graphics = Vec::new();
    let mut other = Vec::new();
    for name in &record.imports {
        pick(&mut graphics, &mut other, name, name.clone());
    }
    for name in &record.delay_imports {
        // `Source::as_str` and not a literal: the findings rows eight lines
        // above spell this fact through `Signal::describe`, and one record
        // carrying two spellings of one fact is the defect named below.
        let marked = format!("{name} ({})", Source::DelayImport.as_str());
        pick(&mut graphics, &mut other, name, marked);
    }

    if graphics.is_empty() && other.is_empty() {
        row(&mut out, "imports", "none");
    } else {
        if !graphics.is_empty() {
            row(&mut out, "graphics", &wrap(&graphics));
        }
        if !other.is_empty() {
            row(&mut out, "other", &wrap(&other));
        }
    }

    out.push('\n');
    out
}

/// A one-line trailer, emitted only when more than one thing was looked at —
/// for a single one it would just restate the block above it.
///
/// `noun` because `--game` counts install directories rather than files, and a
/// trailer that calls them files is a small lie in the line that gets quoted.
pub fn summary(total: usize, failed: usize, noun: &str) -> Option<String> {
    if total < 2 {
        return None;
    }
    Some(match failed {
        0 => format!("{total} {noun}, none failed"),
        _ => format!("{total} {noun}, {failed} failed"),
    })
}

/// One row per finding, each naming the libraries it was drawn from.
///
/// The provenance is printed rather than summarised because it is the part a
/// reader argues with: "Direct3D 11" is a conclusion, while "d3d11.dll
/// (delay-import)" is the observation, and only one of the two can be checked
/// against the file.
fn findings(out: &mut String, label: &str, findings: &[Finding]) {
    for finding in findings {
        let mut parts = Vec::with_capacity(finding.signals.len() + 1);
        parts.push(finding.name.clone());
        // `Signal::describe` and not a format string here: the TUI prints the
        // same line, and a version that appears in one surface and not the
        // other is the exact defect this project has already fixed twice.
        parts.extend(finding.signals.iter().map(dxray_core::Signal::describe));
        row(out, label, &wrap(&parts));
    }
}

fn versions(record: &Record) -> String {
    match (&record.file_version, &record.product_version) {
        (None, None) => "none".to_owned(),
        (file, product) => {
            let file = file.as_deref().unwrap_or("none");
            let product = product.as_deref().unwrap_or("none");
            format!("file {file}  product {product}")
        }
    }
}

/// Files a name under the row it belongs in, asking `dxray-core` which one.
///
/// `name` is the library as the import table spelled it; `text` is what the row
/// prints, which for a delay-loaded entry carries its marker. The question is
/// `dxray-core`'s because the verdict printed above these rows answers it too,
/// and a private table here is how a name got reported as a feature and as
/// non-graphical in the same block.
fn pick(graphics: &mut Vec<String>, other: &mut Vec<String>, name: &str, text: String) {
    if is_graphics_related(name) {
        graphics.push(text);
    } else {
        other.push(text);
    }
}

fn row(out: &mut String, label: &str, value: &str) {
    let _ = writeln!(out, "  {label:<LABEL$}{value}");
}

/// Joins names into wrapped lines, each continuation lined up under the first.
///
/// A name longer than the whole width goes on a line of its own and overflows,
/// because splitting a DLL name in half makes it unsearchable.
fn wrap(names: &[String]) -> String {
    let mut out = String::new();
    let mut column = INDENT;
    for (i, name) in names.iter().enumerate() {
        if i > 0 {
            if column + 2 + name.len() > WIDTH {
                let _ = write!(out, "\n{:INDENT$}", "");
                column = INDENT;
            } else {
                out.push_str("  ");
                column += 2;
            }
        }
        out.push_str(name);
        column += name.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{render, summary};
    use crate::record::Record;
    use dxray_core::{Evidence, analyse};

    fn sample() -> Record {
        let evidence = Evidence {
            imports: vec!["KERNEL32.dll".to_owned(), "DXGI.dll".to_owned()],
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
    fn the_graphics_libraries_are_separated_from_the_rest() {
        // The whole point of the human report: a renderer named among sixty
        // api-set stubs is a fact nobody reads.
        let text = render(&sample());

        assert!(
            text.contains("graphics  DXGI.dll  d3d12.dll (delay-import)"),
            "graphics row, got:\n{text}"
        );
        assert!(
            text.contains("other     KERNEL32.dll"),
            "other row, got:\n{text}"
        );
    }

    #[test]
    fn the_verdict_leads_the_block_and_the_evidence_follows_it() {
        // The conclusion is what a reader came for; the libraries behind it are
        // what makes the conclusion checkable rather than something to believe.
        let text = render(&sample());

        assert!(text.contains("verdict   Direct3D 12"), "got:\n{text}");
        assert!(
            text.contains("renderer  Direct3D 12  d3d12.dll (delay-import)"),
            "the renderer row names its library and how it was seen, got:\n{text}"
        );
        assert!(
            text.contains("shared    DXGI  DXGI.dll (import)"),
            "dxgi is reported apart from the renderers, got:\n{text}"
        );
    }

    #[test]
    fn a_binary_that_can_reach_two_renderers_says_so_on_the_verdict_line() {
        // Printing one of them would read as a finding. The binary links both
        // and decides at run time, and the line has to survive that.
        let mut record = sample();
        record.imports = vec!["d3d11.dll".to_owned(), "d3d12.dll".to_owned()];
        record.delay_imports.clear();
        record.verdict = analyse(&Evidence {
            imports: record.imports.clone(),
            ..Evidence::default()
        });
        let text = render(&record);

        assert!(
            text.contains(
                "verdict   Direct3D 12 or Direct3D 11 (all linked; selected at run time)"
            ),
            "got:\n{text}"
        );
    }

    #[test]
    fn a_proxy_dll_in_the_directory_gets_its_own_row_rather_than_a_renderer_one() {
        // ReShade installed as dxgi.dll is a fact about the install, not about
        // the binary's renderer, and the report must not let the two blur.
        let mut record = sample();
        record.verdict = analyse(&Evidence {
            neighbours: vec!["dxgi.dll".to_owned()],
            ..Evidence::default()
        });
        let text = render(&record);

        assert!(
            text.contains("override  local copy of a system DLL  dxgi.dll (neighbour)"),
            "got:\n{text}"
        );
        assert!(
            !text.contains("renderer"),
            "a neighbouring file is never a renderer signal, got:\n{text}"
        );
    }

    #[test]
    fn a_findings_row_carries_the_version_of_the_file_behind_it() {
        // The whole point of stamping. "nvngx_dlss.dll (neighbour)" names a
        // file that has been called that in every version ever shipped, which
        // is not an answer to the question anyone has in front of a game
        // directory. The number is, and this is the row it has to land in.
        use dxray_core::{FileVersion, Signal, Source, Version, VersionInfo};

        let mut record = sample();
        record.verdict = analyse(&Evidence {
            neighbours: vec!["nvngx_dlss.dll".to_owned()],
            ..Evidence::default()
        });
        record.verdict.features[0].signals = vec![Signal {
            library: "nvngx_dlss.dll".to_owned(),
            source: Source::Neighbour,
            version: FileVersion::Stamped(VersionInfo {
                file: Version {
                    major: 310,
                    minor: 2,
                    patch: 1,
                    build: 0,
                },
                product: Version {
                    major: 310,
                    minor: 2,
                    patch: 1,
                    build: 0,
                },
            }),
        }];
        let text = render(&record);

        assert!(
            text.contains("feature   DLSS Super Resolution  nvngx_dlss.dll (neighbour, 310.2.1.0)"),
            "got:\n{text}"
        );
    }

    #[test]
    fn a_delay_loaded_dependency_is_marked_rather_than_hidden() {
        // A game that delay-loads its renderer imports nothing at startup, and
        // a report that drops the mark reads as though it links eagerly.
        let text = render(&sample());

        assert!(
            text.contains("d3d12.dll (delay-import)"),
            "delay marker, got:\n{text}"
        );
    }

    #[test]
    fn the_import_rows_spell_a_delay_load_the_way_the_findings_rows_do() {
        // One record, one fact. The findings row above says "delay-import"
        // through `Signal::describe`; an import row saying "(delayed)" eight
        // lines below it is the same fact in a second spelling, which is the
        // defect this project has already fixed twice elsewhere.
        let text = render(&sample());

        assert!(
            !text.contains("(delayed)"),
            "a second spelling of delay-import, got:
{text}"
        );
        assert!(
            text.contains("renderer  Direct3D 12  d3d12.dll (delay-import)")
                && text.contains("graphics  DXGI.dll  d3d12.dll (delay-import)"),
            "both rows quote the same library the same way, got:
{text}"
        );
    }

    #[test]
    fn a_library_the_verdict_names_is_never_filed_as_one_of_the_uninteresting_ones() {
        // The shape a mod loader installs: the game imports `dinput8.dll` and
        // ships a local copy of it. The verdict calls that an override; the
        // import rows used to file the same name under `other`, because the CLI
        // kept a prefix table of its own that had never heard of it.
        let mut record = sample();
        record.imports = vec![
            "KERNEL32.dll".to_owned(),
            "dinput8.dll".to_owned(),
            "amd_fidelityfx_dx12.dll".to_owned(),
        ];
        record.delay_imports.clear();
        record.verdict = analyse(&Evidence {
            imports: record.imports.clone(),
            neighbours: vec!["dinput8.dll".to_owned()],
            ..Evidence::default()
        });
        let text = render(&record);

        assert!(
            text.contains("override  local copy of a system DLL  dinput8.dll (neighbour)"),
            "the verdict reports the override, got:
{text}"
        );
        assert!(
            text.contains("feature   FSR  amd_fidelityfx_dx12.dll (import)"),
            "and reports the feature, got:
{text}"
        );
        assert!(
            text.contains(
                "other     KERNEL32.dll
"
            ),
            "only the api-set crowd is filed under other, got:
{text}"
        );
        let row = text
            .lines()
            .find(|line| line.trim_start().starts_with("graphics"))
            .unwrap_or_default();
        for named in ["dinput8.dll", "amd_fidelityfx_dx12.dll"] {
            assert!(
                row.contains(named),
                "{named} belongs in the row the verdict put it in, got:
{text}"
            );
        }
    }

    #[test]
    fn a_failed_file_prints_its_error_and_nothing_it_does_not_know() {
        // Printing "machine unknown" for a file that was never parsed would
        // read as a finding about the file rather than about the scan.
        let path = std::path::Path::new("/mnt/c/notes.txt");
        let text = render(&Record::failed(
            path,
            &"not a PE image: missing PE signature",
        ));

        assert!(text.contains("error     not a PE image"), "got:\n{text}");
        assert!(!text.contains("machine"), "got:\n{text}");
        assert!(!text.contains("imports"), "got:\n{text}");
    }

    #[test]
    fn an_image_with_no_version_resource_says_none_instead_of_looking_broken() {
        let mut record = sample();
        record.file_version = None;
        record.product_version = None;

        assert!(
            render(&record).contains("version   none"),
            "got:\n{}",
            render(&record)
        );
    }

    #[test]
    fn an_image_that_imports_nothing_says_so_rather_than_printing_an_empty_row() {
        let mut record = sample();
        record.imports.clear();
        record.delay_imports.clear();

        assert!(
            render(&record).contains("imports   none"),
            "got:\n{}",
            render(&record)
        );
    }

    #[test]
    fn long_import_lists_wrap_under_the_column_they_started_in() {
        // Unwrapped output is what turns a directory scan into something that
        // has to be piped through `less -S` to be read at all.
        let mut record = sample();
        record.imports = (0..12)
            .map(|i| format!("api-ms-win-core-thing-l1-1-{i}.dll"))
            .collect();
        let text = render(&record);

        assert!(
            text.lines().all(|l| l.len() <= 80),
            "no line may run past the wrap width, got:\n{text}"
        );
        assert!(
            text.contains("\n            api-ms-win-core"),
            "continuations line up under the first name, got:\n{text}"
        );
    }

    #[test]
    fn the_trailer_appears_only_when_there_was_more_than_one_file() {
        // For a single file it would just restate the block above it.
        assert_eq!(summary(1, 0, "files"), None);
        assert_eq!(
            summary(3, 0, "files").as_deref(),
            Some("3 files, none failed")
        );
        assert_eq!(summary(3, 2, "files").as_deref(), Some("3 files, 2 failed"));
        assert_eq!(
            summary(2, 0, "paths").as_deref(),
            Some("2 paths, none failed"),
            "the noun travels with the count"
        );
    }
}
