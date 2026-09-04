use serde::Serialize;

/// Error type returned to the frontend. `kind` lets the UI branch on cause;
/// `message` is display-ready text.
#[derive(Debug, Serialize)]
pub struct AppError {
    pub kind: &'static str,
    pub message: String,
}

impl AppError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<pg_profiles::ProfileStoreError> for AppError {
    fn from(value: pg_profiles::ProfileStoreError) -> Self {
        AppError::new("profile_store", value.to_string())
    }
}

impl From<pg_profiles::KeychainError> for AppError {
    fn from(value: pg_profiles::KeychainError) -> Self {
        AppError::new("keychain", value.to_string())
    }
}

impl From<pg_core::ConnectionManagerError> for AppError {
    fn from(value: pg_core::ConnectionManagerError) -> Self {
        AppError::new("connection", value.to_string())
    }
}

impl From<pg_entra::EntraError> for AppError {
    fn from(value: pg_entra::EntraError) -> Self {
        let kind = match &value {
            pg_entra::EntraError::Timeout => "entra_timeout",
            pg_entra::EntraError::Denied(_) => "entra_denied",
            _ => "entra",
        };
        AppError::new(kind, value.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
