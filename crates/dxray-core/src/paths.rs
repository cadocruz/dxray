//! Bounded discovery of container homes on mounted data volumes.
//!
//! Distrobox users commonly keep a container home at
//! `<mount>/data/distrobox/<name>`.  This is not a general filesystem search:
//! only direct children of those conventional directories on mounted volumes
//! are considered.  Callers still validate the specific Steam or Heroic path
//! below each candidate before using it.

use std::{collections::HashSet, ffi::OsString, fs, path::PathBuf};

/// Environment override for the bounded mounted-container discovery.
///
/// When present, this is a platform-native path list of Distrobox homes to
/// inspect. An empty value deliberately disables the automatic mount scan.
/// That is useful for callers which have already isolated their normal home
/// and need discovery to remain confined to that fixture. When absent, normal
/// runtime discovery reads the mount table as usual.
const CONTAINER_HOMES_ENV: &str = "DXRAY_CONTAINER_HOMES";

/// Conventional Distrobox home roots found on mounted external data volumes.
#[cfg(unix)]
pub(crate) fn container_homes() -> Vec<PathBuf> {
    if let Some(homes) = configured_container_homes() {
        return homes;
    }
    let mounts = fs::read_to_string("/proc/self/mountinfo")
        .map(|text| mount_points(&text))
        .unwrap_or_default();
    container_homes_under(&mounts)
}

#[cfg(not(unix))]
pub(crate) fn container_homes() -> Vec<PathBuf> {
    Vec::new()
}

fn configured_container_homes() -> Option<Vec<PathBuf>> {
    parse_container_homes(std::env::var_os(CONTAINER_HOMES_ENV))
}

fn parse_container_homes(value: Option<OsString>) -> Option<Vec<PathBuf>> {
    value.map(|homes| {
        if homes.is_empty() {
            Vec::new()
        } else {
            std::env::split_paths(&homes).collect()
        }
    })
}

#[cfg(unix)]
fn mount_points(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| line.split(" - ").next())
        .filter_map(|left| left.split_ascii_whitespace().nth(4))
        .map(unescape_mount_path)
        .map(PathBuf::from)
        .filter(|path| {
            path.starts_with("/media") || path.starts_with("/run/media") || path.starts_with("/mnt")
        })
        .collect()
}

/// Reverses the kernel's `mangle_path()` over one `mountinfo` field.
///
/// The kernel escapes exactly four bytes — space, tab, newline and backslash —
/// as three-digit octal, and passes every other byte through untouched. A mount
/// point is therefore a byte string, not text: `/media/café` arrives as its two
/// raw UTF-8 bytes for `é`, and a volume named in a non-UTF-8 encoding arrives
/// as whatever bytes the filesystem holds. Decoding byte by byte into `char`
/// would sign-extend each one into its own code point, turning `café` into
/// `cafÃ©` — a path that silently matches nothing. Bytes are collected instead
/// and handed back as an [`OsString`], which is what the kernel gave us and
/// what [`PathBuf`] wants.
#[cfg(unix)]
fn unescape_mount_path(path: &str) -> OsString {
    use std::os::unix::ffi::OsStringExt;

    let mut out: Vec<u8> = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        at += 1;
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        // Only a full three-digit octal escape is one; anything else is a
        // literal backslash in the name, which the kernel would itself have
        // escaped, so it is left exactly as it was read rather than guessed at.
        if let Some(digits) = bytes.get(at..at + 3)
            && let [a, b, c] = *digits
            && let Some(value) = octal(a, b, c)
        {
            out.push(value);
            at += 3;
            continue;
        }
        out.push(b'\\');
    }
    OsString::from_vec(out)
}

/// The byte a three-digit octal escape names, or `None` if those three digits
/// are not one: `\\8`, `\\12x` and an out-of-range `\\400` are all ordinary text.
#[cfg(unix)]
fn octal(a: u8, b: u8, c: u8) -> Option<u8> {
    let digit = |byte: u8| (b'0'..=b'7').contains(&byte).then_some(byte - b'0');
    let value = u32::from(digit(a)?) * 64 + u32::from(digit(b)?) * 8 + u32::from(digit(c)?);
    u8::try_from(value).ok()
}

#[cfg(unix)]
fn container_homes_under(mounts: &[PathBuf]) -> Vec<PathBuf> {
    let mut homes = Vec::new();
    let mut seen = HashSet::new();
    for mount in mounts {
        for base in [mount.join("data/distrobox"), mount.join("distrobox")] {
            let Ok(entries) = fs::read_dir(base) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                for home in [path.clone(), path.join("home"), path.join("root")] {
                    let identity = fs::canonicalize(&home).unwrap_or_else(|_| home.clone());
                    if home.is_dir() && seen.insert(identity) {
                        homes.push(home);
                    }
                }
            }
        }
    }
    homes
}

#[cfg(all(test, unix))]
mod tests {
    use super::{container_homes_under, mount_points, parse_container_homes, unescape_mount_path};
    use crate::testutil::TempDir;

    #[test]
    fn mountinfo_paths_are_unescaped_and_limited_to_data_mounts() {
        assert_eq!(
            unescape_mount_path("/media/a\\040disk"),
            std::ffi::OsStr::new("/media/a disk")
        );
        let paths = mount_points(
            "1 0 8:1 / /media/a\\040disk rw - ext4 /dev/sda rw\n2 0 0:2 / /proc rw - proc proc rw\n",
        );
        assert_eq!(paths, vec![std::path::PathBuf::from("/media/a disk")]);
    }

    #[test]
    fn non_ascii_mount_names_survive_unescaping_byte_for_byte() {
        // `mangle_path()` escapes space, tab, newline and backslash and nothing
        // else, so every byte of a UTF-8 name arrives raw. Decoding one byte at
        // a time turned `café` into `cafÃ©`: a directory that exists on the
        // volume and never matches anything this module looks for.
        for name in [
            "/media/café",
            "/media/日本語ディスク",
            "/run/media/user/Übisoft",
        ] {
            assert_eq!(
                unescape_mount_path(name),
                std::ffi::OsStr::new(name),
                "a name with no escapes in it has to come back unchanged"
            );
        }
        assert_eq!(
            unescape_mount_path("/media/caf\\303\\251 disk"),
            std::ffi::OsStr::new("/media/café disk"),
            "and octal escapes have to reassemble into the bytes they name"
        );
    }

    #[test]
    fn a_backslash_that_is_not_an_octal_escape_is_left_alone() {
        // `\400` is three digits that are not a byte, and `\8` is not octal at
        // all. Both are literal text in the name; neither may be dropped, and
        // the arithmetic behind them must not overflow.
        assert_eq!(
            unescape_mount_path("/media/a\\400b"),
            std::ffi::OsStr::new("/media/a\\400b")
        );
        assert_eq!(
            unescape_mount_path("/media/a\\8b"),
            std::ffi::OsStr::new("/media/a\\8b")
        );
        assert_eq!(
            unescape_mount_path("/media/trailing\\"),
            std::ffi::OsStr::new("/media/trailing\\")
        );
    }

    #[test]
    fn direct_distrobox_homes_are_found_without_a_recursive_walk() {
        let volume = TempDir::new("distrobox-volume");
        let home = volume.dir("data/distrobox/gaming");
        volume.dir("data/distrobox/gaming/home");
        let found = container_homes_under(&[volume.path().to_path_buf()]);
        assert!(found.contains(&home));
        assert!(found.contains(&home.join("home")));
    }

    #[test]
    fn explicit_empty_container_home_list_disables_the_mount_fallback() {
        // The process environment is shared by unit tests, so test parsing
        // directly rather than mutating it here.
        assert_eq!(parse_container_homes(Some("".into())), Some(Vec::new()));
    }
}
