#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod error;
mod history;
mod mru;
mod state;

use std::sync::Arc;

use dashmap::DashMap;
use pg_core::ConnectionManager;
use pg_profiles::ProfileStore;
use state::AppState;
use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let profile_store =
        Arc::new(ProfileStore::open_default().expect("failed to open profile store"));
    let connections = ConnectionManager::new();
    let mru = mru::MruStore::open_default().expect("failed to open mru store");
    let history = history::HistoryStore::open_default().expect("failed to open history store");

    let state = AppState {
        profiles: profile_store,
        connections,
        schema_cache: pg_schema_cache::SchemaCache::new(),
        mru,
        history,
        active_queries: Arc::new(DashMap::new()),
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::connections::connections_list,
            commands::connections::connection_create,
            commands::connections::connection_update,
            commands::connections::connection_delete,
            commands::connections::connection_test,
            commands::connections::connection_test_transient,
            commands::connections::connection_connect,
            commands::connections::connection_disconnect,
            commands::connections::connection_password_set,
            commands::connections::connection_password_clear,
            commands::query::query_execute,
            commands::query::query_cancel,
            commands::schema::schema_browse,
            commands::schema::schema_flat,
            commands::schema::schema_refresh,
            commands::schema::script_as_select,
            commands::schema::script_as_insert,
            commands::schema::object_definition,
            commands::completion::completion_get,
            commands::completion::completion_accept,
            commands::signature::signature_help,
            commands::history::history_list,
            commands::history::history_clear,
        ])
        .run(tauri::generate_context!())
        .expect("error while running pg-shell");
}
