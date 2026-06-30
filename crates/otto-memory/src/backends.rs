//! Optional **remote** Vault backends — the layers beyond local SQLite.
//!
//! - [`QdrantIndex`] — a [`crate::index::VectorIndex`] backed by a Qdrant cluster
//!   (REST). Used as the `vector` layer at scale / when shared across machines.
//! - [`SurrealClient`] — a SurrealDB graph store (HTTP `/sql`). Used as the
//!   `graph` layer for rich dependency traversal and a shared remote graph.
//! - [`installer`] — plan + (approval-gated) execute for installing Qdrant /
//!   SurrealDB / Ollama locally via Docker / Homebrew.
//!
//! These are admin-configured trusted endpoints (commonly localhost Docker), so
//! unlike agent-/user-supplied URLs they deliberately do NOT go through the SSRF
//! guard (which blocks loopback). Every method degrades gracefully: a down
//! backend never breaks recall — the caller falls back to SQLite.

use std::time::Duration;

use async_trait::async_trait;
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::index::VectorIndex;

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .unwrap_or_default()
}

/// Deterministic u64 point id for Qdrant (which only accepts uint64 / UUID ids),
/// derived from a memory id (kept in the payload for exact recall).
fn point_id(memory_id: &str) -> u64 {
    let mut h = 1469598103934665603u64;
    for b in memory_id.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

// ===========================================================================
// Qdrant — remote vector layer
// ===========================================================================

#[derive(Clone)]
pub struct QdrantIndex {
    base_url: String,
    api_key: Option<String>,
    collection: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct QdrantSearchResp {
    result: Vec<QdrantHit>,
}

#[derive(Deserialize)]
struct QdrantHit {
    score: f32,
    #[serde(default)]
    payload: Option<QdrantPayload>,
}

#[derive(Deserialize)]
struct QdrantPayload {
    memory_id: Option<String>,
}

impl QdrantIndex {
    pub fn new(base_url: &str, api_key: Option<String>, collection: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            collection,
            http: client(),
        }
    }

    fn req(&self, m: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut b = self.http.request(m, format!("{}{}", self.base_url, path));
        if let Some(k) = &self.api_key {
            b = b.header("api-key", k);
        }
        b
    }

    /// Liveness check (`GET /healthz`).
    pub async fn health(&self) -> Result<()> {
        let r = self
            .req(reqwest::Method::GET, "/healthz")
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("qdrant unreachable: {e}")))?;
        if r.status().is_success() {
            Ok(())
        } else {
            Err(Error::Upstream(format!("qdrant health {}", r.status())))
        }
    }

    /// Create the collection if absent (cosine distance, `dim` size). Idempotent.
    pub async fn ensure_collection(&self, dim: usize) -> Result<()> {
        let exists = self
            .req(reqwest::Method::GET, &format!("/collections/{}", self.collection))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        if exists {
            return Ok(());
        }
        let body = serde_json::json!({ "vectors": { "size": dim, "distance": "Cosine" } });
        let r = self
            .req(reqwest::Method::PUT, &format!("/collections/{}", self.collection))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("qdrant create: {e}")))?;
        if r.status().is_success() {
            Ok(())
        } else {
            Err(Error::Upstream(format!("qdrant create {}", r.status())))
        }
    }

    /// Upsert one memory's vector (waits so it is immediately searchable).
    pub async fn upsert(&self, memory_id: &str, vector: &[f32]) -> Result<()> {
        let body = serde_json::json!({
            "points": [{
                "id": point_id(memory_id),
                "vector": vector,
                "payload": { "memory_id": memory_id },
            }]
        });
        let r = self
            .req(
                reqwest::Method::PUT,
                &format!("/collections/{}/points?wait=true", self.collection),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("qdrant upsert: {e}")))?;
        if r.status().is_success() {
            Ok(())
        } else {
            Err(Error::Upstream(format!("qdrant upsert {}", r.status())))
        }
    }

    pub async fn delete(&self, memory_id: &str) -> Result<()> {
        let body = serde_json::json!({ "points": [point_id(memory_id)] });
        let _ = self
            .req(
                reqwest::Method::POST,
                &format!("/collections/{}/points/delete?wait=true", self.collection),
            )
            .json(&body)
            .send()
            .await;
        Ok(())
    }
}

#[async_trait]
impl VectorIndex for QdrantIndex {
    async fn knn(&self, _ws: &str, query: &[f32], k: usize) -> Result<Vec<(String, f32)>> {
        let body = serde_json::json!({ "vector": query, "limit": k, "with_payload": true });
        let r = self
            .req(
                reqwest::Method::POST,
                &format!("/collections/{}/points/search", self.collection),
            )
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("qdrant search: {e}")))?;
        if !r.status().is_success() {
            return Err(Error::Upstream(format!("qdrant search {}", r.status())));
        }
        let resp: QdrantSearchResp = r
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("qdrant decode: {e}")))?;
        Ok(resp
            .result
            .into_iter()
            .filter_map(|h| h.payload.and_then(|p| p.memory_id).map(|id| (id, h.score)))
            .collect())
    }
}

// ===========================================================================
// SurrealDB — remote graph layer
// ===========================================================================

#[derive(Clone)]
pub struct SurrealClient {
    base_url: String,
    ns: String,
    db: String,
    user: Option<String>,
    pass: Option<String>,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct SurrealResult {
    #[serde(default)]
    status: String,
    #[serde(default)]
    result: serde_json::Value,
}

impl SurrealClient {
    pub fn new(base_url: &str, ns: &str, db: &str, user: Option<String>, pass: Option<String>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            ns: ns.to_string(),
            db: db.to_string(),
            user,
            pass,
            http: client(),
        }
    }

    /// Liveness check (`GET /health`).
    pub async fn health(&self) -> Result<()> {
        let r = self
            .http
            .get(format!("{}/health", self.base_url))
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("surreal unreachable: {e}")))?;
        if r.status().is_success() {
            Ok(())
        } else {
            Err(Error::Upstream(format!("surreal health {}", r.status())))
        }
    }

    /// Run a SurrealQL statement, returning the first result set's value.
    pub async fn query(&self, surql: &str) -> Result<serde_json::Value> {
        let mut b = self
            .http
            .post(format!("{}/sql", self.base_url))
            .header("Accept", "application/json")
            // both header spellings (v1 NS/DB, v2 surreal-ns/surreal-db)
            .header("NS", &self.ns)
            .header("DB", &self.db)
            .header("surreal-ns", &self.ns)
            .header("surreal-db", &self.db)
            .body(surql.to_string());
        if let (Some(u), Some(p)) = (&self.user, &self.pass) {
            b = b.basic_auth(u, Some(p));
        }
        let r = b
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("surreal query: {e}")))?;
        if !r.status().is_success() {
            return Err(Error::Upstream(format!("surreal query {}", r.status())));
        }
        let results: Vec<SurrealResult> = r
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("surreal decode: {e}")))?;
        if let Some(first) = results.into_iter().find(|s| s.status == "OK" || s.status.is_empty()) {
            Ok(first.result)
        } else {
            Ok(serde_json::Value::Null)
        }
    }

    /// Mirror a code graph into SurrealDB as `code_node` records + `depends`
    /// graph relations, so it can be traversed with SurrealQL. Best-effort.
    pub async fn mirror_graph(&self, nodes: &[otto_state::CodeNode], edges: &[otto_state::CodeEdge]) -> Result<()> {
        for n in nodes {
            let q = format!(
                "UPDATE type::thing('code_node', {id}) CONTENT {{ kind: {kind}, key: {key}, label: {label} }};",
                id = sq(&n.id),
                kind = sq(&n.kind),
                key = sq(&n.key),
                label = sq(&n.label),
            );
            let _ = self.query(&q).await;
        }
        for e in edges {
            let q = format!(
                "RELATE type::thing('code_node', {src})->depends->type::thing('code_node', {dst}) SET rel = {rel}, detail = {detail};",
                src = sq(&e.src_id),
                dst = sq(&e.dst_id),
                rel = sq(&e.rel),
                detail = sq(&e.detail),
            );
            let _ = self.query(&q).await;
        }
        Ok(())
    }
}

/// Quote + escape a string as a SurrealQL string literal.
fn sq(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

// ===========================================================================
// Installer — plan + execute for local Qdrant / SurrealDB / Ollama
// ===========================================================================

pub mod installer {
    use super::*;
    use std::process::Stdio;
    use tokio::process::Command;

    /// A planned install: the detected method + the exact commands to run.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct InstallPlan {
        pub kind: String,
        /// `docker` | `brew` | `script` | `none`.
        pub method: String,
        pub steps: Vec<String>,
        /// Health-check URL we will poll after install.
        pub health_url: String,
        pub ready: bool,
        pub notes: String,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    pub struct InstallResult {
        pub ok: bool,
        pub log: String,
    }

    async fn have(tool: &str) -> bool {
        Command::new("which")
            .arg(tool)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Build an install plan for a backend kind, detecting available tooling.
    /// Pure of side effects beyond `which` probes — safe to call for a preview.
    pub async fn plan(kind: &str, data_dir: &str) -> Result<InstallPlan> {
        let docker = have("docker").await;
        let brew = have("brew").await;
        match kind {
            "qdrant" => {
                let (method, steps, ready) = if docker {
                    (
                        "docker",
                        vec![format!(
                            "docker run -d --name otto-qdrant -p 6333:6333 -v {data_dir}/qdrant:/qdrant/storage qdrant/qdrant:v1.12.4"
                        )],
                        true,
                    )
                } else {
                    ("none", vec![], false)
                };
                Ok(InstallPlan {
                    kind: kind.into(),
                    method: method.into(),
                    steps,
                    health_url: "http://127.0.0.1:6333/healthz".into(),
                    ready,
                    notes: if ready {
                        "Qdrant will run on 127.0.0.1:6333 (vector layer).".into()
                    } else {
                        "Docker not found — install Docker Desktop, or run Qdrant manually and set the URL.".into()
                    },
                })
            }
            "surreal" => {
                let (method, steps, ready) = if brew {
                    (
                        "brew",
                        vec![
                            "brew install surrealdb/tap/surreal".into(),
                            format!("surreal start --user root --pass root file://{data_dir}/surreal &"),
                        ],
                        true,
                    )
                } else if docker {
                    (
                        "docker",
                        vec![format!(
                            "docker run -d --name otto-surreal -p 8000:8000 -v {data_dir}/surreal:/data surrealdb/surrealdb:v2.1 start --user root --pass root file:/data/db"
                        )],
                        true,
                    )
                } else {
                    ("script", vec!["curl -sSf https://install.surrealdb.com | sh".into()], false)
                };
                Ok(InstallPlan {
                    kind: kind.into(),
                    method: method.into(),
                    steps,
                    health_url: "http://127.0.0.1:8000/health".into(),
                    ready,
                    notes: "SurrealDB will run on 127.0.0.1:8000 (graph layer).".into(),
                })
            }
            "ollama" => {
                let (method, steps, ready) = if brew {
                    (
                        "brew",
                        vec![
                            "brew install ollama".into(),
                            "ollama serve &".into(),
                            "ollama pull nomic-embed-text".into(),
                        ],
                        true,
                    )
                } else {
                    (
                        "script",
                        vec![
                            "curl -fsSL https://ollama.com/install.sh | sh".into(),
                            "ollama pull nomic-embed-text".into(),
                        ],
                        false,
                    )
                };
                Ok(InstallPlan {
                    kind: kind.into(),
                    method: method.into(),
                    steps,
                    health_url: "http://127.0.0.1:11434/api/tags".into(),
                    ready,
                    notes: "Ollama provides real local neural embeddings (nomic-embed-text, 768d).".into(),
                })
            }
            other => Err(Error::Invalid(format!("unknown backend kind: {other}"))),
        }
    }

    /// Execute a plan's steps with `sh -c`, bounded per step. APPROVAL-GATED:
    /// callers must confirm before invoking this (it changes the host).
    pub async fn execute(plan: &InstallPlan) -> InstallResult {
        let mut log = String::new();
        for step in &plan.steps {
            log.push_str(&format!("$ {step}\n"));
            let out = tokio::time::timeout(
                Duration::from_secs(600),
                Command::new("sh").arg("-c").arg(step).output(),
            )
            .await;
            match out {
                Ok(Ok(o)) => {
                    log.push_str(&String::from_utf8_lossy(&o.stdout));
                    log.push_str(&String::from_utf8_lossy(&o.stderr));
                    if !o.status.success() {
                        log.push_str(&format!("\n[step failed: exit {:?}]\n", o.status.code()));
                        return InstallResult { ok: false, log };
                    }
                }
                Ok(Err(e)) => {
                    log.push_str(&format!("[spawn error: {e}]\n"));
                    return InstallResult { ok: false, log };
                }
                Err(_) => {
                    log.push_str("[step timed out]\n");
                    return InstallResult { ok: false, log };
                }
            }
        }
        InstallResult { ok: true, log }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_id_is_stable() {
        assert_eq!(point_id("abc"), point_id("abc"));
        assert_ne!(point_id("abc"), point_id("abd"));
    }

    #[test]
    fn surreal_quoting_escapes() {
        assert_eq!(sq(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    #[tokio::test]
    async fn install_plan_unknown_kind_errors() {
        assert!(installer::plan("nope", "/tmp").await.is_err());
    }

    #[tokio::test]
    async fn install_plan_qdrant_has_health_url() {
        let p = installer::plan("qdrant", "/tmp/data").await.unwrap();
        assert_eq!(p.kind, "qdrant");
        assert!(p.health_url.contains("6333"));
    }
}
