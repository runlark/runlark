//! Interim browser hardening for the legacy REST transport (ADR 0001) until #73 removes it.

use crate::routes::error::api_error;
use axum::extract::Request;
use axum::http::header::{CONTENT_TYPE, HOST};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::IpAddr;

/// Refuses a foreign `Host` (DNS rebinding) with 403 and a POST without a JSON content type
/// (a CORS "simple" request) with 415, before any handler runs.
pub async fn reject_non_local(request: Request, next: Next) -> Response {
    if let Some(host) = request.headers().get(HOST)
        && !is_loopback_authority(host)
    {
        return api_error(
            StatusCode::FORBIDDEN,
            "forbidden_host",
            "the daemon serves only loopback Host names (127.0.0.1, localhost or [::1])",
        )
        .into_response();
    }
    if request.method() == Method::POST
        && !request
            .headers()
            .get(CONTENT_TYPE)
            .is_some_and(is_json_media_type)
    {
        return api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "json_body_required",
            "POST requests must carry a JSON body with Content-Type: application/json",
        )
        .into_response();
    }
    next.run(request).await
}

/// Returns whether `host` is `localhost` or a loopback IP literal; the daemon binds nothing else.
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn is_loopback_authority(value: &HeaderValue) -> bool {
    let Ok(authority) = value.to_str() else {
        return false;
    };
    let host = match authority.strip_prefix('[') {
        Some(bracketed) => bracketed.split_once(']').map(|(ip, _)| ip),
        None => Some(
            authority
                .split_once(':')
                .map_or(authority, |(name, _)| name),
        ),
    };
    host.is_some_and(is_loopback_host)
}

fn is_json_media_type(value: &HeaderValue) -> bool {
    value.to_str().is_ok_and(|content_type| {
        content_type
            .split(';')
            .next()
            .is_some_and(|essence| essence.trim().eq_ignore_ascii_case("application/json"))
    })
}

#[cfg(test)]
mod tests {
    use super::{is_json_media_type, is_loopback_authority, is_loopback_host};
    use axum::http::HeaderValue;

    #[test]
    fn loopback_hosts_are_recognized() {
        for host in ["127.0.0.1", "127.1.2.3", "::1", "localhost", "LocalHost"] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in [
            "0.0.0.0",
            "::",
            "192.0.2.1",
            "example.invalid",
            "localhost.example.invalid",
            "127.0.0.1.nip.io",
            "::ffff:127.0.0.1",
            "[::1]",
            "",
        ] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn host_header_port_is_ignored_and_brackets_are_required_for_ipv6() {
        for value in [
            "127.0.0.1:9876",
            "localhost",
            "LOCALHOST:1",
            "[::1]:9876",
            "[::1]",
        ] {
            assert!(
                is_loopback_authority(&HeaderValue::from_static(value)),
                "{value}"
            );
        }
        for value in [
            "::1",
            "[::1",
            "example.invalid:9876",
            "127.0.0.1.nip.io:9876",
            ":9876",
        ] {
            assert!(
                !is_loopback_authority(&HeaderValue::from_static(value)),
                "{value}"
            );
        }
    }

    #[test]
    fn only_application_json_counts_as_a_json_body() {
        for value in [
            "application/json",
            "Application/JSON",
            "application/json; charset=utf-8",
        ] {
            assert!(
                is_json_media_type(&HeaderValue::from_static(value)),
                "{value}"
            );
        }
        for value in [
            "text/plain",
            "application/x-www-form-urlencoded",
            "application/jsonx",
        ] {
            assert!(
                !is_json_media_type(&HeaderValue::from_static(value)),
                "{value}"
            );
        }
    }
}
