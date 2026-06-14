//! Settings key/value repository (JSON values).

use otto_core::Result;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, json};

#[derive(Clone)]
pub struct SettingsRepo {
    pool: SqlitePool,
}

impl SettingsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let row = sqlx::query("SELECT value_json FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("setting"))?;
        row.map(|r| json(&r.get::<String, _>("value_json")))
            .transpose()
    }

    pub async fn put(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        sqlx::query(
            "INSERT INTO settings (key, value_json) VALUES (?, ?)
             ON CONFLICT (key) DO UPDATE SET value_json = excluded.value_json",
        )
        .bind(key)
        .bind(value.to_string())
        .execute(&self.pool)
        .await
        .map_err(dberr("put setting"))?;
        Ok(())
    }

    pub async fn all(&self) -> Result<serde_json::Map<String, serde_json::Value>> {
        let rows = sqlx::query("SELECT key, value_json FROM settings")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("settings"))?;
        let mut map = serde_json::Map::new();
        for r in rows {
            map.insert(
                r.get::<String, _>("key"),
                json(&r.get::<String, _>("value_json"))?,
            );
        }
        Ok(map)
    }
}
