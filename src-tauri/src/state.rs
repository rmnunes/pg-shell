use std::sync::Arc;

use dashmap::DashMap;
use pg_core::ConnectionManager;
use pg_profiles::ProfileStore;
use pg_schema_cache::SchemaCache;

use crate::entra::EntraSessions;
use crate::history::HistoryStore;
use crate::mru::MruStore;

#[derive(Clone)]
pub struct AppState {
    pub profiles: Arc<ProfileStore>,
    pub connections: ConnectionManager,
    pub schema_cache: SchemaCache,
    pub mru: MruStore,
    pub history: HistoryStore,
    /// Signed-in Microsoft Entra identities, one per Entra-auth profile.
    pub entra: EntraSessions,
    /// Live query registry keyed by `query_id`. Entry holds the backend PID
    /// needed for `pg_cancel_backend`, plus the profile the query runs under.
    pub active_queries: Arc<DashMap<String, ActiveQuery>>,
}

#[derive(Debug, Clone)]
pub struct ActiveQuery {
    pub profile_id: String,
    pub backend_pid: i32,
}
