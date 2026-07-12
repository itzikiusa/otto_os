//! SQLite persistence for the vault index. Every row here is DERIVED from the
//! files on disk and rebuildable by a rescan — the store never holds the only
//! copy of anything.

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use otto_core::{Error, Result};

use crate::types::*;

fn dberr(op: &'static str) -> impl Fn(sqlx::Error) -> Error {
    move |e| Error::Internal(format!("{op}: {e}"))
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A note row as the scanner writes it.
pub struct NoteRow {
    pub path: String,
    pub title: String,
    pub okf_type: Option<String>,
    pub description: Option<String>,
    pub frontmatter_json: String,
    pub tags_json: String,
    pub aliases_json: String,
    pub headings_json: String,
    pub word_count: i64,
    pub size: i64,
    pub mtime_ns: i64,
    pub hash: String,
    pub reserved: bool,
    pub has_frontmatter: bool,
    pub parse_error: bool,
}

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // -- vaults -------------------------------------------------------------

    pub async fn create_vault(&self, ws: &str, name: &str, root: &str, okf: bool) -> Result<i64> {
        // Vaults are global — enforce root uniqueness ACROSS workspaces here
        // (the table's UNIQUE(ws_id, root_path) is a pre-global relic; a
        // migration can't retro-tighten it without risking existing rows).
        let dup = sqlx::query("SELECT 1 FROM vaults WHERE root_path = ?")
            .bind(root)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("vault.create.dup"))?;
        if dup.is_some() {
            return Err(Error::Conflict(format!("vault already registered at {root}")));
        }
        let r = sqlx::query(
            "INSERT INTO vaults (ws_id, name, root_path, okf, created_at) VALUES (?,?,?,?,?)",
        )
        .bind(ws)
        .bind(name)
        .bind(root)
        .bind(okf as i64)
        .bind(now())
        .execute(&self.pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(ref d) if d.message().contains("UNIQUE") => {
                Error::Conflict(format!("vault already registered at {root}"))
            }
            other => Error::Internal(format!("vault.create: {other}")),
        })?;
        Ok(r.last_insert_rowid())
    }

    fn vault_from_row(r: &sqlx::sqlite::SqliteRow) -> VaultRec {
        VaultRec {
            id: r.get("id"),
            ws_id: r.get("ws_id"),
            name: r.get("name"),
            root_path: r.get("root_path"),
            okf: r.get::<i64, _>("okf") != 0,
            created_at: r.get("created_at"),
            last_scan_at: r.get("last_scan_at"),
            scan_state: r.get("scan_state"),
            notes: r.try_get("notes").unwrap_or(0),
            links: r.try_get("links").unwrap_or(0),
        }
    }

    const VAULT_COLS: &'static str = "v.id, v.ws_id, v.name, v.root_path, v.okf, v.created_at, \
         v.last_scan_at, v.scan_state, \
         (SELECT COUNT(*) FROM vault_notes n WHERE n.vault_id = v.id) AS notes, \
         (SELECT COUNT(*) FROM vault_links l WHERE l.vault_id = v.id) AS links";

    pub async fn list_vaults(&self) -> Result<Vec<VaultRec>> {
        // GLOBAL list — vaults are a cross-workspace library. Dedup by
        // root_path (lowest id wins) in case pre-global rows registered the
        // same folder from two workspaces.
        let rows = sqlx::query(&format!(
            "SELECT {} FROM vaults v \
             WHERE v.id IN (SELECT MIN(id) FROM vaults GROUP BY root_path) \
             ORDER BY v.id",
            Self::VAULT_COLS
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.list"))?;
        Ok(rows.iter().map(Self::vault_from_row).collect())
    }

    pub async fn get_vault(&self, id: i64) -> Result<VaultRec> {
        let row = sqlx::query(&format!("SELECT {} FROM vaults v WHERE v.id = ?", Self::VAULT_COLS))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("vault.get"))?
            .ok_or_else(|| Error::NotFound("vault".into()))?;
        Ok(Self::vault_from_row(&row))
    }

    pub async fn patch_vault(&self, id: i64, name: Option<&str>, okf: Option<bool>) -> Result<()> {
        if let Some(n) = name {
            sqlx::query("UPDATE vaults SET name = ? WHERE id = ?")
                .bind(n)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.patch"))?;
        }
        if let Some(o) = okf {
            sqlx::query("UPDATE vaults SET okf = ? WHERE id = ?")
                .bind(o as i64)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.patch"))?;
        }
        Ok(())
    }

    /// Unregister: removes the vault row + every derived index row. Never
    /// touches the files on disk.
    pub async fn delete_vault(&self, id: i64) -> Result<()> {
        for sql in [
            "DELETE FROM vault_links WHERE vault_id = ?",
            "DELETE FROM vault_tags WHERE vault_id = ?",
            "DELETE FROM vault_files WHERE vault_id = ?",
            "DELETE FROM vault_notes WHERE vault_id = ?",
            "DELETE FROM vaults WHERE id = ?",
        ] {
            sqlx::query(sql).bind(id).execute(&self.pool).await.map_err(dberr("vault.delete"))?;
        }
        let _ = sqlx::query("DELETE FROM vault_fts WHERE vault_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await;
        Ok(())
    }

    pub async fn set_scan_state(&self, id: i64, state: &str, touch_scan_time: bool) -> Result<()> {
        if touch_scan_time {
            sqlx::query("UPDATE vaults SET scan_state = ?, last_scan_at = ? WHERE id = ?")
                .bind(state)
                .bind(now())
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.scan_state"))?;
        } else {
            sqlx::query("UPDATE vaults SET scan_state = ? WHERE id = ?")
                .bind(state)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.scan_state"))?;
        }
        Ok(())
    }

    pub async fn status(&self, id: i64) -> Result<VaultStatus> {
        let v = self.get_vault(id).await?;
        let unresolved: i64 = sqlx::query(
            "SELECT COUNT(*) AS c FROM vault_links WHERE vault_id = ? AND dst_path IS NULL",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("vault.status"))?
        .get("c");
        let tags: i64 =
            sqlx::query("SELECT COUNT(DISTINCT tag) AS c FROM vault_tags WHERE vault_id = ?")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .map_err(dberr("vault.status"))?
                .get("c");
        let attachments: i64 =
            sqlx::query("SELECT COUNT(*) AS c FROM vault_files WHERE vault_id = ?")
                .bind(id)
                .fetch_one(&self.pool)
                .await
                .map_err(dberr("vault.status"))?
                .get("c");
        Ok(VaultStatus {
            id,
            scan_state: v.scan_state,
            last_scan_at: v.last_scan_at,
            notes: v.notes,
            links: v.links,
            unresolved,
            tags,
            attachments,
        })
    }

    // -- notes ---------------------------------------------------------------

    /// `(path, size, mtime_ns)` of every indexed note — the incremental-scan diff basis.
    pub async fn note_sigs(&self, vault: i64) -> Result<Vec<(String, i64, i64)>> {
        let rows = sqlx::query("SELECT path, size, mtime_ns FROM vault_notes WHERE vault_id = ?")
            .bind(vault)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("vault.note_sigs"))?;
        Ok(rows.iter().map(|r| (r.get("path"), r.get("size"), r.get("mtime_ns"))).collect())
    }

    pub async fn file_sigs(&self, vault: i64) -> Result<Vec<(String, i64, i64)>> {
        let rows = sqlx::query("SELECT path, size, mtime_ns FROM vault_files WHERE vault_id = ?")
            .bind(vault)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("vault.file_sigs"))?;
        Ok(rows.iter().map(|r| (r.get("path"), r.get("size"), r.get("mtime_ns"))).collect())
    }

    pub async fn upsert_note(&self, vault: i64, n: &NoteRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO vault_notes (vault_id, path, title, okf_type, description, \
             frontmatter_json, tags_json, aliases_json, headings_json, word_count, size, \
             mtime_ns, hash, reserved, has_frontmatter, parse_error) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(vault_id, path) DO UPDATE SET \
             title=excluded.title, okf_type=excluded.okf_type, description=excluded.description, \
             frontmatter_json=excluded.frontmatter_json, tags_json=excluded.tags_json, \
             aliases_json=excluded.aliases_json, headings_json=excluded.headings_json, \
             word_count=excluded.word_count, size=excluded.size, mtime_ns=excluded.mtime_ns, \
             hash=excluded.hash, reserved=excluded.reserved, \
             has_frontmatter=excluded.has_frontmatter, parse_error=excluded.parse_error",
        )
        .bind(vault)
        .bind(&n.path)
        .bind(&n.title)
        .bind(&n.okf_type)
        .bind(&n.description)
        .bind(&n.frontmatter_json)
        .bind(&n.tags_json)
        .bind(&n.aliases_json)
        .bind(&n.headings_json)
        .bind(n.word_count)
        .bind(n.size)
        .bind(n.mtime_ns)
        .bind(&n.hash)
        .bind(n.reserved as i64)
        .bind(n.has_frontmatter as i64)
        .bind(n.parse_error as i64)
        .execute(&self.pool)
        .await
        .map_err(dberr("vault.upsert_note"))?;
        Ok(())
    }

    pub async fn remove_note(&self, vault: i64, path: &str) -> Result<()> {
        for sql in [
            "DELETE FROM vault_notes WHERE vault_id = ? AND path = ?",
            "DELETE FROM vault_links WHERE vault_id = ? AND src_path = ?",
            "DELETE FROM vault_tags WHERE vault_id = ? AND path = ?",
        ] {
            sqlx::query(sql)
                .bind(vault)
                .bind(path)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.remove_note"))?;
        }
        self.fts_remove(vault, path).await;
        Ok(())
    }

    pub async fn upsert_file(&self, vault: i64, path: &str, size: i64, mtime_ns: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO vault_files (vault_id, path, size, mtime_ns) VALUES (?,?,?,?) \
             ON CONFLICT(vault_id, path) DO UPDATE SET size=excluded.size, mtime_ns=excluded.mtime_ns",
        )
        .bind(vault)
        .bind(path)
        .bind(size)
        .bind(mtime_ns)
        .execute(&self.pool)
        .await
        .map_err(dberr("vault.upsert_file"))?;
        Ok(())
    }

    pub async fn remove_file(&self, vault: i64, path: &str) -> Result<()> {
        sqlx::query("DELETE FROM vault_files WHERE vault_id = ? AND path = ?")
            .bind(vault)
            .bind(path)
            .execute(&self.pool)
            .await
            .map_err(dberr("vault.remove_file"))?;
        Ok(())
    }

    pub async fn note_meta(&self, vault: i64, path: &str) -> Result<NoteMeta> {
        let r = sqlx::query(
            "SELECT * FROM vault_notes WHERE vault_id = ? AND path = ?",
        )
        .bind(vault)
        .bind(path)
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("vault.note_meta"))?
        .ok_or_else(|| Error::NotFound(format!("note {path}")))?;
        Ok(Self::meta_from_row(&r))
    }

    fn meta_from_row(r: &sqlx::sqlite::SqliteRow) -> NoteMeta {
        let parse = |s: String| serde_json::from_str(&s).unwrap_or(serde_json::Value::Null);
        let strs = |s: String| -> Vec<String> { serde_json::from_str(&s).unwrap_or_default() };
        NoteMeta {
            path: r.get("path"),
            title: r.get("title"),
            okf_type: r.get("okf_type"),
            description: r.get("description"),
            frontmatter: parse(r.get("frontmatter_json")),
            tags: strs(r.get("tags_json")),
            aliases: strs(r.get("aliases_json")),
            headings: serde_json::from_str(&r.get::<String, _>("headings_json")).unwrap_or_default(),
            word_count: r.get("word_count"),
            size: r.get("size"),
            hash: r.get("hash"),
            reserved: r.get::<i64, _>("reserved") != 0,
            has_frontmatter: r.get::<i64, _>("has_frontmatter") != 0,
            parse_error: r.get::<i64, _>("parse_error") != 0,
        }
    }

    // -- links / tags ---------------------------------------------------------

    pub async fn replace_links(&self, vault: i64, src: &str, links: &[OutgoingLink]) -> Result<()> {
        sqlx::query("DELETE FROM vault_links WHERE vault_id = ? AND src_path = ?")
            .bind(vault)
            .bind(src)
            .execute(&self.pool)
            .await
            .map_err(dberr("vault.replace_links"))?;
        for (i, l) in links.iter().enumerate() {
            sqlx::query(
                "INSERT INTO vault_links (vault_id, src_path, raw_target, dst_path, kind, anchor, alias, pos) \
                 VALUES (?,?,?,?,?,?,?,?)",
            )
            .bind(vault)
            .bind(src)
            .bind(&l.raw_target)
            .bind(&l.dst_path)
            .bind(&l.kind)
            .bind(&l.anchor)
            .bind(&l.alias)
            .bind(i as i64)
            .execute(&self.pool)
            .await
            .map_err(dberr("vault.replace_links"))?;
        }
        Ok(())
    }

    pub async fn replace_tags(&self, vault: i64, path: &str, tags: &[String]) -> Result<()> {
        sqlx::query("DELETE FROM vault_tags WHERE vault_id = ? AND path = ?")
            .bind(vault)
            .bind(path)
            .execute(&self.pool)
            .await
            .map_err(dberr("vault.replace_tags"))?;
        for t in tags {
            sqlx::query("INSERT INTO vault_tags (vault_id, tag, path) VALUES (?,?,?)")
                .bind(vault)
                .bind(t)
                .bind(path)
                .execute(&self.pool)
                .await
                .map_err(dberr("vault.replace_tags"))?;
        }
        Ok(())
    }

    pub async fn outgoing(&self, vault: i64, src: &str) -> Result<Vec<OutgoingLink>> {
        let rows = sqlx::query(
            "SELECT raw_target, dst_path, kind, anchor, alias FROM vault_links \
             WHERE vault_id = ? AND src_path = ? ORDER BY pos",
        )
        .bind(vault)
        .bind(src)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.outgoing"))?;
        Ok(rows
            .iter()
            .map(|r| OutgoingLink {
                raw_target: r.get("raw_target"),
                dst_path: r.get("dst_path"),
                kind: r.get("kind"),
                anchor: r.get("anchor"),
                alias: r.get("alias"),
            })
            .collect())
    }

    /// Sources that link TO `path` (backlinks), with the link kind.
    pub async fn backlinks(&self, vault: i64, path: &str) -> Result<Vec<(String, String, String)>> {
        let rows = sqlx::query(
            "SELECT DISTINCT l.src_path, n.title, l.kind FROM vault_links l \
             JOIN vault_notes n ON n.vault_id = l.vault_id AND n.path = l.src_path \
             WHERE l.vault_id = ? AND l.dst_path = ? ORDER BY l.src_path",
        )
        .bind(vault)
        .bind(path)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.backlinks"))?;
        Ok(rows.iter().map(|r| (r.get("src_path"), r.get("title"), r.get("kind"))).collect())
    }

    /// Every src_path that has at least one link whose dst is `path`.
    pub async fn linking_sources(&self, vault: i64, path: &str) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT DISTINCT src_path FROM vault_links WHERE vault_id = ? AND dst_path = ?",
        )
        .bind(vault)
        .bind(path)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.linking_sources"))?;
        Ok(rows.iter().map(|r| r.get("src_path")).collect())
    }

    /// Paths of notes with at least one UNRESOLVED link (rename may have made
    /// a previously-broken target valid, or vice versa).
    pub async fn unresolved_sources(&self, vault: i64) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT DISTINCT src_path FROM vault_links WHERE vault_id = ? AND dst_path IS NULL",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.unresolved_sources"))?;
        Ok(rows.iter().map(|r| r.get("src_path")).collect())
    }

    /// Every link row `(rowid, src, raw, dst)` — the global re-resolve pass.
    pub async fn all_links_full(&self, vault: i64) -> Result<Vec<(i64, String, String, Option<String>)>> {
        let rows = sqlx::query(
            "SELECT rowid, src_path, raw_target, dst_path FROM vault_links WHERE vault_id = ?",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.all_links_full"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get("rowid"), r.get("src_path"), r.get("raw_target"), r.get("dst_path")))
            .collect())
    }

    pub async fn update_link_dst(&self, rowid: i64, dst: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE vault_links SET dst_path = ? WHERE rowid = ?")
            .bind(dst)
            .bind(rowid)
            .execute(&self.pool)
            .await
            .map_err(dberr("vault.update_link_dst"))?;
        Ok(())
    }

    pub async fn tag_counts(&self, vault: i64) -> Result<Vec<TagCount>> {
        let rows = sqlx::query(
            "SELECT tag, COUNT(*) AS c FROM vault_tags WHERE vault_id = ? \
             GROUP BY tag ORDER BY c DESC, tag",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.tags"))?;
        Ok(rows.iter().map(|r| TagCount { tag: r.get("tag"), count: r.get("c") }).collect())
    }

    // -- listing / switcher / graph -------------------------------------------

    /// `(path, title, okf_type, reserved)` of every note (switcher + graph + tree).
    pub async fn all_notes(&self, vault: i64) -> Result<Vec<(String, String, Option<String>, bool)>> {
        let rows = sqlx::query(
            "SELECT path, title, okf_type, reserved FROM vault_notes WHERE vault_id = ? ORDER BY path",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.all_notes"))?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get("path"),
                    r.get("title"),
                    r.get("okf_type"),
                    r.get::<i64, _>("reserved") != 0,
                )
            })
            .collect())
    }

    /// `(path, aliases_json)` for notes that have aliases.
    pub async fn all_aliases(&self, vault: i64) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT path, aliases_json FROM vault_notes WHERE vault_id = ? AND aliases_json != '[]'",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.aliases"))?;
        Ok(rows.iter().map(|r| (r.get("path"), r.get("aliases_json"))).collect())
    }

    pub async fn all_file_paths(&self, vault: i64) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT path FROM vault_files WHERE vault_id = ?")
            .bind(vault)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("vault.all_files"))?;
        Ok(rows.iter().map(|r| r.get("path")).collect())
    }

    /// `(src, dst, kind)` of every RESOLVED link.
    pub async fn all_edges(&self, vault: i64) -> Result<Vec<(String, String, String)>> {
        let rows = sqlx::query(
            "SELECT src_path, dst_path, kind FROM vault_links \
             WHERE vault_id = ? AND dst_path IS NOT NULL",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.all_edges"))?;
        Ok(rows.iter().map(|r| (r.get("src_path"), r.get("dst_path"), r.get("kind"))).collect())
    }

    /// Unresolved raw targets grouped: `(src, raw_target)`.
    pub async fn all_ghost_edges(&self, vault: i64) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT src_path, raw_target FROM vault_links \
             WHERE vault_id = ? AND dst_path IS NULL",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.ghost_edges"))?;
        Ok(rows.iter().map(|r| (r.get("src_path"), r.get("raw_target"))).collect())
    }

    /// `(path → [tags])` for the graph's tag nodes.
    pub async fn all_note_tags(&self, vault: i64) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT path, tag FROM vault_tags WHERE vault_id = ?")
            .bind(vault)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("vault.note_tags"))?;
        Ok(rows.iter().map(|r| (r.get("path"), r.get("tag"))).collect())
    }

    // -- FTS -------------------------------------------------------------------

    /// Create the FTS5 index if the linked SQLite supports it.
    pub async fn ensure_fts(&self) -> bool {
        sqlx::query(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vault_fts USING fts5(\
             vault_id UNINDEXED, path UNINDEXED, title, body, tokenize='unicode61 remove_diacritics 2')",
        )
        .execute(&self.pool)
        .await
        .is_ok()
    }

    pub async fn fts_index(&self, vault: i64, path: &str, title: &str, body: &str) {
        self.fts_remove(vault, path).await;
        let _ = sqlx::query("INSERT INTO vault_fts (vault_id, path, title, body) VALUES (?,?,?,?)")
            .bind(vault)
            .bind(path)
            .bind(title)
            .bind(body)
            .execute(&self.pool)
            .await;
    }

    pub async fn fts_remove(&self, vault: i64, path: &str) {
        let _ = sqlx::query("DELETE FROM vault_fts WHERE vault_id = ? AND path = ?")
            .bind(vault)
            .bind(path)
            .execute(&self.pool)
            .await;
    }

    /// bm25-ranked FTS search → `(path, snippet, score)`. `match_expr` must be a
    /// sanitized FTS5 MATCH expression.
    pub async fn fts_search(
        &self,
        vault: i64,
        match_expr: &str,
        limit: i64,
    ) -> Result<Vec<(String, String, f32)>> {
        let rows = sqlx::query(
            "SELECT path, snippet(vault_fts, 3, '\u{2039}', '\u{203a}', '…', 14) AS snip, \
             bm25(vault_fts) AS rank FROM vault_fts \
             WHERE vault_id = ? AND vault_fts MATCH ? ORDER BY rank LIMIT ?",
        )
        .bind(vault)
        .bind(match_expr)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.fts_search"))?;
        Ok(rows
            .iter()
            .map(|r| {
                let rank: f64 = r.get("rank");
                (r.get("path"), r.get("snip"), -rank as f32)
            })
            .collect())
    }

    /// LIKE fallback when FTS5 is unavailable or the query has no FTS tokens.
    pub async fn like_search(
        &self,
        vault: i64,
        needle: &str,
        limit: i64,
    ) -> Result<Vec<(String, String, f32)>> {
        let pat = format!("%{}%", needle.replace('%', "\\%").replace('_', "\\_"));
        let rows = sqlx::query(
            "SELECT path, title FROM vault_notes WHERE vault_id = ? AND \
             (title LIKE ? ESCAPE '\\' OR path LIKE ? ESCAPE '\\' OR description LIKE ? ESCAPE '\\') \
             ORDER BY path LIMIT ?",
        )
        .bind(vault)
        .bind(&pat)
        .bind(&pat)
        .bind(&pat)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.like_search"))?;
        Ok(rows.iter().map(|r| (r.get("path"), r.get::<String, _>("title"), 0.1f32)).collect())
    }

    /// Notes for OKF validation: everything the DB knows, one pass.
    pub async fn okf_rows(&self, vault: i64) -> Result<Vec<OkfNoteRow>> {
        let rows = sqlx::query(
            "SELECT path, okf_type, frontmatter_json, headings_json, reserved, \
             has_frontmatter, parse_error FROM vault_notes WHERE vault_id = ? ORDER BY path",
        )
        .bind(vault)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.okf_rows"))?;
        Ok(rows
            .iter()
            .map(|r| OkfNoteRow {
                path: r.get("path"),
                okf_type: r.get("okf_type"),
                frontmatter_json: r.get("frontmatter_json"),
                headings_json: r.get("headings_json"),
                reserved: r.get::<i64, _>("reserved") != 0,
                has_frontmatter: r.get::<i64, _>("has_frontmatter") != 0,
                parse_error: r.get::<i64, _>("parse_error") != 0,
            })
            .collect())
    }

    /// `(path, title, description)` for index.md generation, one directory.
    pub async fn dir_notes(&self, vault: i64, dir: &str) -> Result<Vec<(String, String, Option<String>)>> {
        let (pat, depth_from) = if dir.is_empty() {
            ("%".to_string(), 0usize)
        } else {
            (format!("{}/%", like_escape(dir)), dir.len() + 1)
        };
        let rows = sqlx::query(
            "SELECT path, title, description FROM vault_notes \
             WHERE vault_id = ? AND path LIKE ? ESCAPE '\\' AND reserved = 0 ORDER BY path",
        )
        .bind(vault)
        .bind(&pat)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("vault.dir_notes"))?;
        Ok(rows
            .iter()
            .filter(|r| {
                let p: String = r.get("path");
                !p[depth_from..].contains('/')
            })
            .map(|r| (r.get("path"), r.get("title"), r.get("description")))
            .collect())
    }
}

pub struct OkfNoteRow {
    pub path: String,
    pub okf_type: Option<String>,
    pub frontmatter_json: String,
    pub headings_json: String,
    pub reserved: bool,
    pub has_frontmatter: bool,
    pub parse_error: bool,
}

pub fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}
