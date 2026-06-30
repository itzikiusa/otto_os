//! End-to-end test of the Vault v2 code-intelligence vertical: index a fixture
//! repo, then assert the symbol index, dependency graph, hybrid code search, and
//! the Repo Brain all reflect it.

use std::fs;
use std::path::Path;

use otto_memory::MemoryService;

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, content).unwrap();
}

/// A miniature go_admission-style login flow:
/// Login → GetLimits → http_call(LIMITS via go_casino_kit) + db_call(limits table)
fn fixture(dir: &Path) {
    write(
        dir,
        "app/login.go",
        r#"package app

import (
    "context"
    "bitbucket.org/gamescale-rnd/go_casino_kit/clients"
)

// Login authenticates the player then loads their limits.
func Login(ctx context.Context, brandId int, login string) error {
    if err := authenticate(ctx, login); err != nil {
        return err
    }
    _ = GetLimits(ctx, brandId)
    return nil
}

func authenticate(ctx context.Context, login string) error { return nil }
"#,
    );
    write(
        dir,
        "app/limits.go",
        r#"package app

import "context"

// GetLimits fetches the reality-check limits for a brand, defaulting when absent.
func GetLimits(ctx context.Context, brandId int) int {
    url, _ := serviceLocator.GetBrandService(ctx, brandId, "LIMITS")
    resp, _ := restClient.GetRequest(ctx, url)
    _ = resp
    row, _ := conn.GetContext(ctx, "SELECT max_limit FROM MdlGm_tblLimits WHERE brand_id = ?", brandId)
    if row == 0 {
        return 1000 // default
    }
    return row
}
"#,
    );
    write(
        dir,
        "app/login_test.go",
        "package app\nfunc TestLogin(t *testing.T) {}\n",
    );
}

#[tokio::test]
async fn indexes_repo_and_builds_brain() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);

    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path());

    let res = svc
        .index_repo(&ws, &user, tmp.path(), Some("go_admission"))
        .await
        .expect("index repo");
    assert!(res.symbols >= 3, "expected symbols, got {}", res.symbols);
    assert!(res.edges >= 3, "expected graph edges, got {}", res.edges);
    assert!(res.chunks >= 1, "expected embedded chunks");

    // Repo state recorded.
    let repos = svc.list_repos(&ws).await.unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].status, "ready");
    assert_eq!(repos[0].name, "go_admission");

    // Symbol index.
    let syms = svc.search_symbols(&ws, "limits", None, 10).await.unwrap();
    assert!(syms.iter().any(|s| s.name == "GetLimits"));

    // Dependency graph: http_call → LIMITS, db_call → MdlGm_tblLimits, import → go_casino_kit.
    let g = svc.code_graph(&ws, Some(&res.repo_id)).await.unwrap();
    assert!(g.edges.iter().any(|e| e.rel == "http_call"), "http_call edge");
    assert!(
        g.nodes.iter().any(|n| n.kind == "db_table" && n.key == "MdlGm_tblLimits"),
        "db table node"
    );
    assert!(
        g.nodes.iter().any(|n| n.kind == "service" && n.key == "go_casino_kit"),
        "cross-repo service node"
    );
    assert!(g.edges.iter().any(|e| e.rel == "test_of"), "test_of edge from login_test.go");

    // Hybrid code search returns chunks with structured reasons.
    let hits = svc
        .search(
            &ws,
            otto_memory::MemoryQuery {
                text: Some("limits service".into()),
                k: 5,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!hits.is_empty(), "code search returned hits");
    assert!(
        hits.iter().all(|h| !h.reasons.is_empty()),
        "every hit explains why it was selected"
    );

    // Repo Brain assembles a markdown context block naming the dependency.
    let brain = svc
        .repo_brain(&ws, "login limits", Some(tmp.path()), 1500)
        .await
        .unwrap();
    assert!(!brain.markdown.is_empty(), "brain markdown");
    assert!(brain.markdown.contains("Repo Brain"));
    assert!(
        brain.sections.iter().any(|s| s.heading.contains("Dependencies")),
        "brain includes the dependency neighborhood:\n{}",
        brain.markdown
    );
    assert!(!brain.reasons.is_empty(), "brain carries reasons");
}

#[tokio::test]
async fn doc_links_into_the_code_graph() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);
    let tmp = tempfile::tempdir().unwrap();
    fixture(tmp.path());
    let res = svc.index_repo(&ws, &user, tmp.path(), None).await.unwrap();

    // Link a "Login Flow" doc to the Login symbol node.
    let login_node = svc
        .code_node_id(&ws, Some(&res.repo_id), "symbol", "app/login.go#Login")
        .await
        .unwrap()
        .expect("login node exists");
    let doc = svc
        .upsert_doc(
            &ws,
            &user,
            Some(&res.repo_id),
            "Login Flow",
            "Brief: the login flow authenticates then loads limits via go_casino_kit.",
            std::slice::from_ref(&login_node),
        )
        .await
        .unwrap();
    assert_eq!(doc.collection, "docs");

    // The doc node + `documents` edge now exist in the graph.
    let g = svc.code_graph(&ws, Some(&res.repo_id)).await.unwrap();
    assert!(g.nodes.iter().any(|n| n.kind == "doc" && n.label == "Login Flow"));
    assert!(g.edges.iter().any(|e| e.rel == "documents" && e.dst_id == login_node));
}
