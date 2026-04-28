//! Read-only queries against `pg_catalog`. These are the authoritative source
//! for what the cache knows about a live database.

use sqlx::postgres::PgPool;
use sqlx::Row;

use crate::types::{
    Column, Function, Index, ObjectKind, Relation, RelationKind, Schema, Sequence, Trigger,
};

const EXCLUDED_SCHEMAS: &str = "'pg_toast','pg_catalog','information_schema'";

pub async fn list_schemas(pool: &PgPool) -> Result<Vec<Schema>, sqlx::Error> {
    // `pg_catalog` is filtered out from the tree but pg_catalog objects are
    // still reachable via fully-qualified names in queries.
    let sql = format!(
        "SELECT n.nspname AS name, COALESCE(r.rolname, '') AS owner
           FROM pg_namespace n
      LEFT JOIN pg_roles r ON r.oid = n.nspowner
          WHERE n.nspname NOT IN ({EXCLUDED_SCHEMAS})
            AND n.nspname NOT LIKE 'pg_temp_%'
            AND n.nspname NOT LIKE 'pg_toast_temp_%'
          ORDER BY n.nspname"
    );
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| Schema {
            name: r.get::<String, _>("name"),
            owner: r.get::<String, _>("owner"),
        })
        .collect())
}

pub async fn list_relations(pool: &PgPool, schema: &str) -> Result<Vec<Relation>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT c.oid::bigint AS oid, c.relname AS name, c.relkind::text AS relkind
           FROM pg_class c
           JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = $1
            AND c.relkind IN ('r','v','m','p','f')
          ORDER BY c.relname",
    )
    .bind(schema)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let name: String = r.get("name");
        let oid: i64 = r.get("oid");
        let relkind: String = r.get("relkind");
        if let Some(kind) = relkind.chars().next().and_then(RelationKind::from_relkind) {
            out.push(Relation {
                schema: schema.to_string(),
                name,
                kind,
                oid,
            });
        }
    }
    Ok(out)
}

pub async fn list_columns(
    pool: &PgPool,
    schema: &str,
    relation: &str,
) -> Result<Vec<Column>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT a.attname AS name,
                a.attnum AS ord,
                format_type(a.atttypid, a.atttypmod) AS type_name,
                a.attnotnull AS not_null,
                pg_get_expr(d.adbin, d.adrelid) AS default_expr
           FROM pg_attribute a
           JOIN pg_class c ON c.oid = a.attrelid
           JOIN pg_namespace n ON n.oid = c.relnamespace
      LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
          WHERE n.nspname = $1
            AND c.relname = $2
            AND a.attnum > 0
            AND NOT a.attisdropped
          ORDER BY a.attnum",
    )
    .bind(schema)
    .bind(relation)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Column {
            name: r.get::<String, _>("name"),
            ord: r.get::<i16, _>("ord"),
            type_name: r.get::<String, _>("type_name"),
            not_null: r.get::<bool, _>("not_null"),
            default_expr: r.get::<Option<String>, _>("default_expr"),
        })
        .collect())
}

pub async fn list_sequences(
    pool: &PgPool,
    schema: &str,
) -> Result<Vec<Sequence>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT c.relname AS name, COALESCE(r.rolname, '') AS owner
           FROM pg_class c
           JOIN pg_namespace n ON n.oid = c.relnamespace
      LEFT JOIN pg_roles r ON r.oid = c.relowner
          WHERE n.nspname = $1 AND c.relkind = 'S'
          ORDER BY c.relname",
    )
    .bind(schema)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Sequence {
            schema: schema.to_string(),
            name: r.get::<String, _>("name"),
            owner: r.get::<String, _>("owner"),
        })
        .collect())
}

/// Every user-defined trigger in the schema. `tgisinternal = false` filters
/// out system triggers PG installs under the hood for FK enforcement etc.
///
/// `pg_trigger.tgtype` is a bitmask:
///   bit 1 (2)  BEFORE
///   bit 2 (4)  INSERT event
///   bit 3 (8)  DELETE event
///   bit 4 (16) UPDATE event
///   bit 5 (32) TRUNCATE event
///   bit 6 (64) INSTEAD OF
/// We decode timing + event into readable strings server-side.
pub async fn list_triggers(
    pool: &PgPool,
    schema: &str,
) -> Result<Vec<Trigger>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT n.nspname AS schema,
                c.relname AS table_name,
                t.tgname AS name,
                CASE
                    WHEN (t.tgtype & 66) = 2 THEN 'BEFORE'
                    WHEN (t.tgtype & 66) = 64 THEN 'INSTEAD OF'
                    ELSE 'AFTER'
                END AS timing,
                concat_ws(' OR ',
                    CASE WHEN (t.tgtype & 4) <> 0 THEN 'INSERT' END,
                    CASE WHEN (t.tgtype & 8) <> 0 THEN 'DELETE' END,
                    CASE WHEN (t.tgtype & 16) <> 0 THEN 'UPDATE' END,
                    CASE WHEN (t.tgtype & 32) <> 0 THEN 'TRUNCATE' END
                ) AS event,
                pg_get_triggerdef(t.oid) AS definition
           FROM pg_trigger t
           JOIN pg_class c ON c.oid = t.tgrelid
           JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE NOT t.tgisinternal
            AND n.nspname = $1
          ORDER BY c.relname, t.tgname",
    )
    .bind(schema)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Trigger {
            schema: r.get::<String, _>("schema"),
            table: r.get::<String, _>("table_name"),
            name: r.get::<String, _>("name"),
            timing: r.get::<String, _>("timing"),
            event: r.get::<String, _>("event"),
            definition: r.get::<String, _>("definition"),
        })
        .collect())
}

pub async fn list_indexes(
    pool: &PgPool,
    schema: &str,
    table: &str,
) -> Result<Vec<Index>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT ic.relname AS name,
                i.indisunique AS is_unique,
                i.indisprimary AS is_primary,
                pg_get_indexdef(i.indexrelid) AS definition
           FROM pg_index i
           JOIN pg_class c ON c.oid = i.indrelid
           JOIN pg_class ic ON ic.oid = i.indexrelid
           JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = $1
            AND c.relname = $2
          ORDER BY ic.relname",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Index {
            schema: schema.to_string(),
            table: table.to_string(),
            name: r.get::<String, _>("name"),
            is_unique: r.get::<bool, _>("is_unique"),
            is_primary: r.get::<bool, _>("is_primary"),
            definition: r.get::<String, _>("definition"),
        })
        .collect())
}

pub async fn list_functions(
    pool: &PgPool,
    schema: &str,
) -> Result<Vec<Function>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT n.nspname AS schema,
                p.proname AS name,
                pg_get_function_identity_arguments(p.oid) AS args,
                pg_get_function_result(p.oid) AS result,
                p.prokind::text AS kind
           FROM pg_proc p
           JOIN pg_namespace n ON n.oid = p.pronamespace
          WHERE n.nspname = $1
            AND p.prokind IN ('f','p','a','w')
          ORDER BY p.proname",
    )
    .bind(schema)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let kind_s: String = r.get("kind");
            Function {
                schema: r.get::<String, _>("schema"),
                name: r.get::<String, _>("name"),
                args: r.get::<String, _>("args"),
                result: r.get::<String, _>("result"),
                kind: kind_s.chars().next().unwrap_or('f'),
            }
        })
        .collect())
}

/// Retrieve the SQL definition for a view / matview / function / procedure.
/// Tables have no "definition" in this sense; callers must not pass Table.
pub async fn object_definition(
    pool: &PgPool,
    kind: ObjectKind,
    schema: &str,
    name: &str,
) -> Result<String, sqlx::Error> {
    let qualified = format!("{}.{}", quote_ident(schema), quote_ident(name));
    match kind {
        ObjectKind::Sequence => {
            let r = sqlx::query(
                "SELECT seqstart, seqincrement, seqmin, seqmax, seqcache, seqcycle
                   FROM pg_sequence s
                   JOIN pg_class c ON c.oid = s.seqrelid
                   JOIN pg_namespace n ON n.oid = c.relnamespace
                  WHERE n.nspname = $1 AND c.relname = $2",
            )
            .bind(schema)
            .bind(name)
            .fetch_one(pool)
            .await?;
            let start: i64 = r.get("seqstart");
            let inc: i64 = r.get("seqincrement");
            let min: i64 = r.get("seqmin");
            let max: i64 = r.get("seqmax");
            let cache: i64 = r.get("seqcache");
            let cycle: bool = r.get("seqcycle");
            Ok(format!(
                "CREATE SEQUENCE {qualified}\n    INCREMENT BY {inc}\n    MINVALUE {min}\n    MAXVALUE {max}\n    START WITH {start}\n    CACHE {cache}\n    {};\n",
                if cycle { "CYCLE" } else { "NO CYCLE" }
            ))
        }
        ObjectKind::Trigger => {
            // `name` is the trigger name — look it up (any schema match is fine;
            // triggers are scoped by their table's schema).
            let def: String = sqlx::query_scalar(
                "SELECT pg_get_triggerdef(t.oid)
                   FROM pg_trigger t
                   JOIN pg_class c ON c.oid = t.tgrelid
                   JOIN pg_namespace n ON n.oid = c.relnamespace
                  WHERE n.nspname = $1 AND t.tgname = $2
                    AND NOT t.tgisinternal
                  LIMIT 1",
            )
            .bind(schema)
            .bind(name)
            .fetch_one(pool)
            .await?;
            Ok(format!("{};\n", def.trim_end_matches(';')))
        }
        ObjectKind::Index => {
            let def: String = sqlx::query_scalar(
                "SELECT pg_get_indexdef(ic.oid)
                   FROM pg_class ic
                   JOIN pg_namespace n ON n.oid = ic.relnamespace
                  WHERE n.nspname = $1 AND ic.relname = $2 AND ic.relkind IN ('i','I')
                  LIMIT 1",
            )
            .bind(schema)
            .bind(name)
            .fetch_one(pool)
            .await?;
            Ok(format!("{};\n", def.trim_end_matches(';')))
        }
        ObjectKind::View | ObjectKind::MaterializedView => {
            let def: String = sqlx::query_scalar("SELECT pg_get_viewdef($1::regclass, true)")
                .bind(&qualified)
                .fetch_one(pool)
                .await?;
            Ok(format!(
                "CREATE OR REPLACE VIEW {qualified} AS\n{};\n",
                def.trim_end_matches(';')
            ))
        }
        ObjectKind::Function | ObjectKind::Procedure => {
            // Functions/procedures need regprocedure — include empty args if ambiguous,
            // but for a unique-name best-effort use proname::regproc if single-result.
            let def: String = sqlx::query_scalar(
                "SELECT pg_get_functiondef(p.oid)
                   FROM pg_proc p
                   JOIN pg_namespace n ON n.oid = p.pronamespace
                  WHERE n.nspname = $1 AND p.proname = $2
                  LIMIT 1",
            )
            .bind(schema)
            .bind(name)
            .fetch_one(pool)
            .await?;
            Ok(def)
        }
        ObjectKind::Table => {
            // We don't have a single-call "pg_get_tabledef" in core PG. Emit a
            // CREATE TABLE reconstructed from pg_attribute; good enough for
            // "View Definition" on a table.
            let cols = list_columns(pool, schema, name).await?;
            let mut out = format!("CREATE TABLE {qualified} (\n");
            let col_lines: Vec<String> = cols
                .iter()
                .map(|c| {
                    let mut line = format!("    {} {}", quote_ident(&c.name), c.type_name);
                    if c.not_null {
                        line.push_str(" NOT NULL");
                    }
                    if let Some(d) = &c.default_expr {
                        line.push_str(&format!(" DEFAULT {d}"));
                    }
                    line
                })
                .collect();
            out.push_str(&col_lines.join(",\n"));
            out.push_str("\n);\n");
            Ok(out)
        }
    }
}

/// Minimal identifier quoter: double-quotes and escapes internal quotes. This
/// is the same rule Postgres itself uses for `quote_ident`.
pub(crate) fn quote_ident(ident: &str) -> String {
    let needs_quotes = ident.is_empty()
        || !ident.chars().next().unwrap().is_ascii_lowercase()
            && !matches!(ident.chars().next().unwrap(), 'a'..='z' | '_')
        || ident.chars().any(|c| !matches!(c, 'a'..='z' | '0'..='9' | '_'));
    if needs_quotes {
        format!("\"{}\"", ident.replace('"', "\"\""))
    } else {
        ident.to_string()
    }
}
