//! Script generation for right-click actions in the object tree.

use sqlx::postgres::PgPool;

use crate::introspect::{list_columns, quote_ident};

pub async fn script_as_select(
    pool: &PgPool,
    schema: &str,
    relation: &str,
) -> Result<String, sqlx::Error> {
    let cols = list_columns(pool, schema, relation).await?;
    let col_list = if cols.is_empty() {
        "*".to_string()
    } else {
        cols.iter()
            .map(|c| quote_ident(&c.name))
            .collect::<Vec<_>>()
            .join(",\n    ")
    };
    Ok(format!(
        "SELECT {col_list}\nFROM {}.{}\nLIMIT 100;\n",
        quote_ident(schema),
        quote_ident(relation)
    ))
}

pub async fn script_as_insert(
    pool: &PgPool,
    schema: &str,
    relation: &str,
) -> Result<String, sqlx::Error> {
    let cols = list_columns(pool, schema, relation).await?;
    if cols.is_empty() {
        return Ok(format!(
            "INSERT INTO {}.{} VALUES ();\n",
            quote_ident(schema),
            quote_ident(relation)
        ));
    }
    let col_list = cols
        .iter()
        .map(|c| quote_ident(&c.name))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = cols
        .iter()
        .map(|c| format!("<{}>", c.type_name))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "INSERT INTO {}.{} ({col_list})\nVALUES ({placeholders});\n",
        quote_ident(schema),
        quote_ident(relation)
    ))
}
