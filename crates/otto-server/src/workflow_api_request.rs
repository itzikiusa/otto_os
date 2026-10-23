//! Workflow adapter for the existing API-client execution path.
use super::*;
use axum::http::{HeaderMap, HeaderValue};
use otto_core::api::ExecuteApiReq;

/// Preserve workflow's original object headers and JSON body while accepting
/// the API client's row headers and explicit body modes/settings as well.
fn request(params: &Value) -> Result<ExecuteApiReq> {
    let mut normalized = params
        .as_object()
        .cloned()
        .ok_or_else(|| otto_core::Error::Invalid("api_run: parameters must be an object".into()))?;
    normalized.entry("method").or_insert(json!("GET"));
    if let Some(Value::Object(headers)) = normalized.get("headers") {
        let rows: Vec<Value> = headers
            .iter()
            .filter_map(|(key, value)| value.as_str().map(|value| json!({"key":key,"value":value})))
            .collect();
        normalized.insert("headers".into(), json!(rows));
    }
    let body = normalized.remove("body").unwrap_or(Value::Null);
    let explicit_mode = normalized.contains_key("body_mode");
    let encoded = match body {
        Value::Null => String::new(),
        Value::String(text) if explicit_mode => text,
        value => value.to_string(),
    };
    normalized
        .entry("body_mode")
        .or_insert_with(|| json!(if encoded.is_empty() { "none" } else { "json" }));
    normalized.insert("body".into(), json!(encoded));
    serde_json::from_value(Value::Object(normalized))
        .map_err(|e| otto_core::Error::Invalid(format!("api_run: {e}")))
}

pub(super) async fn execute(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    params: &Value,
) -> Result<(Value, Vec<String>)> {
    let req = request(params)?;
    // This is an automated execution on behalf of the run user. Preserve that
    // attribution in history and the existing stored-secret host binding rules.
    const KIND: &str = "workflow";
    let mut headers = HeaderMap::new();
    headers.insert("x-otto-agent", HeaderValue::from_static(KIND));
    let auth = otto_core::auth::AuthContext::automation(user, KIND);
    let response = otto_apiclient::engine::execute_service(ctx, &ws.id, user, &auth, &headers, req)
        .await
        .map_err(otto_core::Error::from)?;
    let body: Value = serde_json::from_str(&response.body).unwrap_or(Value::String(response.body));
    let mut logs = vec![format!("api_run: HTTP {}", response.status)];
    if response.truncated || response.too_large {
        logs.push("api_run: response body exceeds the API client's inline size limit; inspect truncated/too_large before using it".into());
    }
    Ok((
        json!({"status":response.status,"body":body,
            "truncated":response.truncated,"too_large":response.too_large}),
        logs,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_and_api_client_request_shapes_keep_their_body_semantics() {
        let legacy = request(
            &json!({"url":"https://example.invalid","headers":{"X-Test":"yes"},"body":"text"}),
        )
        .unwrap();
        assert_eq!(legacy.method, "GET");
        assert_eq!(legacy.headers, json!([{"key":"X-Test","value":"yes"}]));
        assert_eq!(legacy.body, "\"text\"");
        assert_eq!(legacy.body_mode, "json");
        let explicit = request(&json!({"url":"https://example.invalid","body_mode":"raw","body":"text","auth":{"type":"bearer","token":"abc"}})).unwrap();
        assert_eq!(explicit.body, "text");
        assert_eq!(explicit.auth["token"], "abc");
        assert_eq!(
            request(&json!({"url":"https://example.invalid"}))
                .unwrap()
                .body_mode,
            "none"
        );
    }
}
