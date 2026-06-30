//! Persistence for the **code-intelligence layer** of Vault v2 — the tree-sitter
//! symbol index and the typed code dependency graph.
//!
//! Three tables back this (see migration `0090_vault_v2.sql`):
//! - `code_repos`   — one row per indexed (workspace, repo root): index state.
//! - `code_symbols` — a flat, queryable symbol index (name/kind/file/line/sig).
//! - `code_nodes` / `code_edges` — the dependency graph: files / symbols /
//!   external services / DB tables / endpoints / docs as nodes, with typed edges
//!   (`calls`, `imports`, `http_call`, `db_call`, `test_of`, `documents`, …).
//!
//! Extraction (parsing repos, detecting HTTP/DB calls) lives in `otto-memory`;
//! this module is purely storage + queries.

use chrono::Utc;
use otto_core::{new_id, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};

/// Index state for one indexed repo.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeRepo {
    pub id: String,
    pub workspace_id: String,
    pub root: String,
    pub name: String,
    pub head: Option<String>,
    pub files: i64,
    pub symbols: i64,
    pub edges: i64,
    pub chunks: i64,
    pub status: String,
    pub message: Option<String>,
    pub indexed_at: Option<String>,
    pub created_at: String,
}

/// A single extracted symbol.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeSymbol {
    pub id: String,
    pub workspace_id: String,
    pub repo_id: String,
    pub name: String,
    pub kind: String,
    pub lang: String,
    pub file: String,
    pub line: i64,
    pub signature: String,
}

/// A graph node. `id` is a stable hash of (workspace, repo, kind, key).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeNode {
    pub id: String,
    pub workspace_id: String,
    pub repo_id: Option<String>,
    pub kind: String,
    pub key: String,
    pub label: String,
    pub file: Option<String>,
    pub line: Option<i64>,
    #[serde(default)]
    pub meta_json: String,
}

/// A typed edge between two graph nodes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeEdge {
    pub id: String,
    pub workspace_id: String,
    pub repo_id: Option<String>,
    pub src_id: String,
    pub dst_id: String,
    pub rel: String,
    pub detail: String,
    pub weight: f64,
    pub file: Option<String>,
    pub line: Option<i64>,
}

/// A graph as returned to the UI / API: nodes + edges.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CodeGraph {
    pub nodes: Vec<CodeNode>,
    pub edges: Vec<CodeEdge>,
}

/// A node spec to upsert (id is derived).
#[derive(Clone, Debug)]
pub struct NewNode {
    pub repo_id: Option<String>,
    pub kind: String,
    pub key: String,
    pub label: String,
    pub file: Option<String>,
    pub line: Option<i64>,
    pub meta_json: String,
}

/// An edge spec to upsert (node ids already resolved).
#[derive(Clone, Debug)]
pub struct NewEdge {
    pub repo_id: Option<String>,
    pub src_id: String,
    pub dst_id: String,
    pub rel: String,
    pub detail: String,
    pub weight: f64,
    pub file: Option<String>,
    pub line: Option<i64>,
}

/// Stable id for a graph node — deterministic so re-indexing upserts in place.
pub fn node_id(ws: &str, repo_id: Option<&str>, kind: &str, key: &str) -> String {
    let mut h = Sha256::new();
    h.update(ws.as_bytes());
    h.update([0]);
    h.update(repo_id.unwrap_or("").as_bytes());
    h.update([0]);
    h.update(kind.as_bytes());
    h.update([0]);
    h.update(key.as_bytes());
    format!("n_{:x}", h.finalize())[..18].to_string()
}

#[derive(Clone)]
pub struct CodeIndexRepo {
    pool: SqlitePool,
}

impl CodeIndexRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // -- repos -------------------------------------------------------------

    /// Insert or fetch the repo row for (ws, root); returns its id.
    pub async fn upsert_repo(&self, ws: &str, root: &str, name: &str) -> Result<String> {
        if let Some(r) = self.get_repo_by_root(ws, root).await? {
            return Ok(r.id);
        }
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO code_repos (id,workspace_id,root,name,status,created_at) \
             VALUES (?,?,?,?,'idle',?)",
        )
        .bind(&id)
        .bind(ws)
        .bind(root)
        .bind(name)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("code_repos.upsert"))?;
        Ok(id)
    }

    pub async fn get_repo(&self, ws: &str, id: &str) -> Result<Option<CodeRepo>> {
        let r = sqlx::query("SELECT * FROM code_repos WHERE id = ? AND workspace_id = ?")
            .bind(id)
            .bind(ws)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("code_repos.get"))?;
        Ok(r.as_ref().map(row_to_repo))
    }

    pub async fn get_repo_by_root(&self, ws: &str, root: &str) -> Result<Option<CodeRepo>> {
        let r = sqlx::query("SELECT * FROM code_repos WHERE workspace_id = ? AND root = ?")
            .bind(ws)
            .bind(root)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("code_repos.get_by_root"))?;
        Ok(r.as_ref().map(row_to_repo))
    }

    pub async fn list_repos(&self, ws: &str) -> Result<Vec<CodeRepo>> {
        let rows = sqlx::query("SELECT * FROM code_repos WHERE workspace_id = ? ORDER BY created_at DESC")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("code_repos.list"))?;
        Ok(rows.iter().map(row_to_repo).collect())
    }

    /// Update status/message and (optionally) the rolled-up counts + head.
    #[allow(clippy::too_many_arguments)]
    pub async fn set_repo_state(
        &self,
        id: &str,
        status: &str,
        message: Option<&str>,
        head: Option<&str>,
        counts: Option<(i64, i64, i64, i64)>,
    ) -> Result<()> {
        let now = fmt(Utc::now());
        if let Some((files, symbols, edges, chunks)) = counts {
            sqlx::query(
                "UPDATE code_repos SET status=?, message=?, head=?, files=?, symbols=?, edges=?, chunks=?, indexed_at=? WHERE id=?",
            )
            .bind(status)
            .bind(message)
            .bind(head)
            .bind(files)
            .bind(symbols)
            .bind(edges)
            .bind(chunks)
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("code_repos.set_state"))?;
        } else {
            sqlx::query("UPDATE code_repos SET status=?, message=? WHERE id=?")
                .bind(status)
                .bind(message)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(dberr("code_repos.set_status"))?;
        }
        Ok(())
    }

    /// Drop all indexed data for a repo (symbols + graph) before a re-index.
    pub async fn clear_repo(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM code_symbols WHERE repo_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("code_symbols.clear"))?;
        // Edges reference nodes (ON DELETE CASCADE), so dropping nodes drops edges.
        sqlx::query("DELETE FROM code_nodes WHERE repo_id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("code_nodes.clear"))?;
        Ok(())
    }

    // -- symbols -----------------------------------------------------------

    /// Bulk-insert symbols in one transaction.
    pub async fn insert_symbols(&self, ws: &str, repo_id: &str, syms: &[CodeSymbol]) -> Result<usize> {
        if syms.is_empty() {
            return Ok(0);
        }
        let now = fmt(Utc::now());
        let mut tx = self.pool.begin().await.map_err(dberr("code_symbols.tx"))?;
        for s in syms {
            sqlx::query(
                "INSERT INTO code_symbols (id,workspace_id,repo_id,name,kind,lang,file,line,signature,created_at) \
                 VALUES (?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(new_id())
            .bind(ws)
            .bind(repo_id)
            .bind(&s.name)
            .bind(&s.kind)
            .bind(&s.lang)
            .bind(&s.file)
            .bind(s.line)
            .bind(&s.signature)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(dberr("code_symbols.insert"))?;
        }
        tx.commit().await.map_err(dberr("code_symbols.commit"))?;
        Ok(syms.len())
    }

    /// Search symbols by name substring (case-insensitive). Optional repo filter.
    pub async fn search_symbols(
        &self,
        ws: &str,
        query: &str,
        repo_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<CodeSymbol>> {
        let mut sql = String::from("SELECT * FROM code_symbols WHERE workspace_id = ?");
        if repo_id.is_some() {
            sql.push_str(" AND repo_id = ?");
        }
        if !query.trim().is_empty() {
            sql.push_str(" AND lower(name) LIKE ?");
        }
        sql.push_str(" ORDER BY length(name) ASC LIMIT ?");
        let mut q = sqlx::query(&sql).bind(ws);
        if let Some(r) = repo_id {
            q = q.bind(r);
        }
        if !query.trim().is_empty() {
            q = q.bind(format!("%{}%", query.to_lowercase()));
        }
        q = q.bind(if limit > 0 { limit } else { 50 });
        let rows = q.fetch_all(&self.pool).await.map_err(dberr("code_symbols.search"))?;
        Ok(rows.iter().map(row_to_symbol).collect())
    }

    /// Symbols defined in a file (1-based line order).
    pub async fn symbols_in_file(&self, repo_id: &str, file: &str) -> Result<Vec<CodeSymbol>> {
        let rows = sqlx::query("SELECT * FROM code_symbols WHERE repo_id = ? AND file = ? ORDER BY line ASC")
            .bind(repo_id)
            .bind(file)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("code_symbols.in_file"))?;
        Ok(rows.iter().map(row_to_symbol).collect())
    }

    // -- graph -------------------------------------------------------------

    /// Upsert a node, returning its (stable) id.
    pub async fn upsert_node(&self, ws: &str, n: &NewNode) -> Result<String> {
        let id = node_id(ws, n.repo_id.as_deref(), &n.kind, &n.key);
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO code_nodes (id,workspace_id,repo_id,kind,key,label,file,line,meta_json,created_at) \
             VALUES (?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(workspace_id,repo_id,kind,key) DO UPDATE SET label=excluded.label, file=excluded.file, line=excluded.line, meta_json=excluded.meta_json",
        )
        .bind(&id)
        .bind(ws)
        .bind(&n.repo_id)
        .bind(&n.kind)
        .bind(&n.key)
        .bind(&n.label)
        .bind(&n.file)
        .bind(n.line)
        .bind(&n.meta_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("code_nodes.upsert"))?;
        Ok(id)
    }

    /// Upsert an edge (dedup by (ws, src, dst, rel, detail)).
    pub async fn upsert_edge(&self, ws: &str, e: &NewEdge) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO code_edges (id,workspace_id,repo_id,src_id,dst_id,rel,detail,weight,file,line,created_at) \
             VALUES (?,?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(workspace_id,src_id,dst_id,rel,detail) DO UPDATE SET weight=excluded.weight",
        )
        .bind(new_id())
        .bind(ws)
        .bind(&e.repo_id)
        .bind(&e.src_id)
        .bind(&e.dst_id)
        .bind(&e.rel)
        .bind(&e.detail)
        .bind(e.weight)
        .bind(&e.file)
        .bind(e.line)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("code_edges.upsert"))?;
        Ok(())
    }

    /// The full graph for a workspace, optionally scoped to one repo.
    pub async fn graph(&self, ws: &str, repo_id: Option<&str>) -> Result<CodeGraph> {
        let (nq, eq) = if repo_id.is_some() {
            (
                "SELECT * FROM code_nodes WHERE workspace_id = ? AND repo_id = ?",
                "SELECT * FROM code_edges WHERE workspace_id = ? AND repo_id = ?",
            )
        } else {
            (
                "SELECT * FROM code_nodes WHERE workspace_id = ?",
                "SELECT * FROM code_edges WHERE workspace_id = ?",
            )
        };
        let mut nqs = sqlx::query(nq).bind(ws);
        let mut eqs = sqlx::query(eq).bind(ws);
        if let Some(r) = repo_id {
            nqs = nqs.bind(r);
            eqs = eqs.bind(r);
        }
        let nrows = nqs.fetch_all(&self.pool).await.map_err(dberr("code_nodes.graph"))?;
        let erows = eqs.fetch_all(&self.pool).await.map_err(dberr("code_edges.graph"))?;
        Ok(CodeGraph {
            nodes: nrows.iter().map(row_to_node).collect(),
            edges: erows.iter().map(row_to_edge).collect(),
        })
    }

    /// Find a node by its natural key.
    pub async fn find_node(
        &self,
        ws: &str,
        repo_id: Option<&str>,
        kind: &str,
        key: &str,
    ) -> Result<Option<CodeNode>> {
        let id = node_id(ws, repo_id, kind, key);
        let r = sqlx::query("SELECT * FROM code_nodes WHERE id = ?")
            .bind(&id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("code_nodes.find"))?;
        Ok(r.as_ref().map(row_to_node))
    }

    /// Breadth-first neighborhood of a node up to `depth` hops. HARD-BOUNDED so a
    /// dense hub (a symbol referenced everywhere) can't explode: at most
    /// `MAX_NODES` nodes and `MAX_EDGES` edges, with a per-node edge cap. Nodes
    /// are batch-fetched at the end (one query per chunk) rather than per-neighbor
    /// — the previous per-neighbor fetch was O(n²) and hung on big graphs.
    pub async fn neighborhood(&self, ws: &str, start: &str, depth: usize) -> Result<CodeGraph> {
        use std::collections::HashSet;
        const MAX_NODES: usize = 250;
        const MAX_EDGES: usize = 600;
        const PER_NODE_EDGES: i64 = 120;
        let depth = depth.clamp(1, 4);

        let mut seen: HashSet<String> = HashSet::new();
        let mut frontier: Vec<String> = vec![start.to_string()];
        let mut edges: Vec<CodeEdge> = Vec::new();
        let mut edge_ids: HashSet<String> = HashSet::new();
        seen.insert(start.to_string());

        'bfs: for _ in 0..depth {
            if frontier.is_empty() {
                break;
            }
            let mut next: Vec<String> = Vec::new();
            for nid in frontier.drain(..) {
                let rows = sqlx::query(
                    "SELECT * FROM code_edges WHERE workspace_id = ? AND (src_id = ? OR dst_id = ?) LIMIT ?",
                )
                .bind(ws)
                .bind(&nid)
                .bind(&nid)
                .bind(PER_NODE_EDGES)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("code_edges.neighborhood"))?;
                for r in &rows {
                    let e = row_to_edge(r);
                    let other = if e.src_id == nid { e.dst_id.clone() } else { e.src_id.clone() };
                    if edge_ids.insert(e.id.clone()) && edges.len() < MAX_EDGES {
                        edges.push(e);
                    }
                    if seen.len() < MAX_NODES && seen.insert(other.clone()) {
                        next.push(other);
                    }
                    if seen.len() >= MAX_NODES || edges.len() >= MAX_EDGES {
                        break 'bfs;
                    }
                }
            }
            frontier = next;
        }

        // Drop edges to nodes we didn't keep (cap overflow), then batch-fetch.
        edges.retain(|e| seen.contains(&e.src_id) && seen.contains(&e.dst_id));
        let ids: Vec<String> = seen.into_iter().collect();
        let nodes = self.nodes_by_ids(ws, &ids).await?;
        Ok(CodeGraph { nodes, edges })
    }

    /// Batch-fetch nodes by id (chunked `IN (...)`), preserving none-found gaps.
    pub async fn nodes_by_ids(&self, ws: &str, ids: &[String]) -> Result<Vec<CodeNode>> {
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(400) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = std::iter::repeat_n("?", chunk.len()).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT * FROM code_nodes WHERE workspace_id = ? AND id IN ({placeholders})"
            );
            let mut q = sqlx::query(&sql).bind(ws);
            for id in chunk {
                q = q.bind(id);
            }
            let rows = q.fetch_all(&self.pool).await.map_err(dberr("code_nodes.by_ids"))?;
            out.extend(rows.iter().map(row_to_node));
        }
        Ok(out)
    }

    /// Distinct external-dependency labels (service + db_table) for a repo — a
    /// cheap targeted query for the Repo Brain summary (avoids loading the full
    /// graph). Returns (label, kind), capped.
    pub async fn dependency_labels(&self, repo_id: &str) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT DISTINCT kind, label FROM code_nodes \
             WHERE repo_id = ? AND kind IN ('service','db_table') ORDER BY kind, label LIMIT 200",
        )
        .bind(repo_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("code_nodes.deps"))?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<String, _>("label"), r.get::<String, _>("kind")))
            .collect())
    }

    pub async fn get_node(&self, ws: &str, id: &str) -> Result<Option<CodeNode>> {
        let r = sqlx::query("SELECT * FROM code_nodes WHERE id = ? AND workspace_id = ?")
            .bind(id)
            .bind(ws)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("code_nodes.get"))?;
        Ok(r.as_ref().map(row_to_node))
    }

    pub async fn count_symbols(&self, repo_id: &str) -> Result<i64> {
        let r = sqlx::query("SELECT COUNT(*) AS c FROM code_symbols WHERE repo_id = ?")
            .bind(repo_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("code_symbols.count"))?;
        Ok(r.get::<i64, _>("c"))
    }

    pub async fn count_edges(&self, repo_id: &str) -> Result<i64> {
        let r = sqlx::query("SELECT COUNT(*) AS c FROM code_edges WHERE repo_id = ?")
            .bind(repo_id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("code_edges.count"))?;
        Ok(r.get::<i64, _>("c"))
    }
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

fn row_to_repo(r: &sqlx::sqlite::SqliteRow) -> CodeRepo {
    CodeRepo {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        root: r.get("root"),
        name: r.get("name"),
        head: r.get("head"),
        files: r.get("files"),
        symbols: r.get("symbols"),
        edges: r.get("edges"),
        chunks: r.get("chunks"),
        status: r.get("status"),
        message: r.get("message"),
        indexed_at: r.get("indexed_at"),
        created_at: r.get("created_at"),
    }
}

fn row_to_symbol(r: &sqlx::sqlite::SqliteRow) -> CodeSymbol {
    CodeSymbol {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        repo_id: r.get("repo_id"),
        name: r.get("name"),
        kind: r.get("kind"),
        lang: r.get("lang"),
        file: r.get("file"),
        line: r.get("line"),
        signature: r.get("signature"),
    }
}

fn row_to_node(r: &sqlx::sqlite::SqliteRow) -> CodeNode {
    CodeNode {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        repo_id: r.get("repo_id"),
        kind: r.get("kind"),
        key: r.get("key"),
        label: r.get("label"),
        file: r.get("file"),
        line: r.get("line"),
        meta_json: r.get("meta_json"),
    }
}

fn row_to_edge(r: &sqlx::sqlite::SqliteRow) -> CodeEdge {
    CodeEdge {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        repo_id: r.get("repo_id"),
        src_id: r.get("src_id"),
        dst_id: r.get("dst_id"),
        rel: r.get("rel"),
        detail: r.get("detail"),
        weight: r.get("weight"),
        file: r.get("file"),
        line: r.get("line"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoriesRepo, NewMemory, Scope, SearchFilter};

    async fn seed() -> (SqlitePool, String) {
        let pool = crate::db::test_pool().await;
        let ws = new_id();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'WS', '/tmp/ws', '2026-01-01T00:00:00+00:00')")
            .bind(&ws)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, disabled, created_at) VALUES (?, 'u', 'x', 'U', 0, 0, '2026-01-01T00:00:00+00:00')")
            .bind("user1")
            .execute(&pool)
            .await
            .unwrap();
        (pool, ws)
    }

    #[tokio::test]
    async fn symbols_insert_and_search() {
        let (pool, ws) = seed().await;
        let repo = CodeIndexRepo::new(pool.clone());
        let rid = repo.upsert_repo(&ws, "/repo", "repo").await.unwrap();
        // upsert is idempotent on (ws, root)
        assert_eq!(rid, repo.upsert_repo(&ws, "/repo", "repo").await.unwrap());
        let syms = vec![
            CodeSymbol {
                id: String::new(),
                workspace_id: ws.clone(),
                repo_id: rid.clone(),
                name: "Login".into(),
                kind: "function".into(),
                lang: "go".into(),
                file: "app/login.go".into(),
                line: 10,
                signature: "func Login()".into(),
            },
            CodeSymbol {
                id: String::new(),
                workspace_id: ws.clone(),
                repo_id: rid.clone(),
                name: "GetLimits".into(),
                kind: "function".into(),
                lang: "go".into(),
                file: "app/limits.go".into(),
                line: 5,
                signature: "func GetLimits()".into(),
            },
        ];
        assert_eq!(repo.insert_symbols(&ws, &rid, &syms).await.unwrap(), 2);
        let hits = repo.search_symbols(&ws, "limit", None, 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "GetLimits");
        assert_eq!(repo.count_symbols(&rid).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn graph_upsert_and_neighborhood() {
        let (pool, ws) = seed().await;
        let repo = CodeIndexRepo::new(pool.clone());
        let rid = repo.upsert_repo(&ws, "/r", "r").await.unwrap();
        let mk = |kind: &str, key: &str| NewNode {
            repo_id: Some(rid.clone()),
            kind: kind.into(),
            key: key.into(),
            label: key.into(),
            file: None,
            line: None,
            meta_json: "{}".into(),
        };
        let login = repo.upsert_node(&ws, &mk("symbol", "Login")).await.unwrap();
        // upsert is stable: same key → same id
        assert_eq!(login, repo.upsert_node(&ws, &mk("symbol", "Login")).await.unwrap());
        let limits = repo.upsert_node(&ws, &mk("symbol", "GetLimits")).await.unwrap();
        let svc = repo.upsert_node(&ws, &mk("service", "go_casino_kit")).await.unwrap();
        let table = repo.upsert_node(&ws, &mk("db_table", "limits")).await.unwrap();
        repo.upsert_edge(&ws, &NewEdge { repo_id: Some(rid.clone()), src_id: login.clone(), dst_id: limits.clone(), rel: "calls".into(), detail: String::new(), weight: 1.0, file: None, line: None }).await.unwrap();
        repo.upsert_edge(&ws, &NewEdge { repo_id: Some(rid.clone()), src_id: limits.clone(), dst_id: svc.clone(), rel: "http_call".into(), detail: "GET /limits".into(), weight: 1.0, file: None, line: None }).await.unwrap();
        repo.upsert_edge(&ws, &NewEdge { repo_id: Some(rid.clone()), src_id: limits.clone(), dst_id: table.clone(), rel: "db_call".into(), detail: "SELECT".into(), weight: 1.0, file: None, line: None }).await.unwrap();

        let g = repo.graph(&ws, Some(&rid)).await.unwrap();
        assert_eq!(g.nodes.len(), 4);
        assert_eq!(g.edges.len(), 3);

        // From Login, 1 hop reaches GetLimits; 2 hops reaches the service + table.
        let near1 = repo.neighborhood(&ws, &login, 1).await.unwrap();
        assert!(near1.nodes.iter().any(|n| n.key == "GetLimits"));
        assert!(!near1.nodes.iter().any(|n| n.key == "go_casino_kit"));
        let near2 = repo.neighborhood(&ws, &login, 2).await.unwrap();
        assert!(near2.nodes.iter().any(|n| n.key == "go_casino_kit"));
        assert!(near2.nodes.iter().any(|n| n.key == "limits"));
    }

    #[tokio::test]
    async fn neighborhood_is_bounded_on_a_dense_hub() {
        // A hub connected to 1200 leaves must NOT explode (the old per-neighbor
        // fetch was O(n²) and hung). The result is capped + returns promptly.
        let (pool, ws) = seed().await;
        let repo = CodeIndexRepo::new(pool.clone());
        let rid = repo.upsert_repo(&ws, "/r", "r").await.unwrap();
        let hub = repo
            .upsert_node(&ws, &NewNode { repo_id: Some(rid.clone()), kind: "symbol".into(), key: "Hub".into(), label: "Hub".into(), file: None, line: None, meta_json: "{}".into() })
            .await
            .unwrap();
        for i in 0..1200 {
            let leaf = repo
                .upsert_node(&ws, &NewNode { repo_id: Some(rid.clone()), kind: "symbol".into(), key: format!("L{i}"), label: format!("L{i}"), file: None, line: None, meta_json: "{}".into() })
                .await
                .unwrap();
            repo.upsert_edge(&ws, &NewEdge { repo_id: Some(rid.clone()), src_id: hub.clone(), dst_id: leaf, rel: "calls".into(), detail: String::new(), weight: 1.0, file: None, line: None }).await.unwrap();
        }
        let g = repo.neighborhood(&ws, &hub, 2).await.unwrap();
        assert!(g.nodes.len() <= 251, "neighborhood must be bounded, got {}", g.nodes.len());
        assert!(g.edges.len() <= 600, "edges must be bounded, got {}", g.edges.len());
        assert!(g.nodes.iter().any(|n| n.key == "Hub"));
    }

    #[tokio::test]
    async fn fts5_is_available_and_ranks_matches() {
        let (pool, ws) = seed().await;
        let mem = MemoriesRepo::new(pool.clone());
        let nm = |title: &str, body: &str| NewMemory {
            collection: "code".into(),
            record_type: "item".into(),
            scope: Scope::Workspace,
            story_id: None,
            kind: "fact".into(),
            title: title.into(),
            body: body.into(),
            entities: vec![],
            tags: vec![],
            source_kind: "manual".into(),
            source_ref: None,
            refs: vec![],
            confidence: None,
            salience: None,
            visibility: "shared".into(),
        };
        let m1 = mem.create(&ws, "user1", nm("Login flow", "the player login calls the limits service")).await.unwrap();
        let _m2 = mem.create(&ws, "user1", nm("Deposit", "the player deposit flow")).await.unwrap();

        // FTS5 must be compiled into this SQLite build (requirement R5).
        assert!(mem.ensure_fts().await.unwrap(), "FTS5 must be available");
        // Keep the index in sync as the service would.
        mem.fts_index(&m1.id, &ws, "Login flow", "the player login calls the limits service").await.unwrap();

        let f = SearchFilter { collection: None, story_id: None, include_inactive: false, limit: 10 };
        let hits = mem.search_fts(&ws, "limits", &f).await.unwrap();
        assert_eq!(hits.len(), 1, "only the login memory mentions limits");
        assert_eq!(hits[0].0.id, m1.id);
        // Both match "player".
        assert_eq!(mem.search_fts(&ws, "player", &f).await.unwrap().len(), 2);
        // A query with FTS operators must not error (sanitized).
        assert!(mem.search_fts(&ws, "limits: (login) -foo*", &f).await.is_ok());
    }
}
