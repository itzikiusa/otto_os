//! API-client secret handling: Keychain-backed auth members and environment
//! variables (see docs/superpowers/specs/2026-07-04-api-client-durability-secrets-design.md §B).
//!
//! Rows keep structure, the Keychain keeps values. A secret member of an
//! `auth` object is replaced in SQLite by a `{"$secret": "<ref>"}` marker;
//! the value lives in one Keychain entry per request/environment:
//! `otto.api.request.<request_id>` / `otto.api.env.<env_id>`, each a flat JSON
//! map `member → value`. Markers resolve ONLY at execute time (here), never in
//! any export path — exporters read the stored rows and see markers.

use std::collections::BTreeMap;

use base64::Engine;
use chrono::{DateTime, Utc};
use otto_core::api::ApiResponse;
use otto_core::secrets::SecretStore;
use serde_json::{json, Map, Value};

/// Serialize the two-store request credential mutation across Save, secure-all,
/// delete and OAuth completion. Weak entries do not retain deleted request ids.
pub async fn request_guard(id: &str) -> tokio::sync::OwnedMutexGuard<()> {
    static LOCKS: otto_core::keyed_lock::KeyedLocks<String> =
        otto_core::keyed_lock::KeyedLocks::new();
    LOCKS.lock(id).await
}

/// Marker field name inside a `{"$secret": "<ref>"}` object.
pub const MARKER_KEY: &str = "$secret";
/// Mask used in every API-client shape that must not expose credentials.
pub const MASK: &str = "***";
/// Redaction placeholder written into history snapshots.
pub const REDACTED: &str = MASK;

pub fn request_ref(request_id: &str) -> String {
    format!("otto.api.request.{request_id}")
}

pub fn env_ref(env_id: &str) -> String {
    format!("otto.api.env.{env_id}")
}

/// The entity id inside a request ref, if `r` is one.
pub fn parse_request_ref(r: &str) -> Option<&str> {
    r.strip_prefix("otto.api.request.")
        .filter(|s| !s.is_empty())
}

/// Which members of an `auth` object are secrets, per auth type.
pub fn secret_members(auth_type: &str) -> &'static [&'static str] {
    match auth_type {
        "bearer" => &["token"],
        "basic" => &["password"],
        "api_key" => &["value"],
        "oauth2" => &["client_secret", "refresh_token", "password", "access_token"],
        _ => &[],
    }
}

/// `Some(ref)` when `v` is a `{"$secret": "<ref>"}` marker object.
pub fn marker_ref(v: &Value) -> Option<&str> {
    v.as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.get(MARKER_KEY))
        .and_then(Value::as_str)
}

fn make_marker(r: &str) -> Value {
    json!({ MARKER_KEY: r })
}

/// Split plaintext secret members out of `auth` for persistence (lazy
/// migration on save).
///
/// Returns `(auth_with_markers, blob)`:
/// - a plaintext non-empty secret member moves into `blob` and becomes a
///   marker referencing `own_ref`;
/// - an existing marker for `own_ref` keeps the member's previously stored
///   value (carried over from `existing_blob`); a marker referencing anything
///   else is rejected (prevents grafting another entity's ref onto this row);
/// - empty/absent members simply drop out of the blob.
///
/// An empty returned blob means the Keychain entry should be deleted.
pub fn split_auth_secrets(
    auth: &Value,
    own_ref: &str,
    existing_blob: &BTreeMap<String, String>,
) -> Result<(Value, BTreeMap<String, String>), String> {
    let Some(obj) = auth.as_object() else {
        return Ok((auth.clone(), BTreeMap::new()));
    };
    let auth_type = obj.get("type").and_then(Value::as_str).unwrap_or("none");
    let members = secret_members(auth_type);
    let mut out = obj.clone();
    let mut blob: BTreeMap<String, String> = BTreeMap::new();
    for member in members {
        match obj.get(*member) {
            Some(Value::String(s)) if !s.is_empty() => {
                blob.insert((*member).to_string(), s.clone());
                out.insert((*member).to_string(), make_marker(own_ref));
            }
            Some(v) => {
                if let Some(r) = marker_ref(v) {
                    if r != own_ref {
                        return Err(format!(
                            "auth member '{member}' references a foreign secret ref"
                        ));
                    }
                    match existing_blob.get(*member) {
                        // Marker kept → carry the stored value forward.
                        Some(prev) => {
                            blob.insert((*member).to_string(), prev.clone());
                        }
                        // Dangling marker (no stored value) → clear the field.
                        None => {
                            out.insert((*member).to_string(), Value::String(String::new()));
                        }
                    }
                }
            }
            None => {}
        }
    }
    Ok((Value::Object(out), blob))
}

/// Resolve every `{"$secret": …}` marker in `auth` in-memory, right before
/// send. `allowed_request_ids`: a request-ref marker must name one of these —
/// requests whose workspace the caller already verified (usually exactly
/// one); any other ref is rejected. Values come
/// from the Keychain blob; a missing member resolves to "".
pub fn resolve_auth_markers(
    secrets: &dyn SecretStore,
    auth: &mut Value,
    allowed_request_ids: &[String],
) -> Result<(), String> {
    let Some(obj) = auth.as_object_mut() else {
        return Ok(());
    };
    // Collect the fields to rewrite first (avoid aliasing the map borrow).
    let marked: Vec<(String, String)> = obj
        .iter()
        .filter_map(|(k, v)| marker_ref(v).map(|r| (k.clone(), r.to_string())))
        .collect();
    if marked.is_empty() {
        return Ok(());
    }
    // All markers in one auth object share one ref; load each blob once.
    let mut blobs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (field, r) in &marked {
        let rid = parse_request_ref(r)
            .ok_or_else(|| format!("auth member '{field}' has an unsupported secret ref"))?;
        if !allowed_request_ids.iter().any(|a| a == rid) {
            return Err(format!(
                "auth member '{field}' references a secret outside this workspace"
            ));
        }
        if !blobs.contains_key(r) {
            blobs.insert(r.clone(), load_blob(secrets, r));
        }
    }
    for (field, r) in marked {
        let value = blobs
            .get(&r)
            .and_then(|b| b.get(&field))
            .cloned()
            .unwrap_or_default();
        obj.insert(field, Value::String(value));
    }
    Ok(())
}

/// Redact secret members (plaintext AND markers) of an auth object for the
/// history snapshot. Non-object / typeless auth passes through unchanged.
pub fn redact_auth(auth: &Value) -> Value {
    let Some(obj) = auth.as_object() else {
        return auth.clone();
    };
    let auth_type = obj.get("type").and_then(Value::as_str).unwrap_or("none");
    let mut out = obj.clone();
    for member in secret_members(auth_type) {
        match obj.get(*member) {
            Some(Value::String(s)) if !s.is_empty() => {
                out.insert((*member).to_string(), Value::String(REDACTED.into()));
            }
            Some(v) if marker_ref(v).is_some() => {
                out.insert((*member).to_string(), Value::String(REDACTED.into()));
            }
            _ => {}
        }
    }
    Value::Object(out)
}

/// Heuristic used by `secure-all` to decide an environment variable is
/// secret-shaped by NAME (the value itself is never inspected).
pub fn secret_shaped(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.contains("token")
        || k.contains("secret")
        || k.contains("passw")
        || k.contains("apikey")
        || k.contains("api_key")
        || k.contains("api-key")
        || k.contains("authorization")
        || k.contains("credential")
}

/// Whether a header/query-row key commonly carries a credential.
pub fn sensitive_header_key(key: &str) -> bool {
    if secret_shaped(key) {
        return true;
    }
    matches!(
        key.to_ascii_lowercase().as_str(),
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
            | "x-auth-token"
    )
}

/// Mask values in `[{key,value,enabled}]` rows whose keys are secret-shaped.
pub fn mask_kv_rows(rows: &Value) -> Value {
    let Some(items) = rows.as_array() else {
        return rows.clone();
    };
    Value::Array(
        items
            .iter()
            .map(|item| {
                let mut item = item.clone();
                if item
                    .get("key")
                    .and_then(Value::as_str)
                    .is_some_and(sensitive_header_key)
                {
                    if let Some(obj) = item.as_object_mut() {
                        obj.insert("value".into(), Value::String(MASK.into()));
                    }
                }
                item
            })
            .collect(),
    )
}

/// Mask values in an environment-variable object whose keys are secret-shaped.
pub fn mask_variables(vars: &Value) -> Value {
    let Some(obj) = vars.as_object() else {
        return vars.clone();
    };
    Value::Object(
        obj.iter()
            .map(|(key, value)| {
                let value = if secret_shaped(key) {
                    Value::String(MASK.into())
                } else {
                    value.clone()
                };
                (key.clone(), value)
            })
            .collect(),
    )
}

/// Replace every resolved secret (four or more characters) in a string.
pub fn scrub_str(input: &str, secrets: &[String]) -> String {
    secrets
        .iter()
        .filter(|secret| secret.len() >= 4)
        .fold(input.to_string(), |out, secret| out.replace(secret, MASK))
}

/// Remove resolved secrets from every response echo channel.
/// Scrub every output field, including script labels/errors and nested metadata.
pub fn scrub_json(value: &mut Value, secrets: &[String]) {
    match value {
        Value::String(text) => *text = scrub_str(text, secrets),
        Value::Array(items) => items.iter_mut().for_each(|v| scrub_json(v, secrets)),
        Value::Object(map) => map.values_mut().for_each(|v| scrub_json(v, secrets)),
        _ => {}
    }
}

pub fn scrub_secrets(resp: &mut ApiResponse, secrets: &[String]) {
    resp.body = scrub_str(&resp.body, secrets);
    if let Some(headers) = resp.headers.as_array_mut() {
        for header in headers {
            let set_cookie = header
                .get("key")
                .and_then(Value::as_str)
                .is_some_and(|key| key.eq_ignore_ascii_case("set-cookie"));
            if let Some(obj) = header.as_object_mut() {
                if set_cookie {
                    obj.insert("value".into(), Value::String(MASK.into()));
                } else if let Some(Value::String(value)) = obj.get_mut("value") {
                    *value = scrub_str(value, secrets);
                }
            }
        }
    }
    for step in &mut resp.trace {
        step.detail = scrub_str(&step.detail, secrets);
    }
}

/// [`scrub_secrets`] on an already-serialized `ApiResponse` (the history
/// copy): same fields, same masking, no typed clone of the body first.
pub fn scrub_response_json(resp: &mut Value, secrets: &[String]) {
    let Some(obj) = resp.as_object_mut() else {
        return;
    };
    if let Some(Value::String(body)) = obj.get_mut("body") {
        *body = scrub_str(body, secrets);
    }
    if let Some(headers) = obj.get_mut("headers").and_then(Value::as_array_mut) {
        for header in headers {
            let set_cookie = header
                .get("key")
                .and_then(Value::as_str)
                .is_some_and(|key| key.eq_ignore_ascii_case("set-cookie"));
            if let Some(h) = header.as_object_mut() {
                if set_cookie {
                    h.insert("value".into(), Value::String(MASK.into()));
                } else if let Some(Value::String(value)) = h.get_mut("value") {
                    *value = scrub_str(value, secrets);
                }
            }
        }
    }
    if let Some(trace) = obj.get_mut("trace").and_then(Value::as_array_mut) {
        for step in trace {
            if let Some(Value::String(detail)) = step.get_mut("detail") {
                *detail = scrub_str(detail, secrets);
            }
        }
    }
}

fn jwt_parts(token: &str) -> Option<(&str, &str, &str)> {
    let mut parts = token.split('.');
    let header = parts.next()?;
    let payload = parts.next()?;
    let signature = parts.next()?;
    if parts.next().is_some()
        || !header.starts_with("eyJ")
        || [header, payload, signature].iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
    {
        return None;
    }
    Some((header, payload, signature))
}

fn safe_jwt_claims(token: &str) -> Option<Value> {
    let (_, payload, _) = jwt_parts(token)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(payload))
        .ok()?;
    let decoded: Value = serde_json::from_slice(&bytes).ok()?;
    let obj = decoded.as_object()?;
    let mut safe = Map::new();
    for key in ["exp", "iat", "nbf", "iss", "aud", "scope"] {
        if let Some(value) = obj.get(key) {
            safe.insert(key.into(), value.clone());
        }
    }
    if let Some(exp) = obj.get("exp").and_then(Value::as_i64) {
        if let Some(dt) = DateTime::<Utc>::from_timestamp(exp, 0) {
            safe.insert("exp_iso".into(), Value::String(dt.to_rfc3339()));
        }
        safe.insert(
            "expires_in_s".into(),
            Value::Number((exp - Utc::now().timestamp()).into()),
        );
    }
    Some(Value::Object(safe))
}

fn pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn walk_json_jwts(value: &Value, path: &str, found: &mut Map<String, Value>) {
    if found.len() >= 8 {
        return;
    }
    match value {
        Value::String(token) => {
            if let Some(claims) = safe_jwt_claims(token) {
                found.insert(path.to_string(), claims);
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk_json_jwts(item, &format!("{path}/{index}"), found);
                if found.len() >= 8 {
                    break;
                }
            }
        }
        Value::Object(obj) => {
            for (key, item) in obj {
                walk_json_jwts(item, &format!("{path}/{}", pointer_segment(key)), found);
                if found.len() >= 8 {
                    break;
                }
            }
        }
        _ => {}
    }
}

/// Decode a bounded, allow-listed view of JWT claims found in a response body.
pub fn jwt_claims(body: &str) -> Option<Value> {
    let mut found = Map::new();
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        walk_json_jwts(&value, "", &mut found);
    } else {
        let mut number = 0;
        for token in
            body.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
        {
            if let Some(claims) = safe_jwt_claims(token) {
                number += 1;
                found.insert(format!("text#{number}"), claims);
                if number == 8 {
                    break;
                }
            }
        }
    }
    (!found.is_empty()).then_some(Value::Object(found))
}

/// Replace every JWT-shaped token (header.payload.signature, header starting `eyJ`)
/// in `input` with [`MASK`]. Used after `jwt_claims` has captured the safe claims.
pub fn mask_jwts(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(pos) = rest.find("eyJ") {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
            .unwrap_or(tail.len());
        let candidate = &tail[..end];
        if jwt_parts(candidate).is_some() {
            out.push_str(MASK);
        } else {
            out.push_str(candidate);
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Read credentials for an operation that may replace them. Only an absent
/// entry means empty; unavailable or corrupt storage must abort the mutation.
/// Error text deliberately excludes the secret store's potentially sensitive
/// payload and serde's offending-value diagnostics.
#[allow(clippy::disallowed_methods)] // pure: only called over a `Prefetched` snapshot (no backend I/O) and in tests
pub fn load_blob_checked(
    secrets: &dyn SecretStore,
    r: &str,
) -> otto_core::Result<BTreeMap<String, String>> {
    let value = secrets.get(r).map_err(|_| otto_core::Error::Internal(
        "Could not read saved credentials from Keychain. Unlock it and retry without replacing credentials.".into()))?;
    match value {
        None => Ok(BTreeMap::new()),
        Some(value) => serde_json::from_str(&value).map_err(|_| otto_core::Error::Internal(
            "Saved credentials are not a valid secret map. Repair the Keychain entry before replacing credentials.".into())),
    }
}

/// Read a Keychain blob (`member → value`); absent/corrupt → empty.
#[allow(clippy::disallowed_methods)] // pure: only reached via resolve_auth_markers over a `Prefetched` snapshot
pub fn load_blob(secrets: &dyn SecretStore, r: &str) -> BTreeMap<String, String> {
    secrets
        .get(r)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Async wrappers — every request handler goes through these, so a Keychain
// miss (IPC, or an ACL dialog waiting on a person) runs on tokio's blocking
// pool instead of parking a runtime worker. Cache hits stay inline.
// ---------------------------------------------------------------------------

/// [`load_blob`] for async callers.
pub async fn load_blob_async(
    secrets: &std::sync::Arc<dyn SecretStore>,
    r: &str,
) -> BTreeMap<String, String> {
    otto_core::secrets::get_async(secrets, r)
        .await
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// [`load_blob_checked`] for async callers (same error texts).
pub async fn load_blob_checked_async(
    secrets: &std::sync::Arc<dyn SecretStore>,
    r: &str,
) -> otto_core::Result<BTreeMap<String, String>> {
    let value = otto_core::secrets::get_async(secrets, r).await;
    load_blob_checked(&Prefetched::one(r, value), r)
}

/// Write (or delete, when empty) a Keychain blob. The Keychain write runs on
/// the blocking pool via `otto_core::secrets::{put,delete}_async`.
pub async fn store_blob_async(
    secrets: &std::sync::Arc<dyn SecretStore>,
    r: &str,
    blob: &BTreeMap<String, String>,
) -> otto_core::Result<()> {
    if blob.is_empty() {
        otto_core::secrets::delete_async(secrets, r).await
    } else {
        let body = serde_json::to_string(blob)
            .map_err(|e| otto_core::Error::Internal(format!("secret blob serialize: {e}")))?;
        otto_core::secrets::put_async(secrets, r, &body).await
    }
}

/// [`resolve_auth_markers`] for async callers: every referenced blob is read
/// up front via `get_async`, then the (pure) resolver runs over that snapshot.
pub async fn resolve_auth_markers_async(
    secrets: &std::sync::Arc<dyn SecretStore>,
    auth: &mut Value,
    allowed_request_ids: &[String],
) -> Result<(), String> {
    let refs: Vec<String> = auth
        .as_object()
        .map(|o| {
            o.values()
                .filter_map(marker_ref)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mut snap = Prefetched::default();
    // Only prefetch refs the resolver will accept — a foreign ref is rejected
    // below without its value ever being read.
    for r in refs.into_iter().filter(|r| {
        parse_request_ref(r).is_some_and(|rid| allowed_request_ids.iter().any(|a| a == rid))
    }) {
        if let std::collections::btree_map::Entry::Vacant(slot) = snap.0.entry(r) {
            let v = otto_core::secrets::get_async(secrets, slot.key()).await;
            slot.insert(v);
        }
    }
    resolve_auth_markers(&snap, auth, allowed_request_ids)
}

/// Read-only snapshot of already-fetched secret reads (results included, so a
/// backend error surfaces exactly as the live store would report it). Lets the
/// pure sync helpers above run over values fetched asynchronously.
#[derive(Default)]
struct Prefetched(BTreeMap<String, otto_core::Result<Option<String>>>);

impl Prefetched {
    fn one(key: &str, value: otto_core::Result<Option<String>>) -> Self {
        Self(BTreeMap::from([(key.to_string(), value)]))
    }
}

impl SecretStore for Prefetched {
    fn put(&self, _key: &str, _value: &str) -> otto_core::Result<()> {
        Err(otto_core::Error::Internal(
            "prefetched secrets are read-only".into(),
        ))
    }
    fn get(&self, key: &str) -> otto_core::Result<Option<String>> {
        match self.0.get(key) {
            Some(Ok(v)) => Ok(v.clone()),
            Some(Err(_)) => Err(otto_core::Error::Internal(
                "secret store read failed".into(),
            )),
            None => Ok(None),
        }
    }
    fn delete(&self, _key: &str) -> otto_core::Result<()> {
        Err(otto_core::Error::Internal(
            "prefetched secrets are read-only".into(),
        ))
    }
}

/// Strip keys listed in `secret_keys` out of a variables object (the row must
/// never hold a secret value once its key is marked secret).
pub fn strip_secret_variables(variables: &Value, secret_keys: &[String]) -> Value {
    let Some(obj) = variables.as_object() else {
        return variables.clone();
    };
    let mut out = Map::new();
    for (k, v) in obj {
        if !secret_keys.iter().any(|s| s == k) {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tests: plain sync fs / process / secret store is fine
mod tests;
