//! SQL intellisense engine for pg-shell.
//!
//! Design (see architecture plan):
//! - Tokenize every keystroke → `Vec<Token>` (never fails).
//! - Detect context by walking tokens backward from the cursor.
//! - Extract FROM-clause bindings to resolve `alias.` completions.
//! - Score candidates with `context_weight × prefix_quality`.
//!
//! The `libpg_query` feature adds an AST-based partial-parse path for stricter
//! alias resolution. It's off by default so the crate builds without LLVM on
//! Windows.

pub mod aliases;
pub mod context;
pub mod ddl;
pub mod engine;
#[cfg(feature = "libpg_query")]
pub mod libpg_query_parse;
pub mod ranker;
pub mod snippets;
pub mod tokenize;

pub use engine::{
    complete, ColumnLite, CompletionItem, CompletionKind, FunctionLite, RelationLite, SchemaView,
};
