//! Connection profile storage and OS keychain access.
//!
//! Profiles are persisted as JSON under the user's app-data directory.
//! Passwords are stored only in the OS keychain (`keyring` crate), keyed by
//! profile id.

mod keychain;
mod store;

pub use keychain::PasswordStore;
pub use store::{Profile, ProfileId, ProfileInput, ProfileStore, ProfileStoreError, SslMode};
