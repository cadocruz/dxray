//! Turns the command line into the files to inspect. A failure becomes a
//! record like any other; the run never stops early.

use std::path::{Path, PathBuf};

use crate::record::Record;

/// One entry to inspect, or the failure that stood in for it.
pub enum Target {
    File(PathBuf),
    Failed(Box<Record>),
}

/// Expands `inputs` into the files to inspect, in argument order. A named path
/// is always inspected; only directory contents are filtered by extension.
pub fn collect(inputs: &[PathBuf], recursive: bool) -> Vec<Target> {
    let mut targets = Vec::new();
    for input in inputs {
        if input.is_dir() {
            walk(input, recursive, &mut targets);
        } else {
            targets.push(Target::File(input.clone()));
        }
    }
    targets
}

/// Breadth-first over an explicit queue rather than by recursion, so a deeply
/// nested tree costs memory instead of stack.
fn walk(root: &Path, recursive: bool, out: &mut Vec<Target>) {
    let mut queue = vec![root.to_path_buf()];
    while let Some(dir) = queue.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                out.push(Target::Failed(Box::new(Record::failed(&dir, &e))));
                continue;
            }
        };

        // Sorted, so output can be diffed between runs.
        let mut files = Vec::new();
        let mut subdirectories = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    out.push(Target::Failed(Box::new(Record::failed(&dir, &e))));
                    continue;
                }
            };
            let path = entry.path();
            // `file_type` does not follow symlinks, so a link pointing back up
            // the tree is never descended into and cannot loop forever.
            match entry.file_type() {
                Ok(t) if t.is_dir() => {
                    if recursive {
                        subdirectories.push(path);
                    }
                }
                Ok(_) => {
                    if is_image(&path) {
                        files.push(path);
                    }
                }
                Err(e) => out.push(Target::Failed(Box::new(Record::failed(&path, &e)))),
            }
        }

        files.sort_unstable();
        out.extend(files.into_iter().map(Target::File));

        subdirectories.sort_unstable();
        // Reversed, because the queue is popped from its end and subdirectories
        // should still be visited in sorted order.
        subdirectories.reverse();
        queue.extend(subdirectories);
    }
}

/// True for the extensions a PE image ships under, in any case.
fn is_image(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("dll"))
}

#[cfg(test)]
mod tests {
    use super::is_image;
    use std::path::Path;

    #[test]
    fn directory_scanning_accepts_both_spellings_of_an_extension() {
        // Game installs mix `.DLL` and `.dll` freely, and a case sensitive
        // match silently halves the results on a case sensitive filesystem.
        assert!(is_image(Path::new("a/d3d12.dll")));
        assert!(is_image(Path::new("a/D3D12.DLL")));
        assert!(is_image(Path::new("a/Game.Exe")));
    }

    #[test]
    fn directory_scanning_skips_everything_that_is_not_an_image() {
        // A game directory is mostly assets; parsing them all would turn a scan
        // into a pile of errors that hide the real ones.
        assert!(!is_image(Path::new("a/readme.txt")));
        assert!(!is_image(Path::new("a/pak0.pak")));
        assert!(!is_image(Path::new("a/noextension")));
        // `.exe.bak` is not an executable: only the last component counts.
        assert!(!is_image(Path::new("a/game.exe.bak")));
    }
}
