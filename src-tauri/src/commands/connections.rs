use std::sync::Arc;

use pg_core::{Credential, ServerInfo, TestOutcome};
use pg_profiles::{AuthMethod, PasswordStore, Profile, ProfileInput};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::entra::{self, EntraSessions, EntraTokenSource};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ConnectionSummary {
    #[serde(flatten)]
    pub profile: Profile,
    pub connected: bool,
    /// A secret for this profile is in the keychain: the password for
    /// password auth, a cached sign-in (refresh token) for Entra.
    pub has_password: bool,
}

fn has_stored_secret(profile: &Profile) -> bool {
    match profile.auth_method {
        AuthMethod::Password => PasswordStore::get(&profile.id)
            .map(|o| o.is_some())
            .unwrap_or(false),
        AuthMethod::EntraMfa => EntraSessions::has_cached_sign_in(&profile.id),
    }
}

fn summary(state: &AppState, profile: Profile) -> ConnectionSummary {
    let connected = state.connections.has_pool(&profile.id);
    let has_password = has_stored_secret(&profile);
    ConnectionSummary {
        profile,
        connected,
        has_password,
    }
}

fn summaries(state: &AppState) -> Vec<ConnectionSummary> {
    state
        .profiles
        .list()
        .into_iter()
        .map(|p| summary(state, p))
        .collect()
}

/// Resolve what a connect/test needs: the credential, and the profile to
/// connect with (an Entra profile with a blank User gets the signed-in
/// account as its role). Entra profiles may open the browser here.
async fn resolve_auth(
    profile: &Profile,
    explicit_password: Option<String>,
    state: &AppState,
    app: &AppHandle,
) -> AppResult<(Profile, Credential)> {
    match profile.auth_method {
        AuthMethod::Password => {
            let password = resolve_password(&profile.id, explicit_password)?;
            Ok((profile.clone(), Credential::Password(password)))
        }
        AuthMethod::EntraMfa => {
            let session = state.entra.acquire(profile, app).await?;
            let profile = entra::with_default_role(profile, session.account().await)?;
            Ok((
                profile,
                Credential::Token(Arc::new(EntraTokenSource(session))),
            ))
        }
    }
}

/// Azure reports a rejected Entra token as a plain password failure, which
/// misleads: there is no password. Say what the server actually checked.
fn explain_connect_error(profile: &Profile, err: pg_core::ConnectionManagerError) -> AppError {
    if profile.auth_method == AuthMethod::EntraMfa && err.is_auth_failure() {
        return AppError::new(
            "entra_role",
            format!(
                "The Microsoft sign-in succeeded, but the server has no Entra principal named \"{}\". \
                 Use your UPN only if an individual principal was created for you (or the server has \
                 pgaadauth.enable_group_sync on); otherwise set User to your Entra group's display \
                 name. Names are case-sensitive.",
                profile.user
            ),
        );
    }
    err.into()
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
    if let (AuthMethod::Password, Some(pw)) = (profile.auth_method, password) {
        PasswordStore::set(&profile.id, &pw)?;
    }
    Ok(summary(&state, profile))
}

#[tauri::command]
pub fn connection_update(
    id: String,
    input: ProfileInput,
    state: State<'_, AppState>,
) -> AppResult<ConnectionSummary> {
    let profile = state.profiles.update(&id, input)?;
    // Tenant / client id / user may have changed; rebuild from the keychain
    // on next connect rather than trust the in-memory session.
    state.entra.forget(&id);
    Ok(summary(&state, profile))
}

#[tauri::command]
pub async fn connection_delete(id: String, state: State<'_, AppState>) -> AppResult<()> {
    state.connections.disconnect(&id).await;
    PasswordStore::delete(&id)?;
    state.entra.sign_out(&id)?;
    state.profiles.delete(&id)?;
    Ok(())
}

#[tauri::command]
pub async fn connection_test(
    id: String,
    password: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<TestOutcome> {
    let profile = state
        .profiles
        .get(&id)
        .ok_or_else(|| AppError::new("profile_store", "profile not found"))?;
    let (profile, credential) = resolve_auth(&profile, password, &state, &app).await?;
    pg_core::ConnectionManager::test(&profile, credential)
        .await
        .map_err(|e| explain_connect_error(&profile, e))
}

/// Test a profile's connection params before it's been saved. Lets the New
/// Connection dialog validate inputs without round-tripping through the
/// profile store + keychain. For Entra profiles this runs a one-off browser
/// sign-in whose tokens are discarded afterwards.
#[tauri::command]
pub async fn connection_test_transient(
    input: ProfileInput,
    password: Option<String>,
    app: AppHandle,
) -> AppResult<TestOutcome> {
    // The id is unused by `test()` — pool key isn't touched on this path.
    let probe = Profile::from_input(String::new(), input);
    let (probe, credential) = match probe.auth_method {
        AuthMethod::Password => {
            let password = password.ok_or_else(|| {
                AppError::new(
                    "missing_password",
                    "enter a password to test the connection",
                )
            })?;
            (probe, Credential::Password(password))
        }
        AuthMethod::EntraMfa => {
            let http = pg_entra::http_client()?;
            let cfg = EntraSessions::config_for(&probe);
            let tokens =
                entra::sign_in(&http, &cfg, entra::login_hint_for(&probe.user), None, &app).await?;
            let probe = entra::with_default_role(&probe, tokens.account.clone())?;
            (probe, Credential::Password(tokens.access_token))
        }
    };
    pg_core::ConnectionManager::test(&probe, credential)
        .await
        .map_err(|e| explain_connect_error(&probe, e))
}

#[tauri::command]
pub async fn connection_connect(
    id: String,
    password: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<ServerInfo> {
    let profile = state
        .profiles
        .get(&id)
        .ok_or_else(|| AppError::new("profile_store", "profile not found"))?;
    let (profile, credential) = resolve_auth(&profile, password, &state, &app).await?;
    let info = state
        .connections
        .connect(&profile, credential)
        .await
        .map_err(|e| explain_connect_error(&profile, e))?;

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
    PasswordStore::set(&id, &password)?;
    Ok(())
}

#[tauri::command]
pub fn connection_password_clear(id: String) -> AppResult<()> {
    PasswordStore::delete(&id)?;
    Ok(())
}

/// Forget the cached Microsoft Entra sign-in for a profile. The next connect
/// goes through the browser again. Does not touch an open connection.
#[tauri::command]
pub fn connection_entra_sign_out(id: String, state: State<'_, AppState>) -> AppResult<()> {
    state.entra.sign_out(&id)
}

fn resolve_password(id: &str, explicit: Option<String>) -> AppResult<String> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    match PasswordStore::get(id)? {
        Some(p) => Ok(p),
        None => Err(AppError::new(
            "missing_password",
            "no password stored for profile and none supplied",
        )),
    }
}
