mod common;

use common::Image;
use dxray_pe::{Pe, Version};

#[test]
fn reads_the_file_version_out_of_the_resource_tree() {
    // A real DLSS runtime: the number is nowhere in the file name, which is
    // nvngx_dlss.dll in every version ever shipped.
    let buf = Image::x64()
        .importing(&["KERNEL32.dll"])
        .versioned([3, 7, 20, 0], [3, 7, 20, 0])
        .build();
    let pe = Pe::parse(&buf).unwrap();

    let info = pe
        .version_info()
        .unwrap()
        .expect("image carries RT_VERSION");
    assert_eq!(
        info.file,
        Version {
            major: 3,
            minor: 7,
            patch: 20,
            build: 0
        }
    );
    assert_eq!(info.file.to_string(), "3.7.20.0");
}

#[test]
fn file_and_product_versions_are_read_separately() {
    // They routinely differ, and conflating them would report the product's
    // number for a DLL that is actually older.
    let buf = Image::x64()
        .versioned([310, 2, 1, 0], [572, 16, 0, 0])
        .build();
    let pe = Pe::parse(&buf).unwrap();

    let info = pe.version_info().unwrap().unwrap();
    assert_eq!(info.file.to_string(), "310.2.1.0");
    assert_eq!(info.product.to_string(), "572.16.0.0");
}

#[test]
fn an_image_with_no_resources_has_no_version_rather_than_an_error() {
    // Plenty of game executables ship without a version resource. That is an
    // absent answer, not a failure, and the two must not look alike.
    let buf = Image::x64().importing(&["d3d12.dll"]).build();
    let pe = Pe::parse(&buf).unwrap();

    assert_eq!(pe.version_info().unwrap(), None);
}

#[test]
fn a_resource_tree_without_rt_version_yields_nothing() {
    // RT_MANIFEST is 24. The walk has to match the type, not take whichever
    // resource happens to be first in the tree.
    let buf = Image::x64()
        .versioned([1, 0, 0, 0], [1, 0, 0, 0])
        .filed_under_type(24)
        .build();
    let pe = Pe::parse(&buf).unwrap();

    assert_eq!(pe.version_info().unwrap(), None);
}

#[test]
fn every_part_survives_the_round_trip_through_two_packed_words() {
    // Windows stores four u16 in two u32, high half first. Getting the halves
    // backwards still produces a plausible-looking version.
    let buf = Image::x64().versioned([1, 2, 3, 4], [5, 6, 7, 8]).build();
    let pe = Pe::parse(&buf).unwrap();

    let info = pe.version_info().unwrap().unwrap();
    assert_eq!(info.file.to_string(), "1.2.3.4");
    assert_eq!(info.product.to_string(), "5.6.7.8");
}

#[test]
fn versions_compare_in_order() {
    let older = Version {
        major: 3,
        minor: 7,
        patch: 20,
        build: 0,
    };
    let newer = Version {
        major: 3,
        minor: 7,
        patch: 100,
        build: 0,
    };

    assert!(older < newer, "3.7.20 must sort before 3.7.100");
}
