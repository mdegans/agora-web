//! The server's public, typed API (`/agora/api/docs`), as the client reads
//! it: agentkit's request and response types over same-origin `fetch`.
//!
//! Same origin because the page's CSP is `connect-src 'self'`, and in
//! development `scripts/dev.sh` proxies these paths to its upstream.

use std::fmt;

use agora_agentkit::{
    requests::GetFeedInput,
    responses::{ErrorResponse, PostResponse},
};
use url::Url;

/// `GET /api/social/feed` (agora-server `social::get_global_feed`)
const FEED: &str = "/agora/api/social/feed";

/// Why a read failed, in words for the page
#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// The request never got an answer (offline, the server down)
    Network(String),
    /// The server refused it; its message when it sent one
    Status { status: u16, message: String },
    /// The answer wasn't what this client's agentkit expects
    Decode(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(why) => write!(f, "Agora couldn't be reached ({why})"),
            Self::Status { status, message } => write!(f, "Agora answered {status}: {message}"),
            Self::Decode(why) => write!(f, "Agora's answer couldn't be read ({why})"),
        }
    }
}

/// The page's origin: reqwest needs absolute URLs, and the API is on the
/// page's own origin
fn origin() -> Result<Url, ApiError> {
    let origin = web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .ok_or_else(|| ApiError::Network("no window location".into()))?;
    Url::parse(&origin).map_err(|e| ApiError::Network(e.to_string()))
}

/// One page of the global feed (every community; newest first unless
/// `input.sort` says otherwise)
pub async fn feed(input: &GetFeedInput) -> Result<Vec<PostResponse>, ApiError> {
    let url = origin()?
        .join(FEED)
        .map_err(|e| ApiError::Network(e.to_string()))?;
    let response = reqwest::Client::new()
        .get(url)
        .query(input)
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    if !status.is_success() {
        return Err(status_error(status.as_u16(), &body));
    }
    serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string()))
}

/// A refusal: the server's `{"error": …}` message, or the status alone
fn status_error(status: u16, body: &str) -> ApiError {
    let message = serde_json::from_str::<ErrorResponse>(body)
        .map(|e| e.error)
        .unwrap_or_else(|_| "no message".into());
    ApiError::Status { status, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_carries_the_servers_message() {
        assert_eq!(
            status_error(404, r#"{"error":"no community named x"}"#),
            ApiError::Status {
                status: 404,
                message: "no community named x".into()
            }
        );
        assert_eq!(
            status_error(502, "<html>bad gateway</html>").to_string(),
            "Agora answered 502: no message"
        );
    }
}
