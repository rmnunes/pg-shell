//! Postgres schema cache.
//!
//! - `introspect.rs` runs the `pg_catalog` queries that read the live catalog.
//! - `store.rs` keeps per-profile catalogs in memory (DashMap) and performs
//!   lazy fetches so expanding an unseen schema triggers exactly one query.
//! - `scripting.rs` produces SELECT / INSERT templates for a relation.

mod introspect;
mod scripting;
mod store;
mod types;

pub use introspect::object_definition;
pub use scripting::{script_as_insert, script_as_select};
pub use store::{SchemaCache, SchemaCacheError, Snapshot};
pub use types::{
    Column, Function, Index, ObjectKind, Relation, RelationKind, Schema, Sequence, Trigger,
};
