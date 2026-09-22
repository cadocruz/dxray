mod common;

use common::{
    DIRECTORIES, DIRECTORY_COUNT, FIRST_SECTION_HEADER, IMPORT_DIRECTORY, Image, OPTIONAL_MAGIC,
    put16, put32,
};
use dxray_pe::{Error, MAX_DESCRIPTORS, Machine, Pe};

#[test]
fn reads_the_import_table_of_a_64_bit_image() {
    let buf = Image::x64()
        .importing(&["d3d12.dll", "dxgi.dll", "KERNEL32.dll"])
        .build();
    let pe = Pe::parse(&buf).unwrap();

    assert_eq!(pe.machine(), Machine::Amd64);
    assert!(pe.is_64bit());
    assert_eq!(
        pe.imports().unwrap(),
        ["d3d12.dll", "dxgi.dll", "KERNEL32.dll"]
    );
}

#[test]
fn reads_the_import_table_of_a_32_bit_image() {
    // PE32 puts the data directory 16 bytes earlier than PE32+. Reading it with
    // the wrong layout does not fail: it finds a zero where the directory count
    // should be and reports no imports at all. Verified by mutation - a silent
    // empty answer is the whole reason both widths get the same assertion.
    let buf = Image::x86()
        .importing(&["d3d9.dll", "opengl32.dll"])
        .build();
    let pe = Pe::parse(&buf).unwrap();

    assert_eq!(pe.machine(), Machine::I386);
    assert!(!pe.is_64bit());
    assert_eq!(pe.imports().unwrap(), ["d3d9.dll", "opengl32.dll"]);
}

#[test]
fn a_delay_loaded_renderer_is_invisible_to_the_import_table() {
    // The case that makes naive detection wrong: at startup this process links
    // against nothing but KERNEL32, yet it is a D3D12 game.
    let buf = Image::x64()
        .importing(&["KERNEL32.dll"])
        .delay_loading(&["d3d12.dll"])
        .build();
    let pe = Pe::parse(&buf).unwrap();

    assert_eq!(pe.imports().unwrap(), ["KERNEL32.dll"]);
    assert_eq!(pe.delay_imports().unwrap(), ["d3d12.dll"]);
}

#[test]
fn an_image_that_delay_loads_nothing_reports_an_empty_list_not_an_error() {
    let buf = Image::x64().importing(&["vulkan-1.dll"]).build();
    let pe = Pe::parse(&buf).unwrap();

    assert_eq!(pe.delay_imports().unwrap(), Vec::<String>::new());
}

#[test]
fn a_non_pe_file_is_rejected_rather_than_misread() {
    assert_eq!(Pe::parse(b"#!/bin/sh").unwrap_err(), Error::NotMz);
}

#[test]
fn a_truncated_image_reports_where_it_ran_out() {
    let buf = Image::x64().importing(&["d3d11.dll"]).build();
    let err = Pe::parse(&buf[..0x90]).unwrap_err();

    assert!(matches!(err, Error::Truncated { .. }), "got {err:?}");
}

#[test]
fn an_unknown_optional_header_magic_is_not_guessed_at() {
    let mut buf = Image::x64().importing(&["d3d11.dll"]).build();
    put16(&mut buf, OPTIONAL_MAGIC, 0x0107); // ROM image

    assert_eq!(
        Pe::parse(&buf).unwrap_err(),
        Error::BadOptionalMagic(0x0107)
    );
}

#[test]
fn an_import_table_reached_through_an_overflowing_section_is_refused_not_answered() {
    // The whole image, not just the arithmetic: a file whose only section
    // claims its bytes start at 0xFFFF_FD00 and whose import directory sits
    // 0x500 into it. The sum leaves u32. In a debug build that used to panic;
    // in a release build it wrapped to file offset 0x200 and the parser
    // reported whatever DLL name happened to be lying there, out of a table
    // the header never declared. A name read from the wrong place is the one
    // failure this crate must never produce, so the answer has to be an error.
    let mut buf = Image::x64().importing(&["d3d12.dll"]).build();
    put32(&mut buf, DIRECTORIES + IMPORT_DIRECTORY * 8, 0x1500);
    put32(&mut buf, FIRST_SECTION_HEADER + 8, 0x2000); // VirtualSize
    put32(&mut buf, FIRST_SECTION_HEADER + 16, 0x2000); // SizeOfRawData
    put32(&mut buf, FIRST_SECTION_HEADER + 20, 0xFFFF_FD00); // PointerToRawData

    let pe = Pe::parse(&buf).expect("the headers themselves are well formed");
    assert_eq!(pe.imports(), Err(Error::UnmappedRva(0x1500)));
}

#[test]
fn a_directory_count_no_file_could_hold_is_an_error_rather_than_an_allocation() {
    // `NumberOfRvaAndSizes` is four bytes nobody validated. Reserving what it
    // claims asked for 34 GB on a file of one kilobyte, and a failed
    // allocation aborts the process: not an `Err` a caller can catch, so one
    // hostile or corrupt file killed a whole directory walk.
    //
    // This test proving anything at all is the proof. An abort takes the test
    // binary down with it, so reaching the assertion below is what says the
    // process survived; the assertion only adds that it survived with the
    // right answer.
    let mut buf = Image::x64().importing(&["d3d12.dll"]).build();
    put32(&mut buf, DIRECTORY_COUNT, 0xFFFF_FFFF);

    let err = Pe::parse(&buf).unwrap_err();
    assert!(
        matches!(err, Error::Truncated { .. }),
        "the walk stops where the file does, got {err:?}"
    );
}

#[test]
fn a_descriptor_array_that_never_terminates_is_an_error_rather_than_a_short_list() {
    // A truncated import list reads exactly like a complete one, so a binary
    // whose `d3d12.dll` sat past the bound came back "no graphics API
    // determined" with nothing saying why. Built from the constant.
    let names = vec!["engine.dll"; MAX_DESCRIPTORS + 1];
    let buf = Image::x64().importing(&names).build();
    let pe = Pe::parse(&buf).unwrap();

    let error = pe.imports().expect_err("the array runs past the bound");

    assert!(
        matches!(error, Error::UnterminatedDescriptors { .. }),
        "got {error:?}"
    );
}

#[test]
fn a_descriptor_array_ending_exactly_at_the_bound_still_reads() {
    let names = vec!["engine.dll"; MAX_DESCRIPTORS - 1];
    let buf = Image::x64().importing(&names).build();
    let pe = Pe::parse(&buf).unwrap();

    assert_eq!(
        pe.imports()
            .expect("the terminator is within the bound")
            .len(),
        MAX_DESCRIPTORS - 1
    );
}
