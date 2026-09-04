//! Microsoft Entra ID token acquisition for Azure Database for PostgreSQL.
//!
//! Azure's Postgres flavours accept an Entra access token as the password
//! for a principal created with `pgaadauth_create_principal`. This crate gets
//! that token the way SSMS's "Microsoft Entra MFA" option does:
//!
//! 1. [`login_interactive`] runs the OAuth 2.0 authorization-code flow with
//!    PKCE. The system browser handles credentials, MFA and Conditional
//!    Access; the code comes back on a loopback listener bound to an
//!    ephemeral port.
//! 2. [`EntraSession`] holds the resulting tokens and silently refreshes the
//!    access token ahead of expiry using the refresh token. Callers persist
//!    the refresh token (it rotates) via a callback; access tokens stay in
//!    memory only.
//!
//! Nothing here touches Postgres or the keychain — those are the caller's.

mod config;
mod error;
mod login;
mod loopback;
mod pkce;
mod session;
mod token;

pub use config::{
    EntraConfig, DEFAULT_AUTHORITY, DEFAULT_CLIENT_ID, DEFAULT_TENANT, OSSRDBMS_SCOPE,
};
pub use error::EntraError;
pub use login::{build_authorize_url, login_interactive, LoginOptions};
pub use session::{AccessToken, EntraSession, PersistFn};
pub use token::TokenSet;

/// HTTP client suitable for talking to the Entra token endpoint.
pub fn http_client() -> Result<reqwest::Client, EntraError> {
    // rustls picks a crypto provider from crate features only when exactly
    // one is compiled in. Installing `ring` explicitly keeps this working if
    // another dependency ever brings in aws-lc-rs. Already-installed is fine.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("pg-shell/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(EntraError::Http)
}
