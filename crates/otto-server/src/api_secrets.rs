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

use otto_core::secrets::SecretStore;
use serde_json::{json, Map, Value};

/// Marker field name inside a `{"$secret": "<ref>"}` object.
pub const MARKER_KEY: &str = "$secret";
/// Redaction placeholder written into history snapshots.
pub const REDACTED: &str = "***";

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

/// The entity id inside an environment ref, if `r` is one.
pub fn parse_env_ref(r: &str) -> Option<&str> {
    r.strip_prefix("otto.api.env.").filter(|s| !s.is_empty())
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
/// send. `expect_request_id`: when the marker is a request ref it must point
/// at a request whose workspace was already verified by the caller — the
/// caller passes the set of legitimate ids (usually exactly one). Values come
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

/// Read a Keychain blob (`member → value`); absent/corrupt → empty.
pub fn load_blob(secrets: &dyn SecretStore, r: &str) -> BTreeMap<String, String> {
    secrets
        .get(r)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Write (or delete, when empty) a Keychain blob.
pub fn store_blob(
    secrets: &dyn SecretStore,
    r: &str,
    blob: &BTreeMap<String, String>,
) -> otto_core::Result<()> {
    if blob.is_empty() {
        secrets.delete(r)
    } else {
        let body = serde_json::to_string(blob)
            .map_err(|e| otto_core::Error::Internal(format!("secret blob serialize: {e}")))?;
        secrets.put(r, &body)
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn split_moves_plaintext_and_keeps_markers() {
        let own = request_ref("r1");
        let auth = json!({"type": "bearer", "token": "s3cret"});
        let (row, blob) = split_auth_secrets(&auth, &own, &BTreeMap::new()).unwrap();
        assert_eq!(row["token"], json!({ MARKER_KEY: own.clone() }));
        assert_eq!(blob.get("token").unwrap(), "s3cret");

        // Round 2: the UI sends the marker back unchanged — value carried over.
        let (row2, blob2) = split_auth_secrets(&row, &own, &blob).unwrap();
        assert_eq!(row2["token"], json!({ MARKER_KEY: own.clone() }));
        assert_eq!(blob2.get("token").unwrap(), "s3cret");

        // A foreign ref is rejected outright.
        let evil = json!({"type": "bearer", "token": { MARKER_KEY: request_ref("other") }});
        assert!(split_auth_secrets(&evil, &own, &blob).is_err());
    }

    #[test]
    fn split_drops_stale_members_on_type_change() {
        let own = request_ref("r1");
        let mut existing = BTreeMap::new();
        existing.insert("token".to_string(), "old".to_string());
        // Switched bearer → basic: only `password` is secret now.
        let auth = json!({"type": "basic", "username": "u", "password": "pw"});
        let (row, blob) = split_auth_secrets(&auth, &own, &existing).unwrap();
        assert_eq!(blob.len(), 1);
        assert_eq!(blob.get("password").unwrap(), "pw");
        assert_eq!(row["username"], "u");
    }

    #[test]
    fn redact_covers_plaintext_and_markers() {
        let own = request_ref("r1");
        let auth = json!({
            "type": "oauth2",
            "client_id": "public",
            "client_secret": "sh",
            "refresh_token": { MARKER_KEY: own },
            "access_token": "",
        });
        let red = redact_auth(&auth);
        assert_eq!(red["client_secret"], REDACTED);
        assert_eq!(red["refresh_token"], REDACTED);
        assert_eq!(red["client_id"], "public"); // non-secret member untouched
        assert_eq!(red["access_token"], ""); // empty stays empty
    }

    #[test]
    fn resolve_rejects_foreign_and_resolves_own() {
        use otto_core::secrets::SecretStore;
        struct Mem(std::sync::Mutex<BTreeMap<String, String>>);
        impl SecretStore for Mem {
            fn put(&self, k: &str, v: &str) -> otto_core::Result<()> {
                self.0.lock().unwrap().insert(k.into(), v.into());
                Ok(())
            }
            fn get(&self, k: &str) -> otto_core::Result<Option<String>> {
                Ok(self.0.lock().unwrap().get(k).cloned())
            }
            fn delete(&self, k: &str) -> otto_core::Result<()> {
                self.0.lock().unwrap().remove(k);
                Ok(())
            }
        }
        let store = Mem(std::sync::Mutex::new(BTreeMap::new()));
        let own = request_ref("r1");
        store
            .put(
                &own,
                &serde_json::to_string(&BTreeMap::from([("token".to_string(), "tk".to_string())]))
                    .unwrap(),
            )
            .unwrap();

        let mut auth = json!({"type": "bearer", "token": { MARKER_KEY: own }});
        resolve_auth_markers(&store, &mut auth, &["r1".to_string()]).unwrap();
        assert_eq!(auth["token"], "tk");

        let mut foreign = json!({"type": "bearer", "token": { MARKER_KEY: request_ref("r2") }});
        assert!(resolve_auth_markers(&store, &mut foreign, &["r1".to_string()]).is_err());
    }

    #[test]
    fn secret_shaped_names() {
        for k in [
            "API_TOKEN",
            "clientSecret",
            "db_password",
            "x-api-key",
            "Authorization",
        ] {
            assert!(secret_shaped(k), "{k}");
        }
        for k in ["base_url", "region", "user"] {
            assert!(!secret_shaped(k), "{k}");
        }
    }

    #[test]
    fn strip_secret_vars() {
        let vars = json!({"base": "https://x", "api_token": "t"});
        let out = strip_secret_variables(&vars, &["api_token".to_string()]);
        assert_eq!(out, json!({"base": "https://x"}));
    }
}
