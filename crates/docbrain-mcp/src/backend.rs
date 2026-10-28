// SPDX-License-Identifier: MIT
//! How a tool reaches DocBrain's REST API (#601).
//!
//! Every tool sends its requests through a [`Backend`]. The stdio binary uses [`HttpBackend`],
//! which calls the server over the network with the person's API key. DocBrain's own
//! `/api/v1/mcp` endpoint uses a backend that calls its router in process. Both doors therefore
//! run the same tool code and send the same requests, and the server's routes stay the only
//! authority on what a caller may do.

use serde_json::Value;
use std::future::Future;

pub use reqwest::Method;

use crate::{DOCBRAIN_CALLER_HEADER, DOCBRAIN_CALLER_VALUE};

/// A REST path made only of fixed text and checked ids, so no tool argument can steer a request
/// to another route (`..` segments, a second path, a query).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiPath(String);

impl ApiPath {
    /// A path with no variable part.
    pub fn fixed(path: &'static str) -> Self {
        Self(path.to_string())
    }

    /// `prefix` + `id` + `suffix`, where `id` must be a UUID. The error names what was expected,
    /// never the value, which came from a tool argument.
    pub fn with_id(prefix: &'static str, id: &str, suffix: &'static str, what: &str) -> Result<Self, String> {
        if is_uuid_shaped(id) {
            Ok(Self(format!("{prefix}{id}{suffix}")))
        } else {
            Err(format!("{what} must be an id"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One request a tool makes.
#[derive(Debug, Clone)]
pub struct ApiRequest {
    pub method: Method,
    pub path: ApiPath,
    /// Query parameters; each backend encodes them.
    pub query: Vec<(&'static str, String)>,
    pub body: Option<Value>,
    /// The call may outlast the usual client timeout: an ask that runs live tools is bounded by
    /// the server's own turn clock (#443), not by the client.
    pub long: bool,
}

impl ApiRequest {
    pub fn get(path: ApiPath) -> Self {
        Self { method: Method::GET, path, query: Vec::new(), body: None, long: false }
    }

    pub fn send_json(method: Method, path: ApiPath, body: Value) -> Self {
        Self { method, path, query: Vec::new(), body: Some(body), long: false }
    }

    pub fn post(path: ApiPath, body: Value) -> Self {
        Self::send_json(Method::POST, path, body)
    }

    pub fn query(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.query.push((name, value.into()));
        self
    }

    pub fn long(mut self) -> Self {
        self.long = true;
        self
    }

    /// The path with its query, form-encoded. Every backend sends exactly this, so both doors
    /// address the server identically.
    pub fn path_and_query(&self) -> Result<String, String> {
        if self.query.is_empty() {
            return Ok(self.path.as_str().to_string());
        }
        let query = serde_urlencoded::to_string(&self.query).map_err(|e| format!("Invalid query: {e}"))?;
        Ok(format!("{}?{}", self.path.as_str(), query))
    }
}

/// The server's answer: its status and body, whatever the status.
#[derive(Debug, Clone)]
pub struct ApiResponse {
    pub status: u16,
    pub body: String,
}

/// Sends a tool's request to DocBrain.
pub trait Backend: Send + Sync {
    /// `Err` only when DocBrain could not be reached or the answer could not be read; a refusal
    /// is an `ApiResponse` with its status.
    fn send(&self, request: ApiRequest) -> impl Future<Output = Result<ApiResponse, String>> + Send;
}

/// How long a request may take, unless it is marked [`ApiRequest::long`]: the stdio connector's
/// client timeout, and the bound the HTTP endpoint gives each in-process request.
pub const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// The stdio connector's backend: the network, with the person's API key.
pub struct HttpBackend {
    pub(crate) server_url: String,
    pub(crate) api_key: Option<String>,
    client: reqwest::Client,
    /// No client timeout, as the CLI's ask client (#443).
    long_client: reqwest::Client,
}

impl HttpBackend {
    pub fn new(server_url: String, api_key: Option<String>) -> Self {
        Self {
            server_url,
            api_key,
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .expect("Failed to build HTTP client"),
            long_client: reqwest::Client::new(),
        }
    }
}

impl Backend for HttpBackend {
    async fn send(&self, request: ApiRequest) -> Result<ApiResponse, String> {
        let client = if request.long { &self.long_client } else { &self.client };
        let url = format!("{}{}", self.server_url, request.path_and_query()?);
        // The caller header goes on before the key, so a call without a key still names its
        // surface when the server answers 401.
        let mut builder = client
            .request(request.method, url)
            .header(DOCBRAIN_CALLER_HEADER, DOCBRAIN_CALLER_VALUE);
        if let Some(body) = &request.body {
            builder = builder.json(body);
        }
        if let Some(key) = &self.api_key {
            builder = builder.header("Authorization", format!("Bearer {key}"));
        }
        let response = builder.send().await.map_err(|e| {
            if e.is_connect() {
                format!("{CANNOT_CONNECT}: {e}")
            } else {
                format!("API call failed: {e}")
            }
        })?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(|e| format!("Invalid response: {e}"))?;
        Ok(ApiResponse { status, body })
    }
}

/// How [`HttpBackend`] begins an error when no connection could be made, so the stdio binary's
/// startup check can say "is a DocBrain server running?" rather than a generic failure.
pub(crate) const CANNOT_CONNECT: &str = "Cannot connect";

/// Exactly a UUID's shape: 36 characters, hyphens at 8, 13, 18, 23, hex elsewhere.
pub(crate) fn is_uuid_shaped(candidate: &str) -> bool {
    let bytes = candidate.as_bytes();
    candidate.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}
