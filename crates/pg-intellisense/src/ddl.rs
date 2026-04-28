//! Token-based DDL detector.
//!
//! After a SQL batch executes successfully we want the schema cache to
//! reflect any structural change. Doing this with the full Postgres parser
//! (libpg_query) would be ideal, but that's gated behind an optional feature
//! and requires LLVM. Token pattern-matching catches the common cases:
//! `CREATE TABLE`, `DROP VIEW`, `ALTER MATERIALIZED VIEW`, `CREATE INDEX
//! ON ...`, `CREATE/DROP SCHEMA`, etc.
//!
//! The detector is intentionally **conservative**: anything we recognize as
//! DDL but can't confidently scope (renames, role/privilege changes, an
//! unfamiliar object kind) emits [`DdlEffect::Profile`], which invalidates
//! everything for the profile. The cost is a single re-introspection on
//! the next completion request — cheap, and never wrong.
//!
//! What we deliberately do NOT detect: `INSERT/UPDATE/DELETE/SELECT/COPY`,
//! `BEGIN/COMMIT/ROLLBACK`, `VACUUM/ANALYZE`, `SET`. These don't change
//! schema shape and produce no effects.

use crate::tokenize::{tokenize, TokKind, Token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DdlEffect {
    /// Catch-all: drop the whole profile's cache.
    Profile,
    /// Drop one schema's tables/views/functions wholesale.
    Schema(String),
    /// Drop a specific relation's column/index/constraint state.
    Relation {
        schema: Option<String>,
        name: String,
    },
}

pub fn detect(sql: &str) -> Vec<DdlEffect> {
    let tokens = tokenize(sql);
    let mut out = Vec::new();
    for stmt in split_statements(&tokens) {
        out.extend(detect_one(stmt));
    }
    out
}

fn detect_one(tokens: &[Token<'_>]) -> Vec<DdlEffect> {
    let mut it = tokens.iter().filter(|t| !t.is_trivia()).peekable();
    let head = match it.next() {
        Some(t) => t.lower(),
        None => return Vec::new(),
    };
    match head.as_str() {
        "create" => parse_create(&mut it),
        "alter" => parse_alter(&mut it),
        "drop" => parse_drop(&mut it),
        // Privilege changes can flip object visibility under search_path —
        // safest to invalidate the whole profile.
        "grant" | "revoke" => vec![DdlEffect::Profile],
        // Comments on objects don't change the cache shape, but `COMMENT ON
        // SCHEMA s IS '…'` is a no-op for us; ignore.
        _ => Vec::new(),
    }
}

/// Split the token stream into top-level statements separated by `;`.
fn split_statements<'a, 'b>(tokens: &'a [Token<'b>]) -> Vec<&'a [Token<'b>]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, tok) in tokens.iter().enumerate() {
        if tok.kind == TokKind::Semi {
            out.push(&tokens[start..i]);
            start = i + 1;
        }
    }
    if start < tokens.len() {
        out.push(&tokens[start..]);
    }
    out
}

fn parse_create<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
) -> Vec<DdlEffect>
where
    'b: 'a,
{
    // Skip OR REPLACE / TEMP / TEMPORARY / UNLOGGED / GLOBAL / LOCAL.
    skip_modifiers(
        it,
        &[
            "or",
            "replace",
            "temp",
            "temporary",
            "unlogged",
            "global",
            "local",
        ],
    );

    let kind = match it.next().map(|t| t.lower()) {
        Some(k) => k,
        None => return vec![DdlEffect::Profile],
    };

    match kind.as_str() {
        "schema" => {
            // CREATE SCHEMA [IF NOT EXISTS] name [AUTHORIZATION ...]
            skip_if_not_exists(it);
            if let Some(name) = next_ident(it) {
                vec![DdlEffect::Schema(name)]
            } else {
                vec![DdlEffect::Profile]
            }
        }
        "table" => {
            skip_if_not_exists(it);
            qualified_relation(it)
        }
        "view" => {
            // CREATE [OR REPLACE] VIEW [IF NOT EXISTS] [schema.]name
            skip_if_not_exists(it);
            qualified_relation(it)
        }
        "materialized" => {
            // CREATE MATERIALIZED VIEW [IF NOT EXISTS] [schema.]name
            if !consume_kw(it, "view") {
                return vec![DdlEffect::Profile];
            }
            skip_if_not_exists(it);
            qualified_relation(it)
        }
        "index" => {
            // CREATE [UNIQUE] INDEX [IF NOT EXISTS] [name] ON [schema.]rel ...
            // Walk forward until we hit `on`; the relation that follows is
            // what we invalidate. Index names are throw-away — they don't
            // appear in the schema cache the same way relations do.
            walk_to_on_then_relation(it)
        }
        "unique" => {
            // CREATE UNIQUE INDEX ...
            if !consume_kw(it, "index") {
                return vec![DdlEffect::Profile];
            }
            walk_to_on_then_relation(it)
        }
        "function" | "procedure" | "aggregate" | "type" | "domain" => {
            // [schema.]name — schema-level effect (the function/type list per
            // schema needs to be rebuilt).
            schema_of_qualified(it)
        }
        // Things we recognize as DDL but can't confidently scope.
        _ => vec![DdlEffect::Profile],
    }
}

fn parse_alter<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
) -> Vec<DdlEffect>
where
    'b: 'a,
{
    let kind = match it.next().map(|t| t.lower()) {
        Some(k) => k,
        None => return vec![DdlEffect::Profile],
    };
    match kind.as_str() {
        "table" => {
            // ALTER TABLE [IF EXISTS] [ONLY] [schema.]name ...
            skip_if_exists(it);
            consume_kw(it, "only");
            qualified_relation(it)
        }
        "view" => {
            skip_if_exists(it);
            qualified_relation(it)
        }
        "materialized" => {
            if !consume_kw(it, "view") {
                return vec![DdlEffect::Profile];
            }
            skip_if_exists(it);
            qualified_relation(it)
        }
        "schema" => {
            // ALTER SCHEMA name RENAME TO new_name → invalidate both. Easier
            // to invalidate the whole profile; catalog has reference counts
            // we'd otherwise miss.
            vec![DdlEffect::Profile]
        }
        "index" | "function" | "procedure" | "type" | "domain" => schema_of_qualified(it),
        _ => vec![DdlEffect::Profile],
    }
}

fn parse_drop<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
) -> Vec<DdlEffect>
where
    'b: 'a,
{
    let kind = match it.next().map(|t| t.lower()) {
        Some(k) => k,
        None => return vec![DdlEffect::Profile],
    };
    skip_if_exists(it);
    match kind.as_str() {
        "table" | "view" => {
            // DROP TABLE [IF EXISTS] [schema.]a, [schema.]b [CASCADE]
            collect_qualified_list(it)
        }
        "materialized" => {
            if !consume_kw(it, "view") {
                return vec![DdlEffect::Profile];
            }
            skip_if_exists(it);
            collect_qualified_list(it)
        }
        "schema" => collect_schemas(it),
        "index" | "function" | "procedure" | "type" | "domain" => {
            // Each entry's schema, deduplicated implicitly by the cache layer.
            schema_of_qualified_list(it)
        }
        _ => vec![DdlEffect::Profile],
    }
}

// ----- helpers -------------------------------------------------------------

fn consume_kw<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
    kw: &str,
) -> bool
where
    'b: 'a,
{
    if let Some(tok) = it.peek() {
        if tok.lower() == kw {
            it.next();
            return true;
        }
    }
    false
}

fn skip_if_not_exists<'a, 'b>(it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>)
where
    'b: 'a,
{
    // `IF NOT EXISTS` — three tokens or none.
    let saved = it.peek().map(|t| t.lower());
    if saved.as_deref() == Some("if") {
        it.next();
        if it.peek().map(|t| t.lower()).as_deref() == Some("not") {
            it.next();
            consume_kw(it, "exists");
        }
    }
}

fn skip_if_exists<'a, 'b>(it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>)
where
    'b: 'a,
{
    if it.peek().map(|t| t.lower()).as_deref() == Some("if") {
        it.next();
        consume_kw(it, "exists");
    }
}

fn skip_modifiers<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
    keywords: &[&str],
) where
    'b: 'a,
{
    while let Some(tok) = it.peek() {
        let lower = tok.lower();
        if keywords.iter().any(|k| *k == lower) {
            it.next();
        } else {
            break;
        }
    }
}

fn next_ident<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
) -> Option<String>
where
    'b: 'a,
{
    let tok = it.next()?;
    match tok.kind {
        TokKind::Ident | TokKind::QuotedIdent | TokKind::Keyword => Some(tok.ident_text()),
        _ => None,
    }
}

/// Read one identifier, then optionally `.identifier`. Returns
/// `(schema_or_None, name)`.
fn qualified_name<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
) -> Option<(Option<String>, String)>
where
    'b: 'a,
{
    let first = next_ident(it)?;
    if it.peek().map(|t| t.kind) == Some(TokKind::Dot) {
        it.next();
        let second = next_ident(it)?;
        Some((Some(first), second))
    } else {
        Some((None, first))
    }
}

fn qualified_relation<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
) -> Vec<DdlEffect>
where
    'b: 'a,
{
    match qualified_name(it) {
        Some((schema, name)) => vec![DdlEffect::Relation { schema, name }],
        None => vec![DdlEffect::Profile],
    }
}

fn schema_of_qualified<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
) -> Vec<DdlEffect>
where
    'b: 'a,
{
    match qualified_name(it) {
        Some((Some(schema), _)) => vec![DdlEffect::Schema(schema)],
        Some((None, _)) => vec![DdlEffect::Profile], // schema unknown — search_path dependent
        None => vec![DdlEffect::Profile],
    }
}

/// Walk forward looking for `ON`, then read the relation name after it.
/// `CREATE INDEX foo ON public.users (col)` → invalidates `public.users`.
fn walk_to_on_then_relation<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
) -> Vec<DdlEffect>
where
    'b: 'a,
{
    while let Some(tok) = it.next() {
        if tok.lower() == "on" {
            return qualified_relation(it);
        }
    }
    vec![DdlEffect::Profile]
}

fn collect_qualified_list<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
) -> Vec<DdlEffect>
where
    'b: 'a,
{
    let mut out = Vec::new();
    loop {
        match qualified_name(it) {
            Some((schema, name)) => out.push(DdlEffect::Relation { schema, name }),
            None => return vec![DdlEffect::Profile],
        }
        if it.peek().map(|t| t.kind) == Some(TokKind::Comma) {
            it.next();
            continue;
        }
        break;
    }
    out
}

fn collect_schemas<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
) -> Vec<DdlEffect>
where
    'b: 'a,
{
    let mut out = Vec::new();
    loop {
        let Some(name) = next_ident(it) else {
            return vec![DdlEffect::Profile];
        };
        out.push(DdlEffect::Schema(name));
        if it.peek().map(|t| t.kind) == Some(TokKind::Comma) {
            it.next();
            continue;
        }
        break;
    }
    out
}

fn schema_of_qualified_list<'a, 'b>(
    it: &mut std::iter::Peekable<impl Iterator<Item = &'a Token<'b>>>,
) -> Vec<DdlEffect>
where
    'b: 'a,
{
    let mut out = Vec::new();
    loop {
        match qualified_name(it) {
            Some((Some(schema), _)) => out.push(DdlEffect::Schema(schema)),
            Some((None, _)) => return vec![DdlEffect::Profile],
            None => return vec![DdlEffect::Profile],
        }
        if it.peek().map(|t| t.kind) == Some(TokKind::Comma) {
            it.next();
            continue;
        }
        break;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(sql: &str) -> Vec<DdlEffect> {
        detect(sql)
    }

    #[test]
    fn ignores_dml_and_session_commands() {
        assert!(d("SELECT 1;").is_empty());
        assert!(d("INSERT INTO t VALUES (1);").is_empty());
        assert!(d("UPDATE t SET x = 1;").is_empty());
        assert!(d("DELETE FROM t;").is_empty());
        assert!(d("BEGIN;").is_empty());
        assert!(d("COMMIT;").is_empty());
        assert!(d("VACUUM;").is_empty());
        assert!(d("SET TIME ZONE 'UTC';").is_empty());
        assert!(d("TRUNCATE foo;").is_empty()); // data, not schema
    }

    #[test]
    fn create_table_qualified() {
        assert_eq!(
            d("CREATE TABLE public.users (id int);"),
            vec![DdlEffect::Relation {
                schema: Some("public".into()),
                name: "users".into()
            }]
        );
    }

    #[test]
    fn create_table_if_not_exists_unqualified() {
        assert_eq!(
            d("CREATE TABLE IF NOT EXISTS users (id int);"),
            vec![DdlEffect::Relation {
                schema: None,
                name: "users".into()
            }]
        );
    }

    #[test]
    fn create_temp_table_recognized_as_relation() {
        assert_eq!(
            d("CREATE TEMP TABLE scratch (id int);"),
            vec![DdlEffect::Relation {
                schema: None,
                name: "scratch".into()
            }]
        );
    }

    #[test]
    fn create_or_replace_view() {
        assert_eq!(
            d("CREATE OR REPLACE VIEW public.v AS SELECT 1;"),
            vec![DdlEffect::Relation {
                schema: Some("public".into()),
                name: "v".into()
            }]
        );
    }

    #[test]
    fn create_materialized_view() {
        assert_eq!(
            d("CREATE MATERIALIZED VIEW analytics.daily AS SELECT 1;"),
            vec![DdlEffect::Relation {
                schema: Some("analytics".into()),
                name: "daily".into()
            }]
        );
    }

    #[test]
    fn create_index_invalidates_target_relation() {
        assert_eq!(
            d("CREATE INDEX idx_users_email ON public.users (email);"),
            vec![DdlEffect::Relation {
                schema: Some("public".into()),
                name: "users".into()
            }]
        );
    }

    #[test]
    fn create_unique_index() {
        assert_eq!(
            d("CREATE UNIQUE INDEX i ON users (a);"),
            vec![DdlEffect::Relation {
                schema: None,
                name: "users".into()
            }]
        );
    }

    #[test]
    fn alter_table() {
        assert_eq!(
            d("ALTER TABLE public.users ADD COLUMN email text;"),
            vec![DdlEffect::Relation {
                schema: Some("public".into()),
                name: "users".into()
            }]
        );
    }

    #[test]
    fn alter_table_only_if_exists() {
        assert_eq!(
            d("ALTER TABLE IF EXISTS ONLY public.users DROP COLUMN x;"),
            vec![DdlEffect::Relation {
                schema: Some("public".into()),
                name: "users".into()
            }]
        );
    }

    #[test]
    fn drop_table_list() {
        let got = d("DROP TABLE IF EXISTS public.a, public.b CASCADE;");
        assert_eq!(
            got,
            vec![
                DdlEffect::Relation {
                    schema: Some("public".into()),
                    name: "a".into()
                },
                DdlEffect::Relation {
                    schema: Some("public".into()),
                    name: "b".into()
                },
            ]
        );
    }

    #[test]
    fn create_drop_schema() {
        assert_eq!(
            d("CREATE SCHEMA staging;"),
            vec![DdlEffect::Schema("staging".into())]
        );
        assert_eq!(
            d("DROP SCHEMA staging CASCADE;"),
            vec![DdlEffect::Schema("staging".into())]
        );
    }

    #[test]
    fn create_function_qualified_emits_schema_effect() {
        assert_eq!(
            d("CREATE FUNCTION public.my_fn() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql;"),
            vec![DdlEffect::Schema("public".into())]
        );
    }

    #[test]
    fn unknown_create_falls_back_to_profile() {
        // CREATE EXTENSION isn't tracked granularly — invalidate the profile.
        assert_eq!(
            d("CREATE EXTENSION pg_stat_statements;"),
            vec![DdlEffect::Profile]
        );
    }

    #[test]
    fn grant_revoke_invalidate_profile() {
        assert_eq!(d("GRANT SELECT ON t TO r;"), vec![DdlEffect::Profile]);
        assert_eq!(d("REVOKE ALL ON t FROM PUBLIC;"), vec![DdlEffect::Profile]);
    }

    #[test]
    fn batched_statements_each_emit_effects() {
        let got = d("CREATE TABLE a (id int); INSERT INTO a VALUES (1); DROP TABLE a;");
        assert_eq!(
            got,
            vec![
                DdlEffect::Relation {
                    schema: None,
                    name: "a".into()
                },
                DdlEffect::Relation {
                    schema: None,
                    name: "a".into()
                },
            ]
        );
    }

    #[test]
    fn quoted_identifiers_preserved_case() {
        assert_eq!(
            d("CREATE TABLE \"MySchema\".\"MyTable\" (id int);"),
            vec![DdlEffect::Relation {
                schema: Some("MySchema".into()),
                name: "MyTable".into()
            }]
        );
    }

    #[test]
    fn comments_dont_confuse_detector() {
        let got = d("-- making a table\nCREATE TABLE /* still here */ public.t (id int);");
        assert_eq!(
            got,
            vec![DdlEffect::Relation {
                schema: Some("public".into()),
                name: "t".into()
            }]
        );
    }
}
