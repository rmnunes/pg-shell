use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use dashmap::DashMap;
use futures_util::future::BoxFuture;
use pg_profiles::{Profile, ProfileId, SslMode};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{PgPool, Row};
use thiserror::Error;

/// Rotate a token-backed pool credential this far ahead of expiry.
const TOKEN_REFRESH_LEAD: Duration = Duration::from_secs(5 * 60);
/// Never hand a connection a token with less validity than this.
const TOKEN_MIN_VALIDITY: Duration = Duration::from_secs(10 * 60);
/// Back-off between failed refresh attempts.
const TOKEN_RETRY_DELAY: Duration = Duration::from_secs(60);
/// Floor on the refresh loop's sleep, so a token already inside the lead
/// window does not spin.
const TOKEN_MIN_WAIT: Duration = Duration::from_secs(15);

#[derive(Debug, Error)]
pub enum ConnectionManagerError {
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("profile has no stored password and none was provided")]
    MissingPassword,
    #[error("could not obtain an access token: {0}")]
    Token(#[from] TokenSourceError),
}

impl ConnectionManagerError {
    /// True for Postgres `28P01 invalid_password`: the server accepted the
    /// connection and rejected the credential, as opposed to network, TLS or
    /// database-name failures.
    pub fn is_auth_failure(&self) -> bool {
        matches!(
            self,
            ConnectionManagerError::Sqlx(sqlx::Error::Database(db))
                if db.code().as_deref() == Some("28P01")
        )
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
pub struct TokenSourceError(pub String);

/// A short-lived secret usable as a Postgres password.
#[derive(Clone)]
pub struct AccessToken {
    pub secret: String,
    pub expires_at: SystemTime,
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessToken")
            .field("secret", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Something that can mint access tokens on demand — an Entra session, for
/// instance. Implementations cache and refresh internally; the pool only asks
/// for "a token good for at least this long".
pub trait TokenSource: Send + Sync + 'static {
    fn access_token(
        &self,
        min_remaining: Duration,
    ) -> BoxFuture<'_, Result<AccessToken, TokenSourceError>>;
}

/// How a pool authenticates.
///
/// sqlx opens physical connections lazily and recycles them (30 min max
/// lifetime by default), so a token that was valid at connect time is not
/// enough: token credentials get a background task that pushes a fresh token
/// into the pool via [`PgPool::set_connect_options`] ahead of expiry.
/// Established connections are unaffected — Postgres validates the password
/// only at login.
#[derive(Clone)]
pub enum Credential {
    Password(String),
    Token(Arc<dyn TokenSource>),
}

impl Credential {
    async fn resolve(&self) -> Result<(String, Option<SystemTime>), ConnectionManagerError> {
        match self {
            Credential::Password(p) => Ok((p.clone(), None)),
            Credential::Token(source) => {
                let token = source.access_token(TOKEN_MIN_VALIDITY).await?;
                Ok((token.secret, Some(token.expires_at)))
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerInfo {
    pub server_version: String,
    pub current_database: String,
    pub current_user: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TestOutcome {
    pub latency_ms: u64,
    pub server: ServerInfo,
}

struct PoolEntry {
    pool: PgPool,
    /// Credential-rotation task for token-backed pools.
    refresher: Option<tokio::task::JoinHandle<()>>,
}

impl PoolEntry {
    async fn shutdown(self) {
        if let Some(task) = self.refresher {
            task.abort();
        }
        self.pool.close().await;
    }
}

/// Owns one `PgPool` per connected profile.
#[derive(Default, Clone)]
pub struct ConnectionManager {
    pools: Arc<DashMap<ProfileId, PoolEntry>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_pool(&self, profile_id: &str) -> bool {
        self.pools.contains_key(profile_id)
    }

    pub fn pool(&self, profile_id: &str) -> Option<PgPool> {
        self.pools.get(profile_id).map(|e| e.value().pool.clone())
    }

    /// Create (or replace) the pool for a profile. Performs an initial
    /// connection to surface auth / network errors immediately and returns
    /// server introspection.
    pub async fn connect(
        &self,
        profile: &Profile,
        credential: Credential,
    ) -> Result<ServerInfo, ConnectionManagerError> {
        let (secret, expires_at) = credential.resolve().await?;
        let base = base_options(profile);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(base.clone().password(&secret))
            .await?;
        let server = server_info(&pool).await?;

        let refresher = match (&credential, expires_at) {
            (Credential::Token(source), Some(expires_at)) => Some(tokio::spawn(refresh_loop(
                pool.clone(),
                base,
                source.clone(),
                expires_at,
                profile.id.clone(),
            ))),
            _ => None,
        };

        if let Some((_, old)) = self.pools.remove(&profile.id) {
            old.shutdown().await;
        }
        self.pools
            .insert(profile.id.clone(), PoolEntry { pool, refresher });
        Ok(server)
    }

    pub async fn disconnect(&self, profile_id: &str) {
        if let Some((_, entry)) = self.pools.remove(profile_id) {
            entry.shutdown().await;
        }
    }

    /// Transient, non-cached connectivity probe. Does not touch the pool map.
    pub async fn test(
        profile: &Profile,
        credential: Credential,
    ) -> Result<TestOutcome, ConnectionManagerError> {
        let (secret, _) = credential.resolve().await?;
        let opts = base_options(profile).password(&secret);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(opts)
            .await?;
        let start = Instant::now();
        let server = server_info(&pool).await?;
        let latency_ms = start.elapsed().as_millis() as u64;
        pool.close().await;
        Ok(TestOutcome { latency_ms, server })
    }
}

/// Keep the pool's connect options holding a token that new connections can
/// log in with. Ends when the pool closes (or the task is aborted).
async fn refresh_loop(
    pool: PgPool,
    base: PgConnectOptions,
    source: Arc<dyn TokenSource>,
    expires_at: SystemTime,
    profile_id: String,
) {
    let mut next_attempt = Instant::now() + time_until_refresh(expires_at);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(next_attempt.into()) => {}
            _ = pool.close_event() => return,
        }
        if pool.is_closed() {
            return;
        }
        match source.access_token(TOKEN_MIN_VALIDITY).await {
            Ok(token) => {
                pool.set_connect_options(base.clone().password(&token.secret));
                tracing::debug!(profile_id, "rotated pool credential");
                next_attempt = Instant::now() + time_until_refresh(token.expires_at);
            }
            Err(e) => {
                tracing::warn!(
                    profile_id,
                    error = %e,
                    "credential refresh failed; new connections may be refused until it succeeds"
                );
                next_attempt = Instant::now() + TOKEN_RETRY_DELAY;
            }
        }
    }
}

fn time_until_refresh(expires_at: SystemTime) -> Duration {
    expires_at
        .duration_since(SystemTime::now())
        .unwrap_or_default()
        .saturating_sub(TOKEN_REFRESH_LEAD)
        .max(TOKEN_MIN_WAIT)
}

fn base_options(profile: &Profile) -> PgConnectOptions {
    let ssl = match profile.ssl_mode {
        SslMode::Disable => PgSslMode::Disable,
        SslMode::Prefer => PgSslMode::Prefer,
        SslMode::Require => PgSslMode::Require,
        SslMode::VerifyCa => PgSslMode::VerifyCa,
        SslMode::VerifyFull => PgSslMode::VerifyFull,
    };
    let mut opts = PgConnectOptions::new()
        .host(&profile.host)
        .port(profile.port)
        .database(&profile.database)
        .username(&profile.user)
        .ssl_mode(ssl);
    if let Some(app) = profile.app_name.as_deref() {
        opts = opts.application_name(app);
    } else {
        opts = opts.application_name("pg-shell");
    }
    opts
}

async fn server_info(pool: &PgPool) -> Result<ServerInfo, sqlx::Error> {
    let row =
        sqlx::query("SELECT version() AS v, current_database() AS db, current_user::text AS u")
            .fetch_one(pool)
            .await?;
    Ok(ServerInfo {
        server_version: row.try_get::<String, _>("v")?,
        current_database: row.try_get::<String, _>("db")?,
        current_user: row.try_get::<String, _>("u")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_is_scheduled_ahead_of_expiry() {
        let in_an_hour = SystemTime::now() + Duration::from_secs(3600);
        let wait = time_until_refresh(in_an_hour);
        assert!(wait > Duration::from_secs(3600 - 300 - 5));
        assert!(wait <= Duration::from_secs(3600 - 300));
    }

    #[test]
    fn refresh_wait_has_a_floor() {
        assert_eq!(
            time_until_refresh(SystemTime::now() - Duration::from_secs(10)),
            TOKEN_MIN_WAIT
        );
        assert_eq!(
            time_until_refresh(SystemTime::now() + Duration::from_secs(60)),
            TOKEN_MIN_WAIT
        );
    }
}
