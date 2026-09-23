//! Valve's `KeyValues` text format. Pure: no paths, no filesystem.
//!
//! Objects keep insertion order, lookups ignore ASCII case (as Valve's own
//! parser does), and a broken file is an error, never a short answer.

#[cfg(test)]
mod tests;

use std::fmt;

/// How deep nesting may go: exactly this many blocks parse and one more is an
/// error, as in [`heroic`](crate::heroic)'s JSON reader. The bound stops a file
/// of open braces from overflowing the stack.
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

/// An ordered key-to-value map. Duplicate keys are kept; [`Object::get`]
/// answers with the first, as Valve's reader does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Object {
    entries: Vec<(String, Value)>,
}

impl Object {
    /// The first value stored under `key`, compared without ASCII case.
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

/// Borrows one stored pair; a named function so both iterators share one type.
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
    /// A `[$WIN32]`-style platform conditional: not implemented, and refused
    /// rather than misread as the next key.
    PlatformConditional,
    /// A `#base` or `#include` directive: not implemented, and refused, since
    /// the keys it would pull in would silently be missing.
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

/// Parses a `KeyValues` document into its root object. A leading byte-order
/// mark is skipped; `//` comments and both quoted and bare tokens are accepted.
///
/// # Errors
///
/// Returns the first structural problem, with its line and column. Nothing
/// partial is returned.
pub fn parse(input: &str) -> Result<Value, Error> {
    let mut parser = Parser::new(input);
    let root = parser.parse_body(0, 1)?;
    Ok(Value::Object(root))
}

/// A saved cursor position, so a fault is reported against the token that
/// caused it.
#[derive(Clone, Copy)]
struct Mark {
    pos: usize,
    line: usize,
    line_start: usize,
}

/// Byte-oriented cursor over the input. Safe because every delimiter is ASCII.
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
        // Editors add byte-order marks; skipping one keeps a healthy file healthy.
        let start = if input.starts_with('\u{feff}') { 3 } else { 0 };
        Self {
            input,
            bytes: input.as_bytes(),
            pos: start,
            line: 1,
            line_start: start,
        }
    }

    /// Where the cursor is now, so a fault noticed later is reported against
    /// the token that caused it.
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
            // Counted in characters, so the column matches what an editor shows.
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

    /// Reads the pairs of one block: the whole document at `depth` 0, up to the
    /// matching `}` deeper. `opened_line` points a truncation at its brace.
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
                // Where a conditional trailing the previous value lands.
                b'[' => return Err(self.error(ErrorKind::PlatformConditional)),
                b'#' => return Err(self.error(ErrorKind::Directive)),
                _ => {}
            }

            let key_at = self.mark();
            let key = self.read_token()?;
            self.skip_trivia();
            let value = match self.peek() {
                // `"key" [$WIN32] "value"`, caught before it is stored as data.
                Some(b'[') => return Err(self.error(ErrorKind::PlatformConditional)),
                Some(b'{') => {
                    let line = self.line;
                    if depth + 1 > MAX_DEPTH {
                        return Err(self.error(ErrorKind::TooDeep));
                    }
                    self.pos += 1;
                    Value::Object(self.parse_body(depth + 1, line)?)
                }
                // Truncation lands here as often as on an unclosed brace.
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
                // A forgotten closing quote, reported at the quote that opened it.
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
                        // An undefined escape keeps its backslash, so a
                        // hand-edited `C:\Games` stays a path. The next byte is
                        // not consumed: it may start a multi-byte character.
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

    /// Reads an unquoted token, up to whitespace, a brace, a quote or a comment.
    /// Backslashes are literal here.
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
