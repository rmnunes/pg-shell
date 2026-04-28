//! Schema tree, scripting, and object-definition commands.
//!
//! The tree is navigated by path segments so a single IPC endpoint
//! (`schema_browse`) serves every level:
//!
//! | Path                               | Returns                                  |
//! |------------------------------------|------------------------------------------|
//! | `[]`                               | schemas                                  |
//! | `["<schema>"]`                     | category groups (Tables / Views / …)     |
//! | `["<schema>", "tables"]`           | relations of that kind                   |
//! | `["<schema>", "functions"]`        | functions                                |
//! | `["<schema>", "tables", "<tbl>"]`  | columns                                  |

use pg_schema_cache::{
    script_as_insert as bck_script_as_insert, script_as_select as bck_script_as_select, Column,
    Function, Index, ObjectKind, Relation, RelationKind, Schema, Sequence, Trigger,
};
use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TreeNodeKind {
    Schema,
    Category,
    Table,
    View,
    MaterializedView,
    PartitionedTable,
    ForeignTable,
    Function,
    Procedure,
    Aggregate,
    Window,
    Column,
    Sequence,
    Trigger,
    Index,
    PrimaryKeyIndex,
    UniqueIndex,
}

#[derive(Debug, Clone, Serialize)]
pub struct TreeNode {
    pub kind: TreeNodeKind,
    /// Path from the root required to re-target this node. Clients pass this
    /// back to expand / refresh this node.
    pub path: Vec<String>,
    /// Display label.
    pub label: String,
    /// Secondary info: type name for columns, args/result for functions,
    /// owner for schemas, etc.
    pub detail: Option<String>,
    /// False for terminal nodes (columns) so the UI doesn't show a chevron.
    pub expandable: bool,
}

fn schema_node(s: &Schema) -> TreeNode {
    TreeNode {
        kind: TreeNodeKind::Schema,
        path: vec![s.name.clone()],
        label: s.name.clone(),
        detail: if s.owner.is_empty() {
            None
        } else {
            Some(format!("owner: {}", s.owner))
        },
        expandable: true,
    }
}

fn category_nodes(schema: &str) -> Vec<TreeNode> {
    let mk = |seg: &str, label: &str| TreeNode {
        kind: TreeNodeKind::Category,
        path: vec![schema.to_string(), seg.to_string()],
        label: label.into(),
        detail: None,
        expandable: true,
    };
    vec![
        mk("tables", "Tables"),
        mk("views", "Views"),
        mk("matviews", "Materialized Views"),
        mk("functions", "Functions"),
        mk("sequences", "Sequences"),
        mk("triggers", "Triggers"),
    ]
}

/// Sub-categories shown when a table is expanded. The path segment after the
/// table name selects which list to fetch (`columns`, `indexes`, `triggers`).
fn table_subcategories(schema: &str, category: &str, table: &str) -> Vec<TreeNode> {
    let mk = |seg: &str, label: &str| TreeNode {
        kind: TreeNodeKind::Category,
        path: vec![
            schema.to_string(),
            category.to_string(),
            table.to_string(),
            seg.to_string(),
        ],
        label: label.into(),
        detail: None,
        expandable: true,
    };
    vec![
        mk("columns", "Columns"),
        mk("indexes", "Indexes"),
        mk("triggers", "Triggers"),
    ]
}

fn relation_node(r: &Relation) -> TreeNode {
    let kind = match r.kind {
        RelationKind::Table => TreeNodeKind::Table,
        RelationKind::View => TreeNodeKind::View,
        RelationKind::MaterializedView => TreeNodeKind::MaterializedView,
        RelationKind::PartitionedTable => TreeNodeKind::PartitionedTable,
        RelationKind::ForeignTable => TreeNodeKind::ForeignTable,
    };
    let category = match r.kind {
        RelationKind::Table | RelationKind::PartitionedTable | RelationKind::ForeignTable => {
            "tables"
        }
        RelationKind::View => "views",
        RelationKind::MaterializedView => "matviews",
    };
    TreeNode {
        kind,
        path: vec![r.schema.clone(), category.into(), r.name.clone()],
        label: r.name.clone(),
        detail: None,
        expandable: true,
    }
}

fn function_node(f: &Function) -> TreeNode {
    let kind = match f.kind {
        'p' => TreeNodeKind::Procedure,
        'a' => TreeNodeKind::Aggregate,
        'w' => TreeNodeKind::Window,
        _ => TreeNodeKind::Function,
    };
    TreeNode {
        kind,
        path: vec![
            f.schema.clone(),
            "functions".into(),
            format!("{}({})", f.name, f.args),
        ],
        label: f.name.clone(),
        detail: Some(format!("({}) \u{2192} {}", f.args, f.result)),
        expandable: false,
    }
}

fn sequence_node(s: &Sequence) -> TreeNode {
    TreeNode {
        kind: TreeNodeKind::Sequence,
        path: vec![s.schema.clone(), "sequences".into(), s.name.clone()],
        label: s.name.clone(),
        detail: if s.owner.is_empty() {
            None
        } else {
            Some(format!("owner: {}", s.owner))
        },
        expandable: false,
    }
}

fn trigger_node(t: &Trigger, schema_level: bool) -> TreeNode {
    // At the schema level the trigger path is
    //   [schema, "triggers", "table:name"]
    // At the per-table level it becomes a leaf under the table:
    //   [schema, "tables", table, "triggers", name]
    let path = if schema_level {
        vec![
            t.schema.clone(),
            "triggers".into(),
            format!("{}:{}", t.table, t.name),
        ]
    } else {
        vec![
            t.schema.clone(),
            "tables".into(),
            t.table.clone(),
            "triggers".into(),
            t.name.clone(),
        ]
    };
    let detail = if schema_level {
        format!("{} {} on {}", t.timing, t.event, t.table)
    } else {
        format!("{} {}", t.timing, t.event)
    };
    TreeNode {
        kind: TreeNodeKind::Trigger,
        path,
        label: t.name.clone(),
        detail: Some(detail),
        expandable: false,
    }
}

fn index_node(i: &Index) -> TreeNode {
    let kind = if i.is_primary {
        TreeNodeKind::PrimaryKeyIndex
    } else if i.is_unique {
        TreeNodeKind::UniqueIndex
    } else {
        TreeNodeKind::Index
    };
    // Strip the `CREATE [UNIQUE] INDEX name ON schema.table USING …` preamble
    // for display, keeping only the USING clause. Falls back to full def if
    // the preamble isn't recognizable.
    let detail = i
        .definition
        .split(" USING ")
        .nth(1)
        .map(|s| format!("USING {}", s))
        .unwrap_or_else(|| i.definition.clone());
    TreeNode {
        kind,
        path: vec![
            i.schema.clone(),
            "tables".into(),
            i.table.clone(),
            "indexes".into(),
            i.name.clone(),
        ],
        label: i.name.clone(),
        detail: Some(detail),
        expandable: false,
    }
}

fn column_node(schema: &str, relation: &str, category: &str, c: &Column) -> TreeNode {
    let mut detail = c.type_name.clone();
    if c.not_null {
        detail.push_str(" NOT NULL");
    }
    if let Some(d) = &c.default_expr {
        detail.push_str(&format!(" = {d}"));
    }
    TreeNode {
        kind: TreeNodeKind::Column,
        path: vec![
            schema.to_string(),
            category.to_string(),
            relation.to_string(),
            c.name.clone(),
        ],
        label: c.name.clone(),
        detail: Some(detail),
        expandable: false,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FlatRelation {
    pub schema: String,
    pub name: String,
    pub kind: TreeNodeKind,
    pub qualified: String,
}

/// Flat view: every cached relation across every cached schema, lightweight
/// enough to render as a single scrollable list.
///
/// Does NOT trigger schema/relation fetches — callers are expected to have
/// `schema_cache::warm` in flight (kicked off at connect time).
#[tauri::command]
pub async fn schema_flat(
    profile_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<FlatRelation>> {
    let pool = state
        .connections
        .pool(&profile_id)
        .ok_or_else(|| AppError::new("not_connected", "no active connection"))?;

    let snapshot = state
        .schema_cache
        .build_snapshot(&profile_id, &pool, &[])
        .await
        .map_err(|e| AppError::new("schema_cache", e.to_string()))?;

    Ok(snapshot
        .relations
        .iter()
        .map(|r| {
            let kind = match r.kind {
                RelationKind::Table => TreeNodeKind::Table,
                RelationKind::View => TreeNodeKind::View,
                RelationKind::MaterializedView => TreeNodeKind::MaterializedView,
                RelationKind::PartitionedTable => TreeNodeKind::PartitionedTable,
                RelationKind::ForeignTable => TreeNodeKind::ForeignTable,
            };
            FlatRelation {
                schema: r.schema.clone(),
                name: r.name.clone(),
                kind,
                qualified: format!("{}.{}", r.schema, r.name),
            }
        })
        .collect())
}

#[tauri::command]
pub async fn schema_browse(
    profile_id: String,
    path: Vec<String>,
    state: State<'_, AppState>,
) -> AppResult<Vec<TreeNode>> {
    let pool = state
        .connections
        .pool(&profile_id)
        .ok_or_else(|| AppError::new("not_connected", "no active connection"))?;

    let sc_err =
        |e: pg_schema_cache::SchemaCacheError| AppError::new("schema_cache", e.to_string());

    let out: Vec<TreeNode> = match path.as_slice() {
        [] => {
            let schemas = state
                .schema_cache
                .schemas(&profile_id, &pool)
                .await
                .map_err(sc_err)?;
            schemas.iter().map(schema_node).collect()
        }
        [schema] => category_nodes(schema),
        [schema, category] => match category.as_str() {
            "tables" | "views" | "matviews" => {
                let relations = state
                    .schema_cache
                    .relations(&profile_id, &pool, schema)
                    .await
                    .map_err(sc_err)?;
                let want = |k: RelationKind| match category.as_str() {
                    "tables" => matches!(
                        k,
                        RelationKind::Table
                            | RelationKind::PartitionedTable
                            | RelationKind::ForeignTable
                    ),
                    "views" => k == RelationKind::View,
                    "matviews" => k == RelationKind::MaterializedView,
                    _ => false,
                };
                relations
                    .iter()
                    .filter(|r| want(r.kind))
                    .map(relation_node)
                    .collect()
            }
            "functions" => {
                let functions = state
                    .schema_cache
                    .functions(&profile_id, &pool, schema)
                    .await
                    .map_err(sc_err)?;
                functions.iter().map(function_node).collect()
            }
            "sequences" => {
                let sequences = state
                    .schema_cache
                    .sequences(&profile_id, &pool, schema)
                    .await
                    .map_err(sc_err)?;
                sequences.iter().map(sequence_node).collect()
            }
            "triggers" => {
                let triggers = state
                    .schema_cache
                    .triggers(&profile_id, &pool, schema)
                    .await
                    .map_err(sc_err)?;
                triggers.iter().map(|t| trigger_node(t, true)).collect()
            }
            _ => Vec::new(),
        },
        [schema, category, relation]
            if matches!(category.as_str(), "tables" | "views" | "matviews") =>
        {
            // Tables get sub-categories (Columns / Indexes / Triggers). Views
            // and materialized views keep the old "columns only" behavior so
            // expanding them is still one click.
            if category == "tables" {
                table_subcategories(schema, category, relation)
            } else {
                let cols = state
                    .schema_cache
                    .columns(&profile_id, &pool, schema, relation)
                    .await
                    .map_err(sc_err)?;
                cols.iter()
                    .map(|c| column_node(schema, relation, category, c))
                    .collect()
            }
        }
        [schema, category, relation, subcat] if category == "tables" => match subcat.as_str() {
            "columns" => {
                let cols = state
                    .schema_cache
                    .columns(&profile_id, &pool, schema, relation)
                    .await
                    .map_err(sc_err)?;
                cols.iter()
                    .map(|c| column_node(schema, relation, category, c))
                    .collect()
            }
            "indexes" => {
                let idx = state
                    .schema_cache
                    .indexes(&profile_id, &pool, schema, relation)
                    .await
                    .map_err(sc_err)?;
                idx.iter().map(index_node).collect()
            }
            "triggers" => {
                let triggers = state
                    .schema_cache
                    .triggers(&profile_id, &pool, schema)
                    .await
                    .map_err(sc_err)?;
                triggers
                    .iter()
                    .filter(|t| t.table.eq_ignore_ascii_case(relation))
                    .map(|t| trigger_node(t, false))
                    .collect()
            }
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    Ok(out)
}

#[tauri::command]
pub async fn schema_refresh(
    profile_id: String,
    path: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> AppResult<()> {
    match path.as_deref() {
        None | Some([]) => state.schema_cache.invalidate_profile(&profile_id),
        Some([schema]) => state.schema_cache.invalidate_schema(&profile_id, schema),
        Some([schema, _, relation]) => {
            state
                .schema_cache
                .invalidate_relation(&profile_id, schema, relation)
        }
        _ => state.schema_cache.invalidate_profile(&profile_id),
    }
    Ok(())
}

#[tauri::command]
pub async fn script_as_select(
    profile_id: String,
    schema: String,
    relation: String,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let pool = state
        .connections
        .pool(&profile_id)
        .ok_or_else(|| AppError::new("not_connected", "no active connection"))?;
    bck_script_as_select(&pool, &schema, &relation)
        .await
        .map_err(|e| AppError::new("schema_cache", e.to_string()))
}

#[tauri::command]
pub async fn script_as_insert(
    profile_id: String,
    schema: String,
    relation: String,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let pool = state
        .connections
        .pool(&profile_id)
        .ok_or_else(|| AppError::new("not_connected", "no active connection"))?;
    bck_script_as_insert(&pool, &schema, &relation)
        .await
        .map_err(|e| AppError::new("schema_cache", e.to_string()))
}

#[tauri::command]
pub async fn object_definition(
    profile_id: String,
    kind: ObjectKind,
    schema: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let pool = state
        .connections
        .pool(&profile_id)
        .ok_or_else(|| AppError::new("not_connected", "no active connection"))?;
    pg_schema_cache::object_definition(&pool, kind, &schema, &name)
        .await
        .map_err(|e| AppError::new("schema_cache", e.to_string()))
}
