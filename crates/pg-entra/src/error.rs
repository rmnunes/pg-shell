use thiserror::Error;

#[derive(Debug, Error)]
pub enum EntraError {
    #[error("could not start the local sign-in listener: {0}")]
    Listener(#[from] std::io::Error),
    #[error("could not open the browser for sign-in: {0}")]
    Browser(String),
    #[error("timed out waiting for the browser sign-in to complete")]
    Timeout,
    #[error("sign-in was cancelled or denied ({0})")]
    Denied(String),
    #[error("sign-in redirect did not match this request (state mismatch)")]
    StateMismatch,
    #[error("request to Microsoft Entra failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The token endpoint answered with an OAuth error body. `error` is the
    /// machine code (`invalid_grant`, `interaction_required`, …).
    #[error("Microsoft Entra rejected the request ({error}): {description}")]
    OAuth { error: String, description: String },
    #[error("unexpected response from Microsoft Entra: {0}")]
    Malformed(String),
    #[error("no refresh token available; sign in again")]
    NoRefreshToken,
    #[error("invalid Entra configuration: {0}")]
    Config(String),
}

impl EntraError {
    /// True when the failure means the cached sign-in is dead and the user
    /// must go through the browser again (as opposed to a transient network
    /// blip that a retry might fix).
    pub fn requires_interactive(&self) -> bool {
        match self {
            EntraError::OAuth { error, .. } => matches!(
                error.as_str(),
                "invalid_grant" | "interaction_required" | "login_required" | "consent_required"
            ),
            EntraError::NoRefreshToken => true,
            _ => false,
        }
    }
}
