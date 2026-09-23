//! Unit tests for private state the public API cannot aim at: sections larger
//! in memory than on disk, unterminated names.

use super::{Error, MAX_NAME, Machine, Pe, Section, read_u16, read_u32};

/// A `Pe` with no directories, so RVA translation can be tested on its own.
fn mapped(buf: &[u8], sections: Vec<Section>) -> Pe<'_> {
    Pe {
        buf,
        machine: Machine::Amd64,
        pe32_plus: true,
        sections,
        directories: Vec::new(),
    }
}

const fn section(
    virtual_address: u32,
    virtual_size: u32,
    raw_pointer: u32,
    raw_size: u32,
) -> Section {
    Section {
        virtual_address,
        virtual_size,
        raw_pointer,
        raw_size,
    }
}

#[test]
fn integers_are_little_endian_and_bounds_checked() {
    assert_eq!(read_u16(&[0x4d, 0x5a], 0), Ok(0x5a4d));
    assert_eq!(read_u32(&[0x50, 0x45, 0x00, 0x00], 0), Ok(0x0000_4550));

    // The offset in the error is the one the caller asked for, not the one the
    // slice ended at, so a failure names the field that was being read.
    let buf = [0u8; 6];
    assert_eq!(read_u32(&buf, 4), Err(Error::Truncated { at: 4 }));
    assert_eq!(read_u16(&buf, 5), Err(Error::Truncated { at: 5 }));
    assert_eq!(read_u32(&buf, 2), Ok(0));
}

#[test]
fn machine_keeps_values_it_does_not_recognise_instead_of_flattening_them() {
    assert_eq!(Machine::from_u16(0x014c), Machine::I386);
    assert_eq!(Machine::from_u16(0x8664), Machine::Amd64);
    assert_eq!(Machine::from_u16(0xaa64), Machine::Arm64);
    assert_eq!(Machine::from_u16(0x5032), Machine::Other(0x5032));

    assert_eq!(Machine::Amd64.to_string(), "x86-64");
    assert_eq!(Machine::Other(0x5032).to_string(), "unknown(0x5032)");
}

#[test]
fn errors_name_the_offset_or_address_they_failed_on() {
    assert_eq!(
        Error::Truncated { at: 0x3c }.to_string(),
        "truncated at offset 0x3c"
    );
    assert_eq!(
        Error::UnmappedRva(0x1000).to_string(),
        "rva 0x1000 is in no section"
    );
    assert_eq!(
        Error::BadOptionalMagic(0x0107).to_string(),
        "unknown optional header magic 0x0107"
    );
    assert_eq!(
        Error::UnterminatedName { at: 0x400 }.to_string(),
        "unterminated name at offset 0x400"
    );
}

#[test]
fn an_rva_inside_a_section_lands_on_its_stored_bytes() {
    let buf = [0u8; 0x600];
    let pe = mapped(&buf, vec![section(0x1000, 0x200, 0x400, 0x200)]);

    assert_eq!(pe.offset_of(0x1000), Ok(0x400));
    assert_eq!(pe.offset_of(0x10ff), Ok(0x4ff));
    assert_eq!(pe.offset_of(0x11ff), Ok(0x5ff));
}

#[test]
fn an_rva_outside_every_section_is_refused_rather_than_clamped() {
    let buf = [0u8; 0x600];
    let pe = mapped(&buf, vec![section(0x1000, 0x200, 0x400, 0x200)]);

    assert_eq!(pe.offset_of(0x0fff), Err(Error::UnmappedRva(0x0fff)));
    assert_eq!(pe.offset_of(0x1200), Err(Error::UnmappedRva(0x1200)));
    assert_eq!(pe.offset_of(0), Err(Error::UnmappedRva(0)));
}

#[test]
fn the_part_of_a_section_that_exists_only_in_memory_is_never_read_from_the_file() {
    // The loader zero-fills past the stored bytes; they are not read from the file.
    let buf = [0u8; 0x600];
    let pe = mapped(&buf, vec![section(0x1000, 0x800, 0x400, 0x200)]);

    assert_eq!(pe.offset_of(0x11ff), Ok(0x5ff));
    assert_eq!(pe.offset_of(0x1200), Err(Error::UnmappedRva(0x1200)));
    assert_eq!(pe.offset_of(0x17ff), Err(Error::UnmappedRva(0x17ff)));
}

#[test]
fn a_raw_pointer_that_overflows_when_the_delta_is_added_is_refused() {
    // A header-supplied sum past `u32` is an error; wrapping used to read a
    // plausible name from the wrong bytes.
    let buf = [0u8; 0x600];
    let pe = mapped(&buf, vec![section(0x1000, 0x2000, 0xFFFF_FD00, 0x2000)]);

    assert_eq!(pe.offset_of(0x1500), Err(Error::UnmappedRva(0x1500)));
    // The first byte of the section is still translated, because that sum
    // fits: only the arithmetic that leaves u32 is refused.
    assert_eq!(pe.offset_of(0x1000), Ok(0xFFFF_FD00));
}

#[test]
fn a_section_list_is_searched_rather_than_assumed_ordered() {
    let buf = [0u8; 0x1000];
    let pe = mapped(
        &buf,
        vec![
            section(0x3000, 0x100, 0x800, 0x100),
            section(0x1000, 0x100, 0x400, 0x100),
        ],
    );

    assert_eq!(pe.offset_of(0x1010), Ok(0x410));
    assert_eq!(pe.offset_of(0x3010), Ok(0x810));
}

#[test]
fn a_name_reads_up_to_its_terminator() {
    let mut buf = vec![0u8; 0x400];
    buf.extend_from_slice(b"d3d12.dll");
    buf.push(0);
    buf.extend_from_slice(b"dxgi.dll");
    buf.push(0);
    let pe = mapped(&buf, vec![section(0x1000, 0x100, 0x400, 0x100)]);

    assert_eq!(pe.cstr_at_rva(0x1000), Ok("d3d12.dll".to_owned()));
    assert_eq!(pe.cstr_at_rva(0x100a), Ok("dxgi.dll".to_owned()));
}

#[test]
fn a_name_with_no_terminator_is_an_error_and_not_an_empty_string() {
    // An unterminated name is an error, not an empty name.
    let buf = vec![b'A'; 0x400 + MAX_NAME + 16];
    let pe = mapped(&buf, vec![section(0x1000, 0x400, 0x400, 0x400)]);

    assert_eq!(
        pe.cstr_at_rva(0x1000),
        Err(Error::UnterminatedName { at: 0x400 })
    );
}

#[test]
fn a_name_is_read_when_its_terminator_sits_just_inside_the_limit() {
    let mut buf = vec![b'A'; 0x400 + MAX_NAME];
    buf[0x400 + MAX_NAME - 1] = 0;
    let pe = mapped(&buf, vec![section(0x1000, 0x400, 0x400, 0x400)]);

    let name = pe
        .cstr_at_rva(0x1000)
        .expect("terminator is within MAX_NAME");
    assert_eq!(name.len(), MAX_NAME - 1);
}

#[test]
fn an_image_whose_directory_table_is_short_reports_no_imports_rather_than_failing() {
    // An image may declare fewer than 14 directories.
    let buf = [0u8; 0x600];
    let mut pe = mapped(&buf, vec![section(0x1000, 0x200, 0x400, 0x200)]);
    pe.directories = vec![(0, 0), (0, 0)];

    assert_eq!(pe.imports(), Ok(Vec::new()));
    assert_eq!(pe.delay_imports(), Ok(Vec::new()));
}

#[test]
fn a_16_bit_new_executable_is_rejected_rather_than_read_as_a_pe() {
    // An NE image behind a valid MZ header, like six OLE1 files in SysWOW64.
    // The signature check stops the parser there.
    let mut buf = vec![0u8; 0x500];
    buf[0] = b'M';
    buf[1] = b'Z';
    buf[0x3c] = 0x00;
    buf[0x3d] = 0x04; // e_lfanew = 0x400
    buf[0x400] = b'N';
    buf[0x401] = b'E';

    assert_eq!(Pe::parse(&buf).unwrap_err(), Error::NotPe);
}

#[test]
fn mz_without_a_pe_signature_is_rejected() {
    let mut buf = vec![0u8; 0x100];
    buf[0] = b'M';
    buf[1] = b'Z';
    buf[0x3c] = 0x80; // e_lfanew points at zeroes, not at "PE"

    assert_eq!(Pe::parse(&buf).unwrap_err(), Error::NotPe);
}

#[test]
fn an_e_lfanew_pointing_past_the_file_is_truncation_not_a_bad_signature() {
    let mut buf = vec![0u8; 0x100];
    buf[0] = b'M';
    buf[1] = b'Z';
    buf[0x3d] = 0xff; // e_lfanew = 0xff00, far past the end

    assert_eq!(
        Pe::parse(&buf).unwrap_err(),
        Error::Truncated { at: 0xff00 }
    );
}
