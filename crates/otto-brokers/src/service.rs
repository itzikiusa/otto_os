//! Brokers service — the façade the HTTP handlers call.
//!
//! Owns cluster CRUD (secrets → Keychain), a per-cluster `rdkafka` client pool
//! (evicted on edit/delete), the schema-registry clients, and the metrics
//! sampler state. Blocking librdkafka calls run on `spawn_blocking`; admin /
//! producer calls are awaited directly.

use crate::decode::{avro_payload, avro_to_json, confluent_frame, decode_payload};
use crate::kafka::{is_internal, KafkaClient, KafkaConnSpec};
use crate::metrics::{self, ClusterMetricState};
use crate::proxy::BrokerTunnel;
use crate::schema_registry::SchemaRegistry;
use crate::types::*;
use dashmap::DashMap;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_state::{
    BrokerAuditRepo, BrokerClusterRow, BrokerClusterSectionRow, BrokerClusterSectionsRepo,
    BrokerClustersRepo, NewBrokerCluster, UpdateBrokerCluster,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const POOL_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_CONSUME: usize = 5000;

struct Pooled {
    client: Arc<KafkaClient>,
    registry: Option<Arc<SchemaRegistry>>,
    /// Held so the SSH tunnel/proxy stays up for the life of the pooled client.
    tunnel: Option<Arc<BrokerTunnel>>,
    created: Instant,
}

pub struct BrokersService {
    repo: BrokerClustersRepo,
    sections: BrokerClusterSectionsRepo,
    secrets: Arc<dyn SecretStore>,
    pool: DashMap<Id, Pooled>,
    /// Per-cluster SSH tunnel + Kafka proxy (only for clusters with `ssh`).
    tunnels: DashMap<Id, Arc<BrokerTunnel>>,
    samplers: DashMap<Id, Mutex<ClusterMetricState>>,
    audit: Option<BrokerAuditRepo>,
}

fn secret_ref_for(id: &Id) -> String {
    format!("broker-{id}")
}
fn sr_secret_ref_for(id: &Id) -> String {
    format!("broker-sr-{id}")
}
fn join(e: tokio::task::JoinError) -> Error {
    Error::Internal(format!("broker task: {e}"))
}

impl BrokersService {
    pub fn new(
        repo: BrokerClustersRepo,
        sections: BrokerClusterSectionsRepo,
        secrets: Arc<dyn SecretStore>,
        audit: Option<BrokerAuditRepo>,
    ) -> Self {
        Self {
            repo,
            sections,
            secrets,
            pool: DashMap::new(),
            tunnels: DashMap::new(),
            samplers: DashMap::new(),
            audit,
        }
    }

    /// Persist an audit row for a broker write (best-effort; no-op without an
    /// audit repo). Called from the HTTP write handlers, which hold the
    /// authenticated `user_id`.
    pub(crate) async fn audit_write(
        &self,
        cluster_id: &Id,
        user_id: &Id,
        op: &str,
        detail: serde_json::Value,
    ) {
        if let Some(ref a) = self.audit {
            let _ = a.record(cluster_id, user_id, op, detail).await;
        }
    }

    // ---- cluster sections (sidebar grouping) ------------------------------

    pub async fn list_sections(&self, ws: &Id) -> Result<Vec<BrokerClusterSection>> {
        Ok(self
            .sections
            .list_for_ws(ws)
            .await?
            .into_iter()
            .map(row_to_section)
            .collect())
    }

    /// Fetch a section row (handlers need `workspace_id` for the role gate).
    pub async fn get_section(&self, id: &Id) -> Result<BrokerClusterSection> {
        Ok(row_to_section(self.sections.get(id).await?))
    }

    pub async fn create_section(
        &self,
        ws: &Id,
        created_by: &Id,
        parent_id: Option<&str>,
        name: &str,
    ) -> Result<BrokerClusterSection> {
        Ok(row_to_section(
            self.sections.create(ws, parent_id, name, created_by).await?,
        ))
    }

    pub async fn rename_section(&self, id: &Id, name: &str) -> Result<BrokerClusterSection> {
        Ok(row_to_section(self.sections.rename(id, name).await?))
    }

    pub async fn move_section(
        &self,
        id: &Id,
        parent_id: Option<&str>,
    ) -> Result<BrokerClusterSection> {
        Ok(row_to_section(self.sections.reparent(id, parent_id).await?))
    }

    pub async fn delete_section(&self, id: &Id) -> Result<()> {
        self.sections.delete(id).await
    }

    // ---- cluster CRUD -----------------------------------------------------

    pub async fn list_clusters(&self, ws: &Id) -> Result<Vec<BrokerCluster>> {
        Ok(self
            .repo
            .list_visible(ws)
            .await?
            .into_iter()
            .map(row_to_cluster)
            .collect())
    }

    /// Fetch the persisted row (handlers need `workspace_id` for the role gate).
    pub async fn get_row(&self, id: &Id) -> Result<BrokerClusterRow> {
        self.repo.get(id).await
    }

    pub async fn get_cluster(&self, id: &Id) -> Result<BrokerCluster> {
        Ok(row_to_cluster(self.repo.get(id).await?))
    }

    pub async fn create_cluster(
        &self,
        workspace_id: Option<Id>,
        created_by: Id,
        req: UpsertClusterReq,
    ) -> Result<BrokerCluster> {
        validate(&req)?;
        let row = self
            .repo
            .create(NewBrokerCluster {
                workspace_id,
                name: req.name.clone(),
                bootstrap_servers: req.bootstrap_servers.clone(),
                security_protocol: req.security_protocol.as_str().to_string(),
                sasl_mechanism: req.sasl_mechanism.map(|m| m.as_str().to_string()),
                sasl_username: norm(req.sasl_username.clone()),
                secret_ref: None,
                tls_skip_verify: req.tls_skip_verify.unwrap_or(false),
                schema_registry_url: norm(req.schema_registry_url.clone()),
                schema_registry_username: norm(req.schema_registry_username.clone()),
                sr_secret_ref: None,
                metrics_url: norm(req.metrics_url.clone()),
                color: norm(req.color.clone()),
                ssh_config: ssh_to_json(req.ssh.clone().flatten().as_ref()),
                section_id: req.section_id.clone().flatten(),
                environment: req.environment.unwrap_or_default().as_str().to_string(),
                read_only: req.read_only.unwrap_or(false),
                created_by,
            })
            .await?;

        // Stash secrets now that we have an id, then point the row at the refs.
        let mut secret_ref = None;
        let mut sr_secret_ref = None;
        if let Some(pw) = nonempty(req.sasl_password) {
            let r = secret_ref_for(&row.id);
            self.secrets.put(&r, &pw)?;
            secret_ref = Some(Some(r));
        }
        if let Some(pw) = nonempty(req.schema_registry_password) {
            let r = sr_secret_ref_for(&row.id);
            self.secrets.put(&r, &pw)?;
            sr_secret_ref = Some(Some(r));
        }
        if secret_ref.is_some() || sr_secret_ref.is_some() {
            let updated = self
                .repo
                .update(
                    &row.id,
                    full_update(&row, secret_ref, sr_secret_ref, None, None, None),
                )
                .await?;
            return Ok(row_to_cluster(updated));
        }
        Ok(row_to_cluster(row))
    }

    pub async fn update_cluster(&self, id: &Id, req: UpsertClusterReq) -> Result<BrokerCluster> {
        validate(&req)?;
        let row = self.repo.get(id).await?;

        // Secret handling: absent password = keep; non-empty = set; empty = clear.
        let secret_ref = self.secret_change(
            id,
            &secret_ref_for(id),
            row.secret_ref.clone(),
            req.sasl_password,
        )?;
        let sr_secret_ref = self.secret_change(
            id,
            &sr_secret_ref_for(id),
            row.sr_secret_ref.clone(),
            req.schema_registry_password,
        )?;

        let u = UpdateBrokerCluster {
            name: req.name,
            bootstrap_servers: req.bootstrap_servers,
            security_protocol: req.security_protocol.as_str().to_string(),
            sasl_mechanism: req.sasl_mechanism.map(|m| m.as_str().to_string()),
            sasl_username: norm(req.sasl_username),
            tls_skip_verify: req.tls_skip_verify.unwrap_or(row.tls_skip_verify),
            schema_registry_url: norm(req.schema_registry_url),
            schema_registry_username: norm(req.schema_registry_username),
            metrics_url: norm(req.metrics_url),
            color: norm(req.color),
            // absent ssh = keep; null = clear; object = set.
            ssh_config: req.ssh.map(|inner| ssh_to_json(inner.as_ref())),
            // absent section = keep; null = ungroup; id = file under section.
            section_id: req.section_id,
            secret_ref,
            sr_secret_ref,
            // absent environment/read_only PRESERVE the write-guard.
            environment: req.environment.map(|e| e.as_str().to_string()),
            read_only: req.read_only,
        };
        let updated = self.repo.update(id, u).await?;
        self.evict(id);
        Ok(row_to_cluster(updated))
    }

    pub async fn delete_cluster(&self, id: &Id) -> Result<()> {
        let row = self.repo.get(id).await?;
        if let Some(r) = &row.secret_ref {
            let _ = self.secrets.delete(r);
        }
        if let Some(r) = &row.sr_secret_ref {
            let _ = self.secrets.delete(r);
        }
        self.repo.delete(id).await?;
        self.evict(id);
        self.samplers.remove(id);
        Ok(())
    }

    /// Apply a secret change and return the repo three-state ref update.
    fn secret_change(
        &self,
        id: &Id,
        key: &str,
        existing: Option<String>,
        provided: Option<String>,
    ) -> Result<Option<Option<String>>> {
        match provided {
            None => Ok(None), // keep
            Some(pw) if pw.is_empty() => {
                // explicit clear
                if let Some(r) = &existing {
                    let _ = self.secrets.delete(r);
                }
                Ok(Some(None))
            }
            Some(pw) => {
                let _ = id;
                self.secrets.put(key, &pw)?;
                Ok(Some(Some(key.to_string())))
            }
        }
    }

    fn evict(&self, id: &Id) {
        self.pool.remove(id);
        // Dropping the tunnel tears down the ssh child + local listeners.
        self.tunnels.remove(id);
    }

    // ---- connection pool --------------------------------------------------

    /// Get-or-open the cached SSH tunnel + Kafka proxy for a cluster.
    async fn tunnel_for(
        &self,
        id: &Id,
        ssh: &SshTunnelConfig,
        bootstrap: &str,
        uses_tls: bool,
        skip_verify: bool,
    ) -> Result<Arc<BrokerTunnel>> {
        if let Some(t) = self.tunnels.get(id) {
            if t.is_alive() {
                return Ok(t.clone());
            }
        }
        let t = Arc::new(BrokerTunnel::open(ssh, bootstrap, uses_tls, skip_verify).await?);
        self.tunnels.insert(id.clone(), t.clone());
        Ok(t)
    }

    /// Resolve a cluster row into a librdkafka spec + schema-registry client,
    /// establishing the SSH tunnel first when the profile carries one. With a
    /// tunnel, librdkafka talks plaintext to the local proxy (TLS/SASL ride
    /// through it), and the registry/metrics clients use the SOCKS proxy.
    async fn prepare(
        &self,
        row: &BrokerClusterRow,
    ) -> Result<(
        KafkaConnSpec,
        Option<Arc<SchemaRegistry>>,
        Option<Arc<BrokerTunnel>>,
    )> {
        let sasl_password = match &row.secret_ref {
            Some(r) => self.secrets.get(r)?,
            None => None,
        };
        let security = SecurityProtocol::parse(&row.security_protocol).unwrap_or_default();
        let mut spec = KafkaConnSpec {
            bootstrap_servers: row.bootstrap_servers.clone(),
            security_protocol: security,
            sasl_mechanism: row.sasl_mechanism.as_deref().and_then(SaslMechanism::parse),
            sasl_username: row.sasl_username.clone(),
            sasl_password,
            tls_skip_verify: row.tls_skip_verify,
        };

        let ssh: Option<SshTunnelConfig> = row
            .ssh_config
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());

        let (tunnel, socks_url) = match &ssh {
            Some(cfg) => {
                let t = self
                    .tunnel_for(
                        &row.id,
                        cfg,
                        &row.bootstrap_servers,
                        security.uses_tls(),
                        row.tls_skip_verify,
                    )
                    .await?;
                // librdkafka connects in plaintext to the local proxy; the proxy
                // performs the real TLS to the broker and relays SASL untouched.
                spec.bootstrap_servers = t.local_bootstrap();
                spec.security_protocol = local_security(security);
                spec.tls_skip_verify = false;
                let url = t.socks_url();
                (Some(t), Some(url))
            }
            None => (None, None),
        };

        let registry = match &row.schema_registry_url {
            Some(url) if !url.is_empty() => {
                let pw = match &row.sr_secret_ref {
                    Some(r) => self.secrets.get(r)?,
                    None => None,
                };
                Some(Arc::new(SchemaRegistry::new(
                    url,
                    row.schema_registry_username.clone(),
                    pw,
                    row.tls_skip_verify,
                    socks_url.clone(),
                )?))
            }
            _ => None,
        };
        Ok((spec, registry, tunnel))
    }

    /// Pooled client for a cluster (reconnects if missing, stale, or its tunnel
    /// has died).
    async fn client_for(&self, id: &Id) -> Result<(Arc<KafkaClient>, Option<Arc<SchemaRegistry>>)> {
        if let Some(p) = self.pool.get(id) {
            let tunnel_ok = p.tunnel.as_ref().is_none_or(|t| t.is_alive());
            if p.created.elapsed() < POOL_TTL && tunnel_ok {
                return Ok((p.client.clone(), p.registry.clone()));
            }
        }
        let row = self.repo.get(id).await?;
        let (spec, registry, tunnel) = self.prepare(&row).await?;
        let client = Arc::new(
            tokio::task::spawn_blocking(move || KafkaClient::connect(&spec))
                .await
                .map_err(join)??,
        );
        self.pool.insert(
            id.clone(),
            Pooled {
                client: client.clone(),
                registry: registry.clone(),
                tunnel,
                created: Instant::now(),
            },
        );
        Ok((client, registry))
    }

    // ---- operations -------------------------------------------------------

    pub async fn test(&self, id: &Id) -> Result<TestClusterResp> {
        let row = self.repo.get(id).await?;
        // `_tunnel` keeps the SSH proxy alive across the blocking connect/test.
        let (spec, _registry, _tunnel) = self.prepare(&row).await?;
        // Transient client so a bad config never poisons the pool.
        let res =
            tokio::task::spawn_blocking(move || KafkaClient::connect(&spec).and_then(|c| c.test()))
                .await
                .map_err(join)?;
        match res {
            Ok(r) => Ok(r),
            Err(e) => Ok(TestClusterResp {
                ok: false,
                latency_ms: 0,
                message: e.to_string(),
                broker_count: 0,
            }),
        }
    }

    pub async fn overview(&self, id: &Id) -> Result<ClusterOverview> {
        let (client, _) = self.client_for(id).await?;
        let mut ov = tokio::task::spawn_blocking(move || client.overview())
            .await
            .map_err(join)??;
        // Over a tunnel the metadata is rewritten to 127.0.0.1:<local>; show the
        // real broker addresses by reverse-mapping through the proxy.
        if let Some(t) = self.tunnels.get(id) {
            for b in &mut ov.brokers {
                if let Some((host, port)) = t.real_endpoint(b.port as u16) {
                    b.host = host;
                    b.port = port as i32;
                }
            }
        }
        Ok(ov)
    }

    pub async fn list_topics(&self, id: &Id) -> Result<Vec<TopicSummary>> {
        let (client, _) = self.client_for(id).await?;
        tokio::task::spawn_blocking(move || client.list_topics())
            .await
            .map_err(join)?
    }

    pub async fn topic_detail(&self, id: &Id, topic: &str) -> Result<TopicDetail> {
        let (client, _) = self.client_for(id).await?;
        let t = topic.to_string();
        let (partitions, message_count) = {
            let client = client.clone();
            let t = t.clone();
            tokio::task::spawn_blocking(move || client.topic_partitions(&t))
                .await
                .map_err(join)??
        };
        let configs = client.topic_configs(&t).await?;
        Ok(TopicDetail {
            name: t.clone(),
            internal: is_internal(&t),
            partitions,
            configs,
            message_count,
        })
    }

    /// Lazily-loaded stats for one topic (message count + cleanup policy). The
    /// topic list is metadata-only; the UI calls this per row in the background.
    pub async fn topic_stats(&self, id: &Id, topic: &str) -> Result<TopicStats> {
        let (client, _) = self.client_for(id).await?;
        let count = {
            let client = client.clone();
            let t = topic.to_string();
            tokio::task::spawn_blocking(move || client.topic_message_count(&t))
                .await
                .map_err(join)??
        };
        // Cleanup policy comes from topic config; some clusters/users can't read
        // configs (e.g. MSK without DESCRIBE_CONFIGS) — degrade gracefully.
        let cleanup_policy = client
            .topic_configs(topic)
            .await
            .ok()
            .and_then(|cfgs| {
                cfgs.into_iter()
                    .find(|c| c.name == "cleanup.policy")
                    .and_then(|c| c.value)
            });
        Ok(TopicStats {
            message_count: count,
            cleanup_policy,
        })
    }

    /// Batch-load message counts for multiple topics in parallel, reusing the
    /// same shared `KafkaClient` (thread-safe). This replaces N individual
    /// `/topics/{name}/stats` calls with a single HTTP round-trip from the UI.
    /// Cleanup policy is best-effort (skipped on DESCRIBE_CONFIGS permission errors).
    pub async fn batch_topic_stats(
        &self,
        id: &Id,
        names: Vec<String>,
    ) -> Result<std::collections::HashMap<String, TopicStats>> {
        if names.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let (client, _) = self.client_for(id).await?;
        // Fan-out message counts using the existing WATERMARK_WORKERS pool.
        let counts: std::collections::HashMap<String, i64> = {
            let client = client.clone();
            let ns = names.clone();
            tokio::task::spawn_blocking(move || {
                use std::sync::atomic::{AtomicUsize, Ordering};
                use std::sync::Mutex;
                let results = Mutex::new(std::collections::HashMap::new());
                let next = AtomicUsize::new(0);
                let workers = crate::kafka::WATERMARK_WORKERS.min(ns.len());
                std::thread::scope(|s| {
                    for _ in 0..workers {
                        s.spawn(|| loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some(name) = ns.get(i) else { break };
                            match client.topic_message_count(name) {
                                Ok(n) => {
                                    results.lock().unwrap().insert(name.clone(), n);
                                }
                                Err(_) => {
                                    results.lock().unwrap().insert(name.clone(), -1);
                                }
                            }
                        });
                    }
                });
                results.into_inner().unwrap()
            })
            .await
            .map_err(join)?
        };

        // Fetch cleanup policies (best-effort; fail silently per topic).
        let mut out = std::collections::HashMap::with_capacity(names.len());
        for name in &names {
            let count = counts.get(name).copied().unwrap_or(-1);
            // Only fetch config if the count succeeded (avoids double-erroring on
            // permission issues where the cluster can't be reached at all).
            let cleanup_policy = if count >= 0 {
                client
                    .topic_configs(name)
                    .await
                    .ok()
                    .and_then(|cfgs| {
                        cfgs.into_iter()
                            .find(|c| c.name == "cleanup.policy")
                            .and_then(|c| c.value)
                    })
            } else {
                None
            };
            out.insert(
                name.clone(),
                TopicStats {
                    message_count: count,
                    cleanup_policy,
                },
            );
        }
        Ok(out)
    }

    pub async fn topic_configs(&self, id: &Id, topic: &str) -> Result<Vec<TopicConfigEntry>> {
        let (client, _) = self.client_for(id).await?;
        client.topic_configs(topic).await
    }

    pub async fn alter_configs(
        &self,
        id: &Id,
        topic: &str,
        kvs: &[ConfigKv],
    ) -> Result<Vec<TopicConfigEntry>> {
        let (client, _) = self.client_for(id).await?;
        client.alter_topic_configs(topic, kvs).await?;
        client.topic_configs(topic).await
    }

    pub async fn create_topic(&self, id: &Id, req: &CreateTopicReq) -> Result<TopicSummary> {
        let (client, _) = self.client_for(id).await?;
        client.create_topic(req).await?;
        Ok(TopicSummary {
            name: req.name.clone(),
            partitions: req.partitions.max(1) as usize,
            replication_factor: req.replication_factor.max(1) as usize,
            message_count: 0,
            cleanup_policy: None,
            internal: is_internal(&req.name),
        })
    }

    pub async fn delete_topic(&self, id: &Id, topic: &str) -> Result<()> {
        let (client, _) = self.client_for(id).await?;
        client.delete_topic(topic).await
    }

    pub async fn produce(&self, id: &Id, topic: &str, req: &ProduceReq) -> Result<ProduceResp> {
        let (client, _) = self.client_for(id).await?;
        client.produce(topic, req).await
    }

    pub async fn list_groups(&self, id: &Id) -> Result<Vec<GroupSummary>> {
        let (client, _) = self.client_for(id).await?;
        tokio::task::spawn_blocking(move || client.list_groups())
            .await
            .map_err(join)?
    }

    pub async fn describe_group(&self, id: &Id, group: &str) -> Result<GroupDetail> {
        let (client, _) = self.client_for(id).await?;
        let g = group.to_string();
        tokio::task::spawn_blocking(move || client.describe_group(&g))
            .await
            .map_err(join)?
    }

    /// Reset consumer-group committed offsets. The group's current committed
    /// offsets are fetched first to discover which topic-partitions to reset,
    /// then each offset is resolved per the requested mode and written back via
    /// `commit_offsets`. Guarded (prod/read-only) clusters require `confirm=true`;
    /// the HTTP handler enforces that before calling here.
    pub async fn reset_group_offsets(
        &self,
        id: &Id,
        group: &str,
        req: &crate::types::GroupResetReq,
    ) -> Result<GroupDetail> {
        let (client, _) = self.client_for(id).await?;
        let g = group.to_string();
        let mode = req.mode.clone();
        let ts_ms = req.timestamp_ms;
        let topic_filter = req.topic.clone();

        // Fetch existing committed offsets to discover the set of partitions to reset.
        let detail = {
            let client = client.clone();
            let g2 = g.clone();
            tokio::task::spawn_blocking(move || client.describe_group(&g2))
                .await
                .map_err(join)??
        };

        // Build (topic, partition) → target offset map.
        let positions: std::collections::HashMap<(String, i32), i64> = {
            let client = client.clone();
            let offsets = detail.offsets.clone();
            let filter = topic_filter.clone();
            tokio::task::spawn_blocking(move || {
                let mut map = std::collections::HashMap::new();
                for o in &offsets {
                    if let Some(f) = &filter {
                        if &o.topic != f {
                            continue;
                        }
                    }
                    let resolved = client.resolve_offset(&o.topic, o.partition, &mode, ts_ms)?;
                    map.insert((o.topic.clone(), o.partition), resolved);
                }
                Ok::<_, Error>(map)
            })
            .await
            .map_err(join)??
        };

        // Write the offsets.
        {
            let client = client.clone();
            let g3 = g.clone();
            tokio::task::spawn_blocking(move || client.reset_offsets(&g3, positions))
                .await
                .map_err(join)??;
        }

        // Return updated detail.
        let client2 = client.clone();
        let g4 = g.clone();
        tokio::task::spawn_blocking(move || client2.describe_group(&g4))
            .await
            .map_err(join)?
    }

    pub async fn schema_subjects(&self, id: &Id) -> Result<Vec<SchemaSubject>> {
        let (_, registry) = self.client_for(id).await?;
        match registry {
            Some(reg) => reg.subjects().await,
            None => Err(Error::Invalid(
                "no schema registry configured for this cluster".into(),
            )),
        }
    }

    pub async fn consume(&self, id: &Id, topic: &str, req: &ConsumeReq) -> Result<ConsumeResp> {
        let (client, registry) = self.client_for(id).await?;
        let raw = {
            let client = client.clone();
            let t = topic.to_string();
            let req2 = ConsumeReq {
                limit: req.limit.clamp(1, MAX_CONSUME),
                ..req.clone()
            };
            tokio::task::spawn_blocking(move || client.consume_raw(&t, &req2))
                .await
                .map_err(join)??
        };

        let key_filter = req.key_filter.as_ref().map(|s| s.to_lowercase());
        let value_filter = req.value_filter.as_ref().map(|s| s.to_lowercase());

        let mut messages = Vec::with_capacity(raw.messages.len());
        for m in raw.messages {
            let key = self
                .decode_kv(m.key.as_deref(), ValueFormat::Auto, &registry)
                .await;
            let value = self
                .decode_kv(m.value.as_deref(), req.decode, &registry)
                .await;
            if let Some(f) = &key_filter {
                if !key
                    .as_ref()
                    .is_some_and(|k| k.text.to_lowercase().contains(f))
                {
                    continue;
                }
            }
            if let Some(f) = &value_filter {
                if !value
                    .as_ref()
                    .is_some_and(|v| v.text.to_lowercase().contains(f))
                {
                    continue;
                }
            }
            messages.push(KafkaMessage {
                partition: m.partition,
                offset: m.offset,
                timestamp_ms: m.timestamp_ms,
                key,
                value,
                headers: m
                    .headers
                    .into_iter()
                    .map(|(k, v)| MessageHeader {
                        key: k,
                        value: String::from_utf8_lossy(&v).into_owned(),
                    })
                    .collect(),
                size_bytes: m.size,
            });
        }
        Ok(ConsumeResp {
            messages,
            partitions: raw.partitions,
            truncated: raw.truncated,
        })
    }

    /// Decode a key/value, consulting the schema registry for Confluent-framed
    /// Avro when `Auto`/`Avro` is requested.
    async fn decode_kv(
        &self,
        bytes: Option<&[u8]>,
        fmt: ValueFormat,
        registry: &Option<Arc<SchemaRegistry>>,
    ) -> Option<DecodedPayload> {
        let bytes = bytes?;
        if matches!(fmt, ValueFormat::Auto | ValueFormat::Avro) {
            if let (Some(reg), Some((schema_id, body))) = (registry, confluent_frame(bytes)) {
                if let Ok(schema) = reg.schema_by_id(schema_id).await {
                    if let Ok(v) = avro_to_json(&schema, body) {
                        return Some(avro_payload(&v, schema_id, bytes));
                    }
                }
            }
        }
        Some(decode_payload(Some(bytes), fmt))
    }

    pub async fn metrics(&self, id: &Id) -> Result<ClusterMetrics> {
        let row = self.repo.get(id).await?;
        let (client, _) = self.client_for(id).await?;

        // Check if the cached watermark total has expired; if so, re-sweep.
        // The sweep is expensive over a tunnel (hundreds of ListOffsets round-trips),
        // so we skip it when the cached value is still fresh (< 8 s).
        let needs_sweep = self
            .samplers
            .entry(id.clone())
            .or_insert_with(|| Mutex::new(ClusterMetricState::default()))
            .lock()
            .map_err(|_| Error::Internal("metrics lock".into()))?
            .needs_sweep();

        let total = if needs_sweep {
            let swept = tokio::task::spawn_blocking(move || client.total_messages())
                .await
                .map_err(join)??;
            // Store fresh total; hold the ref in a named binding to extend lifetime.
            let entry = self
                .samplers
                .entry(id.clone())
                .or_insert_with(|| Mutex::new(ClusterMetricState::default()));
            entry
                .lock()
                .map_err(|_| Error::Internal("metrics lock".into()))?
                .store_watermark(swept);
            swept
        } else {
            self.samplers
                .get(id)
                .and_then(|e| e.lock().ok().and_then(|g| g.cached_total()))
                .unwrap_or(0)
        };

        // Reach a private metrics endpoint through the same SSH SOCKS tunnel.
        let socks = self.tunnels.get(id).map(|t| t.socks_url());
        let prom = match &row.metrics_url {
            Some(url) if !url.is_empty() => {
                metrics::scrape(url, row.tls_skip_verify, socks.as_deref())
                    .await
                    .ok()
            }
            _ => None,
        };

        let entry = self
            .samplers
            .entry(id.clone())
            .or_insert_with(|| Mutex::new(ClusterMetricState::default()));
        let mut guard = entry
            .lock()
            .map_err(|_| Error::Internal("metrics lock".into()))?;
        Ok(guard.build(total, prom.as_deref()))
    }
}

// ---- helpers --------------------------------------------------------------

fn row_to_cluster(r: BrokerClusterRow) -> BrokerCluster {
    BrokerCluster {
        id: r.id,
        workspace_id: r.workspace_id,
        name: r.name,
        bootstrap_servers: r.bootstrap_servers,
        security_protocol: SecurityProtocol::parse(&r.security_protocol).unwrap_or_default(),
        sasl_mechanism: r.sasl_mechanism.as_deref().and_then(SaslMechanism::parse),
        sasl_username: r.sasl_username,
        has_sasl_password: r.secret_ref.is_some(),
        tls_skip_verify: r.tls_skip_verify,
        schema_registry_url: r.schema_registry_url,
        schema_registry_username: r.schema_registry_username,
        has_sr_password: r.sr_secret_ref.is_some(),
        metrics_url: r.metrics_url,
        color: r.color,
        ssh: r
            .ssh_config
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok()),
        section_id: r.section_id,
        environment: otto_core::domain::Environment::parse(&r.environment).unwrap_or_default(),
        read_only: r.read_only,
        created_by: r.created_by,
        created_at: r.created_at,
    }
}

/// Map a persisted section row to the API domain type.
fn row_to_section(r: BrokerClusterSectionRow) -> BrokerClusterSection {
    BrokerClusterSection {
        id: r.id,
        workspace_id: r.workspace_id,
        parent_id: r.parent_id,
        name: r.name,
        position: r.position,
        created_by: r.created_by,
        created_at: r.created_at,
    }
}

/// Build a full update from an existing row, overriding only secrets/guards.
#[allow(clippy::too_many_arguments)]
fn full_update(
    row: &BrokerClusterRow,
    secret_ref: Option<Option<String>>,
    sr_secret_ref: Option<Option<String>>,
    ssh_config: Option<Option<String>>,
    environment: Option<String>,
    read_only: Option<bool>,
) -> UpdateBrokerCluster {
    UpdateBrokerCluster {
        name: row.name.clone(),
        bootstrap_servers: row.bootstrap_servers.clone(),
        security_protocol: row.security_protocol.clone(),
        sasl_mechanism: row.sasl_mechanism.clone(),
        sasl_username: row.sasl_username.clone(),
        tls_skip_verify: row.tls_skip_verify,
        schema_registry_url: row.schema_registry_url.clone(),
        schema_registry_username: row.schema_registry_username.clone(),
        metrics_url: row.metrics_url.clone(),
        color: row.color.clone(),
        ssh_config,
        section_id: None, // preserved (this path only stashes secrets)
        secret_ref,
        sr_secret_ref,
        environment,
        read_only,
    }
}

/// Serialize a tunnel config to the JSON we persist (None → no tunnel).
fn ssh_to_json(ssh: Option<&SshTunnelConfig>) -> Option<String> {
    ssh.and_then(|c| serde_json::to_string(c).ok())
}

/// The security protocol librdkafka uses on the local (loopback) hop to the
/// proxy: TLS is stripped because the proxy terminates it to the real broker,
/// but SASL is kept so the SASL/SCRAM handshake still runs end-to-end.
fn local_security(p: SecurityProtocol) -> SecurityProtocol {
    match p {
        SecurityProtocol::SaslSsl => SecurityProtocol::SaslPlaintext,
        SecurityProtocol::Ssl => SecurityProtocol::Plaintext,
        other => other,
    }
}

fn validate(req: &UpsertClusterReq) -> Result<()> {
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("name is required".into()));
    }
    if req.bootstrap_servers.trim().is_empty() {
        return Err(Error::Invalid("bootstrap_servers is required".into()));
    }
    if req.security_protocol.uses_sasl() && norm(req.sasl_username.clone()).is_none() {
        return Err(Error::Invalid(
            "sasl_username is required for SASL security protocols".into(),
        ));
    }
    Ok(())
}

/// Trim to None when empty/blank.
fn norm(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}
