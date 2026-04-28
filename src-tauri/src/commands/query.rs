//! Query execution and cancellation commands.
//!
//! The execution model is event-driven:
//! - Frontend calls `query_execute(profile_id, sql, query_id)` and immediately
//!   returns.
//! - Backend emits events on the tauri window keyed by `query_id`:
//!     - `query:start`   — columns + backend pid for cancellation
//!     - `query:rows`    — a batch of rows (array of arrays)
//!     - `query:done`    — terminal event with totals and duration
//!     - `query:error`   — terminal event with message
//! - Frontend filters events by `query_id` in the payload.

use pg_core::{cancel_backend, execute_streaming, CommandResult, QueryDone, QueryStart};
use pg_intellisense::ddl::{detect as detect_ddl, DdlEffect};
use pg_schema_cache::SchemaCache;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult};
use crate::state::{ActiveQuery, AppState};

#[derive(Debug, Clone, Serialize)]
struct StartPayload {
    query_id: String,
    #[serde(flatten)]
    info: QueryStart,
}

#[derive(Debug, Clone, Serialize)]
struct RowsPayload {
    query_id: String,
    batch_index: u32,
    rows: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
struct CommandPayload {
    query_id: String,
    #[serde(flatten)]
    info: CommandResult,
}

#[derive(Debug, Clone, Serialize)]
struct DonePayload {
    query_id: String,
    #[serde(flatten)]
    info: QueryDone,
}

#[derive(Debug, Clone, Serialize)]
struct ErrorPayload {
    query_id: String,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
enum InvalidationScope {
    Profile,
    Schema {
        schema: String,
    },
    Relation {
        schema: Option<String>,
        name: String,
    },
}

#[derive(Debug, Clone, Serialize)]
struct SchemaInvalidatedPayload {
    profile_id: String,
    /// Bounded list of effects for diagnostics; the cache itself is already
    /// invalidated by the time this event fires. Frontend can ignore the
    /// detail and just re-fetch — the field is provided for logging.
    effects: Vec<InvalidationScope>,
}

#[tauri::command]
pub async fn query_execute(
    profile_id: String,
    sql: String,
    query_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<()> {
    let pool = state
        .connections
        .pool(&profile_id)
        .ok_or_else(|| AppError::new("not_connected", "no active connection for this profile"))?;

    // Record an in-flight row so even crashed / never-completing queries are
    // visible in history. The row is updated on completion or error.
    let history_id = state
        .history
        .record_started(&profile_id, &sql)
        .map_err(|e| AppError::new("history", e.to_string()))
        .ok();

    let registry = state.active_queries.clone();
    let history = state.history.clone();
    let schema_cache = state.schema_cache.clone();
    let qid = query_id.clone();
    let pid_profile = profile_id.clone();
    let sql_for_ddl = sql.clone();

    tokio::spawn(async move {
        let mut batch_index: u32 = 0;
        let start_emitter = {
            let app = app.clone();
            let qid = qid.clone();
            let registry = registry.clone();
            let profile = pid_profile.clone();
            move |info: QueryStart| {
                registry.insert(
                    qid.clone(),
                    ActiveQuery {
                        profile_id: profile.clone(),
                        backend_pid: info.backend_pid,
                    },
                );
                let _ = app.emit(
                    "query:start",
                    StartPayload {
                        query_id: qid.clone(),
                        info,
                    },
                );
            }
        };
        let batch_emitter = {
            let app = app.clone();
            let qid = qid.clone();
            move |rows: Vec<serde_json::Value>| {
                let payload = RowsPayload {
                    query_id: qid.clone(),
                    batch_index,
                    rows,
                };
                batch_index += 1;
                let _ = app.emit("query:rows", payload);
            }
        };
        let command_emitter = {
            let app = app.clone();
            let qid = qid.clone();
            move |info: CommandResult| {
                let _ = app.emit(
                    "query:command",
                    CommandPayload {
                        query_id: qid.clone(),
                        info,
                    },
                );
            }
        };

        let result =
            execute_streaming(pool, sql, start_emitter, batch_emitter, command_emitter).await;

        registry.remove(&qid);

        match result {
            Ok(done) => {
                if let Some(hid) = history_id {
                    let _ = history.record_complete(
                        hid,
                        done.duration_ms as i64,
                        done.total_rows as i64,
                        done.cancelled,
                    );
                }
                // DDL-triggered cache refresh. Skip when cancelled — partial
                // execution may have been rolled back, and re-introspecting
                // is wasted work.
                if !done.cancelled {
                    let effects = detect_ddl(&sql_for_ddl);
                    if !effects.is_empty() {
                        apply_ddl_effects(&schema_cache, &pid_profile, &effects);
                        let _ = app.emit(
                            "schema:invalidated",
                            SchemaInvalidatedPayload {
                                profile_id: pid_profile.clone(),
                                effects: effects.iter().map(serialize_effect).collect(),
                            },
                        );
                    }
                }
                let _ = app.emit(
                    "query:done",
                    DonePayload {
                        query_id: qid,
                        info: done,
                    },
                );
            }
            Err(e) => {
                if let Some(hid) = history_id {
                    let _ = history.record_error(hid, &e.to_string());
                }
                let _ = app.emit(
                    "query:error",
                    ErrorPayload {
                        query_id: qid,
                        message: e.to_string(),
                    },
                );
            }
        }
    });

    Ok(())
}

fn apply_ddl_effects(cache: &SchemaCache, profile_id: &str, effects: &[DdlEffect]) {
    for effect in effects {
        match effect {
            DdlEffect::Profile => cache.invalidate_profile(profile_id),
            DdlEffect::Schema(s) => cache.invalidate_schema(profile_id, s),
            DdlEffect::Relation { schema, name } => {
                // When the schema isn't qualified we don't know what to drop
                // precisely — fall back to a profile-wide flush. It costs one
                // round-trip on the next completion request.
                match schema {
                    Some(s) => cache.invalidate_relation(profile_id, s, name),
                    None => cache.invalidate_profile(profile_id),
                }
            }
        }
    }
}

fn serialize_effect(effect: &DdlEffect) -> InvalidationScope {
    match effect {
        DdlEffect::Profile => InvalidationScope::Profile,
        DdlEffect::Schema(s) => InvalidationScope::Schema { schema: s.clone() },
        DdlEffect::Relation { schema, name } => InvalidationScope::Relation {
            schema: schema.clone(),
            name: name.clone(),
        },
    }
}

#[tauri::command]
pub async fn query_cancel(query_id: String, state: State<'_, AppState>) -> AppResult<bool> {
    let active = match state.active_queries.get(&query_id) {
        Some(a) => a.clone(),
        None => return Ok(false),
    };
    let pool = state
        .connections
        .pool(&active.profile_id)
        .ok_or_else(|| AppError::new("not_connected", "connection closed"))?;
    let ok = cancel_backend(&pool, active.backend_pid)
        .await
        .map_err(|e| AppError::new("cancel", e.to_string()))?;
    Ok(ok)
}
