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
    let qid = query_id.clone();
    let pid_profile = profile_id.clone();

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
