//! Valve's `KeyValues` text format. Pure: no paths, no filesystem.
//!
//! Steam keeps its install index and its per-game manifests in this format, so
//! everything this crate can say about a real machine's library starts by
//! reading it correctly. Three decisions below exist because the obvious
//! implementation gets a real file wrong.
//!
//! **Order is part of the data.** A `HashMap` would be the natural choice and
//! it would silently reorder `libraryfolders.vdf`, which numbers its entries
//! and is read back by index. Objects here keep insertion order.
//!
//! **Lookups are case-insensitive.** Valve's own parser treats keys that way
//! and Valve's own files rely on it: the root key of `libraryfolders.vdf` was
//! spelled `LibraryFolders` for years and is spelled `libraryfolders` now. A
//! case-sensitive `get` finds nothing on half the installs in the world and
//! reports it as an empty library.
//!
//! **A broken file is an error, never a short answer.** A truncated `.acf`
//! parsed leniently yields an object with no `installdir` in it, which reads
//! exactly like a game that is not installed. Every failure below names what
//! was expected and the line it was expected on.

#[cfg(test)]
mod tests;

use std::fmt;

/// How deep nesting may go before it is treated as malformed.
///
/// Exactly this many nested blocks parse and one more is an error, the same
/// boundary [`heroic`](crate::heroic)'s JSON reader draws with the same
/// constant.
///
/// The number is arbitrary; the bound is not. Steam's own files nest four
/// levels at the most, so 64 is more headroom than any real document needs.
/// Without *some* cap, a file of nothing but open braces overflows the stack,
/// and a stack overflow aborts the process — it is not an error a caller can
/// catch, so one malformed file would kill a whole library scan instead of
/// producing one bad record.
const MAX_DEPTH: usize = 64;

/// A parsed value: either a leaf string or a nested object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A quoted or bare token.
    String(String),
    /// A brace-delimited block.
    Object(Object),
}

impl Value {
    /// The string, if this is a leaf.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            Self::Object(_) => None,
        }
    }

    /// The object, if this is a block.
    #[must_use]
    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Self::Object(o) => Some(o),
            Self::String(_) => None,
        }
    }
}

/// An ordered key-to-value map.
///
/// Duplicate keys are kept rather than merged, because the file said them twice
/// and dropping one would make the parse unfaithful. [`Object::get`] answers
/// with the first, which is what Valve's reader does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Object {
    entries: Vec<(String, Value)>,
}

impl Object {
    /// The first value stored under `key`, compared without ASCII case.
    ///
    /// Case-insensitive because Valve's files are: see the module docs.
    /// Non-ASCII keys compare exactly, which matches Valve and matters to
    /// nobody, since every key in a Steam file is ASCII.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    /// The first value under `key`, if it is a leaf string.
    #[must_use]
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// The first value under `key`, if it is a nested object.
    #[must_use]
    pub fn get_object(&self, key: &str) -> Option<&Object> {
        self.get(key).and_then(Value::as_object)
    }

    /// Every pair, in the order the file spelled them.
    pub fn iter(&self) -> Pairs<'_> {
        self.entries.iter().map(borrow_pair as BorrowPair)
    }

    /// How many pairs the object holds, duplicates counted separately.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the object holds no pairs at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Borrows one stored pair. A named function rather than a closure so that
/// [`Object::iter`] and `IntoIterator for &Object` can share one concrete
/// return type instead of the second one allocating a `Vec` to match the first.
fn borrow_pair(entry: &(String, Value)) -> (&str, &Value) {
    (entry.0.as_str(), &entry.1)
}

type BorrowPair = for<'a> fn(&'a (String, Value)) -> (&'a str, &'a Value);

/// What iterating an [`Object`] yields.
pub type Pairs<'a> = std::iter::Map<std::slice::Iter<'a, (String, Value)>, BorrowPair>;

impl<'a> IntoIterator for &'a Object {
    type Item = (&'a str, &'a Value);
    type IntoIter = Pairs<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// What went wrong, and exactly where.
///
/// The position is carried separately from the kind so a caller can report the
/// file name beside it without having to re-parse the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    /// 1-based line of the offending byte.
    pub line: usize,
    /// 1-based column, counted in characters rather than bytes so it lines up
    /// with what an editor shows.
    pub column: usize,
    /// Byte offset into the string that was handed to [`parse`], including any
    /// byte-order mark.
    pub offset: usize,
}

/// The shapes a malformed `KeyValues` file comes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Input ended inside a block. This is what a truncated file looks like,
    /// and it is the reason this parser refuses to return a partial tree.
    UnclosedObject {
        /// Line of the `{` that was never matched.
        opened_line: usize,
    },
    /// A key was read and then the input ended, or a `}` arrived, before its
    /// value.
    MissingValue,
    /// A quoted string ran to the end of the input without closing.
    UnclosedString,
    /// A raw newline inside a quoted string. Almost always a missing closing
    /// quote, and accepting it swallows the rest of the file into one token.
    NewlineInString,
    /// A `}` with no block open.
    UnmatchedCloseBrace,
    /// A `{` where a key was expected — a block with no name.
    ExpectedKey,
    /// Nesting past `MAX_DEPTH`.
    TooDeep,
    /// A `[$WIN32]`-style platform conditional.
    ///
    /// A real part of `KeyValues` that this parser does not implement, and
    /// refuses rather than misreads. Left alone, the bracketed token would be
    /// taken for the next key and everything after it would shift by one, which
    /// is a file that parses cleanly and means something else. Erroring says
    /// what happened; guessing does not. These do not appear in
    /// `libraryfolders.vdf` or in an `appmanifest`, so reaching this is either a
    /// file from elsewhere or a corrupt one.
    PlatformConditional,
    /// A `#base` or `#include` directive.
    ///
    /// Also real, also unimplemented, and worse than the conditional if it were
    /// waved through: the file it names holds keys that would have been part of
    /// the document, so treating the directive as an ordinary pair leaves that
    /// data missing with nothing to say it ever existed.
    Directive,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            line, column, kind, ..
        } = self;
        match kind {
            ErrorKind::UnclosedObject { opened_line } => write!(
                f,
                "block opened on line {opened_line} was never closed \
                 (input ended at line {line}, column {column})"
            ),
            ErrorKind::MissingValue => {
                write!(f, "key with no value at line {line}, column {column}")
            }
            ErrorKind::UnclosedString => {
                write!(f, "unterminated string at line {line}, column {column}")
            }
            ErrorKind::NewlineInString => write!(
                f,
                "newline inside a quoted string at line {line}, column {column}"
            ),
            ErrorKind::UnmatchedCloseBrace => {
                write!(f, "unmatched '}}' at line {line}, column {column}")
            }
            ErrorKind::ExpectedKey => {
                write!(f, "expected a key at line {line}, column {column}")
            }
            ErrorKind::TooDeep => write!(
                f,
                "nested past {MAX_DEPTH} levels at line {line}, column {column}"
            ),
            ErrorKind::PlatformConditional => write!(
                f,
                "platform conditional at line {line}, column {column} is not supported; \
                 reading it as an ordinary key would shift every pair after it"
            ),
            ErrorKind::Directive => write!(
                f,
                "#base or #include directive at line {line}, column {column} is not \
                 supported; the keys it would pull in would be missing with nothing to say so"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Parses a `KeyValues` document into its root object.
///
/// The root is an object rather than a single pair because the format allows
/// more than one top-level key and some Steam files use that. A leading UTF-8
/// byte-order mark is skipped, CRLF is treated as whitespace, `//` runs to the
/// end of the line, and both quoted and bare tokens are accepted.
///
/// # Errors
///
/// Returns the first structural problem found, with the line and column it was
/// found on. Nothing partial is ever returned: see [`ErrorKind::UnclosedObject`].
pub fn parse(input: &str) -> Result<Value, Error> {
    let mut parser = Parser::new(input);
    let root = parser.parse_body(0, 1)?;
    Ok(Value::Object(root))
}

/// Byte-oriented cursor over the input.
///
/// Scanning bytes is safe here because every delimiter the grammar has is
/// ASCII, so a multi-byte character can only ever appear whole inside a token
/// and is copied through without being inspected.
/// A saved cursor position, used to report a fault against the token that
/// caused it rather than against wherever the scan noticed it.
#[derive(Clone, Copy)]
struct Mark {
    pos: usize,
    line: usize,
    line_start: usize,
}

struct Parser<'a> {
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    /// Byte offset of the start of the current line, for the column count.
    line_start: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        // A byte-order mark is not whitespace and not a key. Steam writes files
        // without one, but editors and sync tools add them, and a parser that
        // chokes on the first three bytes reports a healthy install as broken.
        let start = if input.starts_with('\u{feff}') { 3 } else { 0 };
        Self {
            input,
            bytes: input.as_bytes(),
            pos: start,
            line: 1,
            line_start: start,
        }
    }

    /// Where the cursor is now, so a fault noticed later can still be reported
    /// against the token that caused it. A key with no value is the case that
    /// needs this: by the time it is detected the cursor has walked past the
    /// key, and pointing at the following line helps nobody.
    fn mark(&self) -> Mark {
        Mark {
            pos: self.pos,
            line: self.line,
            line_start: self.line_start,
        }
    }

    fn error(&self, kind: ErrorKind) -> Error {
        self.error_at(kind, self.mark())
    }

    fn error_at(&self, kind: ErrorKind, mark: Mark) -> Error {
        Error {
            kind,
            line: mark.line,
            // Counted in characters: a byte column is off by the width of every
            // non-ASCII character earlier on the line, which is exactly the
            // case where a person needs the number to be right.
            column: self.input[mark.line_start..mark.pos].chars().count() + 1,
            offset: mark.pos,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump_line(&mut self) {
        self.pos += 1;
        self.line += 1;
        self.line_start = self.pos;
    }

    /// Skips whitespace and `//` comments until the next real byte.
    fn skip_trivia(&mut self) {
        while let Some(byte) = self.peek() {
            match byte {
                b'\n' => self.bump_line(),
                b' ' | b'\t' | b'\r' | 0x0b | 0x0c => self.pos += 1,
                b'/' if self.bytes.get(self.pos + 1) == Some(&b'/') => {
                    while let Some(b) = self.peek() {
                        if b == b'\n' {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                _ => return,
            }
        }
    }

    /// Reads the pairs of one block.
    ///
    /// At `depth` 0 the block is the whole document and ends at end of input;
    /// deeper it ends at the matching `}`. `opened_line` is where this block's
    /// `{` was, and is only used to make a truncation message point at the
    /// brace that is missing its partner rather than at the end of the file.
    fn parse_body(&mut self, depth: usize, opened_line: usize) -> Result<Object, Error> {
        let mut object = Object::default();
        loop {
            self.skip_trivia();
            let Some(byte) = self.peek() else {
                if depth == 0 {
                    return Ok(object);
                }
                return Err(self.error(ErrorKind::UnclosedObject { opened_line }));
            };

            match byte {
                b'}' => {
                    if depth == 0 {
                        return Err(self.error(ErrorKind::UnmatchedCloseBrace));
                    }
                    self.pos += 1;
                    return Ok(object);
                }
                // A block has to be named. Accepting an anonymous one would
                // mean inventing a key, and every key here is load-bearing.
                b'{' => return Err(self.error(ErrorKind::ExpectedKey)),
                // Where the next key would be is exactly where a conditional
                // trailing the previous value lands, so this is the position
                // that catches `"key" "value" [$WIN32]`.
                b'[' => return Err(self.error(ErrorKind::PlatformConditional)),
                b'#' => return Err(self.error(ErrorKind::Directive)),
                _ => {}
            }

            let key_at = self.mark();
            let key = self.read_token()?;
            self.skip_trivia();
            let value = match self.peek() {
                // `"key" [$WIN32] "value"` also occurs. Caught here rather than
                // read as the value, which would store the literal text
                // `[$WIN32]` under the key and look like data.
                Some(b'[') => return Err(self.error(ErrorKind::PlatformConditional)),
                Some(b'{') => {
                    let line = self.line;
                    if depth + 1 > MAX_DEPTH {
                        return Err(self.error(ErrorKind::TooDeep));
                    }
                    self.pos += 1;
                    Value::Object(self.parse_body(depth + 1, line)?)
                }
                // End of input, or the block closing, with the key's value
                // still missing. Truncation lands here as often as it lands on
                // an unclosed brace.
                None | Some(b'}') => return Err(self.error_at(ErrorKind::MissingValue, key_at)),
                Some(_) => Value::String(self.read_token()?),
            };
            object.entries.push((key, value));
        }
    }

    /// Reads one token, quoted or bare. The cursor is known not to be on
    /// whitespace, a brace, or end of input.
    fn read_token(&mut self) -> Result<String, Error> {
        if self.peek() == Some(b'"') {
            self.read_quoted()
        } else {
            Ok(self.read_bare())
        }
    }

    /// Reads a `"`-delimited token, resolving escapes.
    fn read_quoted(&mut self) -> Result<String, Error> {
        // Remembered before the scan: an unterminated string is reported
        // against the quote that opened it, not against the end of the file.
        let opened = self.mark();
        self.pos += 1;
        let mut out = String::new();
        let mut chunk = self.pos;

        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error_at(ErrorKind::UnclosedString, opened));
            };
            match byte {
                b'"' => {
                    out.push_str(&self.input[chunk..self.pos]);
                    self.pos += 1;
                    return Ok(out);
                }
                // A closing quote was forgotten. Reading on would fold the rest
                // of the file into this one value and hand back a tree that
                // parses cleanly and says something untrue.
                //
                // Reported against the quote that opened, not the newline that
                // ended the line. They are always on the same line, but the
                // column of the opening quote is the character a person has to
                // put a cursor on; the column of the newline is the end of the
                // line and tells them nothing.
                b'\n' => return Err(self.error_at(ErrorKind::NewlineInString, opened)),
                b'\\' => {
                    out.push_str(&self.input[chunk..self.pos]);
                    let Some(escaped) = self.bytes.get(self.pos + 1).copied() else {
                        return Err(self.error_at(ErrorKind::UnclosedString, opened));
                    };
                    let resolved = match escaped {
                        b'\\' => '\\',
                        b'"' => '"',
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        // An escape nobody defined. Keeping the backslash is
                        // the safe reading: Windows paths reach these files as
                        // `C:\\Games`, and a hand-edited `C:\Games` must come
                        // back as a path rather than as `C:Games`.
                        //
                        // The escaped byte itself is left where it is instead
                        // of being consumed. It may be the first byte of a
                        // multi-byte character — `C:\Übisoft` is the very case
                        // the paragraph above is about — and both halves of
                        // consuming it are wrong: `char::from` on a byte at or
                        // above 0x80 reinterprets a UTF-8 lead byte as a
                        // Latin-1 codepoint, and stepping the cursor by two
                        // leaves it inside the character, so the next copy out
                        // of `self.input` panics on a char boundary. Stepping
                        // by one lets the ordinary copy path carry the
                        // character through whole.
                        _ => {
                            out.push('\\');
                            self.pos += 1;
                            chunk = self.pos;
                            continue;
                        }
                    };
                    out.push(resolved);
                    self.pos += 2;
                    chunk = self.pos;
                }
                _ => self.pos += 1,
            }
        }
    }

    /// Reads an unquoted token, which ends at whitespace, a brace, a quote or
    /// the start of a comment.
    ///
    /// Backslashes are literal here. Valve does not escape bare tokens, and a
    /// bare Windows path in one of these files is written with single
    /// separators.
    fn read_bare(&mut self) -> String {
        let start = self.pos;
        while let Some(byte) = self.peek() {
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c | b'{' | b'}' | b'"' => break,
                b'/' if self.bytes.get(self.pos + 1) == Some(&b'/') => break,
                _ => self.pos += 1,
            }
        }
        self.input[start..self.pos].to_owned()
    }
}
