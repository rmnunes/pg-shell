use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use pg_profiles::{Profile, ProfileId, SslMode};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{PgPool, Row};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConnectionManagerError {
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("profile has no stored password and none was provided")]
    MissingPassword,
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

/// Owns one `PgPool` per connected profile.
#[derive(Default, Clone)]
pub struct ConnectionManager {
    pools: Arc<DashMap<ProfileId, PgPool>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_pool(&self, profile_id: &str) -> bool {
        self.pools.contains_key(profile_id)
    }

    pub fn pool(&self, profile_id: &str) -> Option<PgPool> {
        self.pools.get(profile_id).map(|e| e.value().clone())
    }

    /// Create (or replace) the pool for a profile. Performs an initial
    /// connection to surface auth / network errors immediately and returns
    /// server introspection.
    pub async fn connect(
        &self,
        profile: &Profile,
        password: &str,
    ) -> Result<ServerInfo, ConnectionManagerError> {
        let opts = connect_options(profile, password);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(10))
            .connect_with(opts)
            .await?;
        let server = server_info(&pool).await?;
        self.pools.insert(profile.id.clone(), pool);
        Ok(server)
    }

    pub async fn disconnect(&self, profile_id: &str) {
        if let Some((_, pool)) = self.pools.remove(profile_id) {
            pool.close().await;
        }
    }

    /// Transient, non-cached connectivity probe. Does not touch the pool map.
    pub async fn test(
        profile: &Profile,
        password: &str,
    ) -> Result<TestOutcome, ConnectionManagerError> {
        let opts = connect_options(profile, password);
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

fn connect_options(profile: &Profile, password: &str) -> PgConnectOptions {
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
        .password(password)
        .ssl_mode(ssl);
    if let Some(app) = profile.app_name.as_deref() {
        opts = opts.application_name(app);
    } else {
        opts = opts.application_name("pg-shell");
    }
    opts
}

async fn server_info(pool: &PgPool) -> Result<ServerInfo, sqlx::Error> {
    let row = sqlx::query(
        "SELECT version() AS v, current_database() AS db, current_user::text AS u",
    )
    .fetch_one(pool)
    .await?;
    Ok(ServerInfo {
        server_version: row.try_get::<String, _>("v")?,
        current_database: row.try_get::<String, _>("db")?,
        current_user: row.try_get::<String, _>("u")?,
    })
}
