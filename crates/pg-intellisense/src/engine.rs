//! Completion engine: takes a document + cursor + schema snapshot and emits
//! ranked `CompletionItem`s for the frontend.

use serde::Serialize;

use crate::aliases::extract as extract_bindings;
use crate::context::{detect, ContextKind};
use crate::ranker::prefix_score;
use crate::snippets::{Snippet, BUILTIN};
use crate::tokenize::tokenize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionKind {
    Keyword,
    Snippet,
    Schema,
    Table,
    View,
    MaterializedView,
    Column,
    Function,
    Alias,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionItem {
    pub label: String,
    pub insert_text: String,
    pub detail: Option<String>,
    pub kind: CompletionKind,
    /// Composite score: higher = better. Frontend sorts in descending order.
    pub sort_score: i32,
    /// True when `insert_text` is a Monaco snippet template (tab stops etc).
    pub is_snippet: bool,
    /// Byte range in the source document to replace with `insert_text`.
    pub replace_start: usize,
    pub replace_end: usize,
}

/// Lightweight view of the schema cache. The Tauri adapter materializes this
/// from `pg-schema-cache`; pg-intellisense stays free of sqlx so it can run
/// in tests without a database.
#[derive(Debug, Default, Clone)]
pub struct SchemaView {
    pub schemas: Vec<String>,
    pub relations: Vec<RelationLite>,
    pub columns: Vec<ColumnLite>,
    pub functions: Vec<FunctionLite>,
}

#[derive(Debug, Clone)]
pub struct RelationLite {
    pub schema: String,
    pub name: String,
    pub kind: CompletionKind,
}

#[derive(Debug, Clone)]
pub struct ColumnLite {
    pub schema: String,
    pub relation: String,
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Clone)]
pub struct FunctionLite {
    pub schema: String,
    pub name: String,
    pub args: String,
    pub result: String,
}

/// Weight applied to a candidate based on how strongly the context prefers it.
/// Combined with `prefix_score` produces the final `sort_score`.
fn context_weight(ctx: &ContextKind, kind: CompletionKind) -> i32 {
    match (ctx, kind) {
        (ContextKind::Relation, CompletionKind::Table) => 100,
        (ContextKind::Relation, CompletionKind::View) => 90,
        (ContextKind::Relation, CompletionKind::MaterializedView) => 85,
        (ContextKind::Relation, CompletionKind::Schema) => 70,
        (ContextKind::Relation, CompletionKind::Keyword) => 20,

        (ContextKind::ColumnList, CompletionKind::Column) => 100,
        (ContextKind::ColumnList, CompletionKind::Alias) => 80,
        (ContextKind::ColumnList, CompletionKind::Function) => 60,
        (ContextKind::ColumnList, CompletionKind::Keyword) => 30,

        (ContextKind::Expression, CompletionKind::Column) => 90,
        (ContextKind::Expression, CompletionKind::Function) => 85,
        (ContextKind::Expression, CompletionKind::Keyword) => 40,

        (ContextKind::StatementStart, CompletionKind::Snippet) => 100,
        (ContextKind::StatementStart, CompletionKind::Keyword) => 80,

        (ContextKind::Qualified { .. }, CompletionKind::Column) => 100,
        (ContextKind::Qualified { .. }, CompletionKind::Table) => 90,
        (ContextKind::Qualified { .. }, CompletionKind::View) => 85,

        _ => 10,
    }
}

#[cfg(feature = "libpg_query")]
fn ast_bindings(doc: &str) -> Option<Vec<crate::aliases::FromBinding>> {
    crate::libpg_query_parse::try_extract(doc)
}

#[cfg(not(feature = "libpg_query"))]
fn ast_bindings(_doc: &str) -> Option<Vec<crate::aliases::FromBinding>> {
    None
}

const TOP_SUGGESTION_KEYWORDS: &[&str] = &[
    "select", "from", "where", "join", "left", "inner", "group by", "order by", "having",
    "limit", "insert into", "update", "delete from", "values", "returning", "with", "as",
    "and", "or", "not", "null", "case", "when", "then", "else", "end", "distinct",
    "union", "except", "intersect",
];

pub fn complete(doc: &str, cursor_offset: usize, view: &SchemaView) -> Vec<CompletionItem> {
    let tokens = tokenize(doc);
    let ctx = detect(doc, cursor_offset, &tokens);
    // When the `libpg_query` feature is on AND the buffer parses cleanly,
    // prefer the AST's bindings — it handles joins/subqueries/CTEs canonically.
    // Fall back to the token walker otherwise (partial edits, syntax errors).
    let bindings = ast_bindings(doc).unwrap_or_else(|| extract_bindings(&tokens));

    let mut out: Vec<CompletionItem> = Vec::new();
    let prefix = ctx.prefix.clone();
    let replace = ctx.replace.clone();

    match &ctx.kind {
        ContextKind::Qualified { qualifier } => {
            // A qualifier can plausibly be: an alias in FROM, a schema name, or
            // a relation name used directly. We union the matching sets rather
            // than picking one — if the user typed `schema.` where `schema` is
            // also the relation that got bound to itself by our permissive
            // FROM parser (common when the buffer is mid-edit), they'd
            // otherwise see nothing.
            let qi = qualifier.to_ascii_lowercase();
            let alias_hit = bindings
                .iter()
                .find(|b| b.alias.to_ascii_lowercase() == qi);
            let is_known_schema =
                view.schemas.iter().any(|s| s.eq_ignore_ascii_case(qualifier));

            if let Some(b) = alias_hit {
                collect_columns_for(
                    &mut out,
                    &ctx.kind,
                    &prefix,
                    &b.schema,
                    &b.relation,
                    view,
                    &replace,
                );
            }
            if is_known_schema {
                collect_relations(
                    &mut out,
                    &ctx.kind,
                    &prefix,
                    Some(qualifier),
                    view,
                    &replace,
                );
                collect_functions(
                    &mut out,
                    &ctx.kind,
                    &prefix,
                    Some(qualifier),
                    view,
                    &replace,
                );
            }
            if alias_hit.is_none() && !is_known_schema {
                if let Some(b) = bindings
                    .iter()
                    .find(|b| b.relation.to_ascii_lowercase() == qi)
                {
                    collect_columns_for(
                        &mut out,
                        &ctx.kind,
                        &prefix,
                        &b.schema,
                        &b.relation,
                        view,
                        &replace,
                    );
                }
            }
        }
        ContextKind::Relation => {
            collect_relations(&mut out, &ctx.kind, &prefix, None, view, &replace);
            collect_schemas(&mut out, &ctx.kind, &prefix, view, &replace);
            collect_keywords(&mut out, &ctx.kind, &prefix, &replace);
        }
        ContextKind::ColumnList => {
            // Columns from relations in FROM.
            for b in &bindings {
                collect_columns_for(&mut out, &ctx.kind, &prefix, &b.schema, &b.relation, view, &replace);
                // Also surface the alias itself so `u.` can be completed from `u`.
                push_alias(&mut out, &ctx.kind, &prefix, b, &replace);
            }
            collect_functions(&mut out, &ctx.kind, &prefix, None, view, &replace);
            collect_keywords(&mut out, &ctx.kind, &prefix, &replace);
        }
        ContextKind::Expression => {
            for b in &bindings {
                collect_columns_for(&mut out, &ctx.kind, &prefix, &b.schema, &b.relation, view, &replace);
                push_alias(&mut out, &ctx.kind, &prefix, b, &replace);
            }
            collect_functions(&mut out, &ctx.kind, &prefix, None, view, &replace);
            collect_keywords(&mut out, &ctx.kind, &prefix, &replace);
        }
        ContextKind::StatementStart => {
            collect_snippets(&mut out, &ctx.kind, &prefix, &replace);
            collect_keywords(&mut out, &ctx.kind, &prefix, &replace);
        }
    }

    finalize(out)
}

fn collect_schemas(
    out: &mut Vec<CompletionItem>,
    ctx: &ContextKind,
    prefix: &str,
    view: &SchemaView,
    replace: &std::ops::Range<usize>,
) {
    for s in &view.schemas {
        if let Some(score) = prefix_score(prefix, s) {
            out.push(CompletionItem {
                label: s.clone(),
                insert_text: s.clone(),
                detail: Some("schema".into()),
                kind: CompletionKind::Schema,
                sort_score: context_weight(ctx, CompletionKind::Schema) + score,
                is_snippet: false,
                replace_start: replace.start,
                replace_end: replace.end,
            });
        }
    }
}

fn collect_relations(
    out: &mut Vec<CompletionItem>,
    ctx: &ContextKind,
    prefix: &str,
    schema_filter: Option<&str>,
    view: &SchemaView,
    replace: &std::ops::Range<usize>,
) {
    for r in &view.relations {
        if let Some(sf) = schema_filter {
            if !r.schema.eq_ignore_ascii_case(sf) {
                continue;
            }
        }
        if let Some(score) = prefix_score(prefix, &r.name) {
            // When the user hasn't typed a schema qualifier yet, insert the
            // fully-qualified `schema.name` so the resulting query is
            // copy-paste safe across search_path changes. Label stays the
            // bare name so matching still feels natural as the user types.
            // Qualified context already typed the schema — keep the bare name.
            let qualify = schema_filter.is_none();
            let insert_text = if qualify {
                format!("{}.{}", quote_ident(&r.schema), quote_ident(&r.name))
            } else {
                quote_ident(&r.name)
            };
            let detail = if qualify {
                Some(format!("{} · {}", r.schema, kind_label(r.kind)))
            } else {
                Some(kind_label(r.kind).to_string())
            };
            out.push(CompletionItem {
                label: r.name.clone(),
                insert_text,
                detail,
                kind: r.kind,
                sort_score: context_weight(ctx, r.kind) + score,
                is_snippet: false,
                replace_start: replace.start,
                replace_end: replace.end,
            });
        }
    }
}

/// Quote an identifier only when it needs it — mixed case, non-ASCII, reserved
/// words, or characters outside `[a-z0-9_]` starting with a letter/underscore.
/// Matches Postgres's own `quote_ident` rule.
fn quote_ident(ident: &str) -> String {
    let first_ok = ident
        .chars()
        .next()
        .map(|c| c == '_' || c.is_ascii_lowercase())
        .unwrap_or(false);
    let all_ok = ident
        .chars()
        .all(|c| c == '_' || c.is_ascii_lowercase() || c.is_ascii_digit());
    if first_ok && all_ok && !ident.is_empty() {
        ident.to_string()
    } else {
        format!("\"{}\"", ident.replace('"', "\"\""))
    }
}

fn collect_columns_for(
    out: &mut Vec<CompletionItem>,
    ctx: &ContextKind,
    prefix: &str,
    schema_hint: &Option<String>,
    relation: &str,
    view: &SchemaView,
    replace: &std::ops::Range<usize>,
) {
    for c in &view.columns {
        if !c.relation.eq_ignore_ascii_case(relation) {
            continue;
        }
        if let Some(sh) = schema_hint {
            if !c.schema.eq_ignore_ascii_case(sh) {
                continue;
            }
        }
        if let Some(score) = prefix_score(prefix, &c.name) {
            out.push(CompletionItem {
                label: c.name.clone(),
                insert_text: c.name.clone(),
                detail: Some(format!("{}.{} · {}", c.relation, c.name, c.type_name)),
                kind: CompletionKind::Column,
                sort_score: context_weight(ctx, CompletionKind::Column) + score,
                is_snippet: false,
                replace_start: replace.start,
                replace_end: replace.end,
            });
        }
    }
}

fn collect_functions(
    out: &mut Vec<CompletionItem>,
    ctx: &ContextKind,
    prefix: &str,
    schema_filter: Option<&str>,
    view: &SchemaView,
    replace: &std::ops::Range<usize>,
) {
    for f in &view.functions {
        if let Some(sf) = schema_filter {
            if !f.schema.eq_ignore_ascii_case(sf) {
                continue;
            }
        }
        if let Some(score) = prefix_score(prefix, &f.name) {
            out.push(CompletionItem {
                label: f.name.clone(),
                insert_text: format!("{}($0)", f.name),
                detail: Some(format!("({}) \u{2192} {}", f.args, f.result)),
                kind: CompletionKind::Function,
                sort_score: context_weight(ctx, CompletionKind::Function) + score,
                is_snippet: true,
                replace_start: replace.start,
                replace_end: replace.end,
            });
        }
    }
}

fn collect_keywords(
    out: &mut Vec<CompletionItem>,
    ctx: &ContextKind,
    prefix: &str,
    replace: &std::ops::Range<usize>,
) {
    for k in TOP_SUGGESTION_KEYWORDS {
        if let Some(score) = prefix_score(prefix, k) {
            out.push(CompletionItem {
                label: k.to_string(),
                insert_text: k.to_ascii_uppercase(),
                detail: Some("keyword".into()),
                kind: CompletionKind::Keyword,
                sort_score: context_weight(ctx, CompletionKind::Keyword) + score,
                is_snippet: false,
                replace_start: replace.start,
                replace_end: replace.end,
            });
        }
    }
}

fn collect_snippets(
    out: &mut Vec<CompletionItem>,
    ctx: &ContextKind,
    prefix: &str,
    replace: &std::ops::Range<usize>,
) {
    for snip in BUILTIN {
        if let Some(score) = prefix_score(prefix, snip.trigger) {
            push_snippet(out, ctx, snip, score, replace);
        }
    }
}

fn push_snippet(
    out: &mut Vec<CompletionItem>,
    ctx: &ContextKind,
    snip: &Snippet,
    score: i32,
    replace: &std::ops::Range<usize>,
) {
    out.push(CompletionItem {
        label: snip.label.into(),
        insert_text: snip.body.into(),
        detail: Some(snip.description.into()),
        kind: CompletionKind::Snippet,
        sort_score: context_weight(ctx, CompletionKind::Snippet) + score,
        is_snippet: true,
        replace_start: replace.start,
        replace_end: replace.end,
    });
}

fn push_alias(
    out: &mut Vec<CompletionItem>,
    ctx: &ContextKind,
    prefix: &str,
    b: &crate::aliases::FromBinding,
    replace: &std::ops::Range<usize>,
) {
    if let Some(score) = prefix_score(prefix, &b.alias) {
        out.push(CompletionItem {
            label: b.alias.clone(),
            insert_text: b.alias.clone(),
            detail: Some(format!(
                "alias for {}",
                if let Some(s) = &b.schema {
                    format!("{s}.{}", b.relation)
                } else {
                    b.relation.clone()
                }
            )),
            kind: CompletionKind::Alias,
            sort_score: context_weight(ctx, CompletionKind::Alias) + score,
            is_snippet: false,
            replace_start: replace.start,
            replace_end: replace.end,
        });
    }
}

fn finalize(mut items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    // Stable sort so duplicate labels resolve deterministically.
    items.sort_by(|a, b| b.sort_score.cmp(&a.sort_score).then_with(|| a.label.cmp(&b.label)));
    // Cap to a reasonable ceiling to keep IPC payload and UI responsive.
    items.truncate(200);
    items
}

fn kind_label(k: CompletionKind) -> &'static str {
    match k {
        CompletionKind::Table => "table",
        CompletionKind::View => "view",
        CompletionKind::MaterializedView => "matview",
        CompletionKind::Schema => "schema",
        CompletionKind::Column => "column",
        CompletionKind::Function => "function",
        CompletionKind::Keyword => "keyword",
        CompletionKind::Snippet => "snippet",
        CompletionKind::Alias => "alias",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_view() -> SchemaView {
        SchemaView {
            schemas: vec!["public".into(), "auth".into()],
            relations: vec![
                RelationLite { schema: "public".into(), name: "users".into(), kind: CompletionKind::Table },
                RelationLite { schema: "public".into(), name: "orders".into(), kind: CompletionKind::Table },
                RelationLite { schema: "auth".into(), name: "sessions".into(), kind: CompletionKind::Table },
            ],
            columns: vec![
                ColumnLite { schema: "public".into(), relation: "users".into(), name: "id".into(), type_name: "bigint".into() },
                ColumnLite { schema: "public".into(), relation: "users".into(), name: "email".into(), type_name: "text".into() },
                ColumnLite { schema: "public".into(), relation: "users".into(), name: "created_at".into(), type_name: "timestamptz".into() },
                ColumnLite { schema: "public".into(), relation: "orders".into(), name: "id".into(), type_name: "bigint".into() },
                ColumnLite { schema: "public".into(), relation: "orders".into(), name: "user_id".into(), type_name: "bigint".into() },
            ],
            functions: vec![],
        }
    }

    #[test]
    fn completion_after_from() {
        let sql = "SELECT * FROM u";
        let v = sample_view();
        let items = complete(sql, sql.len(), &v);
        let top: Vec<_> = items.iter().take(3).map(|i| i.label.clone()).collect();
        assert!(top.contains(&"users".to_string()), "top: {top:?}");
    }

    #[test]
    fn alias_dot_returns_columns() {
        let sql = "SELECT u. FROM users u";
        let cursor = "SELECT u.".len();
        let v = sample_view();
        let items = complete(sql, cursor, &v);
        let labels: Vec<_> = items.iter().map(|i| i.label.clone()).collect();
        assert!(labels.contains(&"email".to_string()), "labels: {labels:?}");
        assert!(labels.contains(&"id".to_string()));
    }

    #[test]
    fn ssf_snippet_at_statement_start() {
        let sql = "ssf";
        let items = complete(sql, sql.len(), &SchemaView::default());
        assert!(items.iter().any(|i| i.label.starts_with("ssf")));
    }

    #[test]
    fn from_completion_is_schema_qualified() {
        let sql = "SELECT * FROM u";
        let items = complete(sql, sql.len(), &sample_view());
        let users = items.iter().find(|i| i.label == "users").expect("users");
        assert_eq!(users.insert_text, "public.users");
    }

    #[test]
    fn schema_qualifier_filters_relations() {
        let sql = "SELECT * FROM auth.";
        let cursor = sql.len();
        let v = sample_view();
        let items = complete(sql, cursor, &v);
        let labels: Vec<_> = items.iter().map(|i| i.label.clone()).collect();
        assert!(labels.contains(&"sessions".to_string()), "labels: {labels:?}");
        assert!(!labels.contains(&"users".to_string()), "should not include public.users");
    }
}
