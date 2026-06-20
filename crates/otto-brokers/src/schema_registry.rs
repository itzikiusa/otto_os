//! Confluent Schema Registry client — fetch schemas by id (to decode
//! Confluent-framed Avro values) and list subjects for the schema browser.

use crate::types::SchemaSubject;
use dashmap::DashMap;
use otto_core::{Error, Result};
use std::time::Duration;

pub struct SchemaRegistry {
    base: String,
    client: reqwest::Client,
    auth: Option<(String, Option<String>)>,
    /// schema id → schema document (registries are append-only, so cacheable).
    cache: DashMap<i32, String>,
}

impl SchemaRegistry {
    pub fn new(
        base: &str,
        username: Option<String>,
        password: Option<String>,
        skip_tls_verify: bool,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(skip_tls_verify)
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| Error::Internal(format!("schema registry client: {e}")))?;
        let auth = username.map(|u| (u, password));
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            client,
            auth,
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

    /// Fetch (and cache) the schema document for a registry schema id.
    pub async fn schema_by_id(&self, id: i32) -> Result<String> {
        if let Some(s) = self.cache.get(&id) {
            return Ok(s.clone());
        }
        let resp = self
            .get(format!("{}/schemas/ids/{id}", self.base))
            .send()
            .await
            .map_err(up)?;
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
        let resp = self
            .get(format!("{}/subjects", self.base))
            .send()
            .await
            .map_err(up)?;
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
}

fn up(e: reqwest::Error) -> Error {
    Error::Upstream(format!("schema registry: {e}"))
}
