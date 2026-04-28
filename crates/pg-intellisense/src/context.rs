//! Context detection: given `(doc, cursor)` + tokens, decide what kind of
//! completion fits.
//!
//! Algorithm:
//! 1. Extract the partial prefix at the cursor (word characters immediately
//!    before cursor).
//! 2. Look at the tokens immediately before the prefix. If one is `.`, capture
//!    the qualifier — we'll resolve it against FROM bindings or known schemas
//!    in the engine.
//! 3. Otherwise walk backward over meaningful tokens until we hit an anchor
//!    keyword (FROM, SELECT, WHERE, ON, …). Paren depth is tracked so a
//!    function-call's inner position is classified as expression rather than
//!    column-list.

use crate::tokenize::{TokKind, Token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextKind {
    /// Top of a statement — keywords and snippets.
    StatementStart,
    /// After FROM / JOIN / INTO / UPDATE — expect tables/views/schemas.
    Relation,
    /// After SELECT / WHERE / ON / GROUP BY / ORDER BY / HAVING / SET — expect
    /// columns and functions.
    ColumnList,
    /// Inside a function call or cast — expect expressions (columns + functions).
    Expression,
    /// Qualifier was captured; engine resolves against FROM bindings or schemas.
    Qualified { qualifier: String },
}

#[derive(Debug, Clone)]
pub struct DetectedContext {
    pub kind: ContextKind,
    pub prefix: String,
    /// Byte-range within the document that the completion should replace. This
    /// matches Monaco's `range` field so inserted text cleanly overwrites the
    /// partial prefix rather than getting appended.
    pub replace: std::ops::Range<usize>,
}

pub fn detect(doc: &str, cursor: usize, tokens: &[Token<'_>]) -> DetectedContext {
    let (prefix, replace) = extract_prefix(doc, cursor);

    // Find the last meaningful token whose end is at or before `replace.start`.
    let idx = tokens
        .iter()
        .rposition(|t| !t.is_trivia() && t.span.end <= replace.start);

    // Qualifier detection: `<ident>.` immediately preceding the prefix.
    if let Some(i) = idx {
        if tokens[i].kind == TokKind::Dot {
            if let Some(q) = find_qualifier(tokens, i) {
                return DetectedContext {
                    kind: ContextKind::Qualified { qualifier: q },
                    prefix,
                    replace,
                };
            }
        }
    }

    let kind = classify_anchor(tokens, idx);
    DetectedContext {
        kind,
        prefix,
        replace,
    }
}

/// Return the identifier that sits before a `.` at token index `dot_idx`.
/// Returns `None` if the preceding token isn't an identifier.
fn find_qualifier(tokens: &[Token<'_>], dot_idx: usize) -> Option<String> {
    // Walk backward past trivia.
    let mut j = dot_idx;
    while j > 0 {
        j -= 1;
        if !tokens[j].is_trivia() {
            break;
        }
    }
    let t = &tokens[j];
    if matches!(t.kind, TokKind::Ident | TokKind::QuotedIdent) {
        Some(t.ident_text())
    } else {
        None
    }
}

/// Walk backward from `before_idx` looking for a keyword anchor. Accounts for
/// paren depth so we stay aware of expression position inside function calls.
fn classify_anchor(tokens: &[Token<'_>], before_idx: Option<usize>) -> ContextKind {
    let Some(start) = before_idx else {
        return ContextKind::StatementStart;
    };

    let mut depth_rparen: i32 = 0;
    let mut saw_open_paren_without_anchor = false;
    let mut i = start + 1;
    while i > 0 {
        i -= 1;
        let tok = &tokens[i];
        if tok.is_trivia() {
            continue;
        }
        match tok.kind {
            TokKind::RParen => {
                depth_rparen += 1;
            }
            TokKind::LParen => {
                if depth_rparen > 0 {
                    depth_rparen -= 1;
                } else {
                    // We exited outward into an enclosing expression — if we
                    // hit an anchor next, we're in expression position.
                    saw_open_paren_without_anchor = true;
                }
            }
            TokKind::Semi => return ContextKind::StatementStart,
            TokKind::Keyword => {
                if depth_rparen > 0 {
                    // Keyword is nested inside a parenthesized group we haven't
                    // exited yet — keep looking backward.
                    continue;
                }
                let lower = tok.lower();
                let anchor = match lower.as_str() {
                    "from" | "join" | "into" | "update" | "table" => Some(ContextKind::Relation),
                    "select" | "where" | "on" | "having" | "set" | "returning" | "using" => {
                        Some(ContextKind::ColumnList)
                    }
                    "by" => {
                        // `GROUP BY` / `ORDER BY` — treat as column list.
                        Some(ContextKind::ColumnList)
                    }
                    _ => None,
                };
                if let Some(a) = anchor {
                    return if saw_open_paren_without_anchor {
                        // Anchor is outside the enclosing `(` — inside the
                        // paren group we're in expression position.
                        ContextKind::Expression
                    } else {
                        a
                    };
                }
            }
            _ => {}
        }
    }
    ContextKind::StatementStart
}

/// Grab the word being typed immediately before `cursor`. Word chars are the
/// usual identifier set; quoted-ident mid-edit handled with a `"` prefix.
fn extract_prefix(doc: &str, cursor: usize) -> (String, std::ops::Range<usize>) {
    let cursor = cursor.min(doc.len());
    let bytes = doc.as_bytes();
    let mut start = cursor;
    while start > 0 {
        let c = bytes[start - 1];
        if c == b'_' || (c as char).is_ascii_alphanumeric() {
            start -= 1;
        } else {
            break;
        }
    }
    let prefix = doc[start..cursor].to_string();
    (prefix, start..cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize::tokenize;

    fn ctx(sql: &str, cursor: usize) -> (ContextKind, String) {
        let toks = tokenize(sql);
        let d = detect(sql, cursor, &toks);
        (d.kind, d.prefix)
    }

    #[test]
    fn statement_start_is_keyword_context() {
        let (k, p) = ctx("SEL", 3);
        assert_eq!(k, ContextKind::StatementStart);
        assert_eq!(p, "SEL");
    }

    #[test]
    fn after_from_is_relation() {
        let sql = "SELECT * FROM u";
        let (k, p) = ctx(sql, sql.len());
        assert_eq!(k, ContextKind::Relation);
        assert_eq!(p, "u");
    }

    #[test]
    fn after_select_is_column_list() {
        let sql = "SELECT na";
        let (k, p) = ctx(sql, sql.len());
        assert_eq!(k, ContextKind::ColumnList);
        assert_eq!(p, "na");
    }

    #[test]
    fn after_where_is_column_list() {
        let sql = "SELECT * FROM t WHERE ";
        let (k, p) = ctx(sql, sql.len());
        assert_eq!(k, ContextKind::ColumnList);
        assert_eq!(p, "");
    }

    #[test]
    fn qualified_alias() {
        let sql = "SELECT u. FROM users u";
        // Cursor just after the dot.
        let cursor = "SELECT u.".len();
        let (k, _) = ctx(sql, cursor);
        assert_eq!(k, ContextKind::Qualified { qualifier: "u".into() });
    }

    #[test]
    fn inside_function_is_expression() {
        let sql = "SELECT coalesce(";
        let (k, _) = ctx(sql, sql.len());
        assert_eq!(k, ContextKind::Expression);
    }
}
