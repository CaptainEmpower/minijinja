//! Decoding a string literal's escapes the way Jinja2 decodes them.
//!
//! Jinja2's lexer turns a literal into a value with
//! `value.encode("ascii", "backslashreplace").decode("unicode-escape")`, so
//! its escape rules are Python's `unicode-escape` codec's, not JSON's. The two
//! disagree on enough to change what a template writes: `\/` is not an escape
//! in Python and keeps its backslash, `\a` and `\v` are the bell and the
//! vertical tab, `\UXXXXXXXX` and `\N{NAME}` name any character, an octal
//! escape reaches `\777`, and a backslash before a line end continues the line.
//!
//! The `encode` half matters too, once: a backslash before a non-ASCII
//! character escapes nothing, so the character is written out as the
//! `backslashreplace` form the encode gave it. `'\é'` is the four characters
//! `\xe9`, measured on Jinja2 3.1 under Python 3.13.
//!
//! One difference is kept on purpose. Python decodes `😀` to two
//! lone surrogates, which a Rust string cannot hold and which Python itself
//! cannot write out; here a high and a low surrogate escape written together
//! make the one character they encode, and a lone one is an error.

use std::char::decode_utf16;
use std::iter::once;
use std::str::Chars;

use crate::error::{Error, ErrorKind};

/// Decode `s`, a string literal's contents, by Python's `unicode-escape`.
pub fn unescape(s: &str) -> Result<String, Error> {
    Unescaper {
        out: String::with_capacity(s.len()),
        pending_surrogate: 0,
    }
    .unescape(s)
}

fn bad_escape(detail: &'static str) -> Error {
    Error::new(ErrorKind::BadEscape, detail)
}

struct Unescaper {
    out: String,
    pending_surrogate: u16,
}

impl Unescaper {
    fn unescape(mut self, s: &str) -> Result<String, Error> {
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                ok!(self.push_char(c));
                continue;
            }
            let Some(d) = chars.next() else {
                return Err(bad_escape("\\ at end of string"));
            };
            match d {
                '\n' => {}
                '\\' | '\'' | '"' => ok!(self.push_char(d)),
                'a' => ok!(self.push_char('\x07')),
                'b' => ok!(self.push_char('\x08')),
                'f' => ok!(self.push_char('\x0c')),
                'n' => ok!(self.push_char('\n')),
                'r' => ok!(self.push_char('\r')),
                't' => ok!(self.push_char('\t')),
                'v' => ok!(self.push_char('\x0b')),
                '0'..='7' => {
                    let value = octal(d, &mut chars);
                    ok!(self.push_char(char::from_u32(value).expect("at most \\777")));
                }
                'x' => {
                    let value = ok!(hex(&mut chars, 2, "truncated \\xXX escape"));
                    ok!(self.push_char(char::from_u32(value).expect("at most \\xff")));
                }
                'u' => {
                    let value = ok!(hex(&mut chars, 4, "truncated \\uXXXX escape"));
                    ok!(self.push_code_point(value));
                }
                'U' => {
                    let value = ok!(hex(&mut chars, 8, "truncated \\UXXXXXXXX escape"));
                    if value > 0x10ffff {
                        return Err(bad_escape("illegal Unicode character"));
                    }
                    ok!(self.push_code_point(value));
                }
                'N' => {
                    let value = ok!(named(&mut chars));
                    ok!(self.push_char(value));
                }
                d if !d.is_ascii() => {
                    // What `encode("ascii", "backslashreplace")` wrote for it.
                    ok!(self.push_char('\\'));
                    let code = d as u32;
                    let replaced = if code <= 0xff {
                        format!("x{code:02x}")
                    } else if code <= 0xffff {
                        format!("u{code:04x}")
                    } else {
                        format!("U{code:08x}")
                    };
                    for r in replaced.chars() {
                        ok!(self.push_char(r));
                    }
                }
                _ => {
                    ok!(self.push_char('\\'));
                    ok!(self.push_char(d));
                }
            }
        }
        if self.pending_surrogate != 0 {
            return Err(bad_escape("lone surrogate escape"));
        }
        Ok(self.out)
    }

    fn push_code_point(&mut self, value: u32) -> Result<(), Error> {
        match u16::try_from(value) {
            Ok(unit) if (0xd800..=0xdfff).contains(&unit) => self.push_surrogate(unit),
            _ => self.push_char(char::from_u32(value).expect("not a surrogate")),
        }
    }

    fn push_surrogate(&mut self, unit: u16) -> Result<(), Error> {
        if self.pending_surrogate == 0 {
            self.pending_surrogate = unit;
            return Ok(());
        }
        let high = std::mem::take(&mut self.pending_surrogate);
        match decode_utf16(once(high).chain(once(unit))).next() {
            Some(Ok(c)) => {
                self.out.push(c);
                Ok(())
            }
            _ => Err(bad_escape("lone surrogate escape")),
        }
    }

    fn push_char(&mut self, c: char) -> Result<(), Error> {
        if self.pending_surrogate != 0 {
            return Err(bad_escape("lone surrogate escape"));
        }
        self.out.push(c);
        Ok(())
    }
}

/// Up to three octal digits, the first already read: `\0` to `\777`.
fn octal(first: char, chars: &mut Chars) -> u32 {
    let mut value = first.to_digit(8).expect("an octal digit");
    for _ in 0..2 {
        match chars.as_str().chars().next().and_then(|c| c.to_digit(8)) {
            Some(digit) => {
                value = value * 8 + digit;
                chars.next();
            }
            None => break,
        }
    }
    value
}

/// Exactly `count` hex digits; fewer is Python's "truncated" error.
fn hex(chars: &mut Chars, count: usize, truncated: &'static str) -> Result<u32, Error> {
    let mut value = 0u32;
    for _ in 0..count {
        let digit = chars.as_str().chars().next().and_then(|c| c.to_digit(16));
        match digit {
            Some(digit) => {
                value = value * 16 + digit;
                chars.next();
            }
            None => return Err(bad_escape(truncated)),
        }
    }
    Ok(value)
}

/// `\N{NAME}`, the `\N` already read.
fn named(chars: &mut Chars) -> Result<char, Error> {
    let malformed = || bad_escape("malformed \\N character escape");
    let rest = chars.as_str();
    let Some(body) = rest.strip_prefix('{') else {
        return Err(malformed());
    };
    let Some(end) = body.find('}') else {
        return Err(malformed());
    };
    let name = &body[..end];
    if name.is_empty() {
        return Err(malformed());
    }
    let c = ok!(lookup(name).ok_or_else(|| bad_escape("unknown Unicode character name")));
    // `{`, the name and `}`.
    for _ in 0..name.chars().count() + 2 {
        chars.next();
    }
    Ok(c)
}

/// The character Python's `unicodedata.lookup` names `name`.
///
/// Python matches a name or an alias ignoring case only; the table here also
/// matches loosely (spaces, underscores and medial hyphens ignored), so a
/// candidate whose spelling is not a plain name is refused: Python refuses
/// `\N{ LATIN SMALL LETTER A}` and `\N{latin_small_letter_a}`.
#[cfg(feature = "unicode_names")]
fn lookup(name: &str) -> Option<char> {
    let plain = !name.starts_with(' ')
        && !name.ends_with(' ')
        && !name.contains("  ")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-');
    if !plain {
        return None;
    }
    unicode_names2::character(name)
}

/// Without the name table, every `\N{...}` is refused.
#[cfg(not(feature = "unicode_names"))]
fn lookup(_name: &str) -> Option<char> {
    None
}

#[cfg(test)]
mod tests {
    use super::unescape;

    fn ok(s: &str) -> String {
        unescape(s).unwrap()
    }

    fn err(s: &str) -> String {
        unescape(s)
            .unwrap_err()
            .detail()
            .unwrap_or_default()
            .to_string()
    }

    // Every expected value below is Jinja2 3.1's under Python 3.13,
    // `Environment().compile_expression("'<literal>'")()`.

    #[test]
    fn the_json_escapes_python_shares() {
        assert_eq!(ok(r"foo☃bar"), "foo\u{2603}bar");
        assert_eq!(ok(r#"\t\b\f\r\n\\\'\""#), "\t\x08\x0c\r\n\\'\"");
        assert_eq!(ok("foobarbaz"), "foobarbaz");
        assert_eq!(ok(r"\x42\xab\x0a"), "B\u{ab}\n");
    }

    #[test]
    fn a_slash_is_not_an_escape_in_python() {
        assert_eq!(ok(r"a\/b"), r"a\/b");
    }

    #[test]
    fn the_escapes_json_lacks() {
        assert_eq!(ok(r"x\ay"), "x\x07y");
        assert_eq!(ok(r"\v"), "\x0b");
        assert_eq!(ok(r"\U00000041"), "A");
        assert_eq!(ok(r"\U0001f600"), "\u{1f600}");
        assert_eq!(ok("a\\\nb"), "ab");
    }

    #[test]
    fn octal_escapes_reach_three_digits_and_past_a_byte() {
        assert_eq!(ok(r"\0"), "\0");
        assert_eq!(ok(r"\12"), "\n");
        assert_eq!(ok(r"\101\102\103"), "ABC");
        assert_eq!(ok(r"\1234"), "S4");
        assert_eq!(ok(r"\400"), "\u{100}");
        assert_eq!(ok(r"\777"), "\u{1ff}");
    }

    #[test]
    fn an_unknown_escape_keeps_its_backslash() {
        assert_eq!(ok(r"\d\s\q\8\9"), r"\d\s\q\8\9");
    }

    #[test]
    fn a_backslash_before_a_non_ascii_character_writes_its_replacement() {
        assert_eq!(ok("\\\u{e9}"), r"\xe9");
        assert_eq!(ok("\\\u{2603}"), r"\u2603");
        assert_eq!(ok("\\\u{1f600}"), r"\U0001f600");
        assert_eq!(ok("\u{e9}\\n"), "\u{e9}\n");
    }

    #[test]
    fn truncated_and_illegal_escapes_fail_with_pythons_words() {
        assert_eq!(err(r"\x4"), r"truncated \xXX escape");
        assert_eq!(err(r"\x4g"), r"truncated \xXX escape");
        assert_eq!(err(r"\u12"), r"truncated \uXXXX escape");
        assert_eq!(err(r"\U0041"), r"truncated \UXXXXXXXX escape");
        assert_eq!(err(r"\U0011ffff"), "illegal Unicode character");
        assert_eq!(err("\\"), r"\ at end of string");
    }

    #[test]
    fn surrogate_escapes_pair_up_or_fail() {
        assert_eq!(ok(r"💩"), "\u{1f4a9}");
        assert!(unescape(r"\ud83d").is_err());
        assert!(unescape(r"\ud83dx").is_err());
        assert!(unescape(r"\udca9").is_err());
    }

    #[test]
    fn a_malformed_name_escape_fails() {
        assert_eq!(err(r"\N"), r"malformed \N character escape");
        assert_eq!(err(r"\Nx"), r"malformed \N character escape");
        assert_eq!(err(r"\N{}"), r"malformed \N character escape");
        assert_eq!(err(r"\N{LATIN"), r"malformed \N character escape");
    }

    #[cfg(feature = "unicode_names")]
    #[test]
    fn a_name_escape_matches_a_name_or_an_alias_ignoring_case() {
        assert_eq!(ok(r"\N{LATIN SMALL LETTER A}b"), "ab");
        assert_eq!(ok(r"\N{latin small letter a}"), "a");
        assert_eq!(ok(r"\N{LATIN CAPITAL LETTER GHA}"), "\u{1a2}");
        assert_eq!(ok(r"\N{HANGUL SYLLABLE GA}"), "\u{ac00}");
        assert_eq!(ok(r"\N{CJK UNIFIED IDEOGRAPH-4E00}"), "\u{4e00}");
        assert_eq!(err(r"\N{NOPE}"), "unknown Unicode character name");
        assert_eq!(
            err(r"\N{ LATIN SMALL LETTER A}"),
            "unknown Unicode character name"
        );
        assert_eq!(
            err(r"\N{latin_small_letter_a}"),
            "unknown Unicode character name"
        );
    }

    #[cfg(not(feature = "unicode_names"))]
    #[test]
    fn without_the_name_table_a_name_escape_fails() {
        assert_eq!(
            err(r"\N{LATIN SMALL LETTER A}"),
            "unknown Unicode character name"
        );
    }
}
