//! Connection profile storage and OS keychain access.
//!
//! Profiles are persisted as JSON under the user's app-data directory.
//! Secrets are stored only in the OS keychain (`keyring` crate), keyed by
//! profile id: the password for password-auth profiles, the Entra refresh
//! token for Entra profiles. Access tokens are never persisted.

mod keychain;
mod store;

pub use keychain::{KeychainError, PasswordStore, RefreshTokenStore};
pub use store::{
    AuthMethod, EntraSettings, Profile, ProfileId, ProfileInput, ProfileStore, ProfileStoreError,
    SslMode,
};
