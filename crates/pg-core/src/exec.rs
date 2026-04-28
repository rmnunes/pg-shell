//! Streaming query execution and cancellation.
//!
//! ## Flow
//! 1. Acquire a dedicated pooled connection.
//! 2. Read its `pg_backend_pid()` — this is the handle used for cancellation.
//! 3. Stream rows via `sqlx::query(...).fetch(...)`, invoking the caller's
//!    `on_batch` closure with each batch of `BATCH_SIZE`.
//! 4. On completion or error, emit a terminal event.
//!
//! Cancellation is implemented via `SELECT pg_cancel_backend($pid)` issued on a
//! *sibling* connection from the same pool (the doomed one is blocked inside
//! the running query and would deadlock if it tried to cancel itself).

use std::time::Instant;

use futures_util::stream::StreamExt;
use serde::Serialize;
use sqlx::postgres::PgPool;
use sqlx::{Either, Row};

use crate::types::{column_meta, row_to_json, ColumnMeta};

/// Batch size before a partial result is flushed to the caller.
pub const BATCH_SIZE: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("pool not found for profile")]
    NoPool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryStart {
    pub columns: Vec<ColumnMeta>,
    pub backend_pid: i32,
}

/// A command completion produced by a non-row-returning statement in the
/// batch. Multi-statement executions emit one of these per statement that
/// doesn't produce a result set (TRUNCATE, UPDATE without RETURNING, DDL…).
#[derive(Debug, Clone, Serialize)]
pub struct CommandResult {
    /// Zero-based index within the submitted batch.
    pub index: u32,
    pub rows_affected: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryDone {
    pub total_rows: u64,
    pub total_commands: u32,
    pub duration_ms: u64,
    pub cancelled: bool,
}

/// Execute `sql` streaming row batches through `on_batch` and command
/// completions through `on_command`. Returns when the batch is finished
/// (naturally or via cancellation).
///
/// Multi-statement-aware: uses `fetch_many` internally so a batch like
/// `TRUNCATE x; SELECT * FROM y` yields one `CommandResult` for TRUNCATE
/// followed by streamed rows for the SELECT.
///
/// - `on_start` is called the first time a result-set-producing statement is
///   seen, or — if none in the batch — after completion with empty columns so
///   the UI can still render.
/// - `on_batch` receives `BATCH_SIZE`-sized (or smaller final) batches of
///   rows, each row being a JSON array matching the column order.
/// - `on_command` fires once per non-row-returning statement.
pub async fn execute_streaming<S, B, C>(
    pool: PgPool,
    sql: String,
    mut on_start: S,
    mut on_batch: B,
    mut on_command: C,
) -> Result<QueryDone, ExecError>
where
    S: FnMut(QueryStart),
    B: FnMut(Vec<serde_json::Value>),
    C: FnMut(CommandResult),
{
    let start_time = Instant::now();

    // Dedicated connection so pg_cancel_backend() on the recorded PID targets
    // *this* query's connection specifically.
    let mut conn = pool.acquire().await?;
    let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await?;

    // `fetch_many` yields Either<QueryResult, Row>. A statement that returns
    // rows emits a series of Rows followed by a QueryResult with the command
    // tag; a non-returning statement emits only the QueryResult.
    let query = sqlx::query(&sql);
    // `Query::fetch_many` is deprecated in sqlx 0.8 in favor of the Executor
    // trait approach, but the trait approach lifetimes fight the borrow
    // checker here. Keep the deprecated path until sqlx 0.9 stabilizes.
    #[allow(deprecated)]
    let mut stream = query.fetch_many(&mut *conn);

    let mut columns_reported = false;
    let mut batch: Vec<serde_json::Value> = Vec::with_capacity(BATCH_SIZE);
    let mut total_rows: u64 = 0;
    let mut total_commands: u32 = 0;
    let mut stmt_index: u32 = 0;
    // True once the current statement has emitted at least one row — used to
    // decide whether its terminating QueryResult should be reported as a
    // "command" or swallowed (it's just the end-of-result-set marker).
    let mut current_produced_rows = false;
    let mut cancelled = false;
    let mut last_err: Option<sqlx::Error> = None;

    while let Some(item) = stream.next().await {
        match item {
            Ok(Either::Right(row)) => {
                if !columns_reported {
                    let cols: Vec<ColumnMeta> = row.columns().iter().map(column_meta).collect();
                    on_start(QueryStart {
                        columns: cols,
                        backend_pid,
                    });
                    columns_reported = true;
                }
                batch.push(row_to_json(&row));
                total_rows += 1;
                current_produced_rows = true;
                if batch.len() >= BATCH_SIZE {
                    let drained = std::mem::replace(&mut batch, Vec::with_capacity(BATCH_SIZE));
                    on_batch(drained);
                }
            }
            Ok(Either::Left(qr)) => {
                // Flush any pending rows so the UI sees them before the
                // command summary arrives.
                if !batch.is_empty() {
                    let drained = std::mem::replace(&mut batch, Vec::with_capacity(BATCH_SIZE));
                    on_batch(drained);
                }
                if !current_produced_rows {
                    on_command(CommandResult {
                        index: stmt_index,
                        rows_affected: qr.rows_affected(),
                    });
                    total_commands += 1;
                }
                stmt_index += 1;
                current_produced_rows = false;
            }
            Err(e) => {
                cancelled = is_cancel(&e);
                last_err = Some(e);
                break;
            }
        }
    }
    if !batch.is_empty() {
        on_batch(batch);
    }

    // Nothing row-producing in the batch — tell the UI we're done with empty
    // columns so it can render a command-only summary.
    if !columns_reported {
        on_start(QueryStart {
            columns: Vec::new(),
            backend_pid,
        });
    }

    let duration_ms = start_time.elapsed().as_millis() as u64;

    if let Some(e) = last_err {
        if cancelled {
            return Ok(QueryDone {
                total_rows,
                total_commands,
                duration_ms,
                cancelled: true,
            });
        }
        return Err(ExecError::Sqlx(e));
    }

    Ok(QueryDone {
        total_rows,
        total_commands,
        duration_ms,
        cancelled: false,
    })
}

fn is_cancel(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db) = err {
        // Postgres SQLSTATE 57014 = query_canceled
        if let Some(code) = db.code() {
            return code == "57014";
        }
    }
    false
}

/// Issue `pg_cancel_backend(pid)` over a sibling connection.
pub async fn cancel_backend(pool: &PgPool, pid: i32) -> Result<bool, ExecError> {
    let res: (bool,) = sqlx::query_as("SELECT pg_cancel_backend($1)")
        .bind(pid)
        .fetch_one(pool)
        .await?;
    Ok(res.0)
}

