//! Native (in-process) AWS calls for the few high-frequency paths (F2d): the
//! CloudWatch Logs tail (`FilterLogEvents`), Logs Insights / Athena status
//! polling (`GetQueryResults`, `GetQueryExecution`) and EC2 lists
//! (`DescribeInstances`). Everything else stays on the `aws` CLI.
//!
//! Why not the `aws-sdk-*` crates: `aws-config` + `aws-sdk-cloudwatchlogs` +
//! `aws-sdk-athena` + `aws-sdk-ec2` pull in the smithy runtime stack (~120 new
//! crates; `aws-sdk-ec2` alone is a multi-minute compile and several MB of
//! binary) for four operations. These calls are plain SigV4-signed POSTs, so
//! this is a ~300-line client over crates the workspace already builds
//! (`reqwest` with its pooled keep-alive connections, `ring` for SHA-256 /
//! HMAC, `roxmltree` for EC2's XML).
//!
//! Credentials are the ones the CLI path would hand its child — the cached
//! exported SSO/profile creds, Keychain access keys or the cached assumed
//! role ([`AwsService::env_for`](crate::accounts::AwsService::env_for)). When
//! there are no static creds (a `credential_process` profile whose export
//! failed), a custom endpoint is configured, or the region has no standard
//! endpoint, the caller falls back to the CLI. Error bodies are rendered in
//! the CLI's `An error occurred (Code) when calling the Op operation: …` shape
//! and mapped through [`cli::error_for`], so expiry / denial classify exactly
//! as they do for a CLI call.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use otto_core::{Error, Result};
use ring::{digest, hmac};
use serde_json::{json, Value};

use crate::cli;

/// Per-request budget (the CLI path's default is 30 s; a native call that is
/// still waiting after 20 s is not going to answer a tail tick usefully).
pub const NATIVE_TIMEOUT: Duration = Duration::from_secs(20);
/// Page tokens minted by the native path carry this prefix, so a token from
/// one path is never fed to the other (the CLI's `--starting-token` is its own
/// base64 envelope, not the API's `nextToken`).
pub const TOKEN_PREFIX: &str = "n1:";

/// Whether native calls are used, and where they go.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Mode {
    /// The real AWS endpoints (default).
    #[default]
    Aws,
    /// Never — every call uses the CLI (`OTTO_AWS_NATIVE=off`).
    Off,
    /// Every service at this base URL (tests: a local mock server).
    Endpoint(String),
}

impl Mode {
    /// `OTTO_AWS_NATIVE=off` disables the native path (escape hatch).
    pub fn from_env() -> Self {
        match std::env::var("OTTO_AWS_NATIVE").as_deref() {
            Ok("off" | "0" | "false") => Mode::Off,
            _ => Mode::Aws,
        }
    }
}

/// Static credentials + region for one signed call.
#[derive(Clone)]
pub struct Target {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
    pub region: String,
    /// `Some(base)` overrides `https://{service}.{region}.amazonaws.com`.
    pub endpoint: Option<String>,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target")
            .field("region", &self.region)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// A native target from the env the CLI child would get, or `None` when the
/// call must go through the CLI (no static keys, a custom endpoint, or a
/// region without a standard endpoint).
pub fn target_from_env(mode: &Mode, env: &[(String, String)]) -> Option<Target> {
    let get = |k: &str| {
        env.iter()
            .rev()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    let endpoint = match mode {
        Mode::Off => return None,
        Mode::Aws => None,
        Mode::Endpoint(u) => Some(u.trim_end_matches('/').to_string()),
    };
    // A custom endpoint or CA bundle is CLI configuration this client does not
    // replicate.
    if get("AWS_ENDPOINT_URL").is_some() || get("AWS_CA_BUNDLE").is_some() {
        return None;
    }
    let region = get("AWS_REGION")?;
    if !valid_region(&region) {
        return None;
    }
    Some(Target {
        access_key_id: get("AWS_ACCESS_KEY_ID")?,
        secret_access_key: get("AWS_SECRET_ACCESS_KEY")?,
        session_token: get("AWS_SESSION_TOKEN"),
        region,
        endpoint,
    })
}

/// `eu-west-1`-shaped, and a partition this client knows the host of.
fn valid_region(r: &str) -> bool {
    !r.is_empty()
        && r.len() <= 32
        && r.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !r.starts_with("us-iso")
        && !r.starts_with("eu-isoe")
}

fn host_for(service: &str, region: &str) -> String {
    let suffix = if region.starts_with("cn-") {
        "amazonaws.com.cn"
    } else {
        "amazonaws.com"
    };
    format!("{service}.{region}.{suffix}")
}

/// Marks a transport-level failure (TLS, proxy, DNS, a non-AWS answer) as
/// opposed to an AWS API error. [`fallback`] turns it into "use the CLI".
const TRANSPORT: &str = "native transport: ";
/// After a transport failure the native path stays off this long — e.g. a
/// corporate TLS-inspecting proxy whose CA only the CLI's `ca_bundle` knows.
const TRANSPORT_BACKOFF: Duration = Duration::from_secs(300);

type FailedAt = std::sync::Mutex<std::collections::HashMap<String, Instant>>;

/// Endpoint URL → when it last failed below the API (bounded: one entry per
/// service × region in use).
fn transport_failed_at() -> &'static FailedAt {
    static T: OnceLock<FailedAt> = OnceLock::new();
    T.get_or_init(Default::default)
}

fn transport_error(url: &str, msg: String) -> Error {
    transport_failed_at()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(url.to_string(), Instant::now());
    tracing::debug!(target: "otto_aws::native", "falling back to the CLI: {msg}");
    Error::Upstream(format!("{TRANSPORT}{msg}"))
}

/// `true` while a recent transport failure keeps `url` on the CLI.
fn transport_backoff(url: &str) -> bool {
    transport_failed_at()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(url)
        .is_some_and(|at| at.elapsed() < TRANSPORT_BACKOFF)
}

/// A native result, or `Ok(None)` when the call never reached AWS (the caller
/// then repeats it through the CLI, which may know a proxy / CA we don't).
pub fn fallback<T>(r: Result<T>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::Upstream(m)) if m.starts_with(TRANSPORT) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Native calls made since start (exposed on `/aws/status` → `cli`).
static NATIVE_CALLS: AtomicU64 = AtomicU64::new(0);

pub fn calls_total() -> u64 {
    NATIVE_CALLS.load(Ordering::Relaxed)
}

fn client() -> &'static reqwest::Client {
    // One pooled client: keep-alive TLS connections are what make a 2 s tail
    // tick ~one round trip instead of a Python start-up + TLS handshake.
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(NATIVE_TIMEOUT)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .unwrap_or_default()
    })
}

// ---------------------------------------------------------------------------
// SigV4
// ---------------------------------------------------------------------------

fn sha256_hex(b: &[u8]) -> String {
    hex(digest::digest(&digest::SHA256, b).as_ref())
}

fn hex(b: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for &x in b {
        s.push(H[(x >> 4) as usize] as char);
        s.push(H[(x & 15) as usize] as char);
    }
    s
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), msg)
        .as_ref()
        .to_vec()
}

/// SigV4 `Authorization` header for a request with an empty query string.
/// `headers` must hold every header to sign (lower-case names), including
/// `host` and `x-amz-date`.
#[allow(clippy::too_many_arguments)]
pub fn authorization(
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
    access_key_id: &str,
    secret_access_key: &str,
    region: &str,
    service: &str,
    now: DateTime<Utc>,
) -> String {
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date = &amz_date[..8];
    let mut hs: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    hs.sort();
    let canonical_headers: String = hs.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed_headers = hs
        .iter()
        .map(|(k, _)| k.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_request = format!(
        "{method}\n{path}\n\n{canonical_headers}\n{signed_headers}\n{}",
        sha256_hex(body)
    );
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let k_date = hmac_sha256(
        format!("AWS4{secret_access_key}").as_bytes(),
        date.as_bytes(),
    );
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let signature = hex(&hmac_sha256(&k_signing, string_to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={access_key_id}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    )
}

/// One signed POST to `/`; returns `(status, body, url)`.
async fn post(
    t: &Target,
    service: &str,
    content_type: &str,
    target_header: Option<&str>,
    body: Vec<u8>,
) -> Result<(u16, String, String)> {
    let host = host_for(service, &t.region);
    let url = match &t.endpoint {
        Some(base) => format!("{base}/"),
        None => format!("https://{host}/"),
    };
    if transport_backoff(&url) {
        return Err(Error::Upstream(format!(
            "{TRANSPORT}{service}: recent failure"
        )));
    }
    let now = Utc::now();
    let mut headers: Vec<(String, String)> = vec![
        ("host".into(), host),
        ("content-type".into(), content_type.into()),
        (
            "x-amz-date".into(),
            now.format("%Y%m%dT%H%M%SZ").to_string(),
        ),
    ];
    if let Some(tok) = &t.session_token {
        headers.push(("x-amz-security-token".into(), tok.clone()));
    }
    if let Some(th) = target_header {
        headers.push(("x-amz-target".into(), th.into()));
    }
    let auth = authorization(
        "POST",
        "/",
        &headers,
        &body,
        &t.access_key_id,
        &t.secret_access_key,
        &t.region,
        service,
        now,
    );
    let mut req = client().post(&url).header("authorization", auth);
    for (k, v) in &headers {
        // reqwest derives Host from the URL; with an endpoint override the
        // signed host differs from the URL's, so send the signed one.
        req = req.header(k.as_str(), v.as_str());
    }
    let started = Instant::now();
    NATIVE_CALLS.fetch_add(1, Ordering::Relaxed);
    let resp = req
        .body(body)
        .send()
        .await
        .map_err(|e| transport_error(&url, format!("{service}: {}", e.without_url())))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| transport_error(&url, format!("{service}: {}", e.without_url())))?;
    tracing::debug!(target: "otto_aws::native", service, ms = started.elapsed().as_millis() as u64, status, "aws native call");
    Ok((status, text, url))
}

/// Render a failed call the way the CLI prints it, then classify it with the
/// CLI's rules (expiry → `login required:`, denial → Forbidden).
fn api_error(status: u16, code: &str, message: &str, op: &str) -> Error {
    let code = code.rsplit('#').next().unwrap_or(code);
    let stderr = if code.is_empty() {
        format!("An error occurred (HTTP {status}) when calling the {op} operation: {message}")
    } else {
        format!("An error occurred ({code}) when calling the {op} operation: {message}")
    };
    cli::error_for(&cli::CliOutput {
        status: 254,
        stdout: String::new(),
        stderr,
        duration_ms: 0,
    })
}

/// AWS JSON 1.1 protocol call (`x-amz-target: {prefix}.{op}`).
pub async fn json_call(
    t: &Target,
    service: &str,
    target_prefix: &str,
    op: &str,
    input: &Value,
) -> Result<Value> {
    let body = serde_json::to_vec(input)?;
    let th = format!("{target_prefix}.{op}");
    let (status, text, url) =
        post(t, service, "application/x-amz-json-1.1", Some(&th), body).await?;
    let v: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        // AWS always answers JSON on this protocol; anything else came from a
        // proxy / captive portal in between.
        Err(_) if (200..300).contains(&status) || status == 407 => {
            return Err(transport_error(
                &url,
                format!("{service}: non-JSON answer (HTTP {status})"),
            ))
        }
        Err(_) => Value::Null,
    };
    if (200..300).contains(&status) {
        return Ok(v);
    }
    let code = v
        .get("__type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let message = v
        .get("message")
        .or_else(|| v.get("Message"))
        .and_then(Value::as_str)
        .unwrap_or(text.as_str());
    Err(api_error(status, &code, message, op))
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// Strip [`TOKEN_PREFIX`]; `None` when the token is not a native one.
pub fn native_token(t: &str) -> Option<&str> {
    t.strip_prefix(TOKEN_PREFIX)
}

/// The CLI path got a token only the native path can resume.
pub fn foreign_token_error() -> Error {
    Error::Invalid("this page token has expired — reload the list".into())
}

/// `FilterLogEvents`, collecting up to `max` events across API pages the way
/// `--max-items` does (a filtered search can return empty pages with a
/// `nextToken`). Returns the CLI-shaped `{ events, NextToken }`.
pub async fn filter_log_events(
    t: &Target,
    mut input: serde_json::Map<String, Value>,
    max: usize,
) -> Result<Value> {
    const MAX_PAGES: usize = 10;
    let mut events: Vec<Value> = Vec::new();
    let mut next: Option<String> = None;
    for _ in 0..MAX_PAGES {
        input.insert("limit".into(), json!((max - events.len()).clamp(1, 10_000)));
        let v = json_call(
            t,
            "logs",
            "Logs_20140328",
            "FilterLogEvents",
            &Value::Object(input.clone()),
        )
        .await?;
        if let Some(arr) = v.get("events").and_then(Value::as_array) {
            events.extend(arr.iter().cloned());
        }
        next = v
            .get("nextToken")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        match &next {
            Some(tok) if events.len() < max => {
                input.insert("nextToken".into(), json!(tok));
            }
            _ => break,
        }
    }
    events.truncate(max);
    let mut out = json!({ "events": events });
    if let Some(tok) = next {
        out["NextToken"] = json!(format!("{TOKEN_PREFIX}{tok}"));
    }
    Ok(out)
}

/// `DescribeInstances` (EC2 query protocol, XML), all pages, as the CLI's
/// `{ Reservations: [{ Instances: [...] }] }` JSON.
pub async fn describe_instances(t: &Target, state: Option<&str>, ids: &[&str]) -> Result<Value> {
    const MAX_PAGES: usize = 20;
    let mut reservations: Vec<Value> = Vec::new();
    let mut token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut form: Vec<(String, String)> = vec![
            ("Action".into(), "DescribeInstances".into()),
            ("Version".into(), "2016-11-15".into()),
        ];
        if ids.is_empty() {
            form.push(("MaxResults".into(), "1000".into()));
        }
        for (n, id) in ids.iter().enumerate() {
            form.push((format!("InstanceId.{}", n + 1), (*id).to_string()));
        }
        if let Some(st) = state {
            form.push(("Filter.1.Name".into(), "instance-state-name".into()));
            form.push(("Filter.1.Value.1".into(), st.into()));
        }
        if let Some(tok) = &token {
            form.push(("NextToken".into(), tok.clone()));
        }
        let body = form
            .iter()
            .map(|(k, v)| format!("{}={}", form_enc(k), form_enc(v)))
            .collect::<Vec<_>>()
            .join("&");
        let (status, text, url) = post(
            t,
            "ec2",
            "application/x-www-form-urlencoded; charset=utf-8",
            None,
            body.into_bytes(),
        )
        .await?;
        // Not XML at all: something between us and AWS answered (a proxy page).
        let doc = roxmltree::Document::parse(&text)
            .map_err(|e| transport_error(&url, format!("ec2: unreadable response ({e})")))?;
        if !(200..300).contains(&status) {
            let err = doc.descendants().find(|n| n.has_tag_name("Error"));
            let field = |name: &str| {
                err.and_then(|e| child(e, name))
                    .and_then(|n| n.text())
                    .unwrap_or_default()
                    .to_string()
            };
            return Err(api_error(
                status,
                &field("Code"),
                &field("Message"),
                "DescribeInstances",
            ));
        }
        let root = doc.root_element();
        if let Some(set) = child(root, "reservationSet") {
            reservations.extend(items(set).map(|r| {
                let instances: Vec<Value> = child(r, "instancesSet")
                    .map(|s| items(s).map(instance_json).collect())
                    .unwrap_or_default();
                json!({ "Instances": instances })
            }));
        }
        token = child(root, "nextToken")
            .and_then(|n| n.text())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if token.is_none() {
            break;
        }
    }
    Ok(json!({ "Reservations": reservations }))
}

fn form_enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn child<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    n.children().find(|c| c.has_tag_name(name))
}

fn items<'a>(n: roxmltree::Node<'a, 'a>) -> impl Iterator<Item = roxmltree::Node<'a, 'a>> {
    n.children().filter(|c| c.has_tag_name("item"))
}

fn text_of(n: roxmltree::Node, name: &str) -> Option<String> {
    child(n, name).and_then(|c| c.text()).map(str::to_string)
}

/// One EC2 `<item>` → the CLI's JSON keys that `ec2::normalize_instance` reads.
fn instance_json(i: roxmltree::Node) -> Value {
    let mut o = serde_json::Map::new();
    let mut put = |k: &str, v: Option<String>| {
        if let Some(v) = v {
            o.insert(k.into(), Value::String(v));
        }
    };
    put("InstanceId", text_of(i, "instanceId"));
    put("InstanceType", text_of(i, "instanceType"));
    put("PrivateIpAddress", text_of(i, "privateIpAddress"));
    put("PublicIpAddress", text_of(i, "ipAddress"));
    put("LaunchTime", text_of(i, "launchTime"));
    put("PlatformDetails", text_of(i, "platformDetails"));
    put("Platform", text_of(i, "platform"));
    put("VpcId", text_of(i, "vpcId"));
    put("SubnetId", text_of(i, "subnetId"));
    if let Some(st) = child(i, "instanceState") {
        o.insert("State".into(), json!({ "Name": text_of(st, "name") }));
    }
    if let Some(p) = child(i, "placement") {
        o.insert(
            "Placement".into(),
            json!({ "AvailabilityZone": text_of(p, "availabilityZone") }),
        );
    }
    if let Some(tags) = child(i, "tagSet") {
        let tags: Vec<Value> = items(tags)
            .map(|t| json!({ "Key": text_of(t, "key"), "Value": text_of(t, "value").unwrap_or_default() }))
            .collect();
        o.insert("Tags".into(), Value::Array(tags));
    }
    Value::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS SigV4 test suite, `get-vanilla`.
    #[test]
    fn sigv4_matches_the_aws_test_suite() {
        let now = DateTime::parse_from_rfc3339("2015-08-30T12:36:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let headers = vec![
            ("Host".to_string(), "example.amazonaws.com".to_string()),
            ("X-Amz-Date".to_string(), "20150830T123600Z".to_string()),
        ];
        let auth = authorization(
            "GET",
            "/",
            &headers,
            b"",
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "service",
            now,
        );
        assert_eq!(
            auth,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
        );
    }

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn target_needs_static_keys_a_standard_endpoint_and_a_sane_region() {
        let keys = [
            ("AWS_REGION", "eu-west-1"),
            ("AWS_ACCESS_KEY_ID", "AKIDEXAMPLE"),
            ("AWS_SECRET_ACCESS_KEY", "s"),
        ];
        let t = target_from_env(&Mode::Aws, &env(&keys)).unwrap();
        assert_eq!(t.region, "eu-west-1");
        assert!(t.session_token.is_none());
        assert!(target_from_env(&Mode::Off, &env(&keys)).is_none());
        // Profile resolved by the child (no exported keys) → CLI.
        assert!(target_from_env(
            &Mode::Aws,
            &env(&[("AWS_REGION", "eu-west-1"), ("AWS_PROFILE", "dev")])
        )
        .is_none());
        // A custom endpoint (LocalStack…) → CLI.
        let mut e = env(&keys);
        e.push(("AWS_ENDPOINT_URL".into(), "http://localhost:4566".into()));
        assert!(target_from_env(&Mode::Aws, &e).is_none());
        // A region that would inject into the host name → CLI.
        let mut e = env(&keys);
        e[0].1 = "eu-west-1.evil.com/".into();
        assert!(target_from_env(&Mode::Aws, &e).is_none());
        assert_eq!(
            host_for("logs", "cn-north-1"),
            "logs.cn-north-1.amazonaws.com.cn"
        );
    }

    #[test]
    fn ec2_xml_becomes_the_cli_shape() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <reservationSet><item><instancesSet><item>
    <instanceId>i-0abc1234</instanceId><instanceType>t3.micro</instanceType>
    <instanceState><code>16</code><name>running</name></instanceState>
    <placement><availabilityZone>eu-west-1a</availabilityZone></placement>
    <privateIpAddress>10.0.0.5</privateIpAddress><ipAddress>3.3.3.3</ipAddress>
    <launchTime>2024-01-02T03:04:05.000Z</launchTime><vpcId>vpc-1</vpcId>
    <tagSet><item><key>Name</key><value>web</value></item></tagSet>
  </item></instancesSet></item></reservationSet>
</DescribeInstancesResponse>"#;
        let doc = roxmltree::Document::parse(xml).unwrap();
        let set = child(doc.root_element(), "reservationSet").unwrap();
        let r = items(set).next().unwrap();
        let i = items(child(r, "instancesSet").unwrap()).next().unwrap();
        let v = json!({ "Reservations": [{ "Instances": [instance_json(i)] }] });
        let got = crate::ec2::normalize_instances(&v);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].instance_id, "i-0abc1234");
        assert_eq!(got[0].name.as_deref(), Some("web"));
        assert_eq!(got[0].state, "running");
        assert_eq!(got[0].az.as_deref(), Some("eu-west-1a"));
        assert_eq!(got[0].public_ip.as_deref(), Some("3.3.3.3"));
    }

    #[test]
    fn api_errors_classify_like_the_cli() {
        match api_error(
            400,
            "com.amazonaws#ExpiredTokenException",
            "expired",
            "FilterLogEvents",
        ) {
            Error::Invalid(m) => assert!(m.starts_with("login required:"), "{m}"),
            e => panic!("{e:?}"),
        }
        assert!(matches!(
            api_error(403, "UnauthorizedOperation", "no", "DescribeInstances"),
            Error::Forbidden(_)
        ));
        match api_error(
            400,
            "ResourceNotFoundException",
            "The specified log group does not exist.",
            "FilterLogEvents",
        ) {
            Error::Invalid(m) => assert!(m.contains("ResourceNotFoundException"), "{m}"),
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn transport_failures_fall_back_to_the_cli() {
        assert!(matches!(fallback(Ok::<_, Error>(1)), Ok(Some(1))));
        assert!(matches!(
            fallback::<()>(Err(Error::Upstream(format!("{TRANSPORT}logs: tls")))),
            Ok(None)
        ));
        // An AWS answer is an answer, not a reason to retry on the CLI.
        assert!(fallback::<()>(Err(Error::Forbidden("denied".into()))).is_err());
        assert!(fallback::<()>(Err(Error::Upstream("Throttling".into()))).is_err());
    }

    #[test]
    fn form_encoding_is_rfc3986() {
        assert_eq!(form_enc("a b/c=d"), "a%20b%2Fc%3Dd");
        assert_eq!(form_enc("Filter.1.Value.1"), "Filter.1.Value.1");
    }
}
