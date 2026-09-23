//! The version stamped into a binary's resources: the only way to tell one
//! build of `nvngx_dlss.dll` from another.

use std::fmt;

use crate::{Error, Pe, Result, read_u16, read_u32};

/// Index of the resource table in the optional header's data directory.
const DIR_RESOURCE: usize = 2;
/// `RT_VERSION`, the resource type that carries `VS_VERSIONINFO`.
const RT_VERSION: u32 = 16;
/// `VS_FIXEDFILEINFO.dwSignature`, little-endian `0xFEEF04BD`.
const FIXED_INFO_SIGNATURE: u32 = 0xFEEF_04BD;
/// A version resource is on the order of a kilobyte. This caps what is read in
/// case the size field says otherwise.
const MAX_VERSION_BLOB: usize = 64 << 10;
/// The high bit of a resource entry's offset marks a subdirectory; of its name,
/// a string rather than an integer id.
const HIGH_BIT: u32 = 0x8000_0000;

/// A four-part Windows version, as stored rather than as displayed.
///
/// Windows packs it into two `u32`s, high half first. `3.7.20.0` arrives as
/// `(0x0003_0007, 0x0014_0000)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
    pub build: u16,
}

impl Version {
    #[must_use]
    pub const fn from_parts(ms: u32, ls: u32) -> Self {
        Self {
            major: (ms >> 16) as u16,
            minor: (ms & 0xffff) as u16,
            patch: (ls >> 16) as u16,
            build: (ls & 0xffff) as u16,
        }
    }
}

impl fmt::Display for Version {
    /// All four parts, always. Trimming trailing zeroes would make `3.7.0.0`
    /// and `3.7` print the same, and these numbers get compared.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}.{}.{}",
            self.major, self.minor, self.patch, self.build
        )
    }
}

/// The two versions every `VS_FIXEDFILEINFO` carries. They often differ: a DLL
/// built for a product release gets the product's number and its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionInfo {
    pub file: Version,
    pub product: Version,
}

impl Pe<'_> {
    /// Reads `VS_FIXEDFILEINFO` out of the resource tree. `Ok(None)` when the
    /// image carries no version resource, which is ordinary.
    ///
    /// # Errors
    /// Returns [`Error`] when the resource tree is malformed or runs outside
    /// the image.
    pub fn version_info(&self) -> Result<Option<VersionInfo>> {
        let Some(blob) = self.resource(RT_VERSION)? else {
            return Ok(None);
        };

        // Find the signature inside this small, located blob rather than
        // recompute the key's padding.
        let Some(at) = (0..blob.len().saturating_sub(3))
            .step_by(4)
            .find(|&i| read_u32(blob, i) == Ok(FIXED_INFO_SIGNATURE))
        else {
            return Ok(None);
        };

        Ok(Some(VersionInfo {
            file: Version::from_parts(read_u32(blob, at + 8)?, read_u32(blob, at + 12)?),
            product: Version::from_parts(read_u32(blob, at + 16)?, read_u32(blob, at + 20)?),
        }))
    }

    /// Walks type, name and language, returning the first leaf of `type_id`.
    /// Other names and languages are localisations of the same number.
    fn resource(&self, type_id: u32) -> Result<Option<&[u8]>> {
        let Some(&(base_rva, _)) = self.directories.get(DIR_RESOURCE) else {
            return Ok(None);
        };
        if base_rva == 0 {
            return Ok(None);
        }
        let base = self.offset_of(base_rva)?;

        // Every offset in the tree is relative to the start of the table, not
        // to the file and not to the section.
        let mut at = base;
        for level in 0..3 {
            let want = if level == 0 { Some(type_id) } else { None };
            let Some((offset, is_directory)) = self.resource_entry(at, want)? else {
                return Ok(None);
            };
            let last = level == 2;
            if is_directory == last {
                return Err(Error::MalformedResourceTree { at });
            }
            at = base + offset;
        }

        // IMAGE_RESOURCE_DATA_ENTRY: its OffsetToData is an RVA, unlike every
        // other offset in the tree.
        let data_rva = read_u32(self.buf, at)?;
        let size = read_u32(self.buf, at + 4)? as usize;
        let start = self.offset_of(data_rva)?;
        let end = start
            .checked_add(size.min(MAX_VERSION_BLOB))
            .ok_or(Error::Truncated { at: start })?;

        self.buf
            .get(start..end)
            .ok_or(Error::Truncated { at: start })
            .map(Some)
    }

    /// One entry from a resource directory: `Some((offset, is_directory))`. With
    /// `want`, an integer id past the named entries; without, the first entry.
    fn resource_entry(&self, dir_at: usize, want: Option<u32>) -> Result<Option<(usize, bool)>> {
        let named = read_u16(self.buf, dir_at + 12)? as usize;
        let by_id = read_u16(self.buf, dir_at + 14)? as usize;
        let entries = dir_at + 16;

        let range = match want {
            Some(_) => named..named + by_id,
            None => 0..(named + by_id).min(1),
        };

        for i in range {
            let at = entries + i * 8;
            let name = read_u32(self.buf, at)?;
            let offset = read_u32(self.buf, at + 4)?;
            if let Some(id) = want
                && (name & HIGH_BIT != 0 || name != id)
            {
                continue;
            }
            return Ok(Some((
                (offset & !HIGH_BIT) as usize,
                offset & HIGH_BIT != 0,
            )));
        }
        Ok(None)
    }
}
