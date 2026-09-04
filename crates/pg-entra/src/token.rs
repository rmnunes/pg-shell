//! Token endpoint calls: code redemption and refresh.
//!
//! Response bodies carry bearer tokens, so they are never logged, and error
//! messages never echo them.

use std::time::{Duration, SystemTime};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Deserializer};

use crate::config::EntraConfig;
use crate::error::EntraError;

#[derive(Clone)]
pub struct TokenSet {
    pub access_token: String,
    /// Absent when the client did not request `offline_access` or the tenant
    /// policy withholds it.
    pub refresh_token: Option<String>,
    pub expires_at: SystemTime,
    /// The signed-in account (UPN) from the ID token, when Entra returned
    /// one. Lets callers default the Postgres role to whoever signed in.
    pub account: Option<String>,
}

impl std::fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSet")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at", &self.expires_at)
            .field("account", &self.account)
            .finish()
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(deserialize_with = "number_or_string")]
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
    /// Present because we request the `openid` scope.
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct OAuthErrorBody {
    error: String,
    #[serde(default)]
    error_description: String,
}

/// The v2 endpoint sends `expires_in` as a number; v1 and some proxies send a
/// string. Accept both.
fn number_or_string<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Num(u64),
        Str(String),
    }
    match Raw::deserialize(d)? {
        Raw::Num(n) => Ok(n),
        Raw::Str(s) => s.trim().parse().map_err(serde::de::Error::custom),
    }
}

pub async fn redeem_code(
    http: &reqwest::Client,
    cfg: &EntraConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenSet, EntraError> {
    let scope = cfg.scope();
    post_token(
        http,
        cfg,
        &[
            ("client_id", cfg.client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
            ("scope", scope.as_str()),
        ],
    )
    .await
}

pub async fn refresh(
    http: &reqwest::Client,
    cfg: &EntraConfig,
    refresh_token: &str,
) -> Result<TokenSet, EntraError> {
    let scope = cfg.scope();
    post_token(
        http,
        cfg,
        &[
            ("client_id", cfg.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", scope.as_str()),
        ],
    )
    .await
}

async fn post_token(
    http: &reqwest::Client,
    cfg: &EntraConfig,
    form: &[(&str, &str)],
) -> Result<TokenSet, EntraError> {
    // Encoded by hand rather than via reqwest's `form` feature: `url` is
    // already a dependency and this avoids pulling in serde_urlencoded.
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish();
    let response = http
        .post(cfg.token_endpoint())
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await?;
    let status = response.status().as_u16();
    let body = response.bytes().await?;
    parse_token_body(status, &body)
}

fn parse_token_body(status: u16, body: &[u8]) -> Result<TokenSet, EntraError> {
    if !(200..300).contains(&status) {
        if let Ok(err) = serde_json::from_slice::<OAuthErrorBody>(body) {
            return Err(EntraError::OAuth {
                error: err.error,
                description: first_line(&err.error_description),
            });
        }
        return Err(EntraError::Malformed(format!(
            "token endpoint returned HTTP {status}"
        )));
    }
    let parsed: TokenResponse = serde_json::from_slice(body)
        .map_err(|e| EntraError::Malformed(format!("token response: {e}")))?;
    Ok(TokenSet {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        expires_at: SystemTime::now() + Duration::from_secs(parsed.expires_in),
        account: parsed.id_token.as_deref().and_then(account_from_id_token),
    })
}

/// Read the signed-in account out of an ID token's claims.
///
/// The token arrived straight from the token endpoint over TLS, so its
/// signature is not re-verified here: this value seeds a default user name,
/// it is not an authorization decision (the server validates the access
/// token on its own).
fn account_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    ["preferred_username", "upn", "email"]
        .iter()
        .find_map(|k| claims.get(*k).and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Entra error descriptions are paragraphs with trace ids and timestamps;
/// the first sentence is the part worth showing.
fn first_line(description: &str) -> String {
    description.lines().next().unwrap_or("").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unsigned JWT with the given JSON claims; enough for claim parsing.
    fn fake_id_token(claims: &str) -> String {
        format!(
            "{}.{}.sig",
            URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#),
            URL_SAFE_NO_PAD.encode(claims)
        )
    }

    #[test]
    fn parses_success_with_numeric_expiry() {
        let body = br#"{"token_type":"Bearer","scope":"x","expires_in":3599,"ext_expires_in":3599,"access_token":"AT","refresh_token":"RT"}"#;
        let t = parse_token_body(200, body).unwrap();
        assert_eq!(t.access_token, "AT");
        assert_eq!(t.refresh_token.as_deref(), Some("RT"));
        assert!(t.account.is_none());
        let remaining = t.expires_at.duration_since(SystemTime::now()).unwrap();
        assert!(remaining > Duration::from_secs(3500) && remaining <= Duration::from_secs(3599));
    }

    #[test]
    fn parses_success_with_string_expiry_and_no_refresh_token() {
        let body = br#"{"expires_in":"3600","access_token":"AT"}"#;
        let t = parse_token_body(200, body).unwrap();
        assert!(t.refresh_token.is_none());
    }

    #[test]
    fn account_comes_from_the_id_token() {
        let id = fake_id_token(r#"{"aud":"x","preferred_username":"me@contoso.com","name":"Me"}"#);
        let body = format!(r#"{{"expires_in":3600,"access_token":"AT","id_token":"{id}"}}"#);
        let t = parse_token_body(200, body.as_bytes()).unwrap();
        assert_eq!(t.account.as_deref(), Some("me@contoso.com"));
    }

    #[test]
    fn account_falls_back_to_upn_and_tolerates_garbage() {
        assert_eq!(
            account_from_id_token(&fake_id_token(r#"{"upn":"me@contoso.com"}"#)).as_deref(),
            Some("me@contoso.com")
        );
        assert_eq!(
            account_from_id_token(&fake_id_token(r#"{"sub":"abc"}"#)),
            None
        );
        assert_eq!(account_from_id_token("not-a-jwt"), None);
        assert_eq!(account_from_id_token("a.!!!.c"), None);
    }

    #[test]
    fn maps_oauth_error_body() {
        let body = br#"{"error":"invalid_grant","error_description":"AADSTS70000: The refresh token has expired.\r\nTrace ID: abc","error_codes":[70000]}"#;
        match parse_token_body(400, body).unwrap_err() {
            EntraError::OAuth { error, description } => {
                assert_eq!(error, "invalid_grant");
                assert_eq!(description, "AADSTS70000: The refresh token has expired.");
            }
            other => panic!("unexpected: {other:?}"),
        }
        assert!(parse_token_body(400, body)
            .unwrap_err()
            .requires_interactive());
    }

    #[test]
    fn non_json_failure_does_not_echo_body() {
        let err = parse_token_body(502, b"<html>bad gateway with secret-looking stuff</html>")
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("502"));
        assert!(!msg.contains("secret"));
    }

    #[test]
    fn debug_redacts_tokens() {
        let t = TokenSet {
            access_token: "super-secret".into(),
            refresh_token: Some("also-secret".into()),
            expires_at: SystemTime::UNIX_EPOCH,
            account: Some("me@contoso.com".into()),
        };
        let dbg = format!("{t:?}");
        assert!(!dbg.contains("secret"));
        assert!(dbg.contains("me@contoso.com"));
    }
}
