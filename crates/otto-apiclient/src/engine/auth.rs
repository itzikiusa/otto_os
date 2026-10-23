//! Request auth (`bearer` / `oauth2` / `basic` / `api_key`) onto headers or query.

use super::*;

/// Apply the `auth` object to headers/query. Shapes:
/// `{type:bearer, token}` → Authorization: Bearer …
/// `{type:basic, username, password}` → Authorization: Basic base64
/// `{type:api_key, in:"header"|"query", key, value}`
pub(crate) fn apply_auth(
    auth: &Value,
    vars: &serde_json::Map<String, Value>,
    headers: &mut reqwest::header::HeaderMap,
    query: &mut Vec<(String, String)>,
) -> Result<(), String> {
    use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};

    let kind = auth.get("type").and_then(Value::as_str).unwrap_or("none");
    match kind {
        "bearer" => {
            let token = substitute(
                auth.get("token").and_then(Value::as_str).unwrap_or(""),
                vars,
            );
            if !token.is_empty() {
                let hv = HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| "invalid bearer token".to_string())?;
                headers.insert(AUTHORIZATION, hv);
            }
        }
        "oauth2" => {
            // The token is fetched separately (POST .../oauth2/token) and stored
            // on the auth object; here we just attach it.
            let token = substitute(
                auth.get("access_token")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                vars,
            );
            let ttype = auth
                .get("token_type")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("Bearer");
            if !token.is_empty() {
                let hv = HeaderValue::from_str(&format!("{ttype} {token}"))
                    .map_err(|_| "invalid oauth2 token".to_string())?;
                headers.insert(AUTHORIZATION, hv);
            }
        }
        "basic" => {
            use base64::engine::general_purpose::STANDARD as B64;
            use base64::Engine;
            let user = substitute(
                auth.get("username").and_then(Value::as_str).unwrap_or(""),
                vars,
            );
            let pass = substitute(
                auth.get("password").and_then(Value::as_str).unwrap_or(""),
                vars,
            );
            // Both empty → send NO header. `Basic Og==` (base64 of ":") is a
            // real-but-empty credential some servers treat differently from
            // an anonymous request.
            if !user.is_empty() || !pass.is_empty() {
                let encoded = B64.encode(format!("{user}:{pass}"));
                let hv = HeaderValue::from_str(&format!("Basic {encoded}"))
                    .map_err(|_| "invalid basic auth".to_string())?;
                headers.insert(AUTHORIZATION, hv);
            }
        }
        "api_key" => {
            let key = substitute(auth.get("key").and_then(Value::as_str).unwrap_or(""), vars);
            let value = substitute(
                auth.get("value").and_then(Value::as_str).unwrap_or(""),
                vars,
            );
            if key.is_empty() {
                return Ok(());
            }
            let location = auth.get("in").and_then(Value::as_str).unwrap_or("header");
            if location == "query" {
                query.push((key, value));
            } else {
                let hn = HeaderName::from_bytes(key.as_bytes())
                    .map_err(|_| format!("invalid api_key header name '{key}'"))?;
                let hv = HeaderValue::from_str(&value)
                    .map_err(|_| "invalid api_key header value".to_string())?;
                headers.insert(hn, hv);
            }
        }
        _ => {}
    }
    Ok(())
}
