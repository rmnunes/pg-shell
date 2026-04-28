//! `completion_get` — single fast IPC endpoint Monaco calls on every
//! keystroke.
//!
//! Flow:
//! 1. Quickly tokenize + extract FROM bindings to know which relations need
//!    columns fetched.
//! 2. Build a `Snapshot` of the schema cache. If a relation in the bindings
//!    has never had its columns fetched, the snapshot triggers one.
//! 3. Transform the snapshot into the engine's `SchemaView` and ask for
//!    completions.

use pg_intellisense::{
    complete, ColumnLite, CompletionItem, CompletionKind, FunctionLite, RelationLite, SchemaView,
};
use pg_schema_cache::{RelationKind, Snapshot};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::mru::MruStore;
use crate::state::AppState;

#[tauri::command]
pub async fn completion_get(
    profile_id: String,
    doc: String,
    cursor_offset: usize,
    state: State<'_, AppState>,
) -> AppResult<Vec<CompletionItem>> {
    let pool = state
        .connections
        .pool(&profile_id)
        .ok_or_else(|| AppError::new("not_connected", "no active connection"))?;

    // Peek at the bindings so we know which relations to fetch columns for.
    // We parse the doc here rather than inside the engine because the engine
    // is sync and the snapshot-build is async — separation keeps the engine
    // test-friendly.
    let tokens = pg_intellisense::tokenize::tokenize(&doc);
    let bindings = pg_intellisense::aliases::extract(&tokens);
    let fetch_targets: Vec<(Option<String>, String)> = bindings
        .iter()
        .map(|b| (b.schema.clone(), b.relation.clone()))
        .collect();

    let snapshot = state
        .schema_cache
        .build_snapshot(&profile_id, &pool, &fetch_targets)
        .await
        .map_err(|e| AppError::new("schema_cache", e.to_string()))?;

    let view = to_view(&snapshot);
    let mut items = complete(&doc, cursor_offset, &view);

    // Apply MRU boost so previously-accepted items float up. Read the full
    // set of accept counts for this profile once; SQLite + a small profile
    // map makes this microseconds-cheap.
    if let Ok(counts) = state.mru.counts_for(&profile_id) {
        for it in items.iter_mut() {
            let kind_str = mru_kind_label(it.kind);
            if let Some(c) = counts.get(&(kind_str.to_string(), it.label.clone())) {
                it.sort_score += MruStore::boost(*c);
            }
        }
        items.sort_by(|a, b| {
            b.sort_score
                .cmp(&a.sort_score)
                .then_with(|| a.label.cmp(&b.label))
        });
    }

    Ok(items)
}

#[tauri::command]
pub async fn completion_accept(
    profile_id: String,
    kind: String,
    identifier: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    state
        .mru
        .record(&profile_id, &kind, &identifier)
        .map_err(|e| AppError::new("mru", e.to_string()))
}

fn mru_kind_label(k: CompletionKind) -> &'static str {
    match k {
        CompletionKind::Keyword => "keyword",
        CompletionKind::Snippet => "snippet",
        CompletionKind::Schema => "schema",
        CompletionKind::Table => "table",
        CompletionKind::View => "view",
        CompletionKind::MaterializedView => "materialized_view",
        CompletionKind::Column => "column",
        CompletionKind::Function => "function",
        CompletionKind::Alias => "alias",
    }
}

fn to_view(s: &Snapshot) -> SchemaView {
    let schemas = s.schemas.iter().map(|x| x.name.clone()).collect();

    let relations = s
        .relations
        .iter()
        .map(|r| RelationLite {
            schema: r.schema.clone(),
            name: r.name.clone(),
            kind: match r.kind {
                RelationKind::Table
                | RelationKind::PartitionedTable
                | RelationKind::ForeignTable => CompletionKind::Table,
                RelationKind::View => CompletionKind::View,
                RelationKind::MaterializedView => CompletionKind::MaterializedView,
            },
        })
        .collect();

    let columns = s
        .columns
        .iter()
        .map(|(schema, relation, c)| ColumnLite {
            schema: schema.clone(),
            relation: relation.clone(),
            name: c.name.clone(),
            type_name: c.type_name.clone(),
        })
        .collect();

    let functions = s
        .functions
        .iter()
        .map(|f| FunctionLite {
            schema: f.schema.clone(),
            name: f.name.clone(),
            args: f.args.clone(),
            result: f.result.clone(),
        })
        .collect();

    SchemaView {
        schemas,
        relations,
        columns,
        functions,
    }
}
