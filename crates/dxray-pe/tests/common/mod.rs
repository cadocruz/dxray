//! Builds PE images byte by byte, so every fixture byte is deliberate and
//! awkward shapes can be asked for.

// Each test binary uses part of this; all sizes are small local constants.
#![allow(dead_code, clippy::cast_possible_truncation)]

const PE_SIGNATURE: usize = 0x80;
const COFF: usize = PE_SIGNATURE + 4;
const OPTIONAL: usize = COFF + 20;

const DIR_IMPORT: usize = 1;
const DIR_RESOURCE: usize = 2;
const DIR_DELAY_IMPORT: usize = 13;

const RDATA_RVA: u32 = 0x1000;
const RSRC_RVA: u32 = 0x2000;
const RDATA_RAW: usize = 0x400;
const RSRC_RAW: usize = 0x800;

const RT_VERSION: u32 = 16;
const FIXED_INFO_SIGNATURE: u32 = 0xFEEF_04BD;
const HIGH_BIT: u32 = 0x8000_0000;

pub struct Image {
    machine: u16,
    pe32_plus: bool,
    imports: Vec<String>,
    delayed: Vec<String>,
    version: Option<([u16; 4], [u16; 4])>,
    resource_type: u32,
}

impl Image {
    pub fn x64() -> Self {
        Self {
            machine: 0x8664,
            pe32_plus: true,
            imports: Vec::new(),
            delayed: Vec::new(),
            version: None,
            resource_type: RT_VERSION,
        }
    }

    pub fn x86() -> Self {
        Self {
            machine: 0x014c,
            pe32_plus: false,
            ..Self::x64()
        }
    }

    #[must_use]
    pub fn importing(mut self, names: &[&str]) -> Self {
        self.imports = names.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    #[must_use]
    pub fn delay_loading(mut self, names: &[&str]) -> Self {
        self.delayed = names.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    /// `file` and `product` are the four parts in display order.
    #[must_use]
    pub fn versioned(mut self, file: [u16; 4], product: [u16; 4]) -> Self {
        self.version = Some((file, product));
        self
    }

    /// Files the version resource under a different `RT_*` type, to prove the
    /// walk matches the type rather than taking whatever is first.
    #[must_use]
    pub fn filed_under_type(mut self, type_id: u32) -> Self {
        self.resource_type = type_id;
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let import_bytes = (self.imports.len() + 1) * 20;
        let delay_bytes = (self.delayed.len() + 1) * 32;
        let rdata = self.import_section(import_bytes, delay_bytes);
        let rsrc = self
            .version
            .map(|(file, product)| resource_section(self.resource_type, file, product));

        let optional_size = if self.pe32_plus { 240 } else { 224 };
        let directory_count_at = OPTIONAL + if self.pe32_plus { 108 } else { 92 };
        let section_headers = OPTIONAL + optional_size;
        let section_count = 1 + usize::from(rsrc.is_some());

        let mut buf = vec![0u8; RDATA_RAW];
        buf[0] = b'M';
        buf[1] = b'Z';
        put32(&mut buf, 0x3c, PE_SIGNATURE as u32);
        buf[PE_SIGNATURE] = b'P';
        buf[PE_SIGNATURE + 1] = b'E';

        put16(&mut buf, COFF, self.machine);
        put16(&mut buf, COFF + 2, section_count as u16);
        put16(&mut buf, COFF + 16, optional_size as u16);

        put16(
            &mut buf,
            OPTIONAL,
            if self.pe32_plus { 0x020b } else { 0x010b },
        );
        put32(&mut buf, directory_count_at, 16);
        let directories = directory_count_at + 4;
        directory(
            &mut buf,
            directories,
            DIR_IMPORT,
            RDATA_RVA,
            import_bytes as u32,
        );
        directory(
            &mut buf,
            directories,
            DIR_DELAY_IMPORT,
            RDATA_RVA + import_bytes as u32,
            delay_bytes as u32,
        );

        header(
            &mut buf,
            section_headers,
            b".rdata",
            RDATA_RVA,
            RDATA_RAW,
            rdata.len(),
        );
        if let Some(rsrc) = &rsrc {
            directory(
                &mut buf,
                directories,
                DIR_RESOURCE,
                RSRC_RVA,
                rsrc.len() as u32,
            );
            header(
                &mut buf,
                section_headers + 40,
                b".rsrc",
                RSRC_RVA,
                RSRC_RAW,
                rsrc.len(),
            );
        }

        buf.extend_from_slice(&rdata);
        if let Some(rsrc) = &rsrc {
            buf.resize(RSRC_RAW, 0);
            buf.extend_from_slice(rsrc);
        }
        buf
    }

    /// Descriptors first, then the names they point at.
    fn import_section(&self, import_bytes: usize, delay_bytes: usize) -> Vec<u8> {
        let mut section = vec![0u8; import_bytes + delay_bytes];
        let mut rvas = (Vec::new(), Vec::new());
        for (names, out) in [(&self.imports, &mut rvas.0), (&self.delayed, &mut rvas.1)] {
            for name in names {
                out.push(RDATA_RVA + section.len() as u32);
                section.extend_from_slice(name.as_bytes());
                section.push(0);
            }
        }

        // IMAGE_IMPORT_DESCRIPTOR: 20 bytes, name RVA at +12. OriginalFirstThunk
        // is set so the entry is not the all-zero terminator.
        for (i, rva) in rvas.0.iter().enumerate() {
            put32(&mut section, i * 20, 1);
            put32(&mut section, i * 20 + 12, *rva);
        }
        // IMAGE_DELAYLOAD_DESCRIPTOR is 32 bytes with the name RVA at +4.
        for (i, rva) in rvas.1.iter().enumerate() {
            put32(&mut section, import_bytes + i * 32, 1);
            put32(&mut section, import_bytes + i * 32 + 4, *rva);
        }
        section
    }
}

/// A resource table: type → name → language, then a data entry, then the blob.
/// The levels land at 0, 24 and 48, and the data entry at 72.
fn resource_section(type_id: u32, file: [u16; 4], product: [u16; 4]) -> Vec<u8> {
    const LEVEL: usize = 24;
    let data_entry = LEVEL * 3;
    let blob_at = data_entry + 16;

    let mut section = vec![0u8; blob_at];
    // One integer-id entry per level; NumberOfIdEntries sits at +14.
    for (i, (id, target)) in [
        (type_id, LEVEL | (HIGH_BIT as usize)),
        (1, (LEVEL * 2) | (HIGH_BIT as usize)),
        (1033, data_entry), // en-US, and a leaf: no high bit
    ]
    .into_iter()
    .enumerate()
    {
        put16(&mut section, i * LEVEL + 14, 1);
        put32(&mut section, i * LEVEL + 16, id);
        put32(&mut section, i * LEVEL + 20, target as u32);
    }

    let blob = version_info(file, product);
    // IMAGE_RESOURCE_DATA_ENTRY: OffsetToData is an RVA, not a table offset.
    put32(&mut section, data_entry, RSRC_RVA + blob_at as u32);
    put32(&mut section, data_entry + 4, blob.len() as u32);
    section.extend_from_slice(&blob);
    section
}

/// `VS_VERSIONINFO`: a header, a UTF-16 key, padding to four bytes, then the
/// 52-byte `VS_FIXEDFILEINFO`.
fn version_info(file: [u16; 4], product: [u16; 4]) -> Vec<u8> {
    let mut blob = Vec::new();
    blob.extend_from_slice(&0u16.to_le_bytes()); // wLength, patched below
    blob.extend_from_slice(&52u16.to_le_bytes()); // wValueLength
    blob.extend_from_slice(&0u16.to_le_bytes()); // wType: binary
    for c in "VS_VERSION_INFO".chars() {
        blob.extend_from_slice(&(c as u16).to_le_bytes());
    }
    blob.extend_from_slice(&0u16.to_le_bytes()); // key terminator
    while blob.len() % 4 != 0 {
        blob.push(0);
    }

    let mut fixed = vec![0u8; 52];
    put32(&mut fixed, 0, FIXED_INFO_SIGNATURE);
    put32(&mut fixed, 4, 0x0001_0000); // dwStrucVersion
    put32(&mut fixed, 8, pack(file[0], file[1]));
    put32(&mut fixed, 12, pack(file[2], file[3]));
    put32(&mut fixed, 16, pack(product[0], product[1]));
    put32(&mut fixed, 20, pack(product[2], product[3]));
    blob.extend_from_slice(&fixed);

    let len = blob.len() as u16;
    put16(&mut blob, 0, len);
    blob
}

const fn pack(high: u16, low: u16) -> u32 {
    ((high as u32) << 16) | low as u32
}

fn directory(buf: &mut [u8], base: usize, index: usize, rva: u32, size: u32) {
    put32(buf, base + index * 8, rva);
    put32(buf, base + index * 8 + 4, size);
}

fn header(buf: &mut [u8], at: usize, name: &[u8], rva: u32, raw: usize, size: usize) {
    buf[at..at + name.len()].copy_from_slice(name);
    put32(buf, at + 8, size as u32); // VirtualSize
    put32(buf, at + 12, rva); // VirtualAddress
    put32(buf, at + 16, size as u32); // SizeOfRawData
    put32(buf, at + 20, raw as u32); // PointerToRawData
}

pub fn put16(buf: &mut [u8], at: usize, v: u16) {
    buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

pub fn put32(buf: &mut [u8], at: usize, v: u32) {
    buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// Offset of the optional header magic, which a test corrupts on purpose.
pub const OPTIONAL_MAGIC: usize = OPTIONAL;

// Offsets for shapes no linker emits, patched into an `Image::x64` build
// (240-byte optional header) rather than offered by the builder.

/// `NumberOfRvaAndSizes` in a PE32+ image: four unvalidated bytes that decide
/// how much the parser reserves.
pub const DIRECTORY_COUNT: usize = OPTIONAL + 108;
/// The data directory array, which follows the count.
pub const DIRECTORIES: usize = DIRECTORY_COUNT + 4;
/// The first `IMAGE_SECTION_HEADER`, which follows the optional header.
pub const FIRST_SECTION_HEADER: usize = OPTIONAL + 240;

/// Index of the import table, for tests that repoint it.
pub const IMPORT_DIRECTORY: usize = DIR_IMPORT;
