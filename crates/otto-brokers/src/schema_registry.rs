//! Confluent Schema Registry client — fetch schemas by id (to decode
//! Confluent-framed Avro values), list subjects for the schema browser, and
//! expose version history + compatibility checking for the operator workflow.

use crate::types::{CompatCheckReq, CompatCheckResp, SchemaSubject, SchemaVersion, SchemaVersionDetail};
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
}

impl SchemaRegistry {
    pub fn new(
        base: &str,
        username: Option<String>,
        password: Option<String>,
        skip_tls_verify: bool,
        socks_proxy: Option<String>,
    ) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .danger_accept_invalid_certs(skip_tls_verify)
            // SSRF guard (audit S1): bound + re-validate redirect hops so the
            // user-supplied registry URL can't 30x-bounce into the internal net.
            .redirect(otto_netguard::redirect_policy())
            .timeout(Duration::from_secs(10));
        let via_tunnel = socks_proxy.is_some();
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

    /// Fetch (and cache) the schema document for a registry schema id.
    pub async fn schema_by_id(&self, id: i32) -> Result<String> {
        if let Some(s) = self.cache.get(&id) {
            return Ok(s.clone());
        }
        let url = format!("{}/schemas/ids/{id}", self.base);
        self.guard(&url).await?;
        let resp = self.get(url).send().await.map_err(up)?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "schema registry returned {} for id {id}",
                resp.status()
            )));
        }
        #[derive(serde::Deserialize)]
        struct SchemaResp {
            schema: String,
        }
        let body: SchemaResp = resp.json().await.map_err(up)?;
        self.cache.insert(id, body.schema.clone());
        Ok(body.schema)
    }

    /// List subjects with their latest registered version.
    pub async fn subjects(&self) -> Result<Vec<SchemaSubject>> {
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

        let mut out = Vec::with_capacity(names.len());
        for subject in names {
            let url = format!("{}/subjects/{subject}/versions/latest", self.base);
            let Ok(resp) = self.get(url).send().await else {
                continue;
            };
            if !resp.status().is_success() {
                continue;
            }
            if let Ok(v) = resp.json::<Version>().await {
                out.push(SchemaSubject {
                    subject: v.subject,
                    version: v.version,
                    id: v.id,
                    schema_type: v.schema_type.unwrap_or_else(|| "AVRO".into()),
                    schema: v.schema,
                });
            }
        }
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
        let url = format!("{}/subjects/{}/versions/{version}", self.base, urlenc(subject));
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
