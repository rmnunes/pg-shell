//! FROM-clause binding extraction.
//!
//! When completion is asked for `alias.` we need to know which relation the
//! alias binds to. The authoritative answer would come from `libpg_query`'s
//! AST — but the buffer is often mid-edit and won't parse. This module does a
//! best-effort walk over the token stream looking for `FROM ... (...)` regions
//! and pulling out `[schema.]relation [AS] [alias]` refs.
//!
//! It's intentionally permissive: it treats obvious stop-words (WHERE, GROUP,
//! etc.) as binding terminators and gives up rather than guessing on
//! ambiguous input.

use crate::tokenize::{TokKind, Token};

#[derive(Debug, Clone)]
pub struct FromBinding {
    pub schema: Option<String>,
    pub relation: String,
    /// The name the user will type before a `.`. Falls back to `relation` when
    /// no explicit alias was declared.
    pub alias: String,
}

pub fn extract(tokens: &[Token<'_>]) -> Vec<FromBinding> {
    // Work on a view of meaningful tokens so we can ignore trivia cleanly.
    let meaningful: Vec<&Token> = tokens.iter().filter(|t| !t.is_trivia()).collect();

    let mut out = Vec::new();
    let mut i = 0;
    while i < meaningful.len() {
        let tok = meaningful[i];
        if tok.kind == TokKind::Keyword {
            let lower = tok.lower();
            if matches!(
                lower.as_str(),
                "from" | "join" | "into" | "update" | "table"
            ) {
                i += 1;
                if let Some(b) = parse_ref(&meaningful, &mut i) {
                    out.push(b);
                }
                // Comma-separated additional tables after FROM.
                while i < meaningful.len() && meaningful[i].kind == TokKind::Comma {
                    i += 1;
                    if let Some(b) = parse_ref(&meaningful, &mut i) {
                        out.push(b);
                    }
                }
                continue;
            }
        }
        i += 1;
    }
    out
}

fn parse_ref(toks: &[&Token<'_>], i: &mut usize) -> Option<FromBinding> {
    if *i >= toks.len() {
        return None;
    }
    let first = toks[*i];
    if !is_ident_like(first) || is_stop_word(first) {
        return None;
    }
    let first_text = first.ident_text();
    *i += 1;

    // Optional schema qualifier: `schema.relation`.
    let (schema, relation) = if *i + 1 < toks.len()
        && toks[*i].kind == TokKind::Dot
        && is_ident_like(toks[*i + 1])
    {
        let rel = toks[*i + 1].ident_text();
        *i += 2;
        (Some(first_text), rel)
    } else {
        (None, first_text)
    };

    // Subquery in place of a table reference — skip to matching paren and treat
    // as opaque (no binding produced). Callers still get sibling bindings.
    if *i < toks.len() && toks[*i].kind == TokKind::LParen {
        skip_parens(toks, i);
        // Optional alias after subquery.
        if *i < toks.len() && toks[*i].kind == TokKind::Keyword && toks[*i].lower() == "as" {
            *i += 1;
        }
        if *i < toks.len() && is_ident_like(toks[*i]) && !is_stop_word(toks[*i]) {
            *i += 1;
        }
        return None;
    }

    // Alias handling: `AS alias` or bare `alias`.
    let mut alias = relation.clone();
    if *i < toks.len() && toks[*i].kind == TokKind::Keyword && toks[*i].lower() == "as" {
        *i += 1;
        if *i < toks.len() && is_ident_like(toks[*i]) {
            alias = toks[*i].ident_text();
            *i += 1;
        }
    } else if *i < toks.len() && is_ident_like(toks[*i]) && !is_stop_word(toks[*i]) {
        alias = toks[*i].ident_text();
        *i += 1;
    }

    Some(FromBinding {
        schema,
        relation,
        alias,
    })
}

fn skip_parens(toks: &[&Token<'_>], i: &mut usize) {
    if toks[*i].kind != TokKind::LParen {
        return;
    }
    let mut depth = 0i32;
    while *i < toks.len() {
        match toks[*i].kind {
            TokKind::LParen => depth += 1,
            TokKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    *i += 1;
                    return;
                }
            }
            _ => {}
        }
        *i += 1;
    }
}

fn is_ident_like(t: &Token<'_>) -> bool {
    matches!(t.kind, TokKind::Ident | TokKind::QuotedIdent)
}

/// Keywords that can follow a relation ref but are not aliases.
fn is_stop_word(t: &Token<'_>) -> bool {
    if t.kind != TokKind::Keyword {
        return false;
    }
    matches!(
        t.lower().as_str(),
        "where"
            | "group"
            | "order"
            | "having"
            | "limit"
            | "offset"
            | "fetch"
            | "join"
            | "inner"
            | "left"
            | "right"
            | "full"
            | "cross"
            | "natural"
            | "lateral"
            | "on"
            | "using"
            | "returning"
            | "set"
            | "values"
            | "union"
            | "intersect"
            | "except"
            | "for"
            | "with"
            | "into"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize::tokenize;

    fn extract_str(sql: &str) -> Vec<(Option<String>, String, String)> {
        let toks = tokenize(sql);
        extract(&toks)
            .into_iter()
            .map(|b| (b.schema, b.relation, b.alias))
            .collect()
    }

    #[test]
    fn simple_alias() {
        assert_eq!(
            extract_str("SELECT * FROM users u"),
            vec![(None, "users".into(), "u".into())]
        );
    }

    #[test]
    fn as_alias() {
        assert_eq!(
            extract_str("SELECT * FROM users AS u"),
            vec![(None, "users".into(), "u".into())]
        );
    }

    #[test]
    fn schema_qualified() {
        assert_eq!(
            extract_str("SELECT * FROM public.users u"),
            vec![(Some("public".into()), "users".into(), "u".into())]
        );
    }

    #[test]
    fn multiple_comma() {
        let r = extract_str("SELECT * FROM users u, orders o");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].2, "u");
        assert_eq!(r[1].2, "o");
    }

    #[test]
    fn join() {
        let r = extract_str("SELECT * FROM users u JOIN orders o ON o.user_id = u.id");
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].1, "orders");
    }

    #[test]
    fn where_terminates_alias() {
        // Without this, the tokenizer's naive "next ident" might grab "WHERE".
        let r = extract_str("SELECT * FROM users WHERE id = 1");
        assert_eq!(r, vec![(None, "users".into(), "users".into())]);
    }

    #[test]
    fn subquery_skipped() {
        let r = extract_str("SELECT * FROM (SELECT 1) sub JOIN users u ON true");
        // Subquery doesn't produce a binding; users does.
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].1, "users");
    }
}
