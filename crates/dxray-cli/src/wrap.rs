//! Word wrapping, shared by the listings that print sentences.

/// Wrap width, in characters rather than bytes, so non-ASCII titles wrap
/// right. A wide CJK character still counts as one.
pub const WIDTH: usize = 80;

/// Appends `text` to `out`, wrapping at [`WIDTH`] under `indent`. `column` is
/// where the caller's label left the cursor. A word too long for the line, such
/// as a path, overflows rather than being split.
pub fn prose(out: &mut String, column: usize, indent: usize, text: &str) {
    let mut column = column;
    for (i, word) in text.split_whitespace().enumerate() {
        let width = word.chars().count();
        if i > 0 {
            if column + 1 + width > WIDTH {
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
        column += width;
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
    fn a_line_is_measured_in_characters_rather_than_bytes() {
        // Cyrillic and ASCII wrap alike.
        let mut cyrillic = String::new();
        prose(&mut cyrillic, 4, 4, &"Ведьмак ".repeat(30));
        let mut latin = String::new();
        prose(&mut latin, 4, 4, &"Witcher ".repeat(30));

        assert!(cyrillic.lines().count() > 1, "got:\n{cyrillic}");
        assert_eq!(
            cyrillic.lines().count(),
            latin.lines().count(),
            "the same shape has to wrap the same way, got:\n{cyrillic}"
        );
        assert!(
            cyrillic.lines().all(|line| line.chars().count() <= WIDTH),
            "and no line may overflow, got:\n{cyrillic}"
        );
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
