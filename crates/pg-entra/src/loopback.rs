//! Loopback HTTP listener that catches the authorization-code redirect.
//!
//! Entra treats `http://localhost` as a loopback redirect and ignores the
//! port, so we bind an ephemeral port each sign-in. Browsers may resolve
//! `localhost` to `::1` before `127.0.0.1`, so we listen on both when we can.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::error::EntraError;

/// Upper bound on a single request head we are willing to buffer.
const MAX_HEAD_BYTES: usize = 8 * 1024;
/// A client that connects but never sends a full request must not stall the
/// accept loop while the real redirect is waiting behind it.
const PER_REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Loopback {
    v4: TcpListener,
    v6: Option<TcpListener>,
    port: u16,
}

/// What the browser brought back.
#[derive(Debug, PartialEq, Eq)]
pub enum Redirect {
    Code { code: String, state: String },
    Denied { error: String, description: String },
}

impl Loopback {
    pub async fn bind() -> Result<Self, EntraError> {
        let v4 = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = v4.local_addr()?.port();
        // Best effort: IPv6 may be disabled, or the port may be taken on ::1.
        let v6 = TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await.ok();
        Ok(Self { v4, v6, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Must be `localhost` (not `127.0.0.1`) for Entra's loopback exemption.
    pub fn redirect_uri(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    /// Serve until a redirect carrying a `code` (or an OAuth `error`) arrives,
    /// then answer the browser with a small page and return the code.
    pub async fn wait_for_code(
        self,
        expected_state: &str,
        timeout: Duration,
    ) -> Result<String, EntraError> {
        tokio::time::timeout(timeout, self.serve(expected_state))
            .await
            .map_err(|_| EntraError::Timeout)?
    }

    async fn accept(&self) -> std::io::Result<TcpStream> {
        match &self.v6 {
            Some(v6) => tokio::select! {
                r = self.v4.accept() => r.map(|(s, _)| s),
                r = v6.accept() => r.map(|(s, _)| s),
            },
            None => self.v4.accept().await.map(|(s, _)| s),
        }
    }

    async fn serve(&self, expected_state: &str) -> Result<String, EntraError> {
        loop {
            let mut stream = self.accept().await?;
            let head = match tokio::time::timeout(PER_REQUEST_READ_TIMEOUT, read_head(&mut stream))
                .await
            {
                Ok(Ok(head)) => head,
                Ok(Err(e)) => {
                    tracing::debug!(error = %e, "loopback: dropping unreadable request");
                    continue;
                }
                Err(_) => {
                    tracing::debug!("loopback: dropping stalled request");
                    continue;
                }
            };
            match parse_request(&head) {
                None => {
                    // favicon probes, preflight, stray tabs — not our redirect.
                    respond(&mut stream, "404 Not Found", &page_not_found()).await;
                }
                Some(Redirect::Denied { error, description }) => {
                    respond(&mut stream, "200 OK", &page_denied(&description)).await;
                    return Err(EntraError::Denied(format!("{error}: {description}")));
                }
                Some(Redirect::Code { code, state }) => {
                    if state != expected_state {
                        respond(&mut stream, "400 Bad Request", &page_state_mismatch()).await;
                        return Err(EntraError::StateMismatch);
                    }
                    respond(&mut stream, "200 OK", &page_ok()).await;
                    return Ok(code);
                }
            }
        }
    }
}

async fn read_head(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut buf: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() >= MAX_HEAD_BYTES {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

async fn respond(stream: &mut TcpStream, status: &str, html: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n\
         {html}",
        html.len()
    );
    // The browser closing early is not our problem; the code is already ours.
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Extract the OAuth redirect parameters from an HTTP request head. `None`
/// when the request is unrelated (no `code` and no `error` in the query).
pub fn parse_request(head: &str) -> Option<Redirect> {
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    if method != "GET" {
        return None;
    }
    let (_, query) = target.split_once('?')?;
    parse_query(query)
}

pub fn parse_query(query: &str) -> Option<Redirect> {
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut description = None;
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        match &*k {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            "error_description" => description = Some(v.into_owned()),
            _ => {}
        }
    }
    if let Some(error) = error {
        return Some(Redirect::Denied {
            error,
            description: description.unwrap_or_default(),
        });
    }
    Some(Redirect::Code {
        code: code?,
        state: state.unwrap_or_default(),
    })
}

const PAGE_STYLE: &str = "body{font-family:system-ui,sans-serif;background:#1e1e1e;color:#ddd;display:flex;align-items:center;justify-content:center;height:100vh;margin:0}main{text-align:center;max-width:32rem;padding:2rem}h1{font-size:1.4rem;margin:0 0 .5rem}p{color:#aaa;margin:0}";

fn page(title: &str, body_html: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>pg-shell</title><style>{PAGE_STYLE}</style></head><body><main><h1>{}</h1>{body_html}</main></body></html>",
        html_escape(title)
    )
}

fn page_ok() -> String {
    page(
        "Signed in",
        "<p>You can close this tab and return to pg-shell.</p>",
    )
}

fn page_not_found() -> String {
    "<!doctype html><html><body></body></html>".to_string()
}

fn page_state_mismatch() -> String {
    page(
        "Sign-in rejected",
        "<p>This response did not match the pending request. Return to pg-shell and try again.</p>",
    )
}

fn page_denied(description: &str) -> String {
    page(
        "Sign-in not completed",
        &format!(
            "<p>{}</p><p>Return to pg-shell to try again.</p>",
            html_escape(description)
        ),
    )
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_code_and_state() {
        let head =
            "GET /?code=0.AXAA%2Babc&state=xyz&session_state=q HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(
            parse_request(head),
            Some(Redirect::Code {
                code: "0.AXAA+abc".into(),
                state: "xyz".into()
            })
        );
    }

    #[test]
    fn parses_oauth_error() {
        let head = "GET /?error=access_denied&error_description=AADSTS65004%3A+User+declined&state=s HTTP/1.1\r\n\r\n";
        assert_eq!(
            parse_request(head),
            Some(Redirect::Denied {
                error: "access_denied".into(),
                description: "AADSTS65004: User declined".into()
            })
        );
    }

    #[test]
    fn ignores_unrelated_requests() {
        assert_eq!(parse_request("GET /favicon.ico HTTP/1.1\r\n\r\n"), None);
        assert_eq!(parse_request("GET /?foo=bar HTTP/1.1\r\n\r\n"), None);
        assert_eq!(parse_request("POST /?code=x HTTP/1.1\r\n\r\n"), None);
        assert_eq!(parse_request(""), None);
    }

    #[test]
    fn escapes_html_in_denied_page() {
        let page = page_denied("<script>alert(1)</script>");
        assert!(!page.contains("<script>"));
        assert!(page.contains("&lt;script&gt;"));
    }

    async fn send(port: u16, request: &str) -> String {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .unwrap();
        s.write_all(request.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    #[tokio::test]
    async fn returns_code_after_ignoring_noise() {
        let lb = Loopback::bind().await.unwrap();
        let port = lb.port();
        assert_eq!(lb.redirect_uri(), format!("http://localhost:{port}"));
        let waiter = tokio::spawn(lb.wait_for_code("st4te", Duration::from_secs(5)));

        let noise = send(port, "GET /favicon.ico HTTP/1.1\r\nHost: localhost\r\n\r\n").await;
        assert!(noise.starts_with("HTTP/1.1 404"));

        let ok = send(
            port,
            "GET /?code=the-code&state=st4te HTTP/1.1\r\nHost: localhost\r\n\r\n",
        )
        .await;
        assert!(ok.starts_with("HTTP/1.1 200"));
        assert!(ok.contains("Signed in"));

        assert_eq!(waiter.await.unwrap().unwrap(), "the-code");
    }

    #[tokio::test]
    async fn rejects_state_mismatch() {
        let lb = Loopback::bind().await.unwrap();
        let port = lb.port();
        let waiter = tokio::spawn(lb.wait_for_code("expected", Duration::from_secs(5)));
        let resp = send(port, "GET /?code=c&state=forged HTTP/1.1\r\n\r\n").await;
        assert!(resp.starts_with("HTTP/1.1 400"));
        assert!(matches!(
            waiter.await.unwrap(),
            Err(EntraError::StateMismatch)
        ));
    }

    #[tokio::test]
    async fn surfaces_denial() {
        let lb = Loopback::bind().await.unwrap();
        let port = lb.port();
        let waiter = tokio::spawn(lb.wait_for_code("s", Duration::from_secs(5)));
        let resp = send(
            port,
            "GET /?error=access_denied&error_description=nope&state=s HTTP/1.1\r\n\r\n",
        )
        .await;
        assert!(resp.contains("Sign-in not completed"));
        match waiter.await.unwrap() {
            Err(EntraError::Denied(msg)) => assert!(msg.contains("access_denied")),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn times_out_without_redirect() {
        let lb = Loopback::bind().await.unwrap();
        let result = lb.wait_for_code("s", Duration::from_millis(50)).await;
        assert!(matches!(result, Err(EntraError::Timeout)));
    }
}
