//! Conversion of `PgRow` cells into serde JSON values for frontend rendering.
//!
//! Strategy: dispatch on `PgTypeInfo::name()` for known types, fall back to a
//! best-effort text decode for unknowns. The frontend receives values already
//! shaped for display (numeric as string to preserve precision, bytea as
//! truncated hex, timestamps as ISO-8601).

use bigdecimal::BigDecimal;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::Serialize;
use serde_json::{Number, Value};
use sqlx::postgres::PgRow;
use sqlx::{Column, Row, TypeInfo};

#[derive(Debug, Clone, Serialize)]
pub struct ColumnMeta {
    pub name: String,
    pub type_name: String,
    /// Semantic rendering category so the frontend can pick a renderer without
    /// re-mapping PG type names.
    pub render_kind: RenderKind,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RenderKind {
    Null,
    Bool,
    /// Integer-ish: right-aligned, plain number.
    Int,
    /// Floating point.
    Float,
    /// Arbitrary precision — emitted as string, right-aligned.
    Numeric,
    Text,
    Uuid,
    /// JSON / JSONB — cell value is an inline JSON value.
    Json,
    Date,
    Time,
    Timestamp,
    TimestampTz,
    /// Byte sequence — emitted as hex-preview object.
    Bytea,
    /// Array of any of the above.
    Array,
    /// Fallback: stringified representation.
    Unknown,
}

pub fn column_meta(col: &sqlx::postgres::PgColumn) -> ColumnMeta {
    let ti = col.type_info();
    let name = ti.name().to_string();
    let kind = classify(&name);
    ColumnMeta {
        name: col.name().to_string(),
        type_name: name,
        render_kind: kind,
    }
}

fn classify(type_name: &str) -> RenderKind {
    // Array types in sqlx carry a trailing "[]", e.g. "INT4[]".
    if type_name.ends_with("[]") {
        return RenderKind::Array;
    }
    match type_name {
        "BOOL" => RenderKind::Bool,
        "INT2" | "INT4" | "INT8" | "OID" => RenderKind::Int,
        "FLOAT4" | "FLOAT8" => RenderKind::Float,
        "NUMERIC" | "MONEY" => RenderKind::Numeric,
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CHAR" | "CITEXT" => RenderKind::Text,
        "UUID" => RenderKind::Uuid,
        "JSON" | "JSONB" => RenderKind::Json,
        "DATE" => RenderKind::Date,
        "TIME" | "TIMETZ" => RenderKind::Time,
        "TIMESTAMP" => RenderKind::Timestamp,
        "TIMESTAMPTZ" => RenderKind::TimestampTz,
        "BYTEA" => RenderKind::Bytea,
        _ => RenderKind::Unknown,
    }
}

/// Serialize every column of a row into a JSON array. Null values become
/// `Value::Null`. Unknown types fall back to string representation so the row
/// still renders.
pub fn row_to_json(row: &PgRow) -> Value {
    let mut cells = Vec::with_capacity(row.columns().len());
    for (idx, col) in row.columns().iter().enumerate() {
        cells.push(cell_to_json(row, idx, col.type_info().name()));
    }
    Value::Array(cells)
}

fn cell_to_json(row: &PgRow, idx: usize, type_name: &str) -> Value {
    // Array handling: trailing "[]"
    if type_name.ends_with("[]") {
        return array_cell(row, idx, type_name);
    }
    match type_name {
        "BOOL" => opt_bool(row, idx),
        "INT2" => match row.try_get::<Option<i16>, _>(idx) {
            Ok(Some(v)) => Value::Number(v.into()),
            _ => Value::Null,
        },
        "INT4" => match row.try_get::<Option<i32>, _>(idx) {
            Ok(Some(v)) => Value::Number(v.into()),
            _ => Value::Null,
        },
        "INT8" | "OID" => match row.try_get::<Option<i64>, _>(idx) {
            Ok(Some(v)) => Value::Number(v.into()),
            _ => Value::Null,
        },
        "FLOAT4" => match row.try_get::<Option<f32>, _>(idx) {
            Ok(Some(v)) => Number::from_f64(v as f64)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            _ => Value::Null,
        },
        "FLOAT8" => match row.try_get::<Option<f64>, _>(idx) {
            Ok(Some(v)) => Number::from_f64(v)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            _ => Value::Null,
        },
        "NUMERIC" | "MONEY" => match row.try_get::<Option<BigDecimal>, _>(idx) {
            Ok(Some(v)) => Value::String(v.to_string()),
            _ => Value::Null,
        },
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "CHAR" | "CITEXT" => opt_string(row, idx),
        "UUID" => match row.try_get::<Option<uuid::Uuid>, _>(idx) {
            Ok(Some(v)) => Value::String(v.to_string()),
            _ => Value::Null,
        },
        "JSON" | "JSONB" => match row.try_get::<Option<Value>, _>(idx) {
            Ok(Some(v)) => v,
            _ => Value::Null,
        },
        "DATE" => match row.try_get::<Option<NaiveDate>, _>(idx) {
            Ok(Some(v)) => Value::String(v.format("%Y-%m-%d").to_string()),
            _ => Value::Null,
        },
        "TIME" => match row.try_get::<Option<NaiveTime>, _>(idx) {
            Ok(Some(v)) => Value::String(v.format("%H:%M:%S%.f").to_string()),
            _ => Value::Null,
        },
        "TIMESTAMP" => match row.try_get::<Option<NaiveDateTime>, _>(idx) {
            Ok(Some(v)) => Value::String(v.format("%Y-%m-%dT%H:%M:%S%.f").to_string()),
            _ => Value::Null,
        },
        "TIMESTAMPTZ" => match row.try_get::<Option<DateTime<Utc>>, _>(idx) {
            Ok(Some(v)) => Value::String(v.to_rfc3339()),
            _ => Value::Null,
        },
        "BYTEA" => match row.try_get::<Option<Vec<u8>>, _>(idx) {
            Ok(Some(v)) => {
                let size = v.len();
                const MAX: usize = 64;
                let preview = if size > MAX { &v[..MAX] } else { &v[..] };
                let mut hex_preview = String::from("\\x");
                hex_preview.push_str(&hex::encode(preview));
                Value::Object(
                    [
                        ("kind".to_string(), Value::String("bytea".to_string())),
                        ("size".to_string(), Value::Number(size.into())),
                        ("hex".to_string(), Value::String(hex_preview)),
                        ("truncated".to_string(), Value::Bool(size > MAX)),
                    ]
                    .into_iter()
                    .collect(),
                )
            }
            _ => Value::Null,
        },
        _ => fallback(row, idx, type_name),
    }
}

fn opt_bool(row: &PgRow, idx: usize) -> Value {
    match row.try_get::<Option<bool>, _>(idx) {
        Ok(Some(v)) => Value::Bool(v),
        _ => Value::Null,
    }
}

fn opt_string(row: &PgRow, idx: usize) -> Value {
    match row.try_get::<Option<String>, _>(idx) {
        Ok(Some(v)) => Value::String(v),
        _ => Value::Null,
    }
}

fn fallback(row: &PgRow, idx: usize, type_name: &str) -> Value {
    // Try text decode for unknown types; many PG types have text representation.
    if let Ok(Some(s)) = row.try_get::<Option<String>, _>(idx) {
        return Value::String(s);
    }
    // As a last resort, expose the type name so the UI can show "<type>" instead
    // of erroring.
    Value::String(format!("<{}>", type_name))
}

fn array_cell(row: &PgRow, idx: usize, type_name: &str) -> Value {
    // Known inner types get typed decoding. Everything else falls back to
    // Vec<String>.
    match type_name {
        "INT4[]" => match row.try_get::<Option<Vec<i32>>, _>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(|n| Value::Number(n.into())).collect()),
            _ => Value::Null,
        },
        "INT8[]" => match row.try_get::<Option<Vec<i64>>, _>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(|n| Value::Number(n.into())).collect()),
            _ => Value::Null,
        },
        "TEXT[]" | "VARCHAR[]" | "NAME[]" => match row.try_get::<Option<Vec<String>>, _>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(Value::String).collect()),
            _ => Value::Null,
        },
        "BOOL[]" => match row.try_get::<Option<Vec<bool>>, _>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(Value::Bool).collect()),
            _ => Value::Null,
        },
        "UUID[]" => match row.try_get::<Option<Vec<uuid::Uuid>>, _>(idx) {
            Ok(Some(v)) => Value::Array(
                v.into_iter()
                    .map(|u| Value::String(u.to_string()))
                    .collect(),
            ),
            _ => Value::Null,
        },
        _ => match row.try_get::<Option<Vec<String>>, _>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(Value::String).collect()),
            _ => Value::String(format!("<{}>", type_name)),
        },
    }
}
