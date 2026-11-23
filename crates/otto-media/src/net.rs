//! SSRF-hardened outbound HTTP helpers backed by `otto-netguard`.
//!
//! Remote providers default to `otto_netguard::guarded_client_builder()` (blocks
//! loopback, RFC1918, CGNAT, link-local, cloud metadata). Local GPU / Kokoro
//! servers require explicit Admin opt-in (`allow_local_network = true`), which
//! uses `otto_netguard::allow_local_client_builder()` (always blocking cloud
//! metadata `169.254.169.254` and Otto's own listener port). Credentials over
//! non-loopback plain HTTP are always refused (`require_tls_or_loopback`).

use std::time::Duration;

use otto_core::{Error, Result};

/// Validate a configured provider endpoint URL before saving or calling it.
pub fn validate_endpoint_url(
    raw: &str,
    has_credentials: bool,
    allow_local_network: bool,
) -> Result<reqwest::Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid("endpoint_url cannot be empty".into()));
    }
    let parsed = reqwest::Url::parse(trimmed)
        .map_err(|e| Error::Invalid(format!("invalid endpoint_url: {e}")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(Error::Invalid(format!(
                "endpoint_url scheme '{other}' is not allowed (use http or https)"
            )));
        }
    }
    if has_credentials {
        otto_netguard::require_tls_or_loopback(trimmed).map_err(Error::Invalid)?;
    }
    let guard = otto_netguard::Guard::for_fetch(false, allow_local_network);
    if !guard.hop_allowed(&parsed) {
        return Err(Error::Invalid(format!(
            "endpoint_url '{}' is blocked by SSRF policy (enable allow_local_network for LAN/localhost servers)",
            parsed.as_str()
        )));
    }
    Ok(parsed)
}

/// Build an SSRF-guarded `reqwest::Client` respecting `allow_local_network`.
pub fn build_http_client(allow_local_network: bool, timeout: Duration) -> Result<reqwest::Client> {
    let builder = if allow_local_network {
        otto_netguard::allow_local_client_builder()
    } else {
        otto_netguard::guarded_client_builder()
    };
    builder
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(15))
        .user_agent("Otto-Media/1.0")
        .build()
        .map_err(|e| Error::Internal(format!("build guarded HTTP client: {e}")))
}

/// Read an HTTP response body up to `max_bytes`, aborting early if exceeded.
pub async fn read_bounded_bytes(resp: reqwest::Response, max_bytes: usize) -> Result<Vec<u8>> {
    if let Some(len) = resp.content_length() {
        if len as usize > max_bytes {
            return Err(Error::PayloadTooLarge(format!(
                "upstream response ({len} bytes) exceeds the {max_bytes}-byte limit"
            )));
        }
    }
    let mut out = Vec::new();
    let mut resp = resp;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| Error::Upstream(format!("reading upstream body: {e}")))?
    {
        if out.len().saturating_add(chunk.len()) > max_bytes {
            return Err(Error::PayloadTooLarge(format!(
                "upstream response exceeded the {max_bytes}-byte limit"
            )));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Attach an optional API key to a request builder using the configured header
/// (defaulting to `Authorization: Bearer <key>`).
pub fn apply_auth_header(
    mut req: reqwest::RequestBuilder,
    api_key: Option<&str>,
    auth_header: Option<&str>,
) -> reqwest::RequestBuilder {
    if let Some(key) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
        let header = auth_header
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .unwrap_or("Authorization");
        if header.eq_ignore_ascii_case("authorization") {
            let val = if key.to_ascii_lowercase().starts_with("bearer ") {
                key.to_string()
            } else {
                format!("Bearer {key}")
            };
            req = req.header("Authorization", val);
        } else {
            req = req.header(header, key);
        }
    }
    req
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_metadata_and_private_unless_opted_in() {
        assert!(
            validate_endpoint_url("http://169.254.169.254/latest/meta-data/", false, false)
                .is_err()
        );
        assert!(
            validate_endpoint_url("http://169.254.169.254/latest/meta-data/", false, true).is_err()
        );
        assert!(validate_endpoint_url("http://127.0.0.1:8080/generate", false, false).is_err());
        assert!(validate_endpoint_url("http://127.0.0.1:8080/generate", false, true).is_ok());
        assert!(validate_endpoint_url("http://127.0.0.1:8080/generate", true, true).is_ok());
        // Credentials over non-loopback plain HTTP are always rejected.
        assert!(validate_endpoint_url("http://192.168.1.50:8080/generate", true, true).is_err());
        assert!(validate_endpoint_url("https://api.example.com/v1/3d", true, false).is_ok());
        assert!(validate_endpoint_url("file:///etc/passwd", false, true).is_err());
    }
}
