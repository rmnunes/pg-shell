//! Interactive (browser) sign-in: authorization code + PKCE on a loopback
//! redirect.

use std::time::Duration;

use url::Url;

use crate::config::EntraConfig;
use crate::error::EntraError;
use crate::loopback::Loopback;
use crate::pkce;
use crate::token::{self, TokenSet};

pub struct LoginOptions {
    /// Pre-fills the account picker; typically the profile's user name.
    pub login_hint: Option<String>,
    /// How long to wait for the browser round-trip.
    pub timeout: Duration,
}

impl Default for LoginOptions {
    fn default() -> Self {
        Self {
            login_hint: None,
            timeout: Duration::from_secs(300),
        }
    }
}

/// Run the browser flow. `open_url` receives the authorize URL and must hand
/// it to the user's browser (and may also surface it in the UI); returning an
/// error aborts the sign-in with [`EntraError::Browser`].
pub async fn login_interactive<F>(
    http: &reqwest::Client,
    cfg: &EntraConfig,
    opts: LoginOptions,
    open_url: F,
) -> Result<TokenSet, EntraError>
where
    F: FnOnce(&str) -> Result<(), String>,
{
    let listener = Loopback::bind().await?;
    let redirect_uri = listener.redirect_uri();
    let pkce = pkce::generate();
    let state = pkce::random_urlsafe(16);
    let url = build_authorize_url(
        cfg,
        &redirect_uri,
        &pkce.challenge,
        &state,
        opts.login_hint.as_deref(),
    )?;

    tracing::info!(
        tenant = %cfg.tenant,
        port = listener.port(),
        "starting Entra interactive sign-in"
    );
    open_url(&url).map_err(EntraError::Browser)?;

    let code = listener.wait_for_code(&state, opts.timeout).await?;
    let tokens = token::redeem_code(http, cfg, &code, &pkce.verifier, &redirect_uri).await?;
    tracing::info!(
        has_refresh_token = tokens.refresh_token.is_some(),
        "Entra interactive sign-in complete"
    );
    Ok(tokens)
}

pub fn build_authorize_url(
    cfg: &EntraConfig,
    redirect_uri: &str,
    code_challenge: &str,
    state: &str,
    login_hint: Option<&str>,
) -> Result<String, EntraError> {
    let mut url = Url::parse(&cfg.authorize_endpoint())
        .map_err(|e| EntraError::Config(format!("authorize endpoint: {e}")))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", &cfg.client_id)
            .append_pair("response_type", "code")
            .append_pair("response_mode", "query")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", &cfg.scope())
            .append_pair("state", state)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            // Always show the account picker: the person at the keyboard may
            // be signed into the browser as someone other than the DB user.
            .append_pair("prompt", "select_account");
        if let Some(hint) = login_hint.map(str::trim).filter(|h| !h.is_empty()) {
            q.append_pair("login_hint", hint);
        }
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_carries_pkce_and_loopback_redirect() {
        let cfg = EntraConfig::new(Some("contoso.onmicrosoft.com"), Some("client-1"));
        let url = build_authorize_url(
            &cfg,
            "http://localhost:51234",
            "CHALLENGE",
            "STATE",
            Some("me@contoso.com"),
        )
        .unwrap();
        let parsed = Url::parse(&url).unwrap();
        assert_eq!(parsed.host_str(), Some("login.microsoftonline.com"));
        assert_eq!(
            parsed.path(),
            "/contoso.onmicrosoft.com/oauth2/v2.0/authorize"
        );
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "client-1");
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["redirect_uri"], "http://localhost:51234");
        assert_eq!(q["code_challenge"], "CHALLENGE");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["state"], "STATE");
        assert_eq!(q["login_hint"], "me@contoso.com");
        assert!(q["scope"].contains("ossrdbms-aad.database.windows.net/.default"));
        assert!(q["scope"].contains("offline_access"));
    }

    #[test]
    fn blank_login_hint_is_omitted() {
        let cfg = EntraConfig::default();
        let url = build_authorize_url(&cfg, "http://localhost:1", "c", "s", Some("  ")).unwrap();
        assert!(!url.contains("login_hint"));
    }
}
