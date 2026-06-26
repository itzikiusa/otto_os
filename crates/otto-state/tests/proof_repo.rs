//! Roundtrip tests for `ProofRepo` + migration `0077_proof_packs.sql`.

use otto_core::proof::{ProofArtifactKind, ProofArtifactStatus, ProofStatus, WorkItemKind};
use otto_state::ProofRepo;
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

async fn mem_pool() -> SqlitePool {
    let opts = SqliteConnectOptions::new().in_memory(true).foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("in-memory sqlite");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

#[tokio::test]
async fn proof_tables_exist() {
    let pool = mem_pool().await;
    for t in ["proof_packs", "proof_artifacts"] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {t}"))
            .fetch_one(&pool)
            .await
            .unwrap_or(-1);
        assert_eq!(n, 0, "{t} should exist and be empty");
    }
}

#[tokio::test]
async fn pack_crud_and_ensure_idempotent() {
    let repo = ProofRepo::new(mem_pool().await);
    let p = repo
        .create_pack("w1", WorkItemKind::Session, "sess-1", "My task", "u1", None)
        .await
        .unwrap();
    assert_eq!(p.status, ProofStatus::Missing);
    assert_eq!(p.work_item_kind, WorkItemKind::Session);

    // find by work item
    let found = repo
        .find_by_work_item(WorkItemKind::Session, "sess-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.id, p.id);

    // ensure_pack returns the SAME pack (idempotent, no duplicate row)
    let again = repo
        .ensure_pack("w1", WorkItemKind::Session, "sess-1", "x", "u1")
        .await
        .unwrap();
    assert_eq!(again.id, p.id);
    let all = repo.list_packs("w1", None, None, None).await.unwrap();
    assert_eq!(all.len(), 1);

    // status/risk persist
    repo.set_status_risk(&p.id, ProofStatus::Partial, 42)
        .await
        .unwrap();
    let reloaded = repo.get_pack(&p.id).await.unwrap();
    assert_eq!(reloaded.status, ProofStatus::Partial);
    assert_eq!(reloaded.risk_score, 42);

    // waive
    repo.waive(&p.id, "u1", "manually verified").await.unwrap();
    let waived = repo.get_pack(&p.id).await.unwrap();
    assert_eq!(waived.status, ProofStatus::Waived);
    assert_eq!(waived.waived_by.as_deref(), Some("u1"));
}

#[tokio::test]
async fn artifacts_crud_upsert_and_cascade() {
    let repo = ProofRepo::new(mem_pool().await);
    let p = repo
        .create_pack("w1", WorkItemKind::Session, "sess-2", "t", "u1", None)
        .await
        .unwrap();

    // add a command artifact (failed first run)
    repo.upsert_artifact_by_title(
        &p.id,
        "w1",
        ProofArtifactKind::Command,
        "cargo test",
        Some("FAILED"),
        ProofArtifactStatus::Failed,
        &json!({"exit_code": 101}),
        "otto",
    )
    .await
    .unwrap();
    let arts = repo.list_artifacts(&p.id).await.unwrap();
    assert_eq!(arts.len(), 1);
    assert_eq!(arts[0].status, ProofArtifactStatus::Failed);

    // re-run same command passes -> upsert REPLACES (no duplicate, flips status)
    repo.upsert_artifact_by_title(
        &p.id,
        "w1",
        ProofArtifactKind::Command,
        "cargo test",
        Some("ok"),
        ProofArtifactStatus::Passed,
        &json!({"exit_code": 0}),
        "otto",
    )
    .await
    .unwrap();
    let arts = repo.list_artifacts(&p.id).await.unwrap();
    assert_eq!(arts.len(), 1, "upsert must not duplicate");
    assert_eq!(arts[0].status, ProofArtifactStatus::Passed);

    // a distinct manual artifact appends
    let manual = repo
        .add_artifact(
            &p.id,
            "w1",
            ProofArtifactKind::SelfReview,
            "Agent self-review",
            Some("looks safe"),
            ProofArtifactStatus::Info,
            &json!({}),
            "u1",
        )
        .await
        .unwrap();
    assert_eq!(repo.list_artifacts(&p.id).await.unwrap().len(), 2);

    // delete one artifact
    repo.delete_artifact(&manual.id).await.unwrap();
    assert_eq!(repo.list_artifacts(&p.id).await.unwrap().len(), 1);

    // cascade: deleting the pack removes its artifacts
    repo.delete_pack(&p.id).await.unwrap();
    assert_eq!(repo.list_artifacts(&p.id).await.unwrap().len(), 0);
}

#[tokio::test]
async fn list_filters_and_children() {
    let repo = ProofRepo::new(mem_pool().await);
    let parent = repo
        .create_pack("w1", WorkItemKind::GoalLoop, "loop-1", "Goal", "u1", None)
        .await
        .unwrap();
    repo.create_pack(
        "w1",
        WorkItemKind::Session,
        "sess-3",
        "child",
        "u1",
        Some(&parent.id),
    )
    .await
    .unwrap();
    repo.create_pack("w1", WorkItemKind::Review, "rev-1", "rev", "u1", None)
        .await
        .unwrap();

    let sessions = repo
        .list_packs("w1", None, Some("session"), None)
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);

    let children = repo.list_children(&parent.id).await.unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].work_item_id, "sess-3");
}

// -- v2 --------------------------------------------------------------------

#[tokio::test]
async fn v2_columns_persist_repo_link_done_sha_waived_at() {
    let repo = ProofRepo::new(mem_pool().await);
    let p = repo
        .create_pack("w1", WorkItemKind::Session, "sess-v2", "t", "u1", None)
        .await
        .unwrap();
    assert_eq!(p.done_score, 0);
    assert!(p.repo_id.is_none() && p.pr_number.is_none() && p.waived_at.is_none());

    // repo link (idempotent COALESCE: learn PR number later)
    repo.set_repo_link(&p.id, Some("repo-1"), None).await.unwrap();
    repo.set_repo_link(&p.id, None, Some(42)).await.unwrap();
    let linked = repo.get_pack(&p.id).await.unwrap();
    assert_eq!(linked.repo_id.as_deref(), Some("repo-1"));
    assert_eq!(linked.pr_number, Some(42));

    // status/risk/done together
    repo.set_status_risk_done(&p.id, ProofStatus::Partial, 30, 58)
        .await
        .unwrap();
    let r = repo.get_pack(&p.id).await.unwrap();
    assert_eq!(r.done_score, 58);
    assert_eq!(r.risk_score, 30);

    // content_sha256 stamped for inline content (ref_kind=inline/default)
    let a = repo
        .add_artifact(
            &p.id,
            "w1",
            ProofArtifactKind::Log,
            "log",
            Some("hello world"),
            ProofArtifactStatus::Info,
            &json!({"ref_kind": "inline"}),
            "otto",
        )
        .await
        .unwrap();
    assert_eq!(
        a.content_sha256.as_deref(),
        Some(otto_core::proof::content_sha256("hello world").as_str())
    );
    // url refs get no content hash
    let u = repo
        .add_artifact(
            &p.id,
            "w1",
            ProofArtifactKind::Ci,
            "CI",
            Some("https://ci.example/123"),
            ProofArtifactStatus::Passed,
            &json!({"ref_kind": "url"}),
            "otto",
        )
        .await
        .unwrap();
    assert!(u.content_sha256.is_none());

    // waived_at recorded
    repo.waive(&p.id, "human-1", "reviewed end to end").await.unwrap();
    let w = repo.get_pack(&p.id).await.unwrap();
    assert!(w.waived_at.is_some());
    assert_eq!(w.waived_by.as_deref(), Some("human-1"));
}

#[tokio::test]
async fn snapshots_are_monotonic_and_roundtrip() {
    let repo = ProofRepo::new(mem_pool().await);
    let p = repo
        .create_pack("w1", WorkItemKind::Session, "sess-snap", "t", "u1", None)
        .await
        .unwrap();

    let s1 = repo
        .create_snapshot(&p.id, "w1", "sha-a", "passed", 90, 10, "{}", "# md", "<html>", "first", "u1")
        .await
        .unwrap();
    let s2 = repo
        .create_snapshot(&p.id, "w1", "sha-b", "passed", 95, 8, "{}", "# md2", "<html2>", "second", "u1")
        .await
        .unwrap();
    assert_eq!(s1.seq, 1);
    assert_eq!(s2.seq, 2);

    let list = repo.list_snapshots(&p.id).await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].seq, 2, "newest first");

    let got = repo.get_snapshot(&s1.id).await.unwrap();
    assert_eq!(got.sha256, "sha-a");
    assert_eq!(got.report_md, "# md");
    assert_eq!(got.note, "first");

    // cascade: deleting the pack removes its snapshots
    repo.delete_pack(&p.id).await.unwrap();
    assert_eq!(repo.list_snapshots(&p.id).await.unwrap().len(), 0);
}

#[tokio::test]
async fn blob_roundtrip_and_cascade() {
    let repo = ProofRepo::new(mem_pool().await);
    let p = repo
        .create_pack("w1", WorkItemKind::Manual, "sess-blob", "t", "u1", None)
        .await
        .unwrap();
    let art = repo
        .add_artifact(
            &p.id,
            "w1",
            ProofArtifactKind::Screenshot,
            "shot",
            Some("blob:pending"),
            ProofArtifactStatus::Info,
            &json!({"ref_kind": "blob"}),
            "u1",
        )
        .await
        .unwrap();
    let bytes = vec![0x89u8, 0x50, 0x4e, 0x47, 1, 2, 3];
    let sha = otto_core::proof::bytes_sha256(&bytes);
    repo.add_blob(&art.id, "w1", &sha, "image/png", &bytes)
        .await
        .unwrap();
    let blob = repo.blob_for_artifact(&art.id).await.unwrap().unwrap();
    assert_eq!(blob.data, bytes);
    assert_eq!(blob.mime, "image/png");
    assert_eq!(blob.size_bytes, bytes.len() as i64);
    assert_eq!(blob.sha256, sha);

    // cascade: deleting the artifact removes the blob
    repo.delete_artifact(&art.id).await.unwrap();
    assert!(repo.blob_for_artifact(&art.id).await.unwrap().is_none());
}
