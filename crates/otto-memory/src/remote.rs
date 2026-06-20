//! Remote memory backend: forward memory operations to a shared host Otto over
//! its HTTP API. This is how a team shares ONE memory across many machines —
//! every member's Otto points at the same host (which owns the SQLite, so there
//! is a single writer and no unsafe shared-file access). Each member
//! authenticates to the host as themselves, so `shared`/`private` visibility is
//! enforced per-user on the host.

use otto_core::{Error, Result};
use otto_state::memory::ListFilter;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;

use crate::types::*;

pub struct RemoteClient {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl RemoteClient {
    pub fn new(base_url: String, token: String) -> Self {
        Self {
            base: base_url.trim_end_matches('/').to_string(),
            token,
            http: reqwest::Client::new(),
        }
    }

    fn url(&self, ws: &str, suffix: &str) -> String {
        format!("{}/api/v1/workspaces/{ws}/{suffix}", self.base)
    }

    async fn get_json<R: DeserializeOwned>(&self, url: String) -> Result<R> {
        let resp = self
            .http
            .get(url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("remote memory: {e}")))?;
        decode(resp).await
    }

    async fn body_json<B: Serialize, R: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        url: String,
        body: &B,
    ) -> Result<R> {
        let resp = self
            .http
            .request(method, url)
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("remote memory: {e}")))?;
        decode(resp).await
    }

    pub async fn save(&self, ws: &str, items: Vec<NewMemory>) -> Result<Vec<Memory>> {
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            let m: Memory = self
                .body_json(reqwest::Method::POST, self.url(ws, "memories"), &it)
                .await?;
            out.push(m);
        }
        Ok(out)
    }

    pub async fn get(&self, ws: &str, id: &str) -> Result<Memory> {
        self.get_json(self.url(ws, &format!("memories/{id}"))).await
    }

    pub async fn list(&self, ws: &str, f: &ListFilter) -> Result<Vec<Memory>> {
        let mut q: Vec<(String, String)> = Vec::new();
        if let Some(c) = &f.collection {
            q.push(("collection".into(), c.clone()));
        }
        if let Some(k) = &f.kind {
            q.push(("kind".into(), k.clone()));
        }
        if let Some(s) = &f.story_id {
            q.push(("story_id".into(), s.clone()));
        }
        if f.limit > 0 {
            q.push(("limit".into(), f.limit.to_string()));
        }
        let resp = self
            .http
            .get(self.url(ws, "memories"))
            .bearer_auth(&self.token)
            .query(&q)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("remote memory: {e}")))?;
        decode(resp).await
    }

    pub async fn update(&self, ws: &str, id: &str, p: &MemoryPatch) -> Result<Memory> {
        self.body_json(
            reqwest::Method::PATCH,
            self.url(ws, &format!("memories/{id}")),
            p,
        )
        .await
    }

    pub async fn forget(&self, ws: &str, id: &str) -> Result<()> {
        let resp = self
            .http
            .delete(self.url(ws, &format!("memories/{id}")))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("remote memory: {e}")))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(Error::Upstream(format!("remote memory delete {}", resp.status())))
        }
    }

    pub async fn search(&self, ws: &str, q: &MemoryQuery) -> Result<Vec<MemoryHit>> {
        self.body_json(reqwest::Method::POST, self.url(ws, "memory/search"), q)
            .await
    }

    pub async fn recall_brief(
        &self,
        ws: &str,
        story: &str,
        opts: &RecallOpts,
    ) -> Result<RecallBrief> {
        let body = json!({
            "story_id": story,
            "focus": opts.focus,
            "token_budget": opts.token_budget,
        });
        self.body_json(reqwest::Method::POST, self.url(ws, "memory/recall"), &body)
            .await
    }

    pub async fn links(&self, ws: &str, id: &str) -> Result<Vec<MemoryLink>> {
        self.get_json(self.url(ws, &format!("memories/{id}/links")))
            .await
    }
}

async fn decode<R: DeserializeOwned>(resp: reqwest::Response) -> Result<R> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Upstream(format!("remote memory {status}: {body}")));
    }
    resp.json::<R>()
        .await
        .map_err(|e| Error::Upstream(format!("remote memory decode: {e}")))
}
