//! Confluent Schema Registry client — fetch schemas by id (to decode
//! Confluent-framed Avro values), list subjects for the schema browser, and
//! expose version history + compatibility checking for the operator workflow.

use crate::types::{
    CompatCheckReq, CompatCheckResp, SchemaSubject, SchemaVersion, SchemaVersionDetail,
};
use dashmap::DashMap;
use otto_core::{Error, Result};
use std::time::Duration;

pub struct SchemaRegistry {
    base: String,
    client: reqwest::Client,
    auth: Option<(String, Option<String>)>,
    /// True when requests ride an SSH SOCKS tunnel — the registry is reached
    /// through the user's bastion, so the SSRF guard is intentionally skipped
    /// (private targets are the point, and the bastion creds are the authority).
    via_tunnel: bool,
    /// schema id → schema document (registries are append-only, so cacheable).
    cache: DashMap<i32, String>,
    /// schema id → PARSED Avro schema (parsing per consumed message was the
    /// hot cost of an Avro peek).
    parsed: DashMap<i32, std::sync::Arc<apache_avro::Schema>>,
    /// schema id → (retry-after instant, error) for ids whose lookup FAILED.
    /// Keys that merely look Confluent-framed (first byte 0x00) would otherwise
    /// cost a registry round trip (often through the SSH tunnel) on every 3 s
    /// live-tail tick. Bounded in practice by the distinct ids seen.
    negative: DashMap<i32, (std::time::Instant, String)>,
    /// Last subject listing + when it was fetched (`SUBJECTS_TTL`).
    subjects_cache: std::sync::Mutex<Option<(std::time::Instant, Vec<SchemaSubject>)>>,
}

/// How long a subject listing is reused (the Schema tab reloads on every visit).
const SUBJECTS_TTL: Duration = Duration::from_secs(60);
/// Concurrent `GET /subjects/{s}/versions/latest` requests while listing.
const SUBJECTS_CONCURRENCY: usize = 8;
/// How long a definitive failure (4xx — unknown id — or a schema that is not
/// valid Avro; registries are append-only, so neither heals quickly) is
/// remembered before the id is asked about again.
const NEGATIVE_TTL: Duration = Duration::from_secs(60);
/// How long a transient failure (transport error, 5xx) is remembered: long
/// enough that a tail ticking every 3 s doesn't hammer a struggling registry.
const TRANSIENT_NEGATIVE_TTL: Duration = Duration::from_secs(10);
/// Concurrent schema-id lookups per consumed batch.
const IDS_CONCURRENCY: usize = 8;

impl SchemaRegistry {
    pub fn new(
        base: &str,
        username: Option<String>,
        password: Option<String>,
        skip_tls_verify: bool,
        socks_proxy: Option<String>,
    ) -> Result<Self> {
        // SSRF guard (audit S1): bound + re-validate redirect hops so the
        // user-supplied registry URL can't 30x-bounce into the internal net.
        // Direct (untunnelled) clients also get the guarded resolver, so the
        // address dialled is the one vetted — no DNS rebinding after `guard`.
        let base_builder = if socks_proxy.is_some() {
            reqwest::Client::builder().redirect(otto_netguard::redirect_policy())
        } else {
            otto_netguard::guarded_client_builder()
        };
        let mut builder = base_builder
            .danger_accept_invalid_certs(skip_tls_verify)
            .timeout(Duration::from_secs(10));
        let via_tunnel = socks_proxy.is_some();
        // Basic-auth credentials ride every request: when the profile has
        // them, require https for the registry (plain http stays allowed for
        // loopback — local registries / port-forwards — and for tunneled
        // targets, where the bastion carries the traffic).
        if username.is_some() && !via_tunnel {
            otto_netguard::require_tls_or_loopback(base)
                .map_err(|m| Error::Invalid(format!("schema registry url: {m}")))?;
        }
        if let Some(proxy) = socks_proxy {
            builder = builder.proxy(
                reqwest::Proxy::all(&proxy)
                    .map_err(|e| Error::Internal(format!("schema registry socks proxy: {e}")))?,
            );
        }
        let client = builder
            .build()
            .map_err(|e| Error::Internal(format!("schema registry client: {e}")))?;
        let auth = username.map(|u| (u, password));
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            client,
            auth,
            via_tunnel,
            cache: DashMap::new(),
            parsed: DashMap::new(),
            negative: DashMap::new(),
            subjects_cache: std::sync::Mutex::new(None),
        })
    }

    fn get(&self, url: String) -> reqwest::RequestBuilder {
        let rb = self.client.get(url);
        match &self.auth {
            Some((u, p)) => rb.basic_auth(u, p.as_ref()),
            None => rb,
        }
    }

    /// SSRF pre-flight (audit S1): the registry base URL is a user-supplied
    /// cluster-profile field. Resolve + classify the host before connecting so a
    /// low-privileged caller can't steer the daemon at loopback / RFC1918 /
    /// link-local (169.254.169.254) targets. Every request shares `self.base`'s
    /// host, so guarding the base once per call covers the per-subject fan-out;
    /// redirect hops are re-validated by the client's redirect policy.
    async fn guard(&self, url: &str) -> Result<()> {
        if self.via_tunnel {
            // Reached through the user's SSH bastion; the private endpoint is the
            // intent and the tunnel handles routing, so the loopback SSRF guard
            // would only false-positive here.
            return Ok(());
        }
        otto_netguard::check_url(url)
            .await
            .map_err(|m| Error::Forbidden(format!("schema registry blocked: {m}")))
    }

    /// A remembered failure for `id`, while its TTL runs.
    fn negative_hit(&self, id: i32) -> Option<Error> {
        let hit = self.negative.get(&id).map(|e| e.value().clone());
        match hit {
            Some((until, msg)) if std::time::Instant::now() < until => Some(Error::Upstream(msg)),
            Some(_) => {
                self.negative.remove(&id);
                None
            }
            None => None,
        }
    }

    fn remember_failure(&self, id: i32, ttl: Duration, err: &Error) {
        let msg = match err {
            Error::Upstream(m) => m.clone(),
            other => other.to_string(),
        };
        self.negative
            .insert(id, (std::time::Instant::now() + ttl, msg));
    }

    /// Fetch (and cache) the schema document for a registry schema id. A
    /// failed lookup is remembered (`NEGATIVE_TTL` for 4xx,
    /// `TRANSIENT_NEGATIVE_TTL` otherwise) and answered from memory meanwhile.
    pub async fn schema_by_id(&self, id: i32) -> Result<String> {
        if let Some(s) = self.cache.get(&id) {
            return Ok(s.clone());
        }
        if let Some(e) = self.negative_hit(id) {
            return Err(e);
        }
        let url = format!("{}/schemas/ids/{id}", self.base);
        // An SSRF refusal is policy, not a registry failure: never cached.
        self.guard(&url).await?;
        let resp = match self.get(url).send().await {
            Ok(r) => r,
            Err(e) => {
                let e = up(e);
                self.remember_failure(id, TRANSIENT_NEGATIVE_TTL, &e);
                return Err(e);
            }
        };
        let status = resp.status();
        if !status.is_success() {
            let e = Error::Upstream(format!("schema registry returned {status} for id {id}"));
            let ttl = if status.is_client_error() {
                NEGATIVE_TTL
            } else {
                TRANSIENT_NEGATIVE_TTL
            };
            self.remember_failure(id, ttl, &e);
            return Err(e);
        }
        #[derive(serde::Deserialize)]
        struct SchemaResp {
            schema: String,
        }
        let body: SchemaResp = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                let e = up(e);
                self.remember_failure(id, TRANSIENT_NEGATIVE_TTL, &e);
                return Err(e);
            }
        };
        self.cache.insert(id, body.schema.clone());
        Ok(body.schema)
    }

    /// Resolve a batch of schema ids, `IDS_CONCURRENCY` lookups at a time
    /// (was one awaited round trip per id). Ids that fail are left out; their
    /// failure is negatively cached by `schema_by_id`/`parsed_schema_by_id`.
    pub async fn parsed_schemas_by_ids(
        &self,
        ids: impl IntoIterator<Item = i32>,
    ) -> std::collections::HashMap<i32, std::sync::Arc<apache_avro::Schema>> {
        use futures_util::stream::{self, StreamExt};
        stream::iter(ids)
            .map(|sid| async move { self.parsed_schema_by_id(sid).await.ok().map(|s| (sid, s)) })
            .buffer_unordered(IDS_CONCURRENCY)
            .filter_map(|r| async move { r })
            .collect()
            .await
    }

    /// The parsed Avro schema for a registry id (fetched + parsed once).
    pub async fn parsed_schema_by_id(
        &self,
        id: i32,
    ) -> Result<std::sync::Arc<apache_avro::Schema>> {
        if let Some(s) = self.parsed.get(&id) {
            return Ok(s.clone());
        }
        if let Some(e) = self.negative_hit(id) {
            return Err(e);
        }
        let doc = self.schema_by_id(id).await?;
        let schema = match apache_avro::Schema::parse_str(&doc) {
            Ok(s) => s,
            Err(e) => {
                let e = Error::Upstream(format!("schema {id} is not valid Avro: {e}"));
                self.remember_failure(id, NEGATIVE_TTL, &e);
                return Err(e);
            }
        };
        let schema = std::sync::Arc::new(schema);
        self.parsed.insert(id, schema.clone());
        Ok(schema)
    }

    /// List subjects with their latest registered version. Reused for
    /// `SUBJECTS_TTL`; the per-subject lookups run `SUBJECTS_CONCURRENCY` at a
    /// time (was strictly sequential: 1000 subjects × RTT = minutes).
    pub async fn subjects(&self) -> Result<Vec<SchemaSubject>> {
        if let Some((at, list)) = self
            .subjects_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            if at.elapsed() < SUBJECTS_TTL {
                return Ok(list.clone());
            }
        }
        let out = self.fetch_subjects().await?;
        *self
            .subjects_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some((std::time::Instant::now(), out.clone()));
        Ok(out)
    }

    async fn fetch_subjects(&self) -> Result<Vec<SchemaSubject>> {
        use futures_util::stream::{self, StreamExt};
        let url = format!("{}/subjects", self.base);
        self.guard(&url).await?;
        let resp = self.get(url).send().await.map_err(up)?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "schema registry returned {}",
                resp.status()
            )));
        }
        let names: Vec<String> = resp.json().await.map_err(up)?;

        #[derive(serde::Deserialize)]
        struct Version {
            subject: String,
            version: i32,
            id: i32,
            schema: String,
            #[serde(rename = "schemaType")]
            schema_type: Option<String>,
        }

        let mut out: Vec<SchemaSubject> = stream::iter(names)
            .map(|subject| async move {
                let url = format!("{}/subjects/{subject}/versions/latest", self.base);
                let resp = self.get(url).send().await.ok()?;
                if !resp.status().is_success() {
                    return None;
                }
                let v = resp.json::<Version>().await.ok()?;
                Some(SchemaSubject {
                    subject: v.subject,
                    version: v.version,
                    id: v.id,
                    schema_type: v.schema_type.unwrap_or_else(|| "AVRO".into()),
                    schema: v.schema,
                })
            })
            .buffer_unordered(SUBJECTS_CONCURRENCY)
            .filter_map(|s| async move { s })
            .collect()
            .await;
        out.sort_by(|a, b| a.subject.cmp(&b.subject));
        Ok(out)
    }

    /// Fetch the version history for a subject (all registered versions, oldest first).
    pub async fn subject_versions(&self, subject: &str) -> Result<Vec<SchemaVersion>> {
        // First get the list of version numbers from the registry.
        let url = format!("{}/subjects/{}/versions", self.base, urlenc(subject));
        self.guard(&url).await?;
        let resp = self.get(url).send().await.map_err(up)?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "schema registry returned {} listing versions for {subject}",
                resp.status()
            )));
        }
        let version_nums: Vec<i32> = resp.json().await.map_err(up)?;

        #[derive(serde::Deserialize)]
        struct VersionResp {
            version: i32,
            id: i32,
            schema: String,
            #[serde(rename = "schemaType")]
            schema_type: Option<String>,
        }

        let mut out = Vec::with_capacity(version_nums.len());
        for v in version_nums {
            let vu = format!("{}/subjects/{}/versions/{v}", self.base, urlenc(subject));
            let Ok(r) = self.get(vu).send().await else {
                continue;
            };
            if !r.status().is_success() {
                continue;
            }
            if let Ok(vr) = r.json::<VersionResp>().await {
                out.push(SchemaVersion {
                    version: vr.version,
                    id: vr.id,
                    schema_type: vr.schema_type.unwrap_or_else(|| "AVRO".into()),
                    schema: vr.schema,
                });
            }
        }
        Ok(out)
    }

    /// Fetch one specific version of a subject. `version` may be a number string
    /// or `"latest"`.
    pub async fn subject_version_detail(
        &self,
        subject: &str,
        version: &str,
    ) -> Result<SchemaVersionDetail> {
        let url = format!(
            "{}/subjects/{}/versions/{version}",
            self.base,
            urlenc(subject)
        );
        self.guard(&url).await?;
        let resp = self.get(url).send().await.map_err(up)?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "schema registry returned {} for {subject}/versions/{version}",
                resp.status()
            )));
        }
        #[derive(serde::Deserialize)]
        struct Resp {
            subject: String,
            version: i32,
            id: i32,
            schema: String,
            #[serde(rename = "schemaType")]
            schema_type: Option<String>,
        }
        let r: Resp = resp.json().await.map_err(up)?;
        Ok(SchemaVersionDetail {
            subject: r.subject,
            version: r.version,
            id: r.id,
            schema_type: r.schema_type.unwrap_or_else(|| "AVRO".into()),
            schema: r.schema,
        })
    }

    /// Check compatibility of a candidate schema against the latest registered
    /// version of a subject. Calls the registry's `/compatibility/…/versions/latest`
    /// endpoint and returns the is_compatible flag + any violation messages.
    pub async fn check_compatibility(
        &self,
        subject: &str,
        req: &CompatCheckReq,
    ) -> Result<CompatCheckResp> {
        let url = format!(
            "{}/compatibility/subjects/{}/versions/latest",
            self.base,
            urlenc(subject)
        );
        self.guard(&url).await?;

        #[derive(serde::Serialize)]
        struct Body<'a> {
            schema: &'a str,
            #[serde(rename = "schemaType", skip_serializing_if = "Option::is_none")]
            schema_type: Option<&'a str>,
        }
        let body = Body {
            schema: &req.schema,
            schema_type: req.schema_type.as_deref(),
        };
        let rb = self.client.post(&url).json(&body);
        let rb = match &self.auth {
            Some((u, p)) => rb.basic_auth(u, p.as_ref()),
            None => rb,
        };
        let resp = rb.send().await.map_err(up)?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "schema registry compatibility check returned {}",
                resp.status()
            )));
        }
        #[derive(serde::Deserialize)]
        struct CompatResp {
            is_compatible: bool,
            #[serde(default)]
            messages: Vec<String>,
        }
        let r: CompatResp = resp.json().await.map_err(up)?;
        Ok(CompatCheckResp {
            compatible: r.is_compatible,
            messages: r.messages,
        })
    }
}

/// URL-encode a schema subject name (subjects may contain `/`, `:` etc.).
fn urlenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            other => {
                let mut buf = [0u8; 4];
                for byte in other.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

fn up(e: reqwest::Error) -> Error {
    Error::Upstream(format!("schema registry: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A loopback "registry": answers `GET /schemas/ids/{id}` with a valid
    /// Avro schema for id 1 and 404 for anything else, counting requests.
    async fn mock_registry() -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let h = h.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    h.fetch_add(1, Ordering::SeqCst);
                    let (status, body) = if req.starts_with("GET /schemas/ids/1 ") {
                        ("200 OK", r#"{"schema":"\"string\""}"#.to_string())
                    } else {
                        ("404 Not Found", r#"{"error_code":40403}"#.to_string())
                    };
                    let resp = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        (format!("http://{addr}"), hits)
    }

    fn registry(base: &str) -> SchemaRegistry {
        SchemaRegistry {
            base: base.to_string(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            auth: None,
            // Loopback mock: skip the SSRF guard the way a tunnel does.
            via_tunnel: true,
            cache: DashMap::new(),
            parsed: DashMap::new(),
            negative: DashMap::new(),
            subjects_cache: std::sync::Mutex::new(None),
        }
    }

    /// N3: an unknown id is asked about once per `NEGATIVE_TTL`, not on every
    /// tail tick; a known id is fetched once and then served from cache.
    #[tokio::test]
    async fn failed_schema_ids_are_negatively_cached() {
        let (base, hits) = mock_registry().await;
        let reg = registry(&base);
        assert!(reg.parsed_schema_by_id(7).await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        for _ in 0..5 {
            let err = reg.parsed_schema_by_id(7).await.unwrap_err();
            assert!(err.to_string().contains("404"), "{err}");
        }
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "a failed id must not be re-fetched within the TTL"
        );
        // Expired entries are retried.
        reg.negative
            .insert(7, (std::time::Instant::now(), "stale".into()));
        assert!(reg.schema_by_id(7).await.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 2);

        // A batch resolves the good id and skips the (cached) bad ones with
        // a single new request for id 1.
        let got = reg.parsed_schemas_by_ids([1, 7]).await;
        assert!(got.contains_key(&1) && !got.contains_key(&7));
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        let again = reg.parsed_schemas_by_ids([1, 7]).await;
        assert_eq!(again.len(), 1);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            3,
            "warm batch makes no request"
        );
    }

    /// The ids of one batch are looked up concurrently, not one RTT each.
    #[tokio::test]
    async fn schema_ids_resolve_concurrently() {
        // Each connection stalls 300 ms before answering 404; 8 ids serially
        // would take ≥ 2.4 s.
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = sock.read(&mut buf).await;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let _ = sock
                        .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                        .await;
                });
            }
        });
        let reg = registry(&format!("http://{addr}"));
        let t = std::time::Instant::now();
        let got = reg.parsed_schemas_by_ids(100..108).await;
        assert!(got.is_empty());
        assert!(
            t.elapsed() < Duration::from_millis(1500),
            "8 lookups took {:?}",
            t.elapsed()
        );
        assert_eq!(reg.negative.len(), 8);
    }
}
