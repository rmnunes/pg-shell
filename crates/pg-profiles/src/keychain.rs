use keyring::Entry;
use thiserror::Error;

const SERVICE: &str = "pg-shell";

#[derive(Debug, Error)]
pub enum KeychainError {
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
}

/// Thin wrapper around the OS keychain keyed on profile id.
pub struct PasswordStore;

impl PasswordStore {
    fn entry(profile_id: &str) -> Result<Entry, KeychainError> {
        Ok(Entry::new(SERVICE, profile_id)?)
    }

    pub fn set(profile_id: &str, password: &str) -> Result<(), KeychainError> {
        Self::entry(profile_id)?.set_password(password)?;
        Ok(())
    }

    /// Returns `None` when no password has been stored for this profile.
    pub fn get(profile_id: &str) -> Result<Option<String>, KeychainError> {
        match Self::entry(profile_id)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(KeychainError::Keyring(e)),
        }
    }

    pub fn delete(profile_id: &str) -> Result<(), KeychainError> {
        match Self::entry(profile_id)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(KeychainError::Keyring(e)),
        }
    }
}
