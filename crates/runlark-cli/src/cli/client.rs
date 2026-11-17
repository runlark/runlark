use reqwest::{Client, Method, StatusCode};
use serde::Serialize;
use serde_json::Value;
use std::fmt;

/// Error returned by [`DaemonHttpClient`] when an HTTP request or daemon response fails.
#[derive(Debug)]
pub struct CliHttpError {
    pub status: Option<StatusCode>,
    pub message: String,
}

impl fmt::Display for CliHttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(code) = self.status {
            write!(f, "daemon error ({code}): {}", self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for CliHttpError {}

/// Thin HTTP client used by `runlark` CLI subcommands to talk to the local Runlark daemon.
#[derive(Clone)]
pub struct DaemonHttpClient {
    base_url: String,
    http: Client,
}

impl DaemonHttpClient {
    /// Creates a new [`DaemonHttpClient`] targeting `base_url`.
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http: Client::new(),
        }
    }

    /// Performs a `GET` request returning parsed JSON.
    ///
    /// # Errors
    /// Returns [`CliHttpError`] on transport, status, or JSON decoding errors.
    pub async fn get_json(&self, path: &str) -> Result<Value, CliHttpError> {
        self.request_json(Method::GET, path, None, Option::<&Value>::None)
            .await
    }

    /// Performs a `GET` request with percent-encoded query parameters returning parsed JSON.
    ///
    /// # Errors
    /// Returns [`CliHttpError`] on transport, status, or JSON decoding errors.
    pub async fn get_json_query(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Value, CliHttpError> {
        self.request_json(Method::GET, path, Some(query), Option::<&Value>::None)
            .await
    }

    /// Performs a `POST` request with an optional JSON body returning parsed JSON.
    ///
    /// # Errors
    /// Returns [`CliHttpError`] on transport, status, or JSON decoding errors.
    pub async fn post_json<T: Serialize>(
        &self,
        path: &str,
        body: Option<&T>,
    ) -> Result<Value, CliHttpError> {
        self.request_json(Method::POST, path, None, body).await
    }

    async fn request_json<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        query: Option<&[(&str, String)]>,
        body: Option<&T>,
    ) -> Result<Value, CliHttpError> {
        let url = format!("{}{path}", self.base_url);
        let mut req = self.http.request(method, &url);
        if let Some(params) = query
            && !params.is_empty()
        {
            req = req.query(params);
        }
        if let Some(payload) = body {
            req = req.json(payload);
        }
        let resp = req.send().await.map_err(|err| CliHttpError {
            status: None,
            message: format!("request to {url} failed: {err}"),
        })?;
        let status = resp.status();
        let text = resp.text().await.map_err(|err| CliHttpError {
            status: Some(status),
            message: format!("failed to read response body: {err}"),
        })?;

        if !status.is_success() {
            let parsed_msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_string))
                .unwrap_or(text);
            return Err(CliHttpError {
                status: Some(status),
                message: parsed_msg,
            });
        }

        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|err| CliHttpError {
            status: Some(status),
            message: format!("invalid JSON from daemon: {err}"),
        })
    }
}

/// Prints a JSON value either prettily (`--json`) or via a concise formatter.
///
/// # Errors
/// Returns an error if pretty serialization fails.
pub fn print_output(
    value: &Value,
    json_mode: bool,
    human_summary: impl FnOnce(&Value) -> String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if json_mode {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{}", human_summary(value));
    }
    Ok(())
}
