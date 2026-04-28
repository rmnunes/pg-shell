//! Hand-written PostgreSQL tokenizer.
//!
//! The tokenizer is the ground truth for cursor-context work: it runs on every
//! keystroke, never fails, and preserves exact byte spans so we can locate the
//! cursor precisely. It handles the lexical quirks that matter in practice:
//! quoted identifiers with `""` escapes, dollar-quoted string literals with
//! tags (`$body$…$body$`), standard `'foo''bar'` and `E'…'` strings, and
//! nested block comments.
//!
//! It deliberately does NOT attempt to parse grammar — that's the job of the
//! context detector.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokKind {
    /// Reserved or common PG keyword (see `is_keyword`).
    Keyword,
    /// Unquoted identifier (also typed variable names, table names, etc).
    Ident,
    /// `"foo bar"` — quoted identifier (preserves case).
    QuotedIdent,
    /// Any string literal: `'…'`, `E'…'`, `$tag$…$tag$`.
    StringLit,
    Number,
    Dot,
    Comma,
    Semi,
    LParen,
    RParen,
    LBracket,
    RBracket,
    /// Any run of operator characters: `+`, `||`, `::`, `<>`, etc.
    Op,
    LineComment,
    BlockComment,
    Whitespace,
    /// Fallback for characters we don't recognize.
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Token<'a> {
    pub kind: TokKind,
    pub span: Range<usize>,
    pub text: &'a str,
}

impl<'a> Token<'a> {
    pub fn is_trivia(&self) -> bool {
        matches!(
            self.kind,
            TokKind::Whitespace | TokKind::LineComment | TokKind::BlockComment
        )
    }

    /// Lowercased text — cached-free, allocates each call. Fine for the
    /// low-volume callers we have (context detection, alias extraction).
    pub fn lower(&self) -> String {
        self.text.to_ascii_lowercase()
    }

    /// Returns the identifier's logical name:
    /// - `Ident` → verbatim
    /// - `QuotedIdent` → inside of `"…"`, internal `""` collapsed to `"`
    pub fn ident_text(&self) -> String {
        match self.kind {
            TokKind::QuotedIdent if self.text.len() >= 2 => {
                self.text[1..self.text.len() - 1].replace("\"\"", "\"")
            }
            _ => self.text.to_string(),
        }
    }
}

pub fn tokenize(input: &str) -> Vec<Token<'_>> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let c = bytes[i];

        // Whitespace run.
        if c.is_ascii_whitespace() {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(mk(TokKind::Whitespace, start, i, input));
            continue;
        }

        // Line comment: `-- …` to EOL.
        if c == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            out.push(mk(TokKind::LineComment, start, i, input));
            continue;
        }

        // Block comment, nestable.
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            let mut depth: u32 = 1;
            while i < bytes.len() && depth > 0 {
                if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            out.push(mk(TokKind::BlockComment, start, i, input));
            continue;
        }

        // Escape-string literal `E'…\…'`.
        if (c == b'E' || c == b'e')
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'\''
        {
            i += 2;
            while i < bytes.len() {
                match bytes[i] {
                    b'\\' if i + 1 < bytes.len() => i += 2,
                    b'\'' => {
                        if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                            i += 2;
                        } else {
                            i += 1;
                            break;
                        }
                    }
                    _ => i += 1,
                }
            }
            out.push(mk(TokKind::StringLit, start, i, input));
            continue;
        }

        // Plain string literal `'…''…'`.
        if c == b'\'' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\'' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        i += 2;
                    } else {
                        i += 1;
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            out.push(mk(TokKind::StringLit, start, i, input));
            continue;
        }

        // Dollar-quoted string `$tag$ body $tag$` — or dollar-placeholder `$1`.
        if c == b'$' {
            let tag_start = i + 1;
            let mut tag_end = tag_start;
            while tag_end < bytes.len()
                && (bytes[tag_end] == b'_'
                    || (bytes[tag_end] as char).is_ascii_alphanumeric())
            {
                tag_end += 1;
            }
            if tag_end < bytes.len() && bytes[tag_end] == b'$' {
                // Full dollar-quoted string.
                let terminator = format!("${}$", &input[tag_start..tag_end]);
                let term_bytes = terminator.as_bytes();
                i = tag_end + 1;
                while i < bytes.len() {
                    if i + term_bytes.len() <= bytes.len()
                        && &bytes[i..i + term_bytes.len()] == term_bytes
                    {
                        i += term_bytes.len();
                        break;
                    }
                    i += 1;
                }
                out.push(mk(TokKind::StringLit, start, i, input));
                continue;
            } else if tag_end > tag_start && (bytes[tag_start] as char).is_ascii_digit() {
                // `$1`, `$2` — numbered parameter. Treat as Ident-shaped token.
                i = tag_end;
                out.push(mk(TokKind::Ident, start, i, input));
                continue;
            }
            // Bare `$` — fall through to operator run.
        }

        // Quoted identifier `"foo"` with `""` escapes.
        if c == b'"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'"' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                        i += 2;
                    } else {
                        i += 1;
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            out.push(mk(TokKind::QuotedIdent, start, i, input));
            continue;
        }

        // Identifier / keyword (ASCII letter or underscore start).
        if c == b'_' || (c as char).is_ascii_alphabetic() {
            while i < bytes.len()
                && (bytes[i] == b'_' || (bytes[i] as char).is_ascii_alphanumeric())
            {
                i += 1;
            }
            let text = &input[start..i];
            let kind = if is_keyword(text) {
                TokKind::Keyword
            } else {
                TokKind::Ident
            };
            out.push(Token {
                kind,
                span: start..i,
                text,
            });
            continue;
        }

        // Number: digit-started, allow decimal point and exponent.
        if (c as char).is_ascii_digit()
            || (c == b'.'
                && i + 1 < bytes.len()
                && (bytes[i + 1] as char).is_ascii_digit())
        {
            while i < bytes.len()
                && ((bytes[i] as char).is_ascii_digit() || bytes[i] == b'.')
            {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                    i += 1;
                }
            }
            out.push(mk(TokKind::Number, start, i, input));
            continue;
        }

        // Single-char punctuation.
        let punct = match c {
            b'.' => Some(TokKind::Dot),
            b',' => Some(TokKind::Comma),
            b';' => Some(TokKind::Semi),
            b'(' => Some(TokKind::LParen),
            b')' => Some(TokKind::RParen),
            b'[' => Some(TokKind::LBracket),
            b']' => Some(TokKind::RBracket),
            _ => None,
        };
        if let Some(k) = punct {
            i += 1;
            out.push(mk(k, start, i, input));
            continue;
        }

        // Operator run: `<=`, `>=`, `<>`, `!=`, `::`, `||`, etc.
        if matches!(
            c,
            b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'=' | b'!' | b'|' | b':' | b'@' | b'#' | b'~' | b'&' | b'^' | b'?' | b'$'
        ) {
            while i < bytes.len()
                && matches!(
                    bytes[i],
                    b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'=' | b'!' | b'|' | b':' | b'@' | b'#' | b'~' | b'&' | b'^' | b'?' | b'$'
                )
            {
                i += 1;
            }
            out.push(mk(TokKind::Op, start, i, input));
            continue;
        }

        // Fallback: single-byte unknown.
        i += 1;
        out.push(mk(TokKind::Unknown, start, i, input));
    }
    out
}

fn mk<'a>(kind: TokKind, start: usize, end: usize, input: &'a str) -> Token<'a> {
    Token {
        kind,
        span: start..end,
        text: &input[start..end],
    }
}

/// Reserved words plus common DDL/DML keywords. Not exhaustive — PG has ~750
/// keywords — but covers what the context detector anchors on.
fn is_keyword(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "select"
            | "from"
            | "where"
            | "join"
            | "inner"
            | "left"
            | "right"
            | "full"
            | "outer"
            | "cross"
            | "lateral"
            | "natural"
            | "on"
            | "using"
            | "group"
            | "by"
            | "having"
            | "order"
            | "limit"
            | "offset"
            | "fetch"
            | "insert"
            | "into"
            | "values"
            | "update"
            | "set"
            | "delete"
            | "as"
            | "and"
            | "or"
            | "not"
            | "null"
            | "true"
            | "false"
            | "is"
            | "in"
            | "exists"
            | "between"
            | "like"
            | "ilike"
            | "similar"
            | "case"
            | "when"
            | "then"
            | "else"
            | "end"
            | "distinct"
            | "all"
            | "any"
            | "some"
            | "with"
            | "recursive"
            | "union"
            | "intersect"
            | "except"
            | "create"
            | "table"
            | "view"
            | "materialized"
            | "index"
            | "sequence"
            | "drop"
            | "alter"
            | "add"
            | "column"
            | "constraint"
            | "primary"
            | "key"
            | "foreign"
            | "references"
            | "unique"
            | "default"
            | "check"
            | "returning"
            | "truncate"
            | "begin"
            | "commit"
            | "rollback"
            | "savepoint"
            | "release"
            | "transaction"
            | "cast"
            | "if"
            | "for"
            | "each"
            | "row"
            | "function"
            | "procedure"
            | "returns"
            | "language"
            | "immutable"
            | "stable"
            | "volatile"
            | "explain"
            | "analyze"
            | "vacuum"
            | "copy"
            | "grant"
            | "revoke"
            | "listen"
            | "notify"
            | "do"
            | "declare"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(sql: &str) -> Vec<TokKind> {
        tokenize(sql)
            .into_iter()
            .filter(|t| !t.is_trivia())
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn select_from_simple() {
        assert_eq!(
            kinds("SELECT a, b FROM tbl"),
            vec![
                TokKind::Keyword,
                TokKind::Ident,
                TokKind::Comma,
                TokKind::Ident,
                TokKind::Keyword,
                TokKind::Ident,
            ]
        );
    }

    #[test]
    fn dotted_alias() {
        let tokens: Vec<_> = tokenize("SELECT u.name FROM users u")
            .into_iter()
            .filter(|t| !t.is_trivia())
            .collect();
        assert_eq!(tokens[1].kind, TokKind::Ident); // u
        assert_eq!(tokens[2].kind, TokKind::Dot);
        assert_eq!(tokens[3].kind, TokKind::Ident); // name
    }

    #[test]
    fn quoted_ident_roundtrip() {
        let toks = tokenize(r#"SELECT "My Col" FROM t"#);
        let q = toks.iter().find(|t| t.kind == TokKind::QuotedIdent).unwrap();
        assert_eq!(q.ident_text(), "My Col");
    }

    #[test]
    fn string_literal_with_escaped_quote() {
        let toks = tokenize("SELECT 'it''s fine'");
        assert_eq!(toks.iter().filter(|t| t.kind == TokKind::StringLit).count(), 1);
    }

    #[test]
    fn dollar_quoted_string() {
        let toks = tokenize("SELECT $body$ CREATE TABLE x (a int); $body$");
        let lits: Vec<_> = toks.iter().filter(|t| t.kind == TokKind::StringLit).collect();
        assert_eq!(lits.len(), 1);
        assert!(lits[0].text.contains("CREATE TABLE"));
    }

    #[test]
    fn nested_block_comment() {
        let toks = tokenize("/* outer /* inner */ still outer */ SELECT 1");
        let comments: Vec<_> = toks.iter().filter(|t| t.kind == TokKind::BlockComment).collect();
        assert_eq!(comments.len(), 1);
    }

    #[test]
    fn cast_operator_is_single_op() {
        let toks: Vec<_> = tokenize("SELECT 1::int").into_iter().filter(|t| !t.is_trivia()).collect();
        assert_eq!(toks[2].kind, TokKind::Op);
        assert_eq!(toks[2].text, "::");
    }
}
