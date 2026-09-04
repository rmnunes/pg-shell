//! Postgres connection pool manager + query execution primitives.

mod exec;
mod pool;
pub mod types;

pub use exec::{
    cancel_backend, execute_streaming, CommandResult, ExecError, QueryDone, QueryStart, BATCH_SIZE,
};
pub use futures_util::future::BoxFuture;
pub use pool::{
    AccessToken, ConnectionManager, ConnectionManagerError, Credential, ServerInfo, TestOutcome,
    TokenSource, TokenSourceError,
};
pub use types::{ColumnMeta, RenderKind};
