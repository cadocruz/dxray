//! Reads a PE image without loading it.
//!
//! Everything here works on a byte slice that came off disk. Nothing is mapped,
//! nothing is executed, and no header is trusted: every field is bounds-checked
//! against the slice it came from, because these files are attacker-shaped by
//! definition — they are game executables from the internet.

use std::fmt;

/// Index of the import table in the optional header's data directory.
const DIR_IMPORT: usize = 1;
/// Index of the delay-load import table.
const DIR_DELAY_IMPORT: usize = 13;

/// A descriptor array is terminated by an all-zero entry. This caps the walk in
/// case the terminator was lost to truncation or was never written.
const MAX_DESCRIPTORS: usize = 4096;
/// Longest DLL name accepted. Real ones are far shorter; this only stops a scan
/// that would otherwise run to the end of the file.
const MAX_NAME: usize = 256;

/// How many data directory entries the PE format defines. Used only to bound
/// what [`Pe::parse`] reserves up front; a header declaring more is still read
/// to the end of what the file actually holds.
const MAX_DIRECTORIES: usize = 16;
/// Size of one `IMAGE_SECTION_HEADER`, which is what bounds how many of them a
/// buffer of a given length could possibly contain.
const SECTION_HEADER: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A field ran past the end of the buffer.
    Truncated { at: usize },
    /// No `MZ` at offset 0.
    NotMz,
    /// `e_lfanew` did not point at a `PE\0\0` signature.
    NotPe,
    /// Optional header magic was neither PE32 (0x10b) nor PE32+ (0x20b).
    BadOptionalMagic(u16),
    /// An RVA fell outside every section.
    UnmappedRva(u32),
    /// A name ran for `MAX_NAME` bytes without a terminator. Reported rather
    /// than truncated, because a silently empty DLL name reads as "imports
    /// nothing" and that is a wrong answer wearing a valid one's clothes.
    UnterminatedName { at: usize },
    /// A resource directory entry pointed at a subdirectory where a leaf was
    /// expected, or the reverse.
    MalformedResourceTree { at: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { at } => write!(f, "truncated at offset {at:#x}"),
            Self::NotMz => f.write_str("not a DOS image: missing MZ"),
            Self::NotPe => f.write_str("not a PE image: missing PE signature"),
            Self::BadOptionalMagic(m) => write!(f, "unknown optional header magic {m:#06x}"),
            Self::UnmappedRva(rva) => write!(f, "rva {rva:#x} is in no section"),
            Self::UnterminatedName { at } => {
                write!(f, "unterminated name at offset {at:#x}")
            }
            Self::MalformedResourceTree { at } => {
                write!(f, "malformed resource directory at offset {at:#x}")
            }
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

mod version;

pub use version::{Version, VersionInfo};

/// The COFF `Machine` field, which is what actually tells 32-bit from 64-bit —
/// the file extension never does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Machine {
    I386,
    Amd64,
    Arm64,
    Other(u16),
}

impl Machine {
    #[must_use]
    pub const fn from_u16(v: u16) -> Self {
        match v {
            0x014c => Self::I386,
            0x8664 => Self::Amd64,
            0xaa64 => Self::Arm64,
            other => Self::Other(other),
        }
    }
}

impl fmt::Display for Machine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::I386 => f.write_str("x86"),
            Self::Amd64 => f.write_str("x86-64"),
            Self::Arm64 => f.write_str("arm64"),
            Self::Other(v) => write!(f, "unknown({v:#06x})"),
        }
    }
}

/// Just enough of a section header to translate an RVA into a file offset.
#[derive(Debug, Clone, Copy)]
struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_pointer: u32,
    raw_size: u32,
}

/// A parsed PE image, borrowing the bytes it was read from.
#[derive(Debug)]
pub struct Pe<'a> {
    buf: &'a [u8],
    machine: Machine,
    pe32_plus: bool,
    sections: Vec<Section>,
    /// `(rva, size)` per data directory entry, in file order.
    directories: Vec<(u32, u32)>,
}

impl<'a> Pe<'a> {
    /// Parses the headers of `buf`. Section contents are read lazily by the
    /// accessors, so a file whose import table is corrupt still yields a
    /// `Machine`.
    ///
    /// # Errors
    /// Returns [`Error`] when the buffer is not a PE image or a header field
    /// runs past its end.
    pub fn parse(buf: &'a [u8]) -> Result<Self> {
        if read_u16(buf, 0)? != 0x5a4d {
            return Err(Error::NotMz);
        }
        let pe_offset = read_u32(buf, 0x3c)? as usize;
        if read_u32(buf, pe_offset)? != 0x0000_4550 {
            return Err(Error::NotPe);
        }

        // COFF header, 20 bytes, immediately after the 4-byte signature.
        let coff = pe_offset + 4;
        let machine = Machine::from_u16(read_u16(buf, coff)?);
        let section_count = read_u16(buf, coff + 2)? as usize;
        let optional_size = read_u16(buf, coff + 16)? as usize;

        let optional = coff + 20;
        let magic = read_u16(buf, optional)?;
        // The two layouts differ only in that PE32+ widens four fields, which
        // pushes the directory count and the directories themselves 16 bytes on.
        let (pe32_plus, dir_count_at) = match magic {
            0x010b => (false, optional + 92),
            0x020b => (true, optional + 108),
            other => return Err(Error::BadOptionalMagic(other)),
        };

        let dir_count = read_u32(buf, dir_count_at)? as usize;
        // Reserved against what a file could hold, never against what it
        // claims. `NumberOfRvaAndSizes` is four unvalidated bytes, and
        // `with_capacity(0xFFFF_FFFF)` on a 1 KB file asks for 34 GB and
        // aborts the process — an abort is not an `Err`, so a caller cannot
        // skip the file and a directory walk dies on the first hostile one.
        // The loop still reads every entry the header declared and still fails
        // on the first that runs past the end, so what parses is unchanged.
        let mut directories = Vec::with_capacity(dir_count.min(MAX_DIRECTORIES));
        for i in 0..dir_count {
            let at = dir_count_at + 4 + i * 8;
            directories.push((read_u32(buf, at)?, read_u32(buf, at + 4)?));
        }

        // Section headers follow the optional header, whose length the COFF
        // header states rather than the magic implying it.
        // Same rule, smaller number: `NumberOfSections` is a `u16`, so the
        // worst case is 2 MB rather than an abort, but it is still 2 MB
        // reserved for section headers a 1 KB file cannot contain.
        let mut sections = Vec::with_capacity(section_count.min(buf.len() / SECTION_HEADER));
        for i in 0..section_count {
            let at = optional + optional_size + i * SECTION_HEADER;
            sections.push(Section {
                virtual_size: read_u32(buf, at + 8)?,
                virtual_address: read_u32(buf, at + 12)?,
                raw_size: read_u32(buf, at + 16)?,
                raw_pointer: read_u32(buf, at + 20)?,
            });
        }

        Ok(Self {
            buf,
            machine,
            pe32_plus,
            sections,
            directories,
        })
    }

    #[must_use]
    pub const fn machine(&self) -> Machine {
        self.machine
    }

    /// True for PE32+, which is the honest answer to "is this a 64-bit build".
    #[must_use]
    pub const fn is_64bit(&self) -> bool {
        self.pe32_plus
    }

    /// DLL names from the import table: what the loader resolves before the
    /// program's first instruction runs.
    ///
    /// # Errors
    /// Returns [`Error`] when a descriptor or name runs outside the image.
    pub fn imports(&self) -> Result<Vec<String>> {
        // 20-byte IMAGE_IMPORT_DESCRIPTOR; the name RVA sits at +12.
        self.descriptor_names(DIR_IMPORT, 20, 12)
    }

    /// DLL names from the delay-load table: resolved on first call instead of
    /// at load. Missing these is how a renderer gets misread, because a game
    /// that delay-loads `d3d12.dll` imports nothing at all at startup.
    ///
    /// # Errors
    /// Returns [`Error`] when a descriptor or name runs outside the image.
    pub fn delay_imports(&self) -> Result<Vec<String>> {
        // 32-byte IMAGE_DELAYLOAD_DESCRIPTOR; the name RVA sits at +4.
        self.descriptor_names(DIR_DELAY_IMPORT, 32, 4)
    }

    fn descriptor_names(&self, dir: usize, stride: usize, name_at: usize) -> Result<Vec<String>> {
        let Some(&(rva, _)) = self.directories.get(dir) else {
            return Ok(Vec::new());
        };
        if rva == 0 {
            return Ok(Vec::new());
        }

        let mut names = Vec::new();
        for i in 0..MAX_DESCRIPTORS {
            let step = u32::try_from(i * stride).unwrap_or(u32::MAX);
            let at = self.offset_of(rva.saturating_add(step))?;
            let descriptor = self
                .buf
                .get(at..at.saturating_add(stride))
                .ok_or(Error::Truncated { at })?;
            if descriptor.iter().all(|&b| b == 0) {
                break;
            }
            let name_rva = read_u32(descriptor, name_at)?;
            if name_rva == 0 {
                break;
            }
            names.push(self.cstr_at_rva(name_rva)?);
        }
        Ok(names)
    }

    /// Translates an RVA to an index into the file, using the section that
    /// covers it. A section's mapped size can exceed what is stored on disk, so
    /// the larger of the two decides coverage.
    fn offset_of(&self, rva: u32) -> Result<usize> {
        for s in &self.sections {
            let span = s.virtual_size.max(s.raw_size);
            if rva >= s.virtual_address && rva < s.virtual_address.saturating_add(span) {
                let delta = rva - s.virtual_address;
                if delta >= s.raw_size {
                    // Inside the mapped section but past what the file stores:
                    // zero-fill at run time, nothing to read here.
                    return Err(Error::UnmappedRva(rva));
                }
                // Both halves are header fields, so their sum is attacker
                // controlled too. Unchecked this panics in debug and wraps in
                // release, and a wrapped offset lands somewhere else in the
                // file that still reads as a perfectly good DLL name — a
                // confident answer about bytes the header never pointed at.
                let at = s
                    .raw_pointer
                    .checked_add(delta)
                    .ok_or(Error::UnmappedRva(rva))?;
                return Ok(at as usize);
            }
        }
        Err(Error::UnmappedRva(rva))
    }

    fn cstr_at_rva(&self, rva: u32) -> Result<String> {
        let at = self.offset_of(rva)?;
        let tail = self.buf.get(at..).ok_or(Error::Truncated { at })?;
        let window = &tail[..tail.len().min(MAX_NAME)];
        let end = window
            .iter()
            .position(|&b| b == 0)
            .ok_or(Error::UnterminatedName { at })?;
        Ok(String::from_utf8_lossy(&window[..end]).into_owned())
    }
}

// The end of each window saturates rather than wrapping. Every `at` reaching
// these is derived from a header field, and the only reason none of them can
// reach the top of `usize` today is a chain of bounds elsewhere in this file;
// resting a panic on that chain staying intact is how the overflow in
// `offset_of` came to be. A saturated end is past the buffer, so it reports
// `Truncated` at the offset the caller asked for, which is the same answer any
// other out-of-range read gives.
fn read_u16(buf: &[u8], at: usize) -> Result<u16> {
    let bytes = buf
        .get(at..at.saturating_add(2))
        .ok_or(Error::Truncated { at })?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(buf: &[u8], at: usize) -> Result<u32> {
    let bytes = buf
        .get(at..at.saturating_add(4))
        .ok_or(Error::Truncated { at })?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests;
