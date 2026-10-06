//! The Transmission RPC client: JSON-over-HTTP with CSRF session handling.
//!
//! Transmission protects its RPC endpoint with a session id. The first request
//! of a session is answered `409 Conflict` carrying the current id in the
//! `X-Transmission-Session-Id` header; the request must then be repeated with
//! that header. The id is cached on the client and refreshed whenever the
//! daemon restarts and rejects a stale one.

use std::sync::Mutex;
use std::time::Duration;

use reqwest::{Client, Response, StatusCode};
use serde_json::{json, Value};

use crate::config::Config;
use crate::error::Error;

/// Header carrying the CSRF session id.
const SESSION_HEADER: &str = "X-Transmission-Session-Id";

/// Per-request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A connection to one Transmission RPC endpoint.
///
/// One instance is shared by every tool call; it owns the HTTP connection pool
/// and the cached CSRF session id.
#[derive(Debug)]
pub struct TransmissionClient {
    http: Client,
    url: String,
    auth: Option<(String, String)>,
    session_id: Mutex<Option<String>>,
}

impl TransmissionClient {
    /// Build a client for the endpoint described by `config`.
    pub fn new(config: &Config) -> Result<Self, Error> {
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| Error::from_reqwest(&error))?;

        Ok(Self {
            http,
            url: config.rpc_url(),
            auth: config
                .basic_auth()
                .map(|(user, password)| (user.to_owned(), password.to_owned())),
            session_id: Mutex::new(None),
        })
    }

    /// The URL requests are sent to.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Whether requests carry HTTP basic auth.
    #[must_use]
    pub const fn authenticated(&self) -> bool {
        self.auth.is_some()
    }

    /// Call one RPC method and return the parsed response envelope.
    ///
    /// The `409` handshake is retried exactly once: a second `409` is reported
    /// to the caller as an HTTP error.
    pub async fn request(&self, method: &str, arguments: Value) -> Result<Value, Error> {
        let payload = json!({ "method": method, "arguments": arguments });

        let mut response = self.post(&payload).await?;
        if response.status() == StatusCode::CONFLICT {
            if let Some(session_id) = session_header(&response) {
                *self.session_id() = Some(session_id);
                response = self.post(&payload).await?;
            }
        }

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| Error::from_reqwest(&error))?;
        if !status.is_success() {
            return Err(Error::Http {
                status: status.as_u16(),
                body,
            });
        }
        serde_json::from_str(&body).map_err(|error| Error::Decode(error.to_string()))
    }

    /// Issue one POST carrying the cached session id and credentials.
    async fn post(&self, payload: &Value) -> Result<Response, Error> {
        let mut request = self.http.post(&self.url).json(payload);
        if let Some((user, password)) = &self.auth {
            request = request.basic_auth(user, Some(password));
        }
        let session_id = self.session_id().clone();
        if let Some(session_id) = session_id {
            request = request.header(SESSION_HEADER, session_id);
        }
        request
            .send()
            .await
            .map_err(|error| Error::from_reqwest(&error))
    }

    /// Borrow the cached session id, ignoring lock poisoning.
    ///
    /// Poisoning only means some earlier call panicked while holding the lock;
    /// the worst case is a stale id, which the `409` handshake refreshes. There
    /// is no reason to fail a call — or to panic — over it.
    fn session_id(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.session_id
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Read a response header as a string, if it is present and valid UTF-8.
fn session_header(response: &Response) -> Option<String> {
    response
        .headers()
        .get(SESSION_HEADER)?
        .to_str()
        .ok()
        .map(str::to_owned)
}
