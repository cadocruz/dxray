//! End to end tests: the real binary, real files, real exit codes. JSON is
//! checked as text, since the promise is its byte shape.

mod common;

use common::{Image, TempDir, dxray, stdout_of};

/// The thirteen keys, in order. The first eight are frozen: harnesses read
/// them positionally.
const KEYS: [&str; 13] = [
    "\"path\":",
    "\"machine\":",
    "\"bits\":",
    "\"imports\":",
    "\"delay_imports\":",
    "\"file_version\":",
    "\"product_version\":",
    "\"error\":",
    "\"verdict\":",
    "\"renderers\":",
    "\"infrastructure\":",
    "\"features\":",
    "\"local_overrides\":",
];

/// Asserts that `line` holds all thirteen keys once each, in the promised
/// order.
fn assert_shape(line: &str) {
    let mut at = 0;
    for key in KEYS {
        let found = line[at..]
            .find(key)
            .unwrap_or_else(|| panic!("missing {key} after offset {at} in {line}"));
        at += found + key.len();
    }
    assert!(
        line.starts_with('{') && line.ends_with('}'),
        "a record is one object, got {line}"
    );
}

#[test]
fn a_parsed_image_is_reported_as_one_json_line_with_the_promised_keys() {
    let dir = TempDir::new("one");
    let path = dir.write(
        "game.exe",
        &Image::x64()
            .importing(&["KERNEL32.dll", "dxgi.dll"])
            .delay_loading(&["d3d12.dll"])
            .build(),
    );

    let out = dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        path.as_os_str(),
    ]);
    let text = stdout_of(&out);

    // Compared whole: the contract is the exact line, and asserting on parsed
    // fields would pass just as happily with the keys in another order.
    assert_eq!(
        text,
        format!(
            "{{\"path\":\"{}\",\"machine\":\"x86-64\",\"bits\":64,\
             \"imports\":[\"KERNEL32.dll\",\"dxgi.dll\"],\
             \"delay_imports\":[\"d3d12.dll\"],\
             \"file_version\":null,\"product_version\":null,\"error\":null,\
             \"verdict\":\"Direct3D 12\",\
             \"renderers\":[{{\"name\":\"Direct3D 12\",\
             \"via\":[{{\"library\":\"d3d12.dll\",\"source\":\"delay-import\",\
             \"version\":{{\"state\":\"elsewhere\",\"file\":null,\"product\":null}}}}]}}],\
             \"infrastructure\":[{{\"name\":\"DXGI\",\
             \"via\":[{{\"library\":\"dxgi.dll\",\"source\":\"import\",\
             \"version\":{{\"state\":\"elsewhere\",\"file\":null,\"product\":null}}}}]}}],\
             \"features\":[],\"local_overrides\":[]}}\n",
            path.display()
        )
    );
    assert_eq!(out.status.code(), Some(0), "a clean scan exits 0");
}

#[test]
fn a_binary_that_links_two_renderers_reports_both_and_names_its_proxy_dll() {
    // A set of renderers, and a system DLL file read as an injector.
    let dir = TempDir::new("verdict");
    let path = dir.write(
        "game.exe",
        &Image::x64()
            .importing(&["KERNEL32.dll", "d3d11.dll", "d3d12.dll"])
            .build(),
    );
    // Not a PE image, and it does not need to be: what makes it an injector is
    // the name and the directory, which is exactly the point.
    dir.write("dxgi.dll", b"ReShade would go here");

    let text = stdout_of(&dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        path.as_os_str(),
    ]))
    .to_owned();

    assert!(
        text.contains(
            r#""verdict":"Direct3D 12 or Direct3D 11 (all linked; selected at run time)""#
        ),
        "both renderers survive to the output, got {text}"
    );
    assert!(
        text.contains(
            r#""local_overrides":[{"name":"local copy of a system DLL","via":[{"library":"dxgi.dll","source":"neighbour","version":{"state":"unreadable","file":null,"product":null}}]}]"#
        ),
        "the proxy DLL is reported as one, got {text}"
    );
    // A non-PE file under a system name says it could not be read.
    assert!(
        text.contains(r#""infrastructure":[]"#),
        "a dxgi file is not a dxgi import, got {text}"
    );
    assert_shape(text.trim_end());
}

#[test]
fn a_32_bit_image_reports_32_bits_rather_than_defaulting() {
    // PE32 and PE32+ differ only by a magic and a 16-byte shift; reading the
    // wrong layout yields a plausible-looking record, not an error.
    let dir = TempDir::new("bits");
    let path = dir.write("old.exe", &Image::x86().importing(&["d3d9.dll"]).build());

    let text = stdout_of(&dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        path.as_os_str(),
    ]))
    .to_owned();

    assert!(
        text.contains("\"machine\":\"x86\",\"bits\":32"),
        "got {text}"
    );
}

#[test]
fn a_file_that_cannot_be_parsed_does_not_stop_the_ones_after_it() {
    // A file that is not a PE does not stop the run.
    let dir = TempDir::new("continue");
    let first = dir.write("a.dll", &Image::x64().importing(&["dxgi.dll"]).build());
    let broken = dir.write("b.dll", b"this is not a PE image at all");
    let last = dir.write("c.dll", &Image::x64().importing(&["vulkan-1.dll"]).build());

    let out = dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        first.as_os_str(),
        broken.as_os_str(),
        last.as_os_str(),
    ]);
    let lines: Vec<&str> = stdout_of(&out).lines().collect();

    assert_eq!(lines.len(), 3, "one line per input, got {lines:?}");
    for line in &lines {
        assert_shape(line);
    }
    assert!(lines[0].contains("\"error\":null"), "got {}", lines[0]);
    assert!(
        lines[1].contains("\"error\":\"not a DOS image: missing MZ\""),
        "the failure names what went wrong, got {}",
        lines[1]
    );
    assert!(
        lines[2].contains("\"vulkan-1.dll\""),
        "work continued past the failure, got {}",
        lines[2]
    );
    assert_eq!(out.status.code(), Some(1), "any failure exits 1");
}

#[test]
fn a_failed_record_keeps_every_key_with_the_unknown_ones_emptied() {
    // A consumer should be able to index by key without branching on success,
    // so nothing may be omitted and no array may collapse to null.
    let dir = TempDir::new("failed");
    let path = dir.write("notes.txt", b"plain text");

    let out = dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        path.as_os_str(),
    ]);
    let line = stdout_of(&out).trim_end();

    assert_shape(line);
    assert!(
        line.contains("\"machine\":null,\"bits\":null,\"imports\":[],\"delay_imports\":[]"),
        "got {line}"
    );
    assert!(
        line.contains("\"file_version\":null,\"product_version\":null,\"error\":\""),
        "got {line}"
    );
}

#[test]
fn a_path_that_is_not_there_is_reported_instead_of_aborting_the_run() {
    // Argument lists get pasted together from scripts; one stale entry must not
    // cost the results of every other entry.
    let dir = TempDir::new("missing");
    let good = dir.write("a.dll", &Image::x64().importing(&["dxgi.dll"]).build());
    let gone = dir.path().join("not-here.dll");

    let out = dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        gone.as_os_str(),
        good.as_os_str(),
    ]);
    let lines: Vec<&str> = stdout_of(&out).lines().collect();

    assert_eq!(lines.len(), 2, "got {lines:?}");
    assert!(
        !lines[0].contains("\"error\":null"),
        "the missing path is the failing row, got {}",
        lines[0]
    );
    assert!(lines[1].contains("\"error\":null"), "got {}", lines[1]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn a_directory_yields_its_images_and_leaves_the_rest_alone() {
    // A game directory is mostly assets. Parsing them all would bury the real
    // findings under errors about textures.
    let dir = TempDir::new("scan");
    let image = Image::x64().importing(&["dxgi.dll"]).build();
    dir.write("b.dll", &image);
    dir.write("a.exe", &image);
    dir.write("readme.txt", b"hello");
    dir.write("sub/deep.dll", &image);

    let out = dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        dir.path().as_os_str(),
    ]);
    let lines: Vec<&str> = stdout_of(&out).lines().collect();

    assert_eq!(lines.len(), 2, "no recursion by default, got {lines:?}");
    // Sorted, so two runs of the same scan can be diffed against each other.
    assert!(lines[0].contains("a.exe"), "got {}", lines[0]);
    assert!(lines[1].contains("b.dll"), "got {}", lines[1]);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn recursive_scanning_reaches_the_subdirectories_a_plain_scan_skips() {
    let dir = TempDir::new("recursive");
    let image = Image::x64().importing(&["dxgi.dll"]).build();
    dir.write("a.exe", &image);
    dir.write("sub/deep.dll", &image);

    let out = dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        std::ffi::OsStr::new("--recursive"),
        dir.path().as_os_str(),
    ]);
    let lines: Vec<&str> = stdout_of(&out).lines().collect();

    assert_eq!(lines.len(), 2, "got {lines:?}");
    assert!(
        lines.iter().any(|l| l.contains("deep.dll")),
        "got {lines:?}"
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn a_named_file_is_inspected_whatever_its_extension() {
    // The extension filter exists to make directory scanning useful, not to
    // second-guess a path the caller typed out.
    let dir = TempDir::new("named");
    let path = dir.write(
        "renderer.bin",
        &Image::x64().importing(&["d3d12.dll"]).build(),
    );

    let out = dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        path.as_os_str(),
    ]);

    assert!(
        stdout_of(&out).contains("\"d3d12.dll\""),
        "got {}",
        stdout_of(&out)
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn the_human_report_puts_the_graphics_libraries_where_they_can_be_seen() {
    let dir = TempDir::new("human");
    let path = dir.write(
        "game.exe",
        &Image::x64()
            .importing(&["KERNEL32.dll", "dxgi.dll"])
            .delay_loading(&["d3d12.dll"])
            .build(),
    );

    let out = dxray([std::ffi::OsStr::new("inspect"), path.as_os_str()]);
    let text = stdout_of(&out);

    assert!(text.contains(&path.display().to_string()), "got:\n{text}");
    assert!(text.contains("machine   x86-64 (64-bit)"), "got:\n{text}");
    assert!(
        text.contains("graphics  dxgi.dll  d3d12.dll (delay-import)"),
        "got:\n{text}"
    );
    assert!(text.contains("other     KERNEL32.dll"), "got:\n{text}");
    assert!(
        !text.contains("files,"),
        "a single file gets no trailer, got:\n{text}"
    );
}

#[test]
fn the_human_report_counts_what_failed_when_several_files_were_looked_at() {
    let dir = TempDir::new("trailer");
    let good = dir.write("a.dll", &Image::x64().importing(&["dxgi.dll"]).build());
    let bad = dir.write("b.dll", b"nope");

    let out = dxray([
        std::ffi::OsStr::new("inspect"),
        good.as_os_str(),
        bad.as_os_str(),
    ]);
    let text = stdout_of(&out);

    assert!(text.contains("2 files, 1 failed"), "got:\n{text}");
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn json_output_carries_no_header_footer_or_blank_line() {
    // JSONL is read a line at a time; anything that is not a record breaks the
    // reader on the line it appears.
    let dir = TempDir::new("jsonl");
    let image = Image::x64().importing(&["dxgi.dll"]).build();
    let a = dir.write("a.dll", &image);
    let b = dir.write("b.dll", &image);

    let text = stdout_of(&dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        a.as_os_str(),
        b.as_os_str(),
    ]))
    .to_owned();

    assert_eq!(text.lines().count(), 2, "got:\n{text}");
    assert!(
        text.lines().all(|l| l.starts_with('{') && l.ends_with('}')),
        "every line is a record, got:\n{text}"
    );
    assert!(
        text.ends_with("}\n"),
        "no trailing blank line, got:\n{text}"
    );
}

#[test]
fn a_usage_error_exits_2_and_is_told_apart_from_a_file_that_failed() {
    // 0 every file parsed, 1 some did not, 2 bad command line. A mistyped flag
    // must not look like a corrupt binary.
    let out = dxray(Vec::<String>::new());

    assert_eq!(out.status.code(), Some(2), "no arguments is a usage error");
}

#[test]
fn a_shipped_dlss_runtime_is_reported_with_the_build_it_is() {
    // DLSS rows carry the version: the file name is always the same.
    let dir = TempDir::new("dlss");
    let path = dir.write(
        "game.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );
    dir.write(
        "nvngx_dlss.dll",
        &Image::x64()
            .versioned([310, 2, 1, 0], [310, 2, 1, 0])
            .build(),
    );

    let human = stdout_of(&dxray([std::ffi::OsStr::new("inspect"), path.as_os_str()])).to_owned();
    assert!(
        human.contains("feature   DLSS Super Resolution  nvngx_dlss.dll (neighbour, 310.2.1.0)"),
        "the human report names the build, got:\n{human}"
    );

    let json = stdout_of(&dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        path.as_os_str(),
    ]))
    .to_owned();
    assert!(
        json.contains(
            r#""features":[{"name":"DLSS Super Resolution","via":[{"library":"nvngx_dlss.dll","source":"neighbour","version":{"state":"stamped","file":"310.2.1.0","product":"310.2.1.0"}}]}]"#
        ),
        "and the JSON carries both numbers where they can be compared, got {json}"
    );
    assert_shape(json.trim_end());
}

#[test]
fn the_dlss_5_runtime_reaches_the_printed_report_like_any_other_library() {
    // DLSS 5 is reported through the real binary. The version is not proof the
    // file is NVIDIA's: patched variants carry it too.
    let dir = TempDir::new("dlssnr");
    let path = dir.write(
        "game.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );
    dir.write(
        "nvngx_dlssnr.dll",
        &Image::x64()
            .versioned([310, 8, 0, 0], [310, 8, 0, 0])
            .build(),
    );

    let human = stdout_of(&dxray([std::ffi::OsStr::new("inspect"), path.as_os_str()])).to_owned();
    assert!(
        human.contains("feature   DLSS Neural Rendering  nvngx_dlssnr.dll (neighbour, 310.8.0.0)"),
        "the human report names the feature, the file and its build, got:\n{human}"
    );

    let json = stdout_of(&dxray([
        std::ffi::OsStr::new("inspect"),
        std::ffi::OsStr::new("--json"),
        path.as_os_str(),
    ]))
    .to_owned();
    assert!(
        json.contains(
            r#""features":[{"name":"DLSS Neural Rendering","via":[{"library":"nvngx_dlssnr.dll","source":"neighbour","version":{"state":"stamped","file":"310.8.0.0","product":"310.8.0.0"}}]}]"#
        ),
        "and the JSON says the same thing in its own shape, got {json}"
    );
    assert_shape(json.trim_end());
}

#[test]
fn a_version_resource_of_all_zeroes_prints_as_a_number_rather_than_vanishing() {
    // An all-zero version resource is printed as the number it is.
    let dir = TempDir::new("zeroes");
    let path = dir.write(
        "game.exe",
        &Image::x64().importing(&["KERNEL32.dll"]).build(),
    );
    dir.write(
        "nvngx_dlssg.dll",
        &Image::x64().versioned([0, 0, 0, 0], [0, 0, 0, 0]).build(),
    );

    let human = stdout_of(&dxray([std::ffi::OsStr::new("inspect"), path.as_os_str()])).to_owned();

    assert!(
        human.contains("nvngx_dlssg.dll (neighbour, 0.0.0.0)"),
        "all zeroes is a version and prints as one, got:\n{human}"
    );
}

#[test]
fn a_local_library_with_no_version_resource_says_so_rather_than_staying_silent() {
    // "No version" and "no local copy" are both printed.
    let dir = TempDir::new("unstamped");
    let path = dir.write(
        "game.exe",
        &Image::x64()
            .importing(&["KERNEL32.dll", "d3d12.dll"])
            .build(),
    );
    dir.write("nvngx_dlss.dll", &Image::x64().build());

    let human = stdout_of(&dxray([std::ffi::OsStr::new("inspect"), path.as_os_str()])).to_owned();

    assert!(
        human.contains("nvngx_dlss.dll (neighbour, no version)"),
        "the shipped file says it carries no version, got:\n{human}"
    );
    assert!(
        human.contains("renderer  Direct3D 12  d3d12.dll (import, no local copy)"),
        "and a name with no file behind it says so rather than going blank, \
         got:\n{human}"
    );
}
