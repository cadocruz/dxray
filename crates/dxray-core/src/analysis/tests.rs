//! The awkward cases, which are the only ones worth a test.
//!
//! Every case here is one where the obvious implementation produces an answer
//! that is wrong about a real, shipped binary.

use super::{Evidence, LOCAL_OVERRIDE, Source, Verdict, analyse};

/// Evidence from import names only, which is the common shape.
fn importing(names: &[&str]) -> Evidence {
    Evidence {
        imports: names.iter().map(|s| (*s).to_owned()).collect(),
        ..Evidence::default()
    }
}

fn names(findings: &[super::Finding]) -> Vec<&str> {
    findings.iter().map(|f| f.name.as_str()).collect()
}

/// Every library quoted in support of `name`, as `"library (source)"`.
fn evidence_for(findings: &[super::Finding], name: &str) -> Vec<String> {
    findings
        .iter()
        .find(|f| f.name == name)
        .map(|f| {
            f.signals
                .iter()
                .map(|s| format!("{} ({})", s.library, s.source.as_str()))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_binary_linking_both_direct3d_11_and_12_reports_both() {
    // The case that makes a verdict a set. Shipped games link both and ask the
    // driver at run time which one to use; naming one would be a confident lie
    // about a choice this tool cannot observe.
    let verdict = analyse(&importing(&["KERNEL32.dll", "d3d11.dll", "d3d12.dll"]));

    assert_eq!(
        names(&verdict.renderers),
        ["Direct3D 12", "Direct3D 11"],
        "both renderers survive, newest first"
    );
    assert_eq!(
        verdict.headline(),
        "Direct3D 12 or Direct3D 11 (all linked; selected at run time)",
        "the headline must not collapse a set into one answer"
    );
}

#[test]
fn dxgi_on_its_own_is_infrastructure_and_determines_no_api() {
    // The classic mistake: `dxgi.dll` is the swapchain layer shared by D3D10,
    // 11 and 12. Reporting it as "DirectX" invents a generation that the import
    // table does not name.
    let verdict = analyse(&importing(&["KERNEL32.dll", "DXGI.dll"]));

    assert!(
        verdict.renderers.is_empty(),
        "dxgi is not a renderer, got {:?}",
        names(&verdict.renderers)
    );
    assert_eq!(names(&verdict.infrastructure), ["DXGI"]);
    assert_eq!(
        verdict.headline(),
        "DXGI present, specific API not determined"
    );
}

#[test]
fn dxgi_alongside_a_renderer_stays_infrastructure_rather_than_a_second_answer() {
    // Nearly every D3D11 and D3D12 binary imports dxgi too. Letting it into the
    // renderer set would make the "several renderers" case fire on almost every
    // file and drain that signal of meaning.
    let verdict = analyse(&importing(&["dxgi.dll", "d3d12.dll"]));

    assert_eq!(names(&verdict.renderers), ["Direct3D 12"]);
    assert_eq!(names(&verdict.infrastructure), ["DXGI"]);
    assert_eq!(
        verdict.headline(),
        "Direct3D 12",
        "one renderer, so no run-time-choice qualifier"
    );
}

#[test]
fn a_renderer_reached_only_by_delay_load_is_reported_and_marked_as_such() {
    // A game that delay-loads its renderer imports nothing graphical at start.
    // Dropping delay-load evidence reports it as having no renderer at all;
    // merging it with imports claims the process always loads it, and it may
    // never take that path. Both facts have to survive.
    let evidence = Evidence {
        imports: vec!["KERNEL32.dll".to_owned()],
        delay_imports: vec!["d3d12.dll".to_owned()],
        ..Evidence::default()
    };
    let verdict = analyse(&evidence);

    assert_eq!(names(&verdict.renderers), ["Direct3D 12"]);
    assert_eq!(
        verdict.renderers[0].strength(),
        Source::DelayImport,
        "provenance must say the dependency resolves late"
    );
}

#[test]
fn a_load_time_import_outranks_a_delay_loaded_one_whatever_the_generation() {
    // Evidence strength comes before generation order. A renderer the loader
    // must resolve is a stronger statement than one behind a code path that may
    // never run, even when the delayed one is newer.
    let evidence = Evidence {
        imports: vec!["d3d9.dll".to_owned()],
        delay_imports: vec!["d3d12.dll".to_owned()],
        ..Evidence::default()
    };
    let verdict = analyse(&evidence);

    assert_eq!(
        names(&verdict.renderers),
        ["Direct3D 9", "Direct3D 12"],
        "strength first, generation only as a tie-break"
    );
}

#[test]
fn an_injector_dll_next_to_the_executable_is_not_a_renderer_signal() {
    // The exact shape ReShade, SpecialK and OptiScaler install in: a file named
    // after a system DLL, while the import table never mentions it. Windows
    // resolves the local copy first, so this is a proxy, and reading it as a
    // renderer would report an API the binary never asked for.
    let evidence = Evidence {
        imports: vec!["KERNEL32.dll".to_owned(), "vulkan-1.dll".to_owned()],
        delay_imports: Vec::new(),
        neighbours: vec![
            "game.exe".to_owned(),
            "dxgi.dll".to_owned(),
            "ReShade.ini".to_owned(),
        ],
        ..Evidence::default()
    };
    let verdict = analyse(&evidence);

    assert_eq!(
        names(&verdict.renderers),
        ["Vulkan"],
        "only the import table decides the renderer"
    );
    assert!(
        verdict.infrastructure.is_empty(),
        "a dxgi *file* is not the same fact as a dxgi import"
    );
    assert_eq!(names(&verdict.local_overrides), [LOCAL_OVERRIDE]);
    assert_eq!(
        evidence_for(&verdict.local_overrides, LOCAL_OVERRIDE),
        ["dxgi.dll (neighbour)"],
        "the override names the DLL it shadows"
    );
}

#[test]
fn several_proxy_dlls_in_one_directory_are_one_finding_with_every_file_quoted() {
    // Mod stacks pile up: ReShade as dxgi.dll, a mod loader as version.dll, an
    // input wrapper as dinput8.dll. One finding keeps the report readable while
    // the signal list keeps every file recoverable.
    let evidence = Evidence {
        neighbours: vec![
            "winmm.dll".to_owned(),
            "dinput8.dll".to_owned(),
            "version.dll".to_owned(),
        ],
        ..Evidence::default()
    };
    let verdict = analyse(&evidence);

    assert_eq!(
        evidence_for(&verdict.local_overrides, LOCAL_OVERRIDE),
        [
            "dinput8.dll (neighbour)",
            "version.dll (neighbour)",
            "winmm.dll (neighbour)"
        ],
        "every shadowed name stays quotable"
    );
}

#[test]
fn a_windows_system_directory_produces_no_injector_findings() {
    // Point the tool at System32 and the naive rule fires on every renderer DLL
    // in Windows: the directory holds dxgi.dll, d3d11.dll and opengl32.dll
    // because it is Windows, not because someone installed a proxy. There the
    // neighbour rule inverts, so the directory evidence is dropped whole.
    let evidence = Evidence {
        imports: vec!["msvcp_win.dll".to_owned()],
        delay_imports: Vec::new(),
        neighbours: vec![
            "ntdll.dll".to_owned(),
            "kernel32.dll".to_owned(),
            "user32.dll".to_owned(),
            "dxgi.dll".to_owned(),
            "d3d11.dll".to_owned(),
            "opengl32.dll".to_owned(),
            "_nvngx.dll".to_owned(),
        ],
        ..Evidence::default()
    };
    let verdict = analyse(&evidence);

    assert!(
        verdict.is_empty(),
        "System32 describes Windows, not the binary, got {verdict:?}"
    );
    assert_eq!(verdict.headline(), "no graphics API determined");
}

#[test]
fn two_os_core_dlls_beside_a_game_are_not_enough_to_discard_the_neighbours() {
    // The other side of the System32 rule, and the side no test pinned: the
    // threshold is three, so raising it was caught and lowering it to one or two
    // was not. At one, any game directory shipping a single redistributable copy
    // of a common OS DLL would have all of its neighbour evidence thrown away
    // and a ReShade install beside it would go unreported. Two is the most a
    // real game directory is plausibly seen with; three is the bound.
    let evidence = Evidence {
        neighbours: vec![
            "kernel32.dll".to_owned(),
            "user32.dll".to_owned(),
            "dxgi.dll".to_owned(),
        ],
        ..Evidence::default()
    };
    let verdict = analyse(&evidence);

    assert_eq!(
        evidence_for(&verdict.local_overrides, LOCAL_OVERRIDE),
        ["dxgi.dll (neighbour)"],
        "two OS-core names is a game directory, not Windows' own"
    );
}

#[test]
fn a_binary_with_nothing_recognisable_says_so_instead_of_guessing() {
    // A console tool imports kernel32 and nothing else. The honest answer is
    // that no graphics API was determined, not the oldest one in the table.
    let verdict = analyse(&importing(&["KERNEL32.dll", "msvcrt.dll", "ADVAPI32.dll"]));

    assert!(verdict.is_empty(), "got {verdict:?}");
    assert_eq!(verdict.headline(), "no graphics API determined");
}

#[test]
fn empty_evidence_is_an_empty_verdict_rather_than_a_panic() {
    // The path taken by a file that parsed but imports nothing at all.
    let verdict = analyse(&Evidence::default());

    assert_eq!(verdict, Verdict::default());
    assert_eq!(verdict.headline(), "no graphics API determined");
}

#[test]
fn import_names_are_matched_without_regard_to_case_but_quoted_as_written() {
    // PE import names are spelled however the linker felt that day; `D3D12.DLL`
    // and `d3d12.dll` both occur. A case-sensitive table silently misses half
    // of them, and normalising the input would make the report quote a name
    // that is not in the file.
    let verdict = analyse(&importing(&["D3D12.DLL", "Vulkan-1.Dll"]));

    assert_eq!(names(&verdict.renderers), ["Direct3D 12", "Vulkan"]);
    assert_eq!(
        evidence_for(&verdict.renderers, "Direct3D 12"),
        ["D3D12.DLL (import)"],
        "the signal quotes the file's own spelling"
    );
}

#[test]
fn the_versioned_direct3d_11_interfaces_all_land_on_one_finding() {
    // `d3d11_1.dll` through `d3d11_4.dll` are the versioned device interfaces,
    // not four renderers. Reporting four would make the set look like four
    // run-time choices when there is one.
    let verdict = analyse(&importing(&["d3d11.dll", "d3d11_2.dll", "d3d11_4.dll"]));

    assert_eq!(names(&verdict.renderers), ["Direct3D 11"]);
    assert_eq!(
        evidence_for(&verdict.renderers, "Direct3D 11").len(),
        3,
        "one finding, three supporting libraries"
    );
}

#[test]
fn a_shipped_dlss_dll_is_found_in_the_directory_because_it_is_in_no_import_table() {
    // This is why neighbour provenance earns its place. `nvngx_dlss.dll` is
    // loaded by name at run time by the NGX loader and appears in no import
    // table of any binary; shipping the file is how DLSS arrives at all.
    let evidence = Evidence {
        imports: vec!["d3d12.dll".to_owned(), "nvapi64.dll".to_owned()],
        delay_imports: Vec::new(),
        neighbours: vec![
            "nvngx_dlss.dll".to_owned(),
            "nvngx_dlssg.dll".to_owned(),
            "sl.interposer.dll".to_owned(),
            "sl.reflex.dll".to_owned(),
        ],
        ..Evidence::default()
    };
    let verdict = analyse(&evidence);

    assert_eq!(
        names(&verdict.features),
        [
            "NVAPI",
            "DLSS Frame Generation",
            "DLSS Super Resolution",
            "NVIDIA Streamline"
        ],
        "imported NVAPI outranks the shipped files; then specificity"
    );
    assert_eq!(
        evidence_for(&verdict.features, "NVIDIA Streamline"),
        ["sl.interposer.dll (neighbour)", "sl.reflex.dll (neighbour)"],
        "the Streamline plugins collapse to one finding, names kept"
    );
}

#[test]
fn the_dlss_5_runtime_is_named_and_leads_the_runtimes_it_ships_beside() {
    // A directory with all four runtimes in it used to come back with three
    // findings and silence about the one that makes it a DLSS 5 install. Three
    // true lines and a hole reads as a complete answer, which is worse than no
    // answer at all.
    //
    // The order is the table's specificity rule, not recency: the baseline
    // runtime ships with almost every title that ships any of them and narrows
    // least, so it comes last, and neural rendering — which arrives on top of a
    // directory that already has the other three — comes first.
    let evidence = Evidence {
        neighbours: vec![
            "nvngx_dlss.dll".to_owned(),
            "nvngx_dlssg.dll".to_owned(),
            "nvngx_dlssnr.dll".to_owned(),
            "nvngx_dlssd.dll".to_owned(),
        ],
        ..Evidence::default()
    };
    let verdict = analyse(&evidence);

    assert_eq!(
        names(&verdict.features),
        [
            "DLSS Neural Rendering",
            "DLSS Frame Generation",
            "DLSS Ray Reconstruction",
            "DLSS Super Resolution"
        ],
        "all four are reported, most specific first"
    );
    assert_eq!(
        evidence_for(&verdict.features, "DLSS Neural Rendering"),
        ["nvngx_dlssnr.dll (neighbour)"],
        "and it is quoted as the shipped file it is, since it is in no import \
         table any more than the other three are"
    );
}

#[test]
fn the_fsr_and_xess_libraries_are_recognised_by_their_family_names() {
    // FSR ships under a version-stamped file name that changes every release,
    // so an exact table goes stale on the next one; the prefix does not.
    let evidence = Evidence {
        neighbours: vec![
            "ffx_fsr2_api_x64.dll".to_owned(),
            "amd_fidelityfx_dx12.dll".to_owned(),
            "libxess.dll".to_owned(),
        ],
        ..Evidence::default()
    };
    let verdict = analyse(&evidence);

    assert_eq!(names(&verdict.features), ["FSR", "XeSS"]);
    assert_eq!(
        evidence_for(&verdict.features, "FSR").len(),
        2,
        "both AMD spellings support the one finding"
    );
}

#[test]
fn an_agility_sdk_runtime_in_the_directory_is_a_renderer_signal_not_an_injector() {
    // `D3D12Core.dll` is the one system-adjacent file whose presence next to a
    // game means what it says: it is a Microsoft redistributable that only a
    // Direct3D 12 application ships, and it is loaded by d3d12.dll by name
    // rather than being a loader-order trick. Filing it with the proxies would
    // throw away a genuine signal.
    let evidence = Evidence {
        neighbours: vec!["D3D12Core.dll".to_owned(), "game.exe".to_owned()],
        ..Evidence::default()
    };
    let verdict = analyse(&evidence);

    assert_eq!(names(&verdict.renderers), ["Direct3D 12"]);
    assert_eq!(verdict.renderers[0].strength(), Source::Neighbour);
    assert_eq!(
        verdict.headline(),
        "Direct3D 12 (from files in the directory only)",
        "a shipped file is weaker than an import and the line must say so"
    );
    assert!(
        verdict.local_overrides.is_empty(),
        "the Agility SDK is not a proxy"
    );
}

#[test]
fn the_entry_points_that_only_show_up_in_real_binaries_are_in_the_tables_too() {
    // Found by sweeping the import tables of every binary in System32 rather
    // than by reading documentation. `d3d10_1core.dll` ships beside
    // `d3d10core.dll` and is easy to leave out of a hand-written table, and
    // `d3d11on12.dll` is Direct3D 11 code on a Direct3D 12 device — the
    // application wrote Direct3D 11, so that is what it is reported as.
    let verdict = analyse(&importing(&["d3d11on12.dll", "d3d10_1core.dll"]));

    assert_eq!(names(&verdict.renderers), ["Direct3D 11", "Direct3D 10"]);
}

#[test]
fn the_same_library_named_twice_does_not_count_twice() {
    // A merged or malformed import table can list one name more than once, and
    // duplicated evidence reads as corroboration it is not.
    let verdict = analyse(&importing(&["d3d12.dll", "D3D12.dll", "d3d12.dll"]));

    assert_eq!(
        evidence_for(&verdict.renderers, "Direct3D 12").len(),
        1,
        "one observation, however many times it was written down"
    );
}

#[test]
fn a_renderer_reached_through_a_linked_library_is_attributed_to_that_library() {
    // A Unity executable imports UnityPlayer.dll and nothing graphical. The
    // signal names the intermediary rather than the API it reaches, because
    // `UnityPlayer.dll` is what this import table actually spells and it is the
    // file a reader would go and open.
    let evidence = Evidence {
        imports: vec!["KERNEL32.dll".to_owned(), "UnityPlayer.dll".to_owned()],
        linked: vec![super::Linked {
            library: "UnityPlayer.dll".to_owned(),
            imports: vec!["d3d11.dll".to_owned(), "dxgi.dll".to_owned()],
            delay_imports: Vec::new(),
        }],
        ..Evidence::default()
    };

    let verdict = analyse(&evidence);

    assert_eq!(names(&verdict.renderers), ["Direct3D 11"]);
    assert_eq!(verdict.renderers[0].signals[0].library, "UnityPlayer.dll");
    assert_eq!(
        verdict.renderers[0].signals[0].source,
        super::Source::Linked
    );
    assert_eq!(names(&verdict.infrastructure), ["DXGI"]);
}

#[test]
fn a_linked_library_contributes_no_features() {
    // Features arrive from files in the directory, never from an import table:
    // nvngx_dlss.dll is loaded by name at run time and appears in no import
    // table anywhere. Reading one out of a linked library would report a
    // capability on evidence that cannot exist.
    let evidence = Evidence {
        imports: vec!["Engine.dll".to_owned()],
        linked: vec![super::Linked {
            library: "Engine.dll".to_owned(),
            imports: vec!["nvngx_dlss.dll".to_owned()],
            delay_imports: Vec::new(),
        }],
        ..Evidence::default()
    };

    let verdict = analyse(&evidence);

    assert!(verdict.features.is_empty(), "got {:?}", verdict.features);
}

#[test]
fn a_direct_import_sorts_above_the_same_api_reached_through_a_library() {
    // Both are true at once for plenty of engines, and the stronger claim is
    // the executable's own import table. `Source` is ordered so this falls out
    // of the derive rather than needing a comparator at every use.
    assert!(super::Source::Import < super::Source::Linked);
    assert!(super::Source::Linked < super::Source::Neighbour);
}

#[test]
fn a_neighbouring_vulkan_loader_is_reported_like_every_other_system_renderer_dll() {
    // `vulkan-1.dll` beside an executable is the same shadowing trick as a
    // local `d3d11.dll`, and it was the one system renderer name missing from
    // the table: the verdict came back byte-identical to the one for an empty
    // directory, so the file was not merely unexplained, it was unmentioned.
    let verdict = analyse(&Evidence {
        neighbours: vec!["game.exe".to_owned(), "vulkan-1.dll".to_owned()],
        ..Evidence::default()
    });

    assert!(
        !verdict.is_empty(),
        "a file that is there must not read as an empty directory, got {verdict:?}"
    );
    assert_eq!(
        evidence_for(&verdict.local_overrides, LOCAL_OVERRIDE),
        ["vulkan-1.dll (neighbour)"],
        "the loader is named as a shadowed system DLL, not as a renderer"
    );
    assert!(
        verdict.renderers.is_empty(),
        "and a neighbouring file is still not a renderer signal, got {:?}",
        verdict.renderers
    );
}

#[test]
fn a_binary_importing_only_dxcore_is_told_what_was_actually_found() {
    // DXCore is the newer adapter-enumeration layer and fills the same slot as
    // DXGI. Filed nowhere it produced "no graphics API determined" about a file
    // that names an adapter layer; named DXGI it would claim a library the file
    // does not contain. The sentence is built from what was found instead.
    let verdict = analyse(&importing(&["KERNEL32.dll", "dxcore.dll"]));

    assert_eq!(names(&verdict.infrastructure), ["DXCore"]);
    assert_eq!(
        verdict.headline(),
        "DXCore present, specific API not determined"
    );
    assert_eq!(
        evidence_for(&verdict.infrastructure, "DXCore"),
        ["dxcore.dll (import)"],
        "the sentence stays checkable against the import table"
    );
}

#[test]
fn a_binary_importing_both_adapter_layers_names_both_and_still_claims_no_api() {
    // The case that exists in System32 today (Taskmgr.exe). Naming one of the
    // two would be a choice the evidence does not support, and the headline
    // must still not read as "DirectX".
    let verdict = analyse(&importing(&["dxgi.dll", "dxcore.dll"]));

    assert_eq!(
        verdict.headline(),
        "DXGI and DXCore present, specific API not determined"
    );
    assert!(verdict.renderers.is_empty(), "neither one draws anything");
}

#[test]
fn the_graphics_question_the_report_asks_agrees_with_the_verdict_it_prints() {
    // The report's graphics/other split used to keep a prefix table of its own,
    // and the two disagreed: `amd_fidelityfx_dx12.dll` was reported as FSR in
    // one row and as non-graphical two rows below it. Anything this crate has a
    // rule for answers yes here, by construction.
    for name in [
        "amd_fidelityfx_dx12.dll",
        "AMD_FidelityFX_DX12.dll",
        "dinput8.dll",
        "winhttp.dll",
        "xinput1_4.dll",
        "version.dll",
        "vulkan-1.dll",
        "DXGI.dll",
        "dxcore.dll",
        "nvngx_dlss.dll",
        // The driver's own copy. `analysis.rs` learned that the leading
        // underscore is not a typo and wrote a comment saying so; the CLI's
        // prefix table never did, because `"_nvngx.dll".starts_with("nvngx")`
        // is false. Two tables, one of which learned something.
        "_nvngx.dll",
        // Known to no table, kept by the deliberately broad prefixes.
        "d3dcompiler_47.dll",
        "openvr_api.dll",
    ] {
        assert!(
            super::is_graphics_related(name),
            "{name} is worth pulling out of the crowd"
        );
    }
    for name in [
        "KERNEL32.dll",
        "msvcrt.dll",
        "api-ms-win-core-file-l1-1-0.dll",
    ] {
        assert!(
            !super::is_graphics_related(name),
            "{name} is the crowd itself"
        );
    }
}

#[test]
fn a_versioned_direct3d_interface_beside_a_game_is_an_override_like_any_other() {
    // The neighbour loop asked "is it one of these sixteen names" where the
    // rule it implements is "is it a system DLL a local file can shadow". The
    // versioned interfaces ship, are shadowed by the same loader rule, and were
    // the category — not the single name — that fell through into silence.
    let verdict = analyse(&Evidence {
        neighbours: vec!["d3d11on12.dll".to_owned(), "d3d10core.dll".to_owned()],
        ..Evidence::default()
    });

    assert!(!verdict.is_empty(), "got {verdict:?}");
    assert_eq!(
        evidence_for(&verdict.local_overrides, LOCAL_OVERRIDE),
        ["d3d10core.dll (neighbour)", "d3d11on12.dll (neighbour)"],
        "both are named, and neither is read as a renderer the binary uses"
    );
    assert!(verdict.renderers.is_empty(), "got {:?}", verdict.renderers);
}
