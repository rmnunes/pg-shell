use pg_core::{ServerInfo, TestOutcome};
use pg_profiles::{PasswordStore, Profile, ProfileInput};
use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ConnectionSummary {
    #[serde(flatten)]
    pub profile: Profile,
    pub connected: bool,
    pub has_password: bool,
}

fn summaries(state: &AppState) -> Vec<ConnectionSummary> {
    state
        .profiles
        .list()
        .into_iter()
        .map(|p| {
            let connected = state.connections.has_pool(&p.id);
            let has_password = PasswordStore::get(&p.id)
                .map(|o| o.is_some())
                .unwrap_or(false);
            ConnectionSummary {
                profile: p,
                connected,
                has_password,
            }
        })
        .collect()
}

#[tauri::command]
pub fn connections_list(state: State<'_, AppState>) -> AppResult<Vec<ConnectionSummary>> {
    Ok(summaries(&state))
}

#[tauri::command]
pub fn connection_create(
    input: ProfileInput,
    password: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<ConnectionSummary> {
    let profile = state.profiles.create(input)?;
    if let Some(pw) = password {
        PasswordStore::set(&profile.id, &pw)
            .map_err(|e| AppError::new("keychain", e.to_string()))?;
    }
    let has_password = PasswordStore::get(&profile.id)
        .map(|o| o.is_some())
        .unwrap_or(false);
    Ok(ConnectionSummary {
        profile,
        connected: false,
        has_password,
    })
}

#[tauri::command]
pub fn connection_update(
    id: String,
    input: ProfileInput,
    state: State<'_, AppState>,
) -> AppResult<ConnectionSummary> {
    let profile = state.profiles.update(&id, input)?;
    let connected = state.connections.has_pool(&id);
    let has_password = PasswordStore::get(&id)
        .map(|o| o.is_some())
        .unwrap_or(false);
    Ok(ConnectionSummary {
        profile,
        connected,
        has_password,
    })
}

#[tauri::command]
pub async fn connection_delete(id: String, state: State<'_, AppState>) -> AppResult<()> {
    state.connections.disconnect(&id).await;
    PasswordStore::delete(&id).map_err(|e| AppError::new("keychain", e.to_string()))?;
    state.profiles.delete(&id)?;
    Ok(())
}

#[tauri::command]
pub async fn connection_test(
    id: String,
    password: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<TestOutcome> {
    let profile = state
        .profiles
        .get(&id)
        .ok_or_else(|| AppError::new("profile_store", "profile not found"))?;
    let pw = resolve_password(&id, password)?;
    Ok(pg_core::ConnectionManager::test(&profile, &pw).await?)
}

/// Test a profile's connection params before it's been saved. Lets the New
/// Connection dialog validate inputs without round-tripping through the
/// profile store + keychain.
#[tauri::command]
pub async fn connection_test_transient(
    input: ProfileInput,
    password: String,
) -> AppResult<TestOutcome> {
    let probe = Profile {
        // The id is unused by `test()` — pool key isn't touched on this path.
        id: String::new(),
        name: input.name,
        host: input.host,
        port: input.port,
        database: input.database,
        user: input.user,
        ssl_mode: input.ssl_mode,
        app_name: input.app_name,
        group: input.group,
    };
    Ok(pg_core::ConnectionManager::test(&probe, &password).await?)
}

#[tauri::command]
pub async fn connection_connect(
    id: String,
    password: Option<String>,
    state: State<'_, AppState>,
) -> AppResult<ServerInfo> {
    let profile = state
        .profiles
        .get(&id)
        .ok_or_else(|| AppError::new("profile_store", "profile not found"))?;
    let pw = resolve_password(&id, password)?;
    let info = state.connections.connect(&profile, &pw).await?;

    // Kick off schema-cache warm-up in the background so intellisense has
    // names to suggest by the time the user starts typing. Failures are
    // non-fatal — the cache simply stays cold.
    if let Some(pool) = state.connections.pool(&id) {
        let cache = state.schema_cache.clone();
        let profile_id = id.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = cache.warm(&profile_id, &pool).await {
                tracing::warn!(profile_id, error = %e, "schema cache warm failed");
            }
        });
    }

    Ok(info)
}

#[tauri::command]
pub async fn connection_disconnect(id: String, state: State<'_, AppState>) -> AppResult<()> {
    state.connections.disconnect(&id).await;
    state.schema_cache.drop_profile(&id);
    Ok(())
}

#[tauri::command]
pub fn connection_password_set(id: String, password: String) -> AppResult<()> {
    PasswordStore::set(&id, &password).map_err(|e| AppError::new("keychain", e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub fn connection_password_clear(id: String) -> AppResult<()> {
    PasswordStore::delete(&id).map_err(|e| AppError::new("keychain", e.to_string()))?;
    Ok(())
}

fn resolve_password(id: &str, explicit: Option<String>) -> AppResult<String> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    match PasswordStore::get(id).map_err(|e| AppError::new("keychain", e.to_string()))? {
        Some(p) => Ok(p),
        None => Err(AppError::new(
            "missing_password",
            "no password stored for profile and none supplied",
        )),
    }
}
