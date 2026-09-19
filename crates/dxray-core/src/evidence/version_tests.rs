//! What [`stamp_versions`](super::stamp_versions) reports, the four different
//! ways it can have no number to report, and that a directory is read once
//! however many files in it ask about it.
//!
//! In its own file rather than inside `evidence.rs` because every case here
//! needs a PE image with a real resource tree on disk, and the fixtures are
//! most of the length.

use dxray_pe::{Version, VersionInfo};

use crate::analysis::{Evidence, FileVersion, Verdict, analyse};
use crate::evidence::{LibraryCache, neighbours_of, stamp_versions};
use crate::testutil::TempDir;

/// Reads `path` and stamps the verdict, exactly the way both binaries do it.
fn verdict_for(path: &std::path::Path) -> Verdict {
    let evidence = Evidence::from_executable(path).expect("a readable image");
    let mut verdict = analyse(&evidence);
    stamp_versions(&mut verdict, path, &evidence.neighbours);
    verdict
}

fn version_of(verdict: &Verdict, library: &str) -> FileVersion {
    for finding in verdict
        .renderers
        .iter()
        .chain(&verdict.infrastructure)
        .chain(&verdict.features)
        .chain(&verdict.local_overrides)
    {
        for signal in &finding.signals {
            if signal.library.eq_ignore_ascii_case(library) {
                return signal.version;
            }
        }
    }
    panic!("no signal named {library} in {verdict:?}");
}

const fn parts(v: [u16; 4]) -> Version {
    Version {
        major: v[0],
        minor: v[1],
        patch: v[2],
        build: v[3],
    }
}

#[test]
fn a_shipped_dlss_runtime_reports_which_build_it_is() {
    // The reason the version reader exists at all. `nvngx_dlss.dll` has been
    // called exactly that in every version ever shipped, so the name answers
    // nothing and the number answers the whole question a person has before
    // swapping the file.
    let dir = TempDir::new("dlss");
    let exe = dir.image("game.exe", &["KERNEL32.dll"], &[]);
    dir.versioned_image("nvngx_dlss.dll", &[], [310, 2, 1, 0], [310, 2, 1, 0]);

    let verdict = verdict_for(&exe);

    assert_eq!(
        version_of(&verdict, "nvngx_dlss.dll"),
        FileVersion::Stamped(VersionInfo {
            file: parts([310, 2, 1, 0]),
            product: parts([310, 2, 1, 0]),
        }),
        "the build number has to reach the verdict, not stop at the parser"
    );
}

#[test]
fn a_version_resource_of_all_zeroes_is_a_version_and_not_a_missing_one() {
    // Twenty-one files on one ordinary Windows install carry a resource that is
    // present and all zeroes. `dxray-pe` was written to report them as the
    // number they are; collapsing them into "no version" here would undo that
    // decision one layer further out, where nobody would look for it.
    let dir = TempDir::new("zeroes");
    let exe = dir.image("game.exe", &["KERNEL32.dll"], &[]);
    dir.versioned_image("nvngx_dlss.dll", &[], [0, 0, 0, 0], [0, 0, 0, 0]);

    let version = version_of(&verdict_for(&exe), "nvngx_dlss.dll");

    assert_eq!(version.state(), "stamped", "all zeroes is still stamped");
    assert_eq!(
        version.label().as_deref(),
        Some("0.0.0.0"),
        "and it renders as the number it is"
    );
}

#[test]
fn a_local_file_with_no_version_is_not_the_same_as_a_name_windows_resolves() {
    // Two answers that are both "no number" and mean different things.
    // `nvngx_dlss.dll` sitting there without a resource is a fact about the file
    // somebody shipped; `d3d12.dll` having no local copy is a fact about the
    // directory, and the version the loader would find elsewhere describes
    // Windows rather than this game. Both are printed; neither is a blank.
    let dir = TempDir::new("states");
    let exe = dir.image("game.exe", &["d3d12.dll"], &[]);
    dir.image("nvngx_dlss.dll", &[], &[]);

    let verdict = verdict_for(&exe);

    assert_eq!(
        version_of(&verdict, "nvngx_dlss.dll"),
        FileVersion::Unstamped,
        "the file is there and says nothing"
    );
    assert_eq!(
        version_of(&verdict, "d3d12.dll"),
        FileVersion::Elsewhere,
        "no file of that name is here at all"
    );
}

#[test]
fn a_file_that_is_not_a_pe_image_is_unreadable_rather_than_unversioned() {
    // A zero-byte placeholder where a DLSS runtime should be is worth seeing.
    // Reporting it as "no version" would read as a shipped file that merely
    // lacks a resource.
    let dir = TempDir::new("placeholder");
    let exe = dir.image("game.exe", &["KERNEL32.dll"], &[]);
    dir.write("nvngx_dlss.dll", "");

    assert_eq!(
        version_of(&verdict_for(&exe), "nvngx_dlss.dll"),
        FileVersion::Unreadable
    );
}

#[test]
fn a_proxy_dll_shadowing_a_system_name_is_versioned_because_it_is_what_loads() {
    // The uniform rule paying off. `dxgi.dll` is normally Windows' own and its
    // version is not worth printing — but when somebody has dropped a copy next
    // to the game, that copy is what the loader takes, and which ReShade build
    // it is happens to be the next question a reader asks.
    let dir = TempDir::new("proxy");
    let exe = dir.image("game.exe", &["KERNEL32.dll"], &[]);
    dir.versioned_image("dxgi.dll", &[], [6, 3, 5, 0], [6, 3, 5, 0]);

    let verdict = verdict_for(&exe);

    assert_eq!(
        version_of(&verdict, "dxgi.dll").label().as_deref(),
        Some("6.3.5.0")
    );
}

#[test]
fn a_library_is_matched_against_the_directory_spelling_not_the_import_table() {
    // PE import names are famously inconsistent about case, and this tool runs
    // on Linux, where `join` will not forgive that. A link signal quotes the
    // importing image's spelling; the file on disk has its own.
    let dir = TempDir::new("case");
    let exe = dir.image("StarRail.exe", &["KERNEL32.dll", "UNITYPLAYER.DLL"], &[]);
    dir.versioned_image(
        "UnityPlayer.dll",
        &["d3d11.dll"],
        [2022, 3, 21, 4],
        [2022, 3, 21, 4],
    );

    let verdict = verdict_for(&exe);

    assert_eq!(
        version_of(&verdict, "UNITYPLAYER.DLL").label().as_deref(),
        Some("2022.3.21.4"),
        "the engine version is the one number that identifies a Unity build"
    );
}

#[test]
fn one_file_named_by_two_findings_carries_the_same_version_under_both() {
    // `UnityPlayer.dll` is the signal behind Direct3D 11 and behind DXGI at
    // once. Two lookups that could disagree, and a fifty-megabyte engine DLL
    // read twice, are both avoided by reading it once per verdict.
    let dir = TempDir::new("shared");
    let exe = dir.image("StarRail.exe", &["KERNEL32.dll", "UnityPlayer.dll"], &[]);
    dir.versioned_image(
        "UnityPlayer.dll",
        &["d3d11.dll", "dxgi.dll"],
        [2022, 3, 21, 4],
        [2022, 3, 21, 4],
    );

    let verdict = verdict_for(&exe);

    let renderer = verdict.renderers[0].signals[0].version;
    let shared = verdict.infrastructure[0].signals[0].version;
    assert_eq!(renderer.label().as_deref(), Some("2022.3.21.4"));
    assert_eq!(
        renderer, shared,
        "the same file must not get two different answers"
    );
}

#[test]
fn a_verdict_nobody_stamped_says_so_rather_than_claiming_the_file_is_elsewhere() {
    // `analyse` opens nothing, so every signal it produces is `Unread`. A
    // consumer that reads "elsewhere" is entitled to believe somebody checked,
    // and would not be if the two states were one.
    let verdict = analyse(&Evidence {
        neighbours: vec!["nvngx_dlss.dll".to_owned()],
        ..Evidence::default()
    });

    assert_eq!(
        verdict.features[0].signals[0].version,
        FileVersion::Unread,
        "the pure half never claims to know a version"
    );
}

#[test]
fn a_signal_reads_as_one_sentence_with_the_version_inside_the_provenance() {
    // The single rendering both binaries use. A version printed by one surface
    // and not the other is the defect this project has now fixed twice under
    // different names.
    let dir = TempDir::new("describe");
    let exe = dir.image("game.exe", &["d3d12.dll"], &[]);
    dir.versioned_image("nvngx_dlssg.dll", &[], [310, 2, 1, 0], [310, 2, 1, 0]);

    let verdict = verdict_for(&exe);

    assert_eq!(
        verdict.features[0].signals[0].describe(),
        "nvngx_dlssg.dll (neighbour, 310.2.1.0)"
    );
    assert_eq!(
        verdict.renderers[0].signals[0].describe(),
        "d3d12.dll (import, no local copy)",
        "and a name with no file behind it says so, rather than going blank \
         where a reader cannot tell it from an annotation somebody forgot"
    );
}

#[test]
fn an_unstamped_file_and_an_unreadable_one_word_themselves_differently() {
    // Both are "no number", and a reader acts on them differently: one is a
    // shipped DLL without a resource, the other is something that is not a DLL.
    assert_eq!(
        FileVersion::Unstamped.label().as_deref(),
        Some("no version")
    );
    assert_eq!(
        FileVersion::Unreadable.label().as_deref(),
        Some("version unreadable")
    );
    assert_eq!(
        FileVersion::Elsewhere.label().as_deref(),
        Some("no local copy"),
        "no file of that name is here, which is a fact and not a blank"
    );
    assert_eq!(
        FileVersion::Unread.label(),
        None,
        "the one state with nothing to say, and no surface ever prints it"
    );
}

/// The file's own modification time, used as a proxy for "was it opened".
///
/// Reading a file updates its access time, but `relatime` and `noatime` make
/// that unreliable on the filesystems this actually runs on. So the test below
/// removes the file instead: a cache that kept its answer still has one, and a
/// cache that re-reads gets `Unreadable`. Crude, and it cannot be fooled.
fn remove(dir: &TempDir, name: &str) {
    std::fs::remove_file(dir.path().join(name)).expect("remove the fixture");
}

#[test]
fn a_library_already_read_for_one_executable_is_not_read_again_for_the_next() {
    // Measured, not assumed: an 803-file folder used to open each of its three
    // DLSS runtimes 805 times, because every record listed the directory afresh
    // and nothing was remembered between records. One cache down the walk is
    // what turns that into one read each, and this is the test that keeps it
    // that way — delete the file after the first executable and the second one
    // must still report the version, because it never goes back to the disk.
    let dir = TempDir::new("cached");
    let first = dir.image("first.exe", &["KERNEL32.dll"], &[]);
    let second = dir.image("second.exe", &["KERNEL32.dll"], &[]);
    dir.versioned_image("nvngx_dlss.dll", &[], [310, 2, 1, 0], [310, 2, 1, 0]);

    let mut cache = LibraryCache::default();
    let neighbours = neighbours_of(&first);
    let mut one = analyse(&Evidence {
        neighbours: neighbours.clone(),
        ..Evidence::default()
    });
    cache.stamp(&mut one, &first, &neighbours);
    assert_eq!(
        one.features[0].signals[0].version.label().as_deref(),
        Some("310.2.1.0")
    );

    remove(&dir, "nvngx_dlss.dll");

    let mut two = analyse(&Evidence {
        neighbours: neighbours.clone(),
        ..Evidence::default()
    });
    cache.stamp(&mut two, &second, &neighbours);

    assert_eq!(
        two.features[0].signals[0].version.label().as_deref(),
        Some("310.2.1.0"),
        "the second executable must be answered from the first one's read"
    );
}

#[test]
fn one_read_answers_both_the_link_chase_and_the_version() {
    // The second multiplier: a library that is both imported and named by the
    // verdict used to be opened twice per executable, once for each question.
    // Both answers come out of one `Pe`, so following the link first must leave
    // the version already known.
    let dir = TempDir::new("onemore");
    let exe = dir.image("StarRail.exe", &["KERNEL32.dll", "UnityPlayer.dll"], &[]);
    dir.versioned_image(
        "UnityPlayer.dll",
        &["d3d11.dll"],
        [2022, 3, 21, 4],
        [2022, 3, 21, 4],
    );

    let mut cache = LibraryCache::default();
    let neighbours = neighbours_of(&exe);
    let linked = cache.follow(&exe, &["UnityPlayer.dll".to_owned()], &[], &neighbours);
    assert_eq!(linked.len(), 1, "the link was followed");

    remove(&dir, "UnityPlayer.dll");

    let mut verdict = analyse(&Evidence {
        imports: vec!["UnityPlayer.dll".to_owned()],
        neighbours: neighbours.clone(),
        linked,
        ..Evidence::default()
    });
    cache.stamp(&mut verdict, &exe, &neighbours);

    assert_eq!(
        verdict.renderers[0].signals[0].version.label().as_deref(),
        Some("2022.3.21.4"),
        "the version was already in hand from the read the chase did"
    );
}

#[test]
fn the_directory_is_listed_once_however_many_files_in_it_are_inspected() {
    // The multiplier an open count does not show: `read_dir` once per record is
    // 803 listings of an 803-entry folder, and on a Windows mount that is what
    // a user waits for once the duplicate reads are gone. Proven the same crude
    // way — add a file after the first listing, and a cache that kept its answer
    // does not see it.
    let dir = TempDir::new("listing");
    let first = dir.image("first.exe", &["KERNEL32.dll"], &[]);
    let second = dir.image("second.exe", &["KERNEL32.dll"], &[]);

    let mut cache = LibraryCache::default();
    let before = cache.neighbours(&first);
    assert!(
        before.contains(&"second.exe".to_owned()),
        "the first listing sees what is there, got {before:?}"
    );

    dir.file("appeared-later.dll");

    let after = cache.neighbours(&second);
    assert!(
        !after.contains(&"appeared-later.dll".to_owned()),
        "the second file is answered from the first one's listing, got {after:?}"
    );
    assert!(
        !after.contains(&"second.exe".to_owned()),
        "and each file is still left out of its own neighbours, which is why \
         the exclusion happens per caller rather than in the cached listing"
    );
    assert!(after.contains(&"first.exe".to_owned()));
}

#[test]
fn a_cache_that_moves_to_another_directory_forgets_the_first_one() {
    // The key is a bare library name, so a cache carried across directories
    // would hand one game's `UnityPlayer.dll` version to another's. The walk
    // holds one cache for a whole recursive scan, which makes this the property
    // that keeps that safe — and keeps its memory to one folder's worth.
    let one = TempDir::new("dir-one");
    let exe_one = one.image("game.exe", &["KERNEL32.dll"], &[]);
    one.versioned_image("nvngx_dlss.dll", &[], [310, 2, 1, 0], [310, 2, 1, 0]);

    let two = TempDir::new("dir-two");
    let exe_two = two.image("game.exe", &["KERNEL32.dll"], &[]);
    two.versioned_image("nvngx_dlss.dll", &[], [3, 7, 20, 0], [3, 7, 20, 0]);

    let mut cache = LibraryCache::default();
    for (exe, expected) in [(&exe_one, "310.2.1.0"), (&exe_two, "3.7.20.0")] {
        let neighbours = neighbours_of(exe);
        let mut verdict = analyse(&Evidence {
            neighbours: neighbours.clone(),
            ..Evidence::default()
        });
        cache.stamp(&mut verdict, exe, &neighbours);
        assert_eq!(
            verdict.features[0].signals[0].version.label().as_deref(),
            Some(expected),
            "each directory answers for its own file"
        );
    }
}

#[test]
fn a_path_with_no_directory_beside_it_leaves_every_version_unread() {
    // A root has no neighbours. Answering "elsewhere" would be a claim about a
    // directory that was never listed.
    let mut verdict = analyse(&Evidence {
        neighbours: vec!["nvngx_dlss.dll".to_owned()],
        ..Evidence::default()
    });
    stamp_versions(
        &mut verdict,
        std::path::Path::new("/"),
        &["nvngx_dlss.dll".to_owned()],
    );

    assert_eq!(verdict.features[0].signals[0].version, FileVersion::Unread);
}
