use keyring::Entry;
use thiserror::Error;

const SERVICE: &str = "pg-shell";

#[derive(Debug, Error)]
pub enum KeychainError {
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("stored secret is not valid UTF-8")]
    InvalidUtf8,
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

/// Microsoft Entra refresh tokens, one per profile.
///
/// Lives under a distinct keychain username (`<profile_id>/entra-refresh`) so
/// a profile that switches auth method never mistakes one secret for the
/// other. Stored as raw bytes rather than through `set_password`: the Windows
/// Credential Manager caps a blob at 2560 bytes and `set_password` encodes
/// UTF-16, which would halve the budget — refresh tokens run 1–2 KB.
pub struct RefreshTokenStore;

impl RefreshTokenStore {
    fn entry(profile_id: &str) -> Result<Entry, KeychainError> {
        Ok(Entry::new(SERVICE, &format!("{profile_id}/entra-refresh"))?)
    }

    pub fn set(profile_id: &str, token: &str) -> Result<(), KeychainError> {
        Self::entry(profile_id)?.set_secret(token.as_bytes())?;
        Ok(())
    }

    /// Returns `None` when no refresh token has been stored for this profile.
    pub fn get(profile_id: &str) -> Result<Option<String>, KeychainError> {
        match Self::entry(profile_id)?.get_secret() {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|_| KeychainError::InvalidUtf8),
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
