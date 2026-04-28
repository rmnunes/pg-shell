//! Signature help lookup.
//!
//! Monaco's `SignatureHelpProvider` triggers on `(` and `,`; it tells us the
//! function name and the zero-based argument index. We return every overload
//! matching the name from the warm schema cache — Monaco renders them in a
//! stacked popup with arrow navigation.

use pg_schema_cache::Function;
use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct SignatureParam {
    pub label: String,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionSignature {
    pub schema: String,
    pub name: String,
    /// Full `foo(a int, b text) -> bool`-style header used as Monaco's label.
    pub label: String,
    pub parameters: Vec<SignatureParam>,
    pub result: String,
    /// `f` | `p` | `a` | `w` — function / procedure / aggregate / window.
    pub kind: String,
}

/// Return matching signatures. `name` may be plain (`coalesce`) or
/// schema-qualified (`pg_catalog.now`). Case-insensitive.
#[tauri::command]
pub async fn signature_help(
    profile_id: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<FunctionSignature>> {
    let pool = state
        .connections
        .pool(&profile_id)
        .ok_or_else(|| AppError::new("not_connected", "no active connection"))?;

    let (schema_filter, fn_name) = match name.split_once('.') {
        Some((s, n)) => (Some(s.to_ascii_lowercase()), n.to_ascii_lowercase()),
        None => (None, name.to_ascii_lowercase()),
    };

    let snapshot = state
        .schema_cache
        .build_snapshot(&profile_id, &pool, &[])
        .await
        .map_err(|e| AppError::new("schema_cache", e.to_string()))?;

    Ok(snapshot
        .functions
        .iter()
        .filter(|f| {
            f.name.to_ascii_lowercase() == fn_name
                && match &schema_filter {
                    Some(s) => f.schema.to_ascii_lowercase() == *s,
                    None => true,
                }
        })
        .map(to_signature)
        .collect())
}

fn to_signature(f: &Function) -> FunctionSignature {
    let parameters = parse_args(&f.args);
    let label = format!("{}({}) \u{2192} {}", f.name, f.args, f.result);
    FunctionSignature {
        schema: f.schema.clone(),
        name: f.name.clone(),
        label,
        parameters,
        result: f.result.clone(),
        kind: f.kind.to_string(),
    }
}

/// Split `"a int, b text, c jsonb DEFAULT '{}'::jsonb"` into parameter tokens.
/// Respects balanced parens + single-quoted literals so commas inside default
/// expressions don't split the list.
fn parse_args(args: &str) -> Vec<SignatureParam> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut start = 0usize;
    let bytes = args.as_bytes();
    for i in 0..bytes.len() {
        let c = bytes[i];
        if in_quote {
            if c == b'\'' {
                // doubled '' stays inside the literal
                if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                    continue;
                }
                in_quote = false;
            }
            continue;
        }
        match c {
            b'\'' => in_quote = true,
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => {
                push_param(&args[start..i], &mut out);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < args.len() {
        push_param(&args[start..], &mut out);
    }
    out
}

fn push_param(raw: &str, out: &mut Vec<SignatureParam>) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    out.push(SignatureParam {
        label: trimmed.to_string(),
        documentation: None,
    });
}
