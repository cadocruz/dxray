//! The awkward cases: each is one where the obvious implementation is wrong
//! about a real binary.

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
    // Games link both and choose at run time, so a verdict is a set.
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
    // `dxgi.dll` is shared by D3D10, 11 and 12: no generation is named.
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
    // `dxgi.dll` never joins the renderer set.
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
    // Delay-loaded and imported are kept apart; both survive.
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
    // Evidence strength outranks generation.
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
    // A local file named after a system DLL the image never imports is a proxy.
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
    // Several proxies: one finding, every file kept in the signals.
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
    // A directory full of system DLLs is System32, not proxies.
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
    // The System32 threshold is three: two copies are still a game directory.
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
    // Names match without case and are quoted as spelled.
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
    // Versioned D3D11 interfaces are one renderer.
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
    // `nvngx_dlss.dll` is loaded by name, so only the neighbour shows it.
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
    // All four DLSS runtimes are reported, most specific first: neural
    // rendering first, the baseline last.
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
    // `D3D12Core.dll` beside a game is a real D3D12 signal, not a proxy.
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
    // `d3d11on12.dll` is Direct3D 11 code, and reported as such.
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
    // Unity: the signal names `UnityPlayer.dll`, the file to open.
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
    // Features come only from directory files, never a linked import.
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
    // The image's own import is the stronger claim.
    assert!(super::Source::Import < super::Source::Linked);
    assert!(super::Source::Linked < super::Source::Neighbour);
}

#[test]
fn a_neighbouring_vulkan_loader_is_reported_like_every_other_system_renderer_dll() {
    // A local `vulkan-1.dll` is a proxy too.
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
    // DXCore, like DXGI, names an adapter layer and no renderer.
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
    // DXGI and DXCore together (Taskmgr.exe): neither is picked.
    let verdict = analyse(&importing(&["dxgi.dll", "dxcore.dll"]));

    assert_eq!(
        verdict.headline(),
        "DXGI and DXCore present, specific API not determined"
    );
    assert!(verdict.renderers.is_empty(), "neither one draws anything");
}

#[test]
fn the_graphics_question_the_report_asks_agrees_with_the_verdict_it_prints() {
    // One answer to "is this graphical", shared with the report.
    for name in [
        "amd_fidelityfx_dx12.dll",
        "AMD_FidelityFX_DX12.dll",
        "vulkan-1.dll",
        "DXGI.dll",
        "dxcore.dll",
        "nvngx_dlss.dll",
        // The driver's copy, leading underscore and all.
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
fn a_shadowable_name_that_draws_nothing_is_the_crowd_in_an_import_list() {
    // Mod-loader proxy names are not graphical imports.
    for name in [
        "winmm.dll",
        "dsound.dll",
        "dinput.dll",
        "dinput8.dll",
        "version.dll",
        "winhttp.dll",
        "xinput1_3.dll",
        "xinput1_4.dll",
    ] {
        assert!(
            super::is_shadowable(name),
            "{name} has to stay a name a local file can shadow"
        );
        assert!(
            !super::is_graphics_related(name),
            "{name} draws nothing, so an import of it belongs with the crowd"
        );
        assert!(
            analyse(&importing(&[name])).is_empty(),
            "{name} imported must produce no finding, or the split would \
             contradict the verdict above it"
        );
    }
}

#[test]
fn every_shadowable_name_that_does_draw_is_still_pulled_out_of_the_crowd() {
    // These are shadowable too, and they answer yes through `renderer` rather
    // than through `PROXYABLE`. Dropping that list must not have taken them.
    for name in [
        "dxgi.dll",
        "d3d12.dll",
        "d3d11.dll",
        "d3d10.dll",
        "d3d9.dll",
        "d3d8.dll",
        "ddraw.dll",
        "opengl32.dll",
        "vulkan-1.dll",
    ] {
        assert!(
            super::is_graphics_related(name),
            "{name} names a graphics API, however else it is used"
        );
    }
}

#[test]
fn a_shipped_agility_runtime_and_an_imported_d3d12_are_one_finding() {
    // Direct3D names come from `renderer`, not a literal.
    let verdict = analyse(&Evidence {
        imports: vec!["d3d12.dll".to_owned()],
        neighbours: vec!["D3D12Core.dll".to_owned()],
        ..Evidence::default()
    });

    assert_eq!(
        verdict.renderers.len(),
        1,
        "one API is one finding, however many files name it: {:?}",
        verdict.renderers
    );
    assert_eq!(
        evidence_for(&verdict.renderers, "Direct3D 12"),
        ["d3d12.dll (import)", "D3D12Core.dll (neighbour)"]
    );
}

#[test]
fn a_versioned_direct3d_interface_beside_a_game_is_an_override_like_any_other() {
    // Any shadowable system DLL counts, including versioned interfaces.
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
