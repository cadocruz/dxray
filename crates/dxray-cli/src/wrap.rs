//! Word wrapping, shared by the listings that print sentences.
//!
//! Extracted when the ranked-candidate listing became the second thing in this
//! crate that had to wrap a paragraph of prose under a label. Three lines of
//! explanation running off the right edge of a terminal is a caveat nobody
//! finishes reading, which defeats printing it at all.

/// Wrap width. Terminals narrower than this exist; output that is impossible to
/// scan on a normal one because it was built for them does not.
pub const WIDTH: usize = 80;

/// Appends `text` to `out`, wrapping at [`WIDTH`] with every continuation line
/// starting at `indent`.
///
/// The caller has already written whatever sits to the left of `indent` on the
/// first line — a label, a rank — and `column` says where that left the cursor.
///
/// A single word longer than the space left, which is what a long path is, goes
/// on its own line and overflows rather than being split. Half a path is not
/// findable, and a search for it fails silently.
pub fn prose(out: &mut String, column: usize, indent: usize, text: &str) {
    let mut column = column;
    for (i, word) in text.split_whitespace().enumerate() {
        if i > 0 {
            if column + 1 + word.len() > WIDTH {
                out.push('\n');
                for _ in 0..indent {
                    out.push(' ');
                }
                column = indent;
            } else {
                out.push(' ');
                column += 1;
            }
        }
        out.push_str(word);
        column += word.len();
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::{WIDTH, prose};

    #[test]
    fn a_continuation_starts_under_the_column_it_was_given() {
        let mut out = String::new();
        prose(&mut out, 4, 4, &"word ".repeat(40));

        assert!(out.lines().count() > 1, "got:\n{out}");
        assert!(
            out.lines().skip(1).all(|l| l.starts_with("    word")),
            "got:\n{out}"
        );
        assert!(out.lines().all(|l| l.len() <= WIDTH), "got:\n{out}");
    }

    #[test]
    fn a_word_longer_than_the_line_overflows_rather_than_being_cut_in_half() {
        // It is always a path, and half of one is unsearchable.
        let long = "/".to_owned() + &"x".repeat(100);
        let mut out = String::new();
        prose(&mut out, 4, 4, &format!("at {long}"));

        assert!(out.contains(&long), "the path survives whole, got:\n{out}");
    }
}
