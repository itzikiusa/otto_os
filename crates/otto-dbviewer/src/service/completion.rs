//! Authorized completion snapshots, selected Mongo fields, and cache refresh.
use super::*;

impl DbViewerService {
    pub async fn completion(
        &self,
        conn_id: &Id,
        user_id: &Id,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        let child = crate::access::child(ctx.database.as_deref().or(ctx.node.as_deref()));
        // One request per word typed: reuse a recent Keychain read.
        let r = self
            .resolve_with(
                conn_id,
                user_id,
                child.as_deref(),
                "db_browse",
                SecretRead::CompletionCached,
            )
            .await?;
        if self.is_enforced(conn_id).await? {
            let schema = child.ok_or_else(|| {
                Error::Forbidden(
                    "select an authorized database before requesting completion".into(),
                )
            })?;
            // The access-scoped schema graph (only what this user may browse),
            // cached per (connection, user, schema) with a short TTL — it used
            // to be rebuilt on EVERY request, one `object_detail` round-trip per
            // table per word typed. Built in a detached task (see below) and
            // single-flight, then ranked by the engine's own assembler so an
            // enforced connection completes like an unrestricted one: tables
            // after FROM, in-scope columns PK-first after WHERE, keywords.
            let svc = self.clone();
            let (cid, uid) = (conn_id.clone(), user_id.clone());
            let collection = (r.config.engine == Engine::Mongodb)
                .then(|| metadata::mongo_completion_collection(ctx))
                .flatten();
            let config_key = r.config.cache_key();
            let snap = tokio::spawn(async move {
                let scope = format!("{uid}\0{schema}");
                let snapshot = svc
                    .enforced_completions
                    .snapshot_or_build(cid.as_str(), &scope, || async {
                        svc.schema_graph(&cid, &uid, &schema, ENFORCED_COMPLETION_MAX_TABLES)
                            .await
                            .ok()
                            .map(|graph| crate::complete::snapshot_from_graph(&graph))
                    })
                    .await;
                // The bulk Mongo graph is intentionally top-level only. Keep
                // that shared snapshot; cache just the selected collection's
                // nested fields separately, under its resolved credential scope.
                let Some(collection) = collection else {
                    return snapshot;
                };
                let Some(object) = snapshot
                    .objects
                    .iter()
                    .find(|object| object.name == collection)
                else {
                    return snapshot;
                };
                let fields_scope = format!("{uid}\0{config_key}\0{schema}");
                let fields = svc
                    .enforced_completions
                    .fields_or_build(cid.as_str(), &fields_scope, &collection, || async {
                        let path = NodePath::parse(&format!("db:{schema}"))
                            .child("coll", &collection)
                            .to_id();
                        match svc.object_detail(&cid, &uid, &path, false).await {
                            Ok(detail) => metadata::mongo_completion_fields(&detail),
                            Err(_) => Vec::new(),
                        }
                    })
                    .await;
                // A field slot only needs this collection. Do not clone/cache
                // every other collection's columns for every word typed.
                Arc::new(crate::complete::SchemaSnapshot {
                    databases: snapshot.databases.clone(),
                    objects: vec![crate::complete::ObjectSnap {
                        name: object.name.clone(),
                        kind: object.kind,
                        fields: (*fields).clone(),
                        fields_ready: true,
                    }],
                    routines: Vec::new(),
                })
            })
            .await
            .map_err(|e| Error::Internal(format!("completion task failed: {e}")))?;
            let mut resp = CompletionResponse {
                items: r.driver.assemble_completion(&snap, ctx),
                ..Default::default()
            };
            crate::complete::finalize(&mut resp, &ctx.prefix);
            return Ok(resp);
        }
        // Detached from the request: the editor aborts a superseded completion
        // fetch on the next keystroke, which drops this handler future. The
        // first request for a connection also pays the schema-snapshot build
        // (seconds on a remote DB); run in the request's future it was
        // cancelled part-way on every keystroke and nothing was ever cached,
        // so completion stayed empty until the user paused for the whole build.
        // A spawned task finishes (and caches) the build even when nobody waits
        // for the answer; later requests wait on its single-flight gate.
        let prefix = ctx.prefix.clone();
        let ctx = ctx.clone();
        let mut resp =
            tokio::spawn(
                async move { r.with_lifecycle(r.driver.completion(&r.config, &ctx)).await },
            )
            .await
            .map_err(|e| Error::Internal(format!("completion task failed: {e}")))??;
        // Bound what one keystroke ships: only items that can match the typed
        // word, at most MAX_COMPLETION_ITEMS of them (`truncated` past that).
        crate::complete::finalize(&mut resp, &prefix);
        Ok(resp)
    }

    /// Drop the cached completion snapshot for a connection so the next
    /// completion re-introspects the live schema. Backs the UI "Refresh schema"
    /// action — keeping smart completion in sync with a schema the user just
    /// changed — and is a no-op for engines without a snapshot cache (Redis).
    pub async fn refresh_completion_cache(&self, conn_id: &Id, user_id: &Id) -> Result<()> {
        let child = crate::access::child(None);
        let r = self
            .resolve(conn_id, user_id, child.as_deref(), "discover")
            .await?;
        r.driver.invalidate_completion_cache(&r.config).await;
        self.enforced_completions.invalidate(conn_id.as_str());
        self.graphs.invalidate_prefix(&format!("{conn_id}\0"));
        Ok(())
    }
}
