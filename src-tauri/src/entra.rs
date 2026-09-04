//! Glue between `pg-entra` sessions, the OS keychain and the UI.
//!
//! One [`EntraSession`] per Entra profile, kept for the life of the process
//! so reconnects are silent. Acquisition order: live session → persisted
//! refresh token → browser sign-in. Only the refresh token ever reaches the
//! keychain; access tokens live in memory.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use pg_core::{AccessToken, BoxFuture, TokenSource, TokenSourceError};
use pg_entra::{EntraConfig, EntraSession, LoginOptions, PersistFn, TokenSet};
use pg_profiles::{Profile, ProfileId, RefreshTokenStore};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, AppResult};

/// Emitted when a browser sign-in starts, so the UI can tell the user to look
/// at their browser (and offer the URL if the browser did not open).
pub const SIGN_IN_EVENT: &str = "entra:sign_in";

/// Wait this long for the user to finish in the browser.
const BROWSER_TIMEOUT: Duration = Duration::from_secs(300);
/// Validity demanded when reusing a cached session at connect time.
const MIN_TOKEN_VALIDITY: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Serialize)]
struct SignInPayload<'a> {
    /// `None` for the pre-save "Test" flow in the connection dialog.
    profile_id: Option<&'a str>,
    url: &'a str,
}

#[derive(Clone, Default)]
pub struct EntraSessions {
    inner: Arc<DashMap<ProfileId, Arc<EntraSession>>>,
}

impl EntraSessions {
    pub fn config_for(profile: &Profile) -> EntraConfig {
        let settings = profile.entra.as_ref();
        EntraConfig::new(
            settings.and_then(|s| s.tenant.as_deref()),
            settings.and_then(|s| s.client_id.as_deref()),
        )
    }

    /// A session able to mint tokens for `profile`, signing in through the
    /// browser only if neither the live session nor the persisted refresh
    /// token can be used.
    pub async fn acquire(
        &self,
        profile: &Profile,
        app: &AppHandle,
    ) -> AppResult<Arc<EntraSession>> {
        let http = pg_entra::http_client()?;
        let cfg = Self::config_for(profile);

        if let Some(existing) = self.inner.get(&profile.id).map(|e| e.value().clone()) {
            if existing.config() == &cfg {
                match existing.access_token(MIN_TOKEN_VALIDITY).await {
                    Ok(_) => return Ok(existing),
                    Err(e) => tracing::info!(
                        profile_id = %profile.id,
                        error = %e,
                        "live Entra session unusable; re-acquiring"
                    ),
                }
            }
            self.inner.remove(&profile.id);
        }

        let stored = RefreshTokenStore::get(&profile.id).unwrap_or_else(|e| {
            tracing::warn!(profile_id = %profile.id, error = %e, "keychain read failed");
            None
        });
        if let Some(refresh_token) = stored {
            let session = Arc::new(EntraSession::from_refresh_token(
                http.clone(),
                cfg.clone(),
                refresh_token,
                Some(persist_fn(profile.id.clone())),
            ));
            match session.access_token(MIN_TOKEN_VALIDITY).await {
                Ok(_) => {
                    tracing::info!(profile_id = %profile.id, "Entra sign-in restored silently");
                    self.inner.insert(profile.id.clone(), session.clone());
                    return Ok(session);
                }
                Err(e) if e.requires_interactive() => {
                    tracing::info!(
                        profile_id = %profile.id,
                        error = %e,
                        "cached Entra sign-in rejected; opening browser"
                    );
                    let _ = RefreshTokenStore::delete(&profile.id);
                }
                // Network trouble etc.: the browser flow would hit the same
                // wall, and we must not throw away a refresh token over it.
                Err(e) => return Err(e.into()),
            }
        }

        let tokens = sign_in(
            &http,
            &cfg,
            login_hint_for(&profile.user),
            Some(&profile.id),
            app,
        )
        .await?;
        let session = Arc::new(EntraSession::new(
            http,
            cfg,
            tokens,
            Some(persist_fn(profile.id.clone())),
        ));
        self.inner.insert(profile.id.clone(), session.clone());
        Ok(session)
    }

    /// Drop the in-memory session; the next connect refreshes from the
    /// keychain. Use after a profile edit, whose settings may have changed.
    pub fn forget(&self, profile_id: &str) {
        self.inner.remove(profile_id);
    }

    /// Drop the session *and* the persisted refresh token, so the next
    /// connect goes through the browser again.
    pub fn sign_out(&self, profile_id: &str) -> AppResult<()> {
        self.inner.remove(profile_id);
        RefreshTokenStore::delete(profile_id)?;
        Ok(())
    }

    pub fn has_cached_sign_in(profile_id: &str) -> bool {
        RefreshTokenStore::get(profile_id)
            .map(|t| t.is_some())
            .unwrap_or(false)
    }
}

/// One browser round-trip. Emits [`SIGN_IN_EVENT`] before opening the
/// browser so the UI can react.
pub async fn sign_in(
    http: &reqwest::Client,
    cfg: &EntraConfig,
    login_hint: Option<&str>,
    profile_id: Option<&str>,
    app: &AppHandle,
) -> AppResult<TokenSet> {
    let opts = LoginOptions {
        login_hint: login_hint.map(str::to_string),
        timeout: BROWSER_TIMEOUT,
    };
    let tokens = pg_entra::login_interactive(http, cfg, opts, |url| {
        if let Err(e) = app.emit(SIGN_IN_EVENT, SignInPayload { profile_id, url }) {
            tracing::warn!(error = %e, "could not emit sign-in event");
        }
        open::that_detached(url).map_err(|e| e.to_string())
    })
    .await?;
    Ok(tokens)
}

/// Only a UPN can pre-select an account in the Microsoft picker. Azure also
/// lets the Postgres user be an Entra *group* name; passing that as a hint
/// makes the picker render it as a phantom account, so send nothing and let
/// the person choose their own account.
pub fn login_hint_for(user: &str) -> Option<&str> {
    let user = user.trim();
    user.contains('@').then_some(user)
}

/// The profile to actually connect with. A blank User on an Entra profile
/// means "whoever signs in", so the Postgres role becomes the signed-in
/// account's UPN. Group-role logins set User explicitly and pass through.
pub fn with_default_role(profile: &Profile, account: Option<String>) -> AppResult<Profile> {
    let mut profile = profile.clone();
    if profile.user.trim().is_empty() {
        profile.user = account.ok_or_else(|| {
            AppError::new(
                "entra",
                "Microsoft did not report the signed-in account; set User to your UPN or an Entra group name",
            )
        })?;
    }
    Ok(profile)
}

fn persist_fn(profile_id: String) -> PersistFn {
    Box::new(move |refresh_token: &str| {
        if let Err(e) = RefreshTokenStore::set(&profile_id, refresh_token) {
            tracing::warn!(
                profile_id,
                error = %e,
                "could not persist Entra refresh token; browser sign-in will be needed next launch"
            );
        }
    })
}

/// Adapts an Entra session to pg-core's rotating-credential hook.
pub struct EntraTokenSource(pub Arc<EntraSession>);

impl TokenSource for EntraTokenSource {
    fn access_token(
        &self,
        min_remaining: Duration,
    ) -> BoxFuture<'_, Result<AccessToken, TokenSourceError>> {
        Box::pin(async move {
            let token = self
                .0
                .access_token(min_remaining)
                .await
                .map_err(|e| TokenSourceError(e.to_string()))?;
            Ok(AccessToken {
                secret: token.token,
                expires_at: token.expires_at,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{login_hint_for, with_default_role};
    use pg_profiles::{AuthMethod, Profile, SslMode};

    fn entra_profile(user: &str) -> Profile {
        Profile {
            id: "p1".into(),
            name: "azure".into(),
            host: "h".into(),
            port: 5432,
            database: "d".into(),
            user: user.into(),
            ssl_mode: SslMode::Require,
            app_name: None,
            group: None,
            auth_method: AuthMethod::EntraMfa,
            entra: None,
        }
    }

    #[test]
    fn upn_is_used_as_hint_but_group_names_are_not() {
        assert_eq!(
            login_hint_for(" rodrigo@contoso.com "),
            Some("rodrigo@contoso.com")
        );
        assert_eq!(login_hint_for("directus.sql.non-prod.admins"), None);
        assert_eq!(login_hint_for(""), None);
    }

    #[test]
    fn blank_user_defaults_to_signed_in_account() {
        let p = with_default_role(&entra_profile("  "), Some("me@contoso.com".into())).unwrap();
        assert_eq!(p.user, "me@contoso.com");
    }

    #[test]
    fn explicit_user_wins_over_signed_in_account() {
        let p = with_default_role(
            &entra_profile("directus.sql.non-prod.admins"),
            Some("me@contoso.com".into()),
        )
        .unwrap();
        assert_eq!(p.user, "directus.sql.non-prod.admins");
    }

    #[test]
    fn blank_user_without_account_is_an_error() {
        let err = with_default_role(&entra_profile(""), None).unwrap_err();
        assert_eq!(err.kind, "entra");
    }
}
