//! Browser module: tabs (per workspace) + DOM annotations.
//!
//! Annotations key on URL rather than tab id, so a mark survives the tab that
//! made it being closed — it reattaches to any tab that later opens the same
//! URL. `tab_id` is kept as a best-effort origin hint only.

use chrono::Utc;
use otto_core::{new_id, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::convert::{dberr, fmt};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BrowserTab {
    pub id: Id,
    pub workspace_id: Id,
    pub url: String,
    pub title: String,
    pub mode: String,
    pub created_at: String,
}

pub struct NewBrowserTab {
    pub workspace_id: Id,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BrowserAnnotation {
    pub id: Id,
    pub workspace_id: Id,
    pub tab_id: Option<Id>,
    pub url: String,
    pub selector: String,
    pub excerpt: String,
    pub text: String,
    pub comment: String,
    pub color: String,
    pub created_at: String,
}

pub struct NewBrowserAnnotation {
    pub workspace_id: Id,
    pub tab_id: Option<Id>,
    pub url: String,
    pub selector: String,
    pub excerpt: String,
    pub text: String,
    pub comment: String,
    pub color: String,
}

#[derive(Clone)]
pub struct BrowserTabsRepo {
    pool: SqlitePool,
}

impl BrowserTabsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, new: NewBrowserTab) -> Result<BrowserTab> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO browser_tabs (id, workspace_id, url, title, mode, created_at)
             VALUES (?, ?, ?, '', 'reader', ?)",
        )
        .bind(&id)
        .bind(&new.workspace_id)
        .bind(&new.url)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create browser tab"))?;
        sqlx::query_as::<_, BrowserTab>(
            "SELECT id, workspace_id, url, title, mode, created_at FROM browser_tabs WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("browser tab"))
    }

    pub async fn list(&self, workspace_id: &str) -> Result<Vec<BrowserTab>> {
        sqlx::query_as::<_, BrowserTab>(
            "SELECT id, workspace_id, url, title, mode, created_at FROM browser_tabs
             WHERE workspace_id = ? ORDER BY created_at",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("browser tabs"))
    }

    /// Load one tab by id — the flat `/browser/tabs/{id}` routes need this to
    /// resolve `workspace_id` for the IDOR guard (`require_ws_role`) before
    /// mutating, mirroring `BrowserAnnotationsRepo::get`.
    pub async fn get(&self, id: &Id) -> Result<Option<BrowserTab>> {
        sqlx::query_as::<_, BrowserTab>(
            "SELECT id, workspace_id, url, title, mode, created_at FROM browser_tabs WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("browser tab"))
    }

    pub async fn update_nav(&self, id: &Id, url: &str, title: &str) -> Result<()> {
        sqlx::query("UPDATE browser_tabs SET url = ?, title = ? WHERE id = ?")
            .bind(url)
            .bind(title)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update browser tab nav"))?;
        Ok(())
    }

    pub async fn set_mode(&self, id: &Id, mode: &str) -> Result<()> {
        sqlx::query("UPDATE browser_tabs SET mode = ? WHERE id = ?")
            .bind(mode)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update browser tab mode"))?;
        Ok(())
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM browser_tabs WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete browser tab"))?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct BrowserAnnotationsRepo {
    pool: SqlitePool,
}

impl BrowserAnnotationsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, new: NewBrowserAnnotation) -> Result<BrowserAnnotation> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO browser_annotations
               (id, workspace_id, tab_id, url, selector, excerpt, text, comment, color, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&new.workspace_id)
        .bind(&new.tab_id)
        .bind(&new.url)
        .bind(&new.selector)
        .bind(&new.excerpt)
        .bind(&new.text)
        .bind(&new.comment)
        .bind(&new.color)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create browser annotation"))?;
        self.get(&id).await.and_then(|a| {
            a.ok_or_else(|| dberr("browser annotation")(sqlx::Error::RowNotFound))
        })
    }

    pub async fn get(&self, id: &Id) -> Result<Option<BrowserAnnotation>> {
        sqlx::query_as::<_, BrowserAnnotation>(
            "SELECT id, workspace_id, tab_id, url, selector, excerpt, text, comment, color, created_at
             FROM browser_annotations WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("browser annotation"))
    }

    pub async fn list_for_url(&self, workspace_id: &str, url: &str) -> Result<Vec<BrowserAnnotation>> {
        sqlx::query_as::<_, BrowserAnnotation>(
            "SELECT id, workspace_id, tab_id, url, selector, excerpt, text, comment, color, created_at
             FROM browser_annotations WHERE workspace_id = ? AND url = ? ORDER BY created_at",
        )
        .bind(workspace_id)
        .bind(url)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("browser annotations"))
    }

    pub async fn list(&self, workspace_id: &str) -> Result<Vec<BrowserAnnotation>> {
        sqlx::query_as::<_, BrowserAnnotation>(
            "SELECT id, workspace_id, tab_id, url, selector, excerpt, text, comment, color, created_at
             FROM browser_annotations WHERE workspace_id = ? ORDER BY created_at",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("browser annotations"))
    }

    pub async fn update_comment(&self, id: &Id, comment: &str) -> Result<()> {
        sqlx::query("UPDATE browser_annotations SET comment = ? WHERE id = ?")
            .bind(comment)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update browser annotation comment"))?;
        Ok(())
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM browser_annotations WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete browser annotation"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
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

    #[tokio::test]
    async fn tab_crud_roundtrip() {
        let pool = test_pool().await;
        let repo = BrowserTabsRepo::new(pool.clone());
        let tab = repo
            .create(NewBrowserTab {
                workspace_id: "ws1".into(),
                url: "https://example.com".into(),
            })
            .await
            .unwrap();
        repo.update_nav(&tab.id, "https://example.com/a", "Example A")
            .await
            .unwrap();
        let listed = repo.list("ws1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Example A");
        let got = repo.get(&tab.id).await.unwrap().expect("tab found");
        assert_eq!(got.workspace_id, "ws1");
        assert_eq!(got.title, "Example A");
        repo.delete(&tab.id).await.unwrap();
        assert!(repo.list("ws1").await.unwrap().is_empty());
        assert!(repo.get(&tab.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn annotations_key_on_url_and_survive_tab_delete() {
        let pool = test_pool().await;
        let tabs = BrowserTabsRepo::new(pool.clone());
        let anns = BrowserAnnotationsRepo::new(pool.clone());
        let tab = tabs
            .create(NewBrowserTab {
                workspace_id: "ws1".into(),
                url: "https://a.io".into(),
            })
            .await
            .unwrap();
        anns.create(NewBrowserAnnotation {
            workspace_id: "ws1".into(),
            tab_id: Some(tab.id.clone()),
            url: "https://a.io".into(),
            selector: "#hero > h1".into(),
            excerpt: "<h1>Hi</h1>".into(),
            text: "Hi".into(),
            comment: "check this".into(),
            color: "yellow".into(),
        })
        .await
        .unwrap();
        tabs.delete(&tab.id).await.unwrap();
        let per_url = anns.list_for_url("ws1", "https://a.io").await.unwrap();
        assert_eq!(per_url.len(), 1, "annotation must survive tab close");
    }
}
