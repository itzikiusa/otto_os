//! Broker (Kafka) cluster profiles repository.
//!
//! Self-contained: returns a plain [`BrokerClusterRow`] (the `otto-brokers`
//! service maps it to its own domain type, so `otto-state` keeps no dependency
//! on the Kafka crate). Secrets are never stored here — only Keychain refs.

use chrono::{DateTime, Utc};
use otto_core::{new_id, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct BrokerClustersRepo {
    pool: SqlitePool,
}

/// A persisted cluster row (string-typed enums; the service parses them).
#[derive(Debug, Clone)]
pub struct BrokerClusterRow {
    pub id: Id,
    pub workspace_id: Option<Id>,
    pub name: String,
    pub bootstrap_servers: String,
    pub security_protocol: String,
    pub sasl_mechanism: Option<String>,
    pub sasl_username: Option<String>,
    pub secret_ref: Option<String>,
    pub tls_skip_verify: bool,
    pub schema_registry_url: Option<String>,
    pub schema_registry_username: Option<String>,
    pub sr_secret_ref: Option<String>,
    pub metrics_url: Option<String>,
    pub color: Option<String>,
    pub environment: String,
    pub read_only: bool,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

/// Fields for a new cluster (id/created_at are generated).
pub struct NewBrokerCluster {
    pub workspace_id: Option<Id>,
    pub name: String,
    pub bootstrap_servers: String,
    pub security_protocol: String,
    pub sasl_mechanism: Option<String>,
    pub sasl_username: Option<String>,
    pub secret_ref: Option<String>,
    pub tls_skip_verify: bool,
    pub schema_registry_url: Option<String>,
    pub schema_registry_username: Option<String>,
    pub sr_secret_ref: Option<String>,
    pub metrics_url: Option<String>,
    pub color: Option<String>,
    pub environment: String,
    pub read_only: bool,
    pub created_by: Id,
}

/// Update payload. Plain fields are always written (the edit form sends the full
/// current state); the three-state options handle secrets/guard semantics:
/// `None` = keep, `Some(None)` = clear, `Some(Some(v))` = set.
pub struct UpdateBrokerCluster {
    pub name: String,
    pub bootstrap_servers: String,
    pub security_protocol: String,
    pub sasl_mechanism: Option<String>,
    pub sasl_username: Option<String>,
    pub tls_skip_verify: bool,
    pub schema_registry_url: Option<String>,
    pub schema_registry_username: Option<String>,
    pub metrics_url: Option<String>,
    pub color: Option<String>,
    pub secret_ref: Option<Option<String>>,
    pub sr_secret_ref: Option<Option<String>>,
    pub environment: Option<String>,
    pub read_only: Option<bool>,
}

fn row_to_cluster(r: &sqlx::sqlite::SqliteRow) -> Result<BrokerClusterRow> {
    Ok(BrokerClusterRow {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        name: r.get("name"),
        bootstrap_servers: r.get("bootstrap_servers"),
        security_protocol: r.get("security_protocol"),
        sasl_mechanism: r.get("sasl_mechanism"),
        sasl_username: r.get("sasl_username"),
        secret_ref: r.get("secret_ref"),
        tls_skip_verify: r.get::<i64, _>("tls_skip_verify") != 0,
        schema_registry_url: r.get("schema_registry_url"),
        schema_registry_username: r.get("schema_registry_username"),
        sr_secret_ref: r.get("sr_secret_ref"),
        metrics_url: r.get("metrics_url"),
        color: r.get("color"),
        environment: r.get("environment"),
        read_only: r.get::<i64, _>("read_only") != 0,
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
    })
}

impl BrokerClustersRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, c: NewBrokerCluster) -> Result<BrokerClusterRow> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO broker_clusters (
                id, workspace_id, name, bootstrap_servers, security_protocol,
                sasl_mechanism, sasl_username, secret_ref, tls_skip_verify,
                schema_registry_url, schema_registry_username, sr_secret_ref,
                metrics_url, color, environment, read_only, created_by, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&c.workspace_id)
        .bind(&c.name)
        .bind(&c.bootstrap_servers)
        .bind(&c.security_protocol)
        .bind(&c.sasl_mechanism)
        .bind(&c.sasl_username)
        .bind(&c.secret_ref)
        .bind(i64::from(c.tls_skip_verify))
        .bind(&c.schema_registry_url)
        .bind(&c.schema_registry_username)
        .bind(&c.sr_secret_ref)
        .bind(&c.metrics_url)
        .bind(&c.color)
        .bind(&c.environment)
        .bind(i64::from(c.read_only))
        .bind(&c.created_by)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create broker cluster"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<BrokerClusterRow> {
        let r = sqlx::query("SELECT * FROM broker_clusters WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("broker cluster"))?;
        row_to_cluster(&r)
    }

    /// Clusters visible to a workspace: its own plus global (NULL workspace).
    pub async fn list_visible(&self, ws: &Id) -> Result<Vec<BrokerClusterRow>> {
        let rows = sqlx::query(
            "SELECT * FROM broker_clusters WHERE workspace_id = ? OR workspace_id IS NULL
             ORDER BY name",
        )
        .bind(ws)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("broker clusters"))?;
        rows.iter().map(row_to_cluster).collect()
    }

    pub async fn update(&self, id: &Id, u: UpdateBrokerCluster) -> Result<BrokerClusterRow> {
        sqlx::query(
            "UPDATE broker_clusters SET
                name = ?, bootstrap_servers = ?, security_protocol = ?,
                sasl_mechanism = ?, sasl_username = ?, tls_skip_verify = ?,
                schema_registry_url = ?, schema_registry_username = ?,
                metrics_url = ?, color = ?
             WHERE id = ?",
        )
        .bind(&u.name)
        .bind(&u.bootstrap_servers)
        .bind(&u.security_protocol)
        .bind(&u.sasl_mechanism)
        .bind(&u.sasl_username)
        .bind(i64::from(u.tls_skip_verify))
        .bind(&u.schema_registry_url)
        .bind(&u.schema_registry_username)
        .bind(&u.metrics_url)
        .bind(&u.color)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update broker cluster"))?;

        if let Some(secret_ref) = u.secret_ref {
            sqlx::query("UPDATE broker_clusters SET secret_ref = ? WHERE id = ?")
                .bind(secret_ref)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update broker cluster"))?;
        }
        if let Some(sr) = u.sr_secret_ref {
            sqlx::query("UPDATE broker_clusters SET sr_secret_ref = ? WHERE id = ?")
                .bind(sr)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update broker cluster"))?;
        }
        if let Some(env) = u.environment {
            sqlx::query("UPDATE broker_clusters SET environment = ? WHERE id = ?")
                .bind(env)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update broker cluster"))?;
        }
        if let Some(ro) = u.read_only {
            sqlx::query("UPDATE broker_clusters SET read_only = ? WHERE id = ?")
                .bind(i64::from(ro))
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("update broker cluster"))?;
        }
        self.get(id).await
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM broker_clusters WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete broker cluster"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    async fn seed_user(pool: &SqlitePool) -> Id {
        let user = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&user)
        .bind("u")
        .bind("x")
        .bind("U")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        user
    }

    fn new_cluster(user: &Id) -> NewBrokerCluster {
        NewBrokerCluster {
            workspace_id: None,
            name: "local".into(),
            bootstrap_servers: "localhost:9092".into(),
            security_protocol: "plaintext".into(),
            sasl_mechanism: None,
            sasl_username: None,
            secret_ref: None,
            tls_skip_verify: false,
            schema_registry_url: None,
            schema_registry_username: None,
            sr_secret_ref: None,
            metrics_url: None,
            color: None,
            environment: "dev".into(),
            read_only: false,
            created_by: user.clone(),
        }
    }

    #[tokio::test]
    async fn crud_round_trip() {
        let pool = mem_pool().await;
        let user = seed_user(&pool).await;
        let repo = BrokerClustersRepo::new(pool.clone());

        let created = repo.create(new_cluster(&user)).await.unwrap();
        assert_eq!(created.name, "local");
        assert!(created.secret_ref.is_none());

        let visible = repo.list_visible(&new_id()).await.unwrap();
        assert_eq!(visible.len(), 1); // global is visible to any workspace

        // Update: set a SASL password ref + flip to prod read-only.
        let updated = repo
            .update(
                &created.id,
                UpdateBrokerCluster {
                    name: "prod-cluster".into(),
                    bootstrap_servers: "broker:9093".into(),
                    security_protocol: "sasl_ssl".into(),
                    sasl_mechanism: Some("scram_sha_256".into()),
                    sasl_username: Some("svc".into()),
                    tls_skip_verify: false,
                    schema_registry_url: Some("http://sr:8081".into()),
                    schema_registry_username: None,
                    metrics_url: Some("http://broker:9644/metrics".into()),
                    color: Some("#ff5f57".into()),
                    secret_ref: Some(Some(format!("broker-{}", created.id))),
                    sr_secret_ref: None,
                    environment: Some("prod".into()),
                    read_only: Some(true),
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "prod-cluster");
        assert_eq!(updated.security_protocol, "sasl_ssl");
        assert_eq!(updated.sasl_mechanism.as_deref(), Some("scram_sha_256"));
        assert!(updated.secret_ref.is_some());
        assert_eq!(updated.environment, "prod");
        assert!(updated.read_only);

        // Clearing the secret ref.
        let cleared = repo
            .update(
                &created.id,
                UpdateBrokerCluster {
                    name: updated.name.clone(),
                    bootstrap_servers: updated.bootstrap_servers.clone(),
                    security_protocol: updated.security_protocol.clone(),
                    sasl_mechanism: updated.sasl_mechanism.clone(),
                    sasl_username: updated.sasl_username.clone(),
                    tls_skip_verify: updated.tls_skip_verify,
                    schema_registry_url: updated.schema_registry_url.clone(),
                    schema_registry_username: None,
                    metrics_url: updated.metrics_url.clone(),
                    color: updated.color.clone(),
                    secret_ref: Some(None),
                    sr_secret_ref: None,
                    environment: None,
                    read_only: None,
                },
            )
            .await
            .unwrap();
        assert!(cleared.secret_ref.is_none());
        assert_eq!(cleared.environment, "prod"); // preserved (None)

        repo.delete(&created.id).await.unwrap();
        assert!(repo.get(&created.id).await.is_err());
    }
}
