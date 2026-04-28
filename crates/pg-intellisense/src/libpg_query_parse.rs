//! AST-based FROM binding extractor, active only under the `libpg_query`
//! feature.
//!
//! Why it exists: the pure-Rust `aliases::extract` walker is permissive and
//! does a good job on simple queries but drifts on LATERAL joins, CTEs, and
//! unusual punctuation. When the buffer parses cleanly under Postgres's own
//! grammar (via the `pg_query` crate → libpg_query), we can ask the AST for
//! the canonical bindings and return those instead.
//!
//! ## Build requirement
//! The `pg_query` crate pulls in `bindgen`, which requires LLVM. On Windows:
//! ```
//! winget install LLVM.LLVM
//! # Reopen the shell so LIBCLANG_PATH resolves, then:
//! cargo build --features pg-intellisense/libpg_query
//! ```
//! Without LLVM this entire module is compiled out — the token walker is the
//! sole implementation.

#![cfg(feature = "libpg_query")]

use crate::aliases::FromBinding;

/// Try to extract FROM bindings via a full parse. Returns `None` when the
/// buffer doesn't parse (in which case the caller falls back to the token
/// walker). Currently handles top-level SELECTs and their JOINs; LATERAL /
/// subquery / CTE expansion land in a follow-up.
pub fn try_extract(sql: &str) -> Option<Vec<FromBinding>> {
    let parsed = pg_query::parse(sql).ok()?;
    let mut out = Vec::new();

    for raw in parsed.protobuf.stmts.iter() {
        let Some(stmt) = raw.stmt.as_ref() else {
            continue;
        };
        if let Some(n) = stmt.node.as_ref() {
            walk_stmt(n, &mut out);
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn walk_stmt(node: &pg_query::protobuf::node::Node, out: &mut Vec<FromBinding>) {
    use pg_query::protobuf::node::Node;
    match node {
        Node::SelectStmt(sel) => {
            for f in sel.from_clause.iter() {
                if let Some(inner) = f.node.as_ref() {
                    walk_range(inner, out);
                }
            }
        }
        Node::UpdateStmt(u) => {
            if let Some(rel) = u.relation.as_ref() {
                push_range_var(rel, out);
            }
        }
        Node::InsertStmt(i) => {
            if let Some(rel) = i.relation.as_ref() {
                push_range_var(rel, out);
            }
        }
        Node::DeleteStmt(d) => {
            if let Some(rel) = d.relation.as_ref() {
                push_range_var(rel, out);
            }
        }
        _ => {}
    }
}

fn walk_range(node: &pg_query::protobuf::node::Node, out: &mut Vec<FromBinding>) {
    use pg_query::protobuf::node::Node;
    match node {
        Node::RangeVar(rv) => push_range_var(rv, out),
        Node::JoinExpr(j) => {
            if let Some(l) = j.larg.as_ref().and_then(|x| x.node.as_ref()) {
                walk_range(l, out);
            }
            if let Some(r) = j.rarg.as_ref().and_then(|x| x.node.as_ref()) {
                walk_range(r, out);
            }
        }
        _ => {}
    }
}

fn push_range_var(rv: &pg_query::protobuf::RangeVar, out: &mut Vec<FromBinding>) {
    let schema = if rv.schemaname.is_empty() {
        None
    } else {
        Some(rv.schemaname.clone())
    };
    let relation = rv.relname.clone();
    if relation.is_empty() {
        return;
    }
    let alias = rv
        .alias
        .as_ref()
        .map(|a| a.aliasname.clone())
        .filter(|a| !a.is_empty())
        .unwrap_or_else(|| relation.clone());
    out.push(FromBinding {
        schema,
        relation,
        alias,
    });
}
