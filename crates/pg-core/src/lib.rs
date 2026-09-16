//! Postgres connection pool manager + query execution primitives.

mod exec;
mod meta;
mod pool;
pub mod types;

pub use exec::{
    cancel_backend, execute_streaming, CommandResult, ExecError, QueryDone, QueryStart, BATCH_SIZE,
};
pub use futures_util::future::BoxFuture;
pub use meta::{strip_psql_meta, PsqlMetaCommand};
pub use pool::{
    AccessToken, ConnectionManager, ConnectionManagerError, Credential, ServerInfo, TestOutcome,
    TokenSource, TokenSourceError,
};
pub use types::{ColumnMeta, RenderKind};
