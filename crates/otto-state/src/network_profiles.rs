//! Saved workspace TCP-forward profiles. Credentials remain on SSH connections.
use crate::convert::{dberr, fmt, ts};
use crate::{ConnectionsRepo, SqlitePool};
use chrono::Utc;
use otto_core::domain::ConnectionKind;
use otto_core::network_profiles::{NetworkProfile, NetworkProfileInput};
use otto_core::{new_id, Error, Id, Result};
use sqlx::Row;

#[derive(Clone)]
pub struct NetworkProfilesRepo {
    pool: SqlitePool,
}
fn row(row: &sqlx::sqlite::SqliteRow) -> Result<NetworkProfile> {
    Ok(NetworkProfile {
        id: row.get("id"),
        workspace_id: row.get("workspace_id"),
        input: NetworkProfileInput {
            name: row.get("name"),
            ssh_connection_id: row.get("ssh_connection_id"),
            endpoints: serde_json::from_str(&row.get::<String, _>("endpoints_json"))
                .map_err(|e| Error::Internal(e.to_string()))?,
            archived: row.get::<i64, _>("archived") != 0,
        },
        version: row.get("version"),
        created_by: row.get("created_by"),
        created_at: ts(&row.get::<String, _>("created_at"))?,
        updated_at: ts(&row.get::<String, _>("updated_at"))?,
    })
}
impl NetworkProfilesRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
    pub async fn get(&self, id: &str) -> Result<NetworkProfile> {
        row(&sqlx::query("SELECT * FROM network_profiles WHERE id=?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("network profile"))?)
    }
    pub async fn list(&self, workspace: &Id) -> Result<Vec<NetworkProfile>> {
        sqlx::query("SELECT * FROM network_profiles WHERE workspace_id=? ORDER BY name,id")
            .bind(workspace)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list network profiles"))?
            .iter()
            .map(row)
            .collect()
    }
    async fn validate_connection(&self, workspace: &Id, input: &NetworkProfileInput) -> Result<()> {
        input.validate()?;
        let connection = ConnectionsRepo::new(self.pool.clone())
            .get(&input.ssh_connection_id)
            .await?;
        if connection.kind != ConnectionKind::Ssh {
            return Err(Error::Invalid(
                "network profiles require an SSH connection".into(),
            ));
        }
        if connection
            .workspace_id
            .as_ref()
            .is_some_and(|id| id != workspace)
        {
            return Err(Error::Forbidden(
                "SSH connection belongs to another workspace".into(),
            ));
        }
        Ok(())
    }
    pub async fn create(
        &self,
        workspace: &Id,
        user: &Id,
        input: NetworkProfileInput,
    ) -> Result<NetworkProfile> {
        self.validate_connection(workspace, &input).await?;
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO network_profiles(id,workspace_id,name,ssh_connection_id,endpoints_json,archived,created_by,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)")
            .bind(&id).bind(workspace).bind(input.name.trim()).bind(input.ssh_connection_id)
            .bind(serde_json::to_string(&input.endpoints).unwrap()).bind(input.archived).bind(user).bind(&now).bind(&now)
            .execute(&self.pool).await.map_err(dberr("create network profile"))?;
        self.get(&id).await
    }
    pub async fn update(
        &self,
        id: &str,
        input: NetworkProfileInput,
        version: i64,
    ) -> Result<NetworkProfile> {
        let previous = self.get(id).await?;
        self.validate_connection(&previous.workspace_id, &input)
            .await?;
        let changed=sqlx::query("UPDATE network_profiles SET name=?,ssh_connection_id=?,endpoints_json=?,archived=?,version=version+1,updated_at=? WHERE id=? AND version=?")
            .bind(input.name.trim()).bind(input.ssh_connection_id).bind(serde_json::to_string(&input.endpoints).unwrap())
            .bind(input.archived).bind(fmt(Utc::now())).bind(id).bind(version).execute(&self.pool).await.map_err(dberr("update network profile"))?;
        if changed.rows_affected() == 0 {
            return Err(Error::Conflict(
                "network profile changed; reload before saving".into(),
            ));
        }
        self.get(id).await
    }
    pub async fn selected(
        &self,
        workspace: &Id,
        meta: &serde_json::Value,
    ) -> Result<Option<NetworkProfile>> {
        let id = match meta.get("network_profile_id") {
            None | Some(serde_json::Value::Null) => return Ok(None),
            Some(serde_json::Value::String(id)) if !id.trim().is_empty() => id,
            _ => {
                return Err(Error::Invalid(
                    "network_profile_id must be a nonempty ID or null".into(),
                ))
            }
        };
        let profile = self.get(id).await?;
        if &profile.workspace_id != workspace {
            return Err(Error::Forbidden(
                "network profile belongs to another workspace".into(),
            ));
        }
        if profile.input.archived {
            return Err(Error::Invalid(
                "network profile is archived; select another profile".into(),
            ));
        }
        profile.input.validate()?;
        Ok(Some(profile))
    }
}
