//! A signed-in Entra identity that hands out valid access tokens on demand.

use std::time::{Duration, SystemTime};

use tokio::sync::Mutex;

use crate::config::EntraConfig;
use crate::error::EntraError;
use crate::token::{self, TokenSet};

/// Called with every new refresh token so the caller can persist it. Entra
/// rotates refresh tokens on use, so the value stored at sign-in goes stale.
pub type PersistFn = Box<dyn Fn(&str) + Send + Sync>;

#[derive(Clone)]
pub struct AccessToken {
    pub token: String,
    pub expires_at: SystemTime,
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessToken")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

pub struct EntraSession {
    cfg: EntraConfig,
    http: reqwest::Client,
    /// Async mutex so concurrent callers near expiry trigger one refresh, not
    /// one each.
    tokens: Mutex<TokenSet>,
    persist: Option<PersistFn>,
}

impl EntraSession {
    /// Wrap tokens fresh from an interactive sign-in. The refresh token, if
    /// any, is persisted immediately.
    pub fn new(
        http: reqwest::Client,
        cfg: EntraConfig,
        tokens: TokenSet,
        persist: Option<PersistFn>,
    ) -> Self {
        if let (Some(persist), Some(rt)) = (&persist, &tokens.refresh_token) {
            persist(rt);
        }
        Self {
            cfg,
            http,
            tokens: Mutex::new(tokens),
            persist,
        }
    }

    /// Rebuild a session from a persisted refresh token. Nothing is validated
    /// here; the first [`access_token`](Self::access_token) call performs the
    /// refresh and surfaces any failure.
    pub fn from_refresh_token(
        http: reqwest::Client,
        cfg: EntraConfig,
        refresh_token: String,
        persist: Option<PersistFn>,
    ) -> Self {
        Self {
            cfg,
            http,
            tokens: Mutex::new(TokenSet {
                access_token: String::new(),
                refresh_token: Some(refresh_token),
                expires_at: SystemTime::UNIX_EPOCH,
                account: None,
            }),
            persist,
        }
    }

    pub fn config(&self) -> &EntraConfig {
        &self.cfg
    }

    /// UPN of the signed-in account, once a token response has carried an ID
    /// token (the interactive sign-in always does; refreshes usually do).
    pub async fn account(&self) -> Option<String> {
        self.tokens.lock().await.account.clone()
    }

    /// An access token valid for at least `min_remaining`, refreshing if the
    /// cached one is too close to expiry.
    pub async fn access_token(&self, min_remaining: Duration) -> Result<AccessToken, EntraError> {
        let mut guard = self.tokens.lock().await;
        let remaining = guard
            .expires_at
            .duration_since(SystemTime::now())
            .unwrap_or_default();
        if !guard.access_token.is_empty() && remaining >= min_remaining {
            return Ok(AccessToken {
                token: guard.access_token.clone(),
                expires_at: guard.expires_at,
            });
        }

        let refresh_token = guard
            .refresh_token
            .clone()
            .ok_or(EntraError::NoRefreshToken)?;
        let mut fresh = token::refresh(&self.http, &self.cfg, &refresh_token).await?;
        match &fresh.refresh_token {
            Some(rotated) => {
                if let Some(persist) = &self.persist {
                    persist(rotated);
                }
            }
            // Not every response rotates the refresh token; keep the old one.
            None => fresh.refresh_token = guard.refresh_token.take(),
        }
        // Likewise the ID token: keep what we already know about the account.
        if fresh.account.is_none() {
            fresh.account = guard.account.take();
        }
        tracing::debug!("Entra access token refreshed");
        *guard = fresh;
        Ok(AccessToken {
            token: guard.access_token.clone(),
            expires_at: guard.expires_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Minimal token endpoint: answers each connection with the next canned
    /// (status, json) pair. Returns the authority URL and a hit counter.
    async fn mock_token_server(replies: Vec<(u16, String)>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits2 = hits.clone();
        tokio::spawn(async move {
            for (status, body) in replies {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let mut read = 0;
                // Read head + form body (Content-Length driven).
                let (head_end, content_length) = loop {
                    let n = stream.read(&mut buf[read..]).await.unwrap();
                    read += n;
                    let head = String::from_utf8_lossy(&buf[..read]);
                    if let Some(idx) = head.find("\r\n\r\n") {
                        let cl = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        break (idx + 4, cl);
                    }
                    if n == 0 {
                        break (read, 0);
                    }
                };
                while read < head_end + content_length {
                    let n = stream.read(&mut buf[read..]).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    read += n;
                }
                hits2.fetch_add(1, Ordering::SeqCst);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(resp.as_bytes()).await.unwrap();
                stream.shutdown().await.ok();
            }
        });
        (format!("http://127.0.0.1:{port}"), hits)
    }

    fn cfg_for(authority: &str) -> EntraConfig {
        EntraConfig {
            authority: authority.to_string(),
            ..EntraConfig::default()
        }
    }

    fn id_token(upn: &str) -> String {
        format!(
            "h.{}.s",
            URL_SAFE_NO_PAD.encode(format!(r#"{{"preferred_username":"{upn}"}}"#))
        )
    }

    #[tokio::test]
    async fn cached_token_is_reused_until_near_expiry() {
        let (authority, hits) = mock_token_server(vec![]).await;
        let session = EntraSession::new(
            crate::http_client().unwrap(),
            cfg_for(&authority),
            TokenSet {
                access_token: "AT1".into(),
                refresh_token: Some("RT1".into()),
                expires_at: SystemTime::now() + Duration::from_secs(3600),
                account: Some("me@contoso.com".into()),
            },
            None,
        );
        let t = session
            .access_token(Duration::from_secs(600))
            .await
            .unwrap();
        assert_eq!(t.token, "AT1");
        assert_eq!(session.account().await.as_deref(), Some("me@contoso.com"));
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn refreshes_when_stale_and_persists_rotated_refresh_token() {
        let (authority, hits) = mock_token_server(vec![
            (
                200,
                format!(
                    r#"{{"access_token":"AT2","expires_in":3600,"refresh_token":"RT2","id_token":"{}"}}"#,
                    id_token("me@contoso.com")
                ),
            ),
            (200, r#"{"access_token":"AT3","expires_in":3600}"#.to_string()),
        ])
        .await;
        let persisted = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = persisted.clone();
        let session = EntraSession::from_refresh_token(
            crate::http_client().unwrap(),
            cfg_for(&authority),
            "RT1".into(),
            Some(Box::new(move |rt: &str| {
                sink.lock().unwrap().push(rt.to_string())
            })),
        );
        assert!(session.account().await.is_none());

        let t = session
            .access_token(Duration::from_secs(600))
            .await
            .unwrap();
        assert_eq!(t.token, "AT2");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(*persisted.lock().unwrap(), vec!["RT2".to_string()]);
        assert_eq!(session.account().await.as_deref(), Some("me@contoso.com"));

        // Force a refresh by demanding more validity than the token has. The
        // reply carries neither a refresh token nor an ID token, so RT2 and
        // the account must be kept, not dropped.
        let t = session
            .access_token(Duration::from_secs(4000))
            .await
            .unwrap();
        assert_eq!(t.token, "AT3");
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert_eq!(persisted.lock().unwrap().len(), 1);
        assert_eq!(
            session.tokens.lock().await.refresh_token.as_deref(),
            Some("RT2")
        );
        assert_eq!(session.account().await.as_deref(), Some("me@contoso.com"));
    }

    #[tokio::test]
    async fn expired_refresh_token_requires_interactive() {
        let (authority, _) = mock_token_server(vec![(
            400,
            r#"{"error":"invalid_grant","error_description":"AADSTS700082: expired"}"#.to_string(),
        )])
        .await;
        let session = EntraSession::from_refresh_token(
            crate::http_client().unwrap(),
            cfg_for(&authority),
            "dead".into(),
            None,
        );
        let err = session
            .access_token(Duration::from_secs(60))
            .await
            .unwrap_err();
        assert!(err.requires_interactive(), "{err}");
    }

    #[tokio::test]
    async fn session_without_refresh_token_cannot_renew() {
        let (authority, _) = mock_token_server(vec![]).await;
        let session = EntraSession::new(
            crate::http_client().unwrap(),
            cfg_for(&authority),
            TokenSet {
                access_token: "AT".into(),
                refresh_token: None,
                expires_at: SystemTime::now(),
                account: None,
            },
            None,
        );
        assert!(matches!(
            session.access_token(Duration::from_secs(60)).await,
            Err(EntraError::NoRefreshToken)
        ));
    }
}
