//! Postgres connection pool manager + query execution primitives.

mod exec;
mod pool;
pub mod types;

pub use exec::{
    cancel_backend, execute_streaming, CommandResult, ExecError, QueryDone, QueryStart, BATCH_SIZE,
};
pub use pool::{ConnectionManager, ConnectionManagerError, ServerInfo, TestOutcome};
pub use types::{ColumnMeta, RenderKind};
