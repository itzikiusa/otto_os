//! Unit tests for [`super`] (moved with `otto-server`'s `api_secrets`).

struct ReadFixture(Option<&'static str>, bool);
impl otto_core::secrets::SecretStore for ReadFixture {
    fn get(&self, _: &str) -> otto_core::Result<Option<String>> {
        if self.1 {
            Err(otto_core::Error::Internal(
                "fixture-private-credential".into(),
            ))
        } else {
            Ok(self.0.map(str::to_string))
        }
    }
    fn put(&self, _: &str, _: &str) -> otto_core::Result<()> {
        panic!("checked read must never write credentials")
    }
    fn delete(&self, _: &str) -> otto_core::Result<()> {
        panic!("checked read must never delete credentials")
    }
}
#[test]
fn checked_secret_read_distinguishes_missing_valid_unavailable_and_corrupt() {
    assert!(load_blob_checked(&ReadFixture(None, false), "own")
        .unwrap()
        .is_empty());
    let saved = load_blob_checked(
        &ReadFixture(Some(r#"{"refresh_token":"keep-me"}"#), false),
        "own",
    )
    .unwrap();
    assert_eq!(saved["refresh_token"], "keep-me");
    for fixture in [
        ReadFixture(None, true),
        ReadFixture(Some("fixture-private-credential"), false),
        ReadFixture(Some(r#"{"refresh_token":42}"#), false),
    ] {
        let error = load_blob_checked(&fixture, "own").unwrap_err().to_string();
        assert!(!error.contains("fixture-private-credential"));
        assert!(!error.contains("42"));
    }
}
use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::json;

fn response() -> ApiResponse {
    ApiResponse {
        status: 200,
        status_text: "OK".into(),
        headers: json!([]),
        body: String::new(),
        body_base64: String::new(),
        body_id: None,
        truncated: false,
        too_large: false,
        duration_ms: 1,
        size_bytes: 0,
        content_type: None,
        trace: Vec::new(),
    }
}

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

/// The async resolver matches the sync one, reads each allowed ref once
/// through `get_async`, and never reads a foreign ref's value; the async
/// blob helpers round-trip and keep the "unavailable store" error.
#[tokio::test]
async fn async_helpers_match_sync_and_skip_foreign_reads() {
    use otto_core::secrets::SecretStore;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Default)]
    struct Counting(
        std::sync::Mutex<BTreeMap<String, String>>,
        AtomicUsize,
        bool,
    );
    impl SecretStore for Counting {
        fn put(&self, k: &str, v: &str) -> otto_core::Result<()> {
            self.0.lock().unwrap().insert(k.into(), v.into());
            Ok(())
        }
        fn get(&self, k: &str) -> otto_core::Result<Option<String>> {
            self.1.fetch_add(1, Ordering::SeqCst);
            if self.2 {
                return Err(otto_core::Error::Internal("locked".into()));
            }
            Ok(self.0.lock().unwrap().get(k).cloned())
        }
        fn delete(&self, k: &str) -> otto_core::Result<()> {
            self.0.lock().unwrap().remove(k);
            Ok(())
        }
    }
    let inner = std::sync::Arc::new(Counting::default());
    let store: std::sync::Arc<dyn SecretStore> = inner.clone();
    let own = request_ref("r1");
    let blob = BTreeMap::from([
        ("token".to_string(), "tk".to_string()),
        ("password".to_string(), "pw".to_string()),
    ]);
    store_blob_async(&store, &own, &blob).await.unwrap();
    assert_eq!(load_blob_async(&store, &own).await, blob);
    assert_eq!(load_blob_checked_async(&store, &own).await.unwrap(), blob);

    inner.1.store(0, Ordering::SeqCst);
    let mut auth = json!({
        "type": "basic",
        "token": { MARKER_KEY: own.clone() },
        "password": { MARKER_KEY: own.clone() },
    });
    resolve_auth_markers_async(&store, &mut auth, &["r1".to_string()])
        .await
        .unwrap();
    assert_eq!(
        (auth["token"].as_str(), auth["password"].as_str()),
        (Some("tk"), Some("pw"))
    );
    assert_eq!(
        inner.1.load(Ordering::SeqCst),
        1,
        "one read per distinct ref"
    );

    inner.1.store(0, Ordering::SeqCst);
    let mut foreign = json!({"type": "bearer", "token": { MARKER_KEY: request_ref("r2") }});
    assert!(
        resolve_auth_markers_async(&store, &mut foreign, &["r1".to_string()])
            .await
            .is_err()
    );
    assert_eq!(inner.1.load(Ordering::SeqCst), 0, "foreign ref never read");

    // An unavailable store is an error for the checked loader, not "empty".
    let locked: std::sync::Arc<dyn SecretStore> =
        std::sync::Arc::new(Counting(Default::default(), AtomicUsize::new(0), true));
    assert!(load_blob_checked_async(&locked, &own).await.is_err());
    assert!(load_blob_async(&locked, &own).await.is_empty());
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

#[test]
fn scrub_secrets_masks_every_occurrence() {
    let mut resp = response();
    resp.body = "before long-secret after xyz".into();
    resp.headers = json!([{"key":"x-echo","value":"long-secret/xyz"}]);
    resp.trace.push(otto_core::api::TraceStep {
        label: "Request".into(),
        detail: "https://x.test/?token=long-secret&short=xyz".into(),
        ms: None,
        level: "info".into(),
    });

    scrub_secrets(&mut resp, &["long-secret".into(), "xyz".into()]);
    assert_eq!(resp.body, "before *** after xyz");
    assert_eq!(resp.headers[0]["value"], "***/xyz");
    assert_eq!(resp.trace[0].detail, "https://x.test/?token=***&short=xyz");
}

#[test]
fn set_cookie_always_masked() {
    let mut resp = response();
    resp.headers = json!([
        {"key":"Set-Cookie","value":"session=abc"},
        {"key":"content-type","value":"text/plain"}
    ]);
    scrub_secrets(&mut resp, &[]);
    assert_eq!(resp.headers[0]["value"], MASK);
    assert_eq!(resp.headers[1]["value"], "text/plain");
}

fn test_jwt(payload: &Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(payload).unwrap());
    format!("{header}.{payload}.signature")
}

#[test]
fn jwt_claims_decodes_exp_never_returns_token() {
    let token = test_jwt(&json!({
        "exp": 1_893_456_000_i64,
        "iat": 1_893_452_400_i64,
        "iss": "issuer",
        "sub": "private-subject",
        "email": "private@example.test"
    }));
    let claims = jwt_claims(&json!({"access_token": token}).to_string()).unwrap();
    assert_eq!(claims["/access_token"]["exp"], 1_893_456_000_i64);
    assert_eq!(
        claims["/access_token"]["exp_iso"],
        "2030-01-01T00:00:00+00:00"
    );
    let serialized = claims.to_string();
    assert!(!serialized.contains("eyJ"));
    assert!(!serialized.contains("private-subject"));
    assert!(!serialized.contains("private@example.test"));
}

#[test]
fn jwt_claims_scans_plain_text() {
    let first = test_jwt(&json!({"scope":"read"}));
    let second = test_jwt(&json!({"aud":["otto"]}));
    let claims = jwt_claims(&format!("first={first}; second: {second}")).unwrap();
    assert_eq!(claims["text#1"]["scope"], "read");
    assert_eq!(claims["text#2"]["aud"], json!(["otto"]));
}

#[test]
fn mask_jwts_replaces_tokens_and_keeps_other_text() {
    let token = test_jwt(&json!({"exp": 1_893_456_000_i64}));
    assert_eq!(
        mask_jwts(&format!("a={token}; b=eyJnot-a-jwt")),
        "a=***; b=eyJnot-a-jwt"
    );
}

#[test]
fn mask_kv_rows_masks_authorization_and_secret_shaped() {
    let rows = json!([
        {"key":"Authorization","value":"Bearer live","enabled":true},
        {"key":"client_secret","value":"secret","enabled":true},
        {"key":"Accept","value":"application/json","enabled":true}
    ]);
    let masked = mask_kv_rows(&rows);
    assert_eq!(masked[0]["value"], MASK);
    assert_eq!(masked[1]["value"], MASK);
    assert_eq!(masked[2]["value"], "application/json");
    assert_eq!(mask_kv_rows(&json!({"key":"x"})), json!({"key":"x"}));
}

#[test]
fn mask_variables_masks_secret_shaped_keys() {
    let masked = mask_variables(&json!({
        "base_url": "https://api.test",
        "api_token": "live-token",
        "passwordHint": "also-sensitive"
    }));
    assert_eq!(masked["base_url"], "https://api.test");
    assert_eq!(masked["api_token"], MASK);
    assert_eq!(masked["passwordHint"], MASK);
}
#[test]
fn scrubs_script_tests_warnings_and_nested_result_metadata() {
    let mut result = json!({"script_tests":[{"desc":"secret-abc echoed","passed":false}],
        "warnings":["failure: secret-abc"],"jwt_claims":{"nested":["secret-abc"]}});
    scrub_json(&mut result, &["secret-abc".into()]);
    assert!(!result.to_string().contains("secret-abc"));
    assert_eq!(result["script_tests"][0]["passed"], false);
}

/// perf N7: the in-place JSON scrub of the history copy redacts exactly
/// what the typed `scrub_secrets` does.
#[test]
fn scrub_response_json_matches_typed_scrub() {
    use otto_core::api::{ApiResponse, TraceStep};
    use serde_json::json;
    let secrets = vec!["s3cr3t-value".to_string()];
    let resp: ApiResponse = serde_json::from_value(json!({
        "status": 200, "status_text": "OK",
        "headers": [
            {"key": "X-Echo", "value": "Bearer s3cr3t-value"},
            {"key": "Set-Cookie", "value": "sid=abc"},
        ],
        "body": "{\"token\":\"s3cr3t-value\"}",
        "duration_ms": 3, "size_bytes": 10, "content_type": null,
    }))
    .unwrap();
    let mut resp = resp;
    resp.trace.push(TraceStep {
        label: "Request".into(),
        detail: "GET https://h/?k=s3cr3t-value".into(),
        ms: None,
        level: "info".into(),
    });
    let mut typed = resp.clone();
    super::scrub_secrets(&mut typed, &secrets);
    let mut json = serde_json::to_value(&resp).unwrap();
    super::scrub_response_json(&mut json, &secrets);
    assert_eq!(json, serde_json::to_value(&typed).unwrap());
    assert!(!json.to_string().contains("s3cr3t-value"));
    assert!(!json.to_string().contains("sid=abc"));
}
