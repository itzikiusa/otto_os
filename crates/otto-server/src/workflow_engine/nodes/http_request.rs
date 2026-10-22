//! The `http_request` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall { node, .. } = call;
    let p = &node.params;
    let method = p
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_uppercase();
    let url = p
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| otto_core::Error::Invalid("http node: missing url".into()))?;
    // SSRF guard + guarded redirects — every other outbound path in the
    // daemon checks; workflow nodes must not be the loophole to
    // loopback/private/metadata addresses.
    otto_netguard::check_url(url)
        .await
        .map_err(otto_core::Error::Upstream)?;
    // Guarded resolver pins the vetted address (no DNS rebinding).
    let client = otto_netguard::guarded_client_builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| otto_core::Error::Internal(e.to_string()))?;
    let mut rb = client.request(method.parse().unwrap_or(reqwest::Method::GET), url);
    if let Some(body) = p.get("body") {
        if !body.is_null() {
            rb = rb.json(body);
        }
    }
    let resp = rb
        .send()
        .await
        .map_err(|e| otto_core::Error::Upstream(format!("http: {e}")))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    Ok((
        json!({ "status": status, "body": body }),
        vec![format!("HTTP {status}")],
    ))
}
