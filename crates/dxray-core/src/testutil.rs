//! A throwaway directory for tests, and the smallest PE image the parser will
//! accept. Small enough to keep here rather than take a dependency for, and
//! shared so that [`evidence`](crate::evidence), [`steam`](crate::steam) and
//! [`install`](crate::install) do not each grow their own copy.
//!
//! The images are assembled rather than copied off the system, because a test
//! that reads `C:\Windows` passes or fails for reasons that have nothing to do
//! with this crate.

// Every size below is a small constant chosen in this file, so the cast lints
// have nothing real to warn about. `dead_code` because each test module uses a
// different part of this.
#![allow(dead_code, clippy::cast_possible_truncation)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Makes each directory name unique. The process id alone is not enough: every
/// test in a crate runs in one process, and several of them run at once.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A directory under the system temp dir that removes itself when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!("dxray-core-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Creates a one-byte file called `name` directly in the directory.
    pub fn file(&self, name: &str) -> PathBuf {
        self.write(name, "x")
    }

    /// Writes `contents` to `rel`, creating any parent directories it names.
    pub fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent dirs");
        }
        std::fs::write(&path, contents).expect("write");
        path
    }

    /// Creates the directory `rel`, and every parent it names.
    pub fn dir(&self, rel: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(&path).expect("create dir");
        path
    }
}

impl TempDir {
    /// Writes a PE image at `rel` that imports `imports` and delay-loads
    /// `delayed`, creating any parent directories the path names.
    pub fn image(&self, rel: &str, imports: &[&str], delayed: &[&str]) -> PathBuf {
        self.write_image(rel, &Image::new(imports, delayed))
    }

    /// The same, with a `VS_FIXEDFILEINFO` carrying `file` and `product`.
    ///
    /// `[0, 0, 0, 0]` is a legitimate argument and produces a resource that is
    /// present and all zeroes, which is a real state on a real Windows install
    /// and must not come back looking like a missing one.
    pub fn versioned_image(
        &self,
        rel: &str,
        imports: &[&str],
        file: [u16; 4],
        product: [u16; 4],
    ) -> PathBuf {
        self.write_image(rel, &Image::new(imports, &[]).versioned(file, product))
    }

    fn write_image(&self, rel: &str, image: &Image) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent dirs");
        }
        std::fs::write(&path, image.build()).expect("write image");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PE_SIGNATURE: usize = 0x80;
const COFF: usize = PE_SIGNATURE + 4;
const OPTIONAL: usize = COFF + 20;

const DIR_IMPORT: usize = 1;
const DIR_DELAY_IMPORT: usize = 13;

const DIR_RESOURCE: usize = 2;

const RDATA_RVA: u32 = 0x1000;
const RDATA_RAW: usize = 0x400;
/// The resource section, mapped clear of `.rdata` and stored where the fixed
/// 0x800-byte buffer below ends.
const RSRC_RVA: u32 = 0x2000;
const RSRC_RAW: usize = 0x800;

const RT_VERSION: u32 = 16;
const FIXED_INFO_SIGNATURE: u32 = 0xFEEF_04BD;
const HIGH_BIT: u32 = 0x8000_0000;
/// Where the delay-load descriptors and the name pool start inside `.rdata`,
/// far enough apart that neither table has to know the other's length.
const DELAY_AT: usize = 0x100;
const NAMES_AT: usize = 0x200;

/// A 64-bit PE image with nothing in it but the two import tables and, when
/// asked for, a version resource.
pub struct Image {
    imports: Vec<String>,
    delayed: Vec<String>,
    /// `(file, product)`, each in display order.
    version: Option<([u16; 4], [u16; 4])>,
}

impl Image {
    pub fn new(imports: &[&str], delayed: &[&str]) -> Self {
        Self {
            imports: imports.iter().map(|&n| n.to_owned()).collect(),
            delayed: delayed.iter().map(|&n| n.to_owned()).collect(),
            version: None,
        }
    }

    #[must_use]
    pub fn versioned(mut self, file: [u16; 4], product: [u16; 4]) -> Self {
        self.version = Some((file, product));
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let mut buf = vec![0u8; RSRC_RAW];
        buf[0] = b'M';
        buf[1] = b'Z';
        put32(&mut buf, 0x3c, PE_SIGNATURE as u32);
        buf[PE_SIGNATURE..PE_SIGNATURE + 4].copy_from_slice(&[b'P', b'E', 0, 0]);

        // PE32+ keeps the directory count 108 bytes into the optional header;
        // the COFF header states that header's length either way.
        let count_at = OPTIONAL + 108;
        let dirs = count_at + 4;
        let optional_size = dirs + 16 * 8 - OPTIONAL;

        put16(&mut buf, COFF, 0x8664);
        put16(&mut buf, COFF + 2, 1 + u16::from(self.version.is_some()));
        put16(&mut buf, COFF + 16, optional_size as u16);
        put16(&mut buf, OPTIONAL, 0x20b);
        put32(&mut buf, count_at, 16);

        let section = OPTIONAL + optional_size;
        buf[section..section + 6].copy_from_slice(b".rdata");
        put32(&mut buf, section + 8, 0x400);
        put32(&mut buf, section + 12, RDATA_RVA);
        put32(&mut buf, section + 16, 0x400);
        put32(&mut buf, section + 20, RDATA_RAW as u32);

        let rsrc = self
            .version
            .map(|(file, product)| resource_section(file, product));
        if let Some(rsrc) = &rsrc {
            put32(&mut buf, dirs + DIR_RESOURCE * 8, RSRC_RVA);
            put32(&mut buf, dirs + DIR_RESOURCE * 8 + 4, rsrc.len() as u32);
            let at = section + 40;
            buf[at..at + 5].copy_from_slice(b".rsrc");
            put32(&mut buf, at + 8, rsrc.len() as u32);
            put32(&mut buf, at + 12, RSRC_RVA);
            put32(&mut buf, at + 16, rsrc.len() as u32);
            put32(&mut buf, at + 20, RSRC_RAW as u32);
        }

        // The name pool grows forwards while the two descriptor tables are
        // written at fixed offsets, so neither depends on the other's size.
        let mut names = NAMES_AT;
        if !self.imports.is_empty() {
            put32(&mut buf, dirs + DIR_IMPORT * 8, RDATA_RVA);
            put32(&mut buf, dirs + DIR_IMPORT * 8 + 4, 20);
            for (i, name) in self.imports.iter().enumerate() {
                let rva = push_name(&mut buf, &mut names, name);
                put32(&mut buf, RDATA_RAW + i * 20 + 12, rva);
            }
        }
        if !self.delayed.is_empty() {
            let rva = RDATA_RVA + DELAY_AT as u32;
            put32(&mut buf, dirs + DIR_DELAY_IMPORT * 8, rva);
            put32(&mut buf, dirs + DIR_DELAY_IMPORT * 8 + 4, 32);
            for (i, name) in self.delayed.iter().enumerate() {
                let name_rva = push_name(&mut buf, &mut names, name);
                put32(&mut buf, RDATA_RAW + DELAY_AT + i * 32 + 4, name_rva);
            }
        }

        if let Some(rsrc) = &rsrc {
            buf.extend_from_slice(rsrc);
        }
        buf
    }
}

/// A resource table: type -> name -> language, a data entry, then the blob.
///
/// Each directory is a 16-byte header plus one 8-byte entry, so the three
/// levels land at 0, 24 and 48 and the data entry at 72.
fn resource_section(file: [u16; 4], product: [u16; 4]) -> Vec<u8> {
    const LEVEL: usize = 24;
    let data_entry = LEVEL * 3;
    let blob_at = data_entry + 16;

    let mut section = vec![0u8; blob_at];
    for (i, (id, target)) in [
        (RT_VERSION, LEVEL | (HIGH_BIT as usize)),
        (1, (LEVEL * 2) | (HIGH_BIT as usize)),
        // en-US, and a leaf: no high bit.
        (1033, data_entry),
    ]
    .into_iter()
    .enumerate()
    {
        // NumberOfIdEntries sits at +14.
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

/// Writes a NUL-terminated name into the pool and returns its RVA.
fn push_name(buf: &mut [u8], cursor: &mut usize, name: &str) -> u32 {
    let at = RDATA_RAW + *cursor;
    buf[at..at + name.len()].copy_from_slice(name.as_bytes());
    let rva = RDATA_RVA + *cursor as u32;
    *cursor += name.len() + 1;
    rva
}

fn put16(buf: &mut [u8], at: usize, v: u16) {
    buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put32(buf: &mut [u8], at: usize, v: u32) {
    buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
