//! Per-profile schema cache. Schemas, relations, columns, and functions are
//! fetched lazily on first access and then served from memory until the UI
//! asks for a refresh.

use std::sync::Arc;

use dashmap::DashMap;
use parking_lot::RwLock;
use sqlx::postgres::PgPool;
use thiserror::Error;

use crate::introspect::{
    list_columns, list_functions, list_indexes, list_relations, list_schemas, list_sequences,
    list_triggers,
};
use crate::types::{Column, Function, Index, Relation, Schema, Sequence, Trigger};

#[derive(Debug, Error)]
pub enum SchemaCacheError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// A lazily-filled slot. `None` = never fetched; `Some(Vec)` = last snapshot.
type Slot<T> = Arc<RwLock<Option<Vec<T>>>>;
type SchemasSlot = Arc<RwLock<Option<Vec<Schema>>>>;

#[derive(Default)]
struct DatabaseCatalog {
    schemas: SchemasSlot,
    relations: DashMap<String, Slot<Relation>>,
    columns: DashMap<(String, String), Slot<Column>>,
    functions: DashMap<String, Slot<Function>>,
    sequences: DashMap<String, Slot<Sequence>>,
    /// All triggers in a schema. Per-table filtering happens in the command
    /// layer so we keep one fetch per schema rather than one per table.
    triggers: DashMap<String, Slot<Trigger>>,
    /// Per-table index list.
    indexes: DashMap<(String, String), Slot<Index>>,
}

impl DatabaseCatalog {
    fn invalidate_schema(&self, schema: &str) {
        self.relations.remove(schema);
        self.functions.remove(schema);
        self.sequences.remove(schema);
        self.triggers.remove(schema);
        self.columns.retain(|(s, _), _| s != schema);
        self.indexes.retain(|(s, _), _| s != schema);
    }

    fn invalidate_relation(&self, schema: &str, name: &str) {
        let key = (schema.to_string(), name.to_string());
        self.columns.remove(&key);
        self.indexes.remove(&key);
        // Bust relations + triggers for the schema so a possibly-dropped or
        // renamed table disappears and its triggers stop showing up.
        self.relations.remove(schema);
        self.triggers.remove(schema);
    }

    fn invalidate_all(&self) {
        *self.schemas.write() = None;
        self.relations.clear();
        self.columns.clear();
        self.functions.clear();
        self.sequences.clear();
        self.triggers.clear();
        self.indexes.clear();
    }
}

/// Thread-safe, per-profile cache.
#[derive(Default, Clone)]
pub struct SchemaCache {
    by_profile: Arc<DashMap<String, Arc<DatabaseCatalog>>>,
}

impl SchemaCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn catalog(&self, profile_id: &str) -> Arc<DatabaseCatalog> {
        self.by_profile
            .entry(profile_id.to_string())
            .or_insert_with(|| Arc::new(DatabaseCatalog::default()))
            .clone()
    }

    pub async fn schemas(
        &self,
        profile_id: &str,
        pool: &PgPool,
    ) -> Result<Vec<Schema>, SchemaCacheError> {
        let cat = self.catalog(profile_id);
        if let Some(hit) = cat.schemas.read().clone() {
            return Ok(hit);
        }
        let fresh = list_schemas(pool).await?;
        *cat.schemas.write() = Some(fresh.clone());
        Ok(fresh)
    }

    pub async fn relations(
        &self,
        profile_id: &str,
        pool: &PgPool,
        schema: &str,
    ) -> Result<Vec<Relation>, SchemaCacheError> {
        let cat = self.catalog(profile_id);
        let slot = cat
            .relations
            .entry(schema.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(None)))
            .clone();
        if let Some(hit) = slot.read().clone() {
            return Ok(hit);
        }
        let fresh = list_relations(pool, schema).await?;
        *slot.write() = Some(fresh.clone());
        Ok(fresh)
    }

    pub async fn columns(
        &self,
        profile_id: &str,
        pool: &PgPool,
        schema: &str,
        relation: &str,
    ) -> Result<Vec<Column>, SchemaCacheError> {
        let cat = self.catalog(profile_id);
        let key = (schema.to_string(), relation.to_string());
        let slot = cat
            .columns
            .entry(key)
            .or_insert_with(|| Arc::new(RwLock::new(None)))
            .clone();
        if let Some(hit) = slot.read().clone() {
            return Ok(hit);
        }
        let fresh = list_columns(pool, schema, relation).await?;
        *slot.write() = Some(fresh.clone());
        Ok(fresh)
    }

    pub async fn functions(
        &self,
        profile_id: &str,
        pool: &PgPool,
        schema: &str,
    ) -> Result<Vec<Function>, SchemaCacheError> {
        let cat = self.catalog(profile_id);
        let slot = cat
            .functions
            .entry(schema.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(None)))
            .clone();
        if let Some(hit) = slot.read().clone() {
            return Ok(hit);
        }
        let fresh = list_functions(pool, schema).await?;
        *slot.write() = Some(fresh.clone());
        Ok(fresh)
    }

    pub async fn sequences(
        &self,
        profile_id: &str,
        pool: &PgPool,
        schema: &str,
    ) -> Result<Vec<Sequence>, SchemaCacheError> {
        let cat = self.catalog(profile_id);
        let slot = cat
            .sequences
            .entry(schema.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(None)))
            .clone();
        if let Some(hit) = slot.read().clone() {
            return Ok(hit);
        }
        let fresh = list_sequences(pool, schema).await?;
        *slot.write() = Some(fresh.clone());
        Ok(fresh)
    }

    pub async fn triggers(
        &self,
        profile_id: &str,
        pool: &PgPool,
        schema: &str,
    ) -> Result<Vec<Trigger>, SchemaCacheError> {
        let cat = self.catalog(profile_id);
        let slot = cat
            .triggers
            .entry(schema.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(None)))
            .clone();
        if let Some(hit) = slot.read().clone() {
            return Ok(hit);
        }
        let fresh = list_triggers(pool, schema).await?;
        *slot.write() = Some(fresh.clone());
        Ok(fresh)
    }

    pub async fn indexes(
        &self,
        profile_id: &str,
        pool: &PgPool,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Index>, SchemaCacheError> {
        let cat = self.catalog(profile_id);
        let key = (schema.to_string(), table.to_string());
        let slot = cat
            .indexes
            .entry(key)
            .or_insert_with(|| Arc::new(RwLock::new(None)))
            .clone();
        if let Some(hit) = slot.read().clone() {
            return Ok(hit);
        }
        let fresh = list_indexes(pool, schema, table).await?;
        *slot.write() = Some(fresh.clone());
        Ok(fresh)
    }

    pub fn invalidate_profile(&self, profile_id: &str) {
        if let Some(cat) = self.by_profile.get(profile_id) {
            cat.invalidate_all();
        }
    }

    pub fn invalidate_schema(&self, profile_id: &str, schema: &str) {
        if let Some(cat) = self.by_profile.get(profile_id) {
            cat.invalidate_schema(schema);
        }
    }

    pub fn invalidate_relation(&self, profile_id: &str, schema: &str, relation: &str) {
        if let Some(cat) = self.by_profile.get(profile_id) {
            cat.invalidate_relation(schema, relation);
        }
    }

    /// Drop everything for a profile (used on disconnect).
    pub fn drop_profile(&self, profile_id: &str) {
        self.by_profile.remove(profile_id);
    }

    /// Background-friendly pre-warm: load schemas, then all relations, then
    /// functions. Columns are intentionally NOT warmed — they're pulled on
    /// demand for the specific relations intellisense needs.
    ///
    /// Runs all schema-scoped fetches concurrently but caps concurrency
    /// implicitly via the underlying pool's max_connections. Errors for a
    /// single schema are logged (via `tracing`) but don't abort the warm.
    pub async fn warm(&self, profile_id: &str, pool: &PgPool) -> Result<(), SchemaCacheError> {
        let schemas = self.schemas(profile_id, pool).await?;
        for s in schemas {
            if let Err(e) = self.relations(profile_id, pool, &s.name).await {
                tracing::warn!(schema = s.name, error = %e, "warm: relations failed");
            }
            if let Err(e) = self.functions(profile_id, pool, &s.name).await {
                tracing::warn!(schema = s.name, error = %e, "warm: functions failed");
            }
            if let Err(e) = self.sequences(profile_id, pool, &s.name).await {
                tracing::warn!(schema = s.name, error = %e, "warm: sequences failed");
            }
            if let Err(e) = self.triggers(profile_id, pool, &s.name).await {
                tracing::warn!(schema = s.name, error = %e, "warm: triggers failed");
            }
        }
        Ok(())
    }

    /// Snapshot of the currently-cached schemas, relations, and functions for
    /// a profile. Returns empty when nothing has been warmed yet. Column data
    /// for specified relations is fetched on demand; anything not provided
    /// returns whatever is already cached.
    pub async fn build_snapshot(
        &self,
        profile_id: &str,
        pool: &PgPool,
        fetch_columns_for: &[(Option<String>, String)],
    ) -> Result<Snapshot, SchemaCacheError> {
        let cat = self.catalog(profile_id);

        let schemas = cat.schemas.read().clone().unwrap_or_default();

        let mut relations: Vec<Relation> = Vec::new();
        for s in &schemas {
            if let Some(slot) = cat.relations.get(&s.name) {
                if let Some(v) = slot.read().clone() {
                    relations.extend(v);
                }
            }
        }

        let mut functions: Vec<Function> = Vec::new();
        for s in &schemas {
            if let Some(slot) = cat.functions.get(&s.name) {
                if let Some(v) = slot.read().clone() {
                    functions.extend(v);
                }
            }
        }

        // Demand-fetch columns for targeted relations.
        let mut columns: Vec<(String, String, Column)> = Vec::new();
        for (schema_hint, rel_name) in fetch_columns_for {
            let resolved_schema = schema_hint.clone().or_else(|| {
                relations
                    .iter()
                    .find(|r| r.name.eq_ignore_ascii_case(rel_name))
                    .map(|r| r.schema.clone())
            });
            let Some(schema) = resolved_schema else {
                continue;
            };
            let cols = self
                .columns(profile_id, pool, &schema, rel_name)
                .await
                .unwrap_or_default();
            for c in cols {
                columns.push((schema.clone(), rel_name.clone(), c));
            }
        }

        // Columns already cached for other relations (cheap to include so the
        // engine can do schema-qualified column lookups without another trip).
        for entry in cat.columns.iter() {
            let (schema, relation) = entry.key().clone();
            if let Some(v) = entry.value().read().clone() {
                // Skip dupes already added via fetch_columns_for.
                if !columns
                    .iter()
                    .any(|(s, r, _)| s == &schema && r == &relation)
                {
                    for c in v {
                        columns.push((schema.clone(), relation.clone(), c));
                    }
                }
            }
        }

        Ok(Snapshot {
            schemas,
            relations,
            columns,
            functions,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub schemas: Vec<Schema>,
    pub relations: Vec<Relation>,
    pub columns: Vec<(String, String, Column)>, // (schema, relation, col)
    pub functions: Vec<Function>,
}
