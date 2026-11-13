use super::*;
use crate::store::{NewArtifactRow, NewVersion};

async fn store() -> Store {
    Store::new(otto_state::db::test_pool().await)
}

async fn artifact(s: &Store, id: &str) {
    s.insert_artifact(&NewArtifactRow {
        id: id.into(),
        project_id: None,
        workspace_id: "w1".into(),
        studio: "frames".into(),
        format: "html".into(),
        mime: "text/html".into(),
        title: format!("Artifact {id}"),
        status: "draft".into(),
        tags: vec![],
        meta: serde_json::json!({}),
        source_kind: None,
        source_id: None,
        created_by: "u1".into(),
        created_by_kind: "user".into(),
        created_session_id: None,
        created_at: None,
    })
    .await
    .unwrap();
}

async fn version(s: &Store, aid: &str, sha: &str) -> crate::types::DesignVersion {
    s.commit_version(
        NewVersion {
            artifact_id: aid.into(),
            blob_sha256: sha.into(),
            size_bytes: 3,
            kind: "autosave".into(),
            branch: "main".into(),
            author_kind: "user".into(),
            author_id: "u1".into(),
            session_id: None,
            message: String::new(),
            provenance: serde_json::json!({}),
        },
        None,
    )
    .await
    .unwrap()
}

fn new(aid: &str, body: &str, v: Option<&crate::types::DesignVersion>) -> NewComment {
    NewComment {
        artifact_id: aid.into(),
        workspace_id: "w1".into(),
        version_id: v.map(|v| v.id.clone()),
        version_seq: v.map(|v| v.seq),
        parent_id: None,
        author_kind: "user".into(),
        author_id: "u1".into(),
        author_name: "Ana".into(),
        session_id: None,
        body: body.into(),
        pin: None,
    }
}

#[test]
fn bodies_are_trimmed_bounded_and_never_empty() {
    assert_eq!(
        clean_body("  fix the CTA \u{0007}\n").unwrap(),
        "fix the CTA"
    );
    assert_eq!(clean_body("a\nb").unwrap(), "a\nb");
    assert!(clean_body("   ").is_err());
    assert!(clean_body(&"x".repeat(MAX_BODY_CHARS)).is_ok());
    assert!(clean_body(&"x".repeat(MAX_BODY_CHARS + 1)).is_err());
}

#[test]
fn pins_must_sit_on_the_stage() {
    let p = |x: f64, y: f64| {
        Some(CommentPin {
            x,
            y,
            frame: Some("  ".into()),
        })
    };
    assert_eq!(
        clean_pin(p(0.0, 1.0)).unwrap(),
        Some(CommentPin {
            x: 0.0,
            y: 1.0,
            frame: None
        })
    );
    assert!(clean_pin(p(-0.01, 0.5)).is_err());
    assert!(clean_pin(p(0.5, 1.01)).is_err());
    assert!(clean_pin(p(f64::NAN, 0.5)).is_err());
    assert!(clean_pin(p(0.5, f64::INFINITY)).is_err());
    assert_eq!(clean_pin(None).unwrap(), None);
    let long = Some(CommentPin {
        x: 0.5,
        y: 0.5,
        frame: Some("f".repeat(MAX_FRAME_CHARS + 1)),
    });
    assert!(clean_pin(long).is_err());
}

#[tokio::test]
async fn create_list_edit_resolve_delete_roundtrip() {
    let s = store().await;
    artifact(&s, "A").await;
    let v1 = version(&s, "A", "s1").await;
    let mut n = new("A", "The CTA is too small", Some(&v1));
    n.pin = Some(CommentPin {
        x: 0.42,
        y: 0.18,
        frame: Some("hero".into()),
    });
    let c = s.insert_comment(n).await.unwrap();
    assert_eq!(c.version_seq, Some(1));
    assert_eq!(c.pin.as_ref().unwrap().frame.as_deref(), Some("hero"));
    assert!(!c.resolved && c.edited_at.is_none());
    // An unknown user id falls back to the stored name.
    assert_eq!(c.author_name, "Ana");

    let mut r = new("A", "Agreed", Some(&v1));
    r.parent_id = Some(c.id.clone());
    let reply = s.insert_comment(r).await.unwrap();

    let all = s
        .list_comments("A", &CommentFilter::default())
        .await
        .unwrap();
    assert_eq!(
        all.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec![c.id.as_str(), reply.id.as_str()],
        "oldest first"
    );

    let edited = s
        .update_comment_body(&c.id, "Make the CTA 44px")
        .await
        .unwrap();
    assert_eq!(edited.body, "Make the CTA 44px");
    assert!(edited.edited_at.is_some());

    let done = s
        .set_comment_resolved(&c.id, Some(("u2", "Ben")))
        .await
        .unwrap();
    assert!(done.resolved);
    assert_eq!(done.resolved_by.as_deref(), Some("u2"));
    assert_eq!(done.resolved_by_name.as_deref(), Some("Ben"));
    assert!(done.resolved_at.is_some());
    // A reply carries no resolved state of its own.
    assert!(s
        .set_comment_resolved(&reply.id, Some(("u2", "Ben")))
        .await
        .is_err());

    // The resolved filter selects threads; the reply follows its thread.
    let open = CommentFilter {
        resolved: Some(false),
        ..Default::default()
    };
    assert!(s.list_comments("A", &open).await.unwrap().is_empty());
    let closed = CommentFilter {
        resolved: Some(true),
        ..Default::default()
    };
    assert_eq!(s.list_comments("A", &closed).await.unwrap().len(), 2);

    let reopened = s.set_comment_resolved(&c.id, None).await.unwrap();
    assert!(!reopened.resolved && reopened.resolved_by.is_none());

    // Deleting a thread takes its replies.
    assert_eq!(s.delete_comment(&c.id).await.unwrap(), 2);
    assert!(s.get_comment(&reply.id).await.unwrap().is_none());
    assert!(s.delete_comment(&c.id).await.is_err());
}

#[tokio::test]
async fn version_filter_keeps_each_comment_on_its_own_version() {
    let s = store().await;
    artifact(&s, "A").await;
    let v1 = version(&s, "A", "s1").await;
    let v2 = version(&s, "A", "s2").await;
    s.insert_comment(new("A", "on one", Some(&v1)))
        .await
        .unwrap();
    s.insert_comment(new("A", "on two", Some(&v2)))
        .await
        .unwrap();
    let on1 = s
        .list_comments(
            "A",
            &CommentFilter {
                version_id: Some(v1.id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(on1.len(), 1);
    assert_eq!(on1[0].body, "on one");
    assert_eq!(on1[0].version_seq, Some(1));
}

#[tokio::test]
async fn list_is_bounded_and_keeps_the_newest() {
    let s = store().await;
    artifact(&s, "A").await;
    for i in 0..5 {
        s.insert_comment(new("A", &format!("c{i}"), None))
            .await
            .unwrap();
    }
    let page = s
        .list_comments(
            "A",
            &CommentFilter {
                limit: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        page.iter().map(|c| c.body.as_str()).collect::<Vec<_>>(),
        vec!["c3", "c4"]
    );
    // A zero / negative limit is still one row, never "everything".
    let one = s
        .list_comments(
            "A",
            &CommentFilter {
                limit: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(one.len(), 1);
}

#[tokio::test]
async fn hard_deleting_the_artifact_removes_its_comments() {
    let s = store().await;
    artifact(&s, "A").await;
    artifact(&s, "B").await;
    let c = s.insert_comment(new("A", "gone soon", None)).await.unwrap();
    let keep = s.insert_comment(new("B", "stays", None)).await.unwrap();
    s.delete_artifact("A").await.unwrap();
    assert!(s.get_comment(&c.id).await.unwrap().is_none());
    assert!(s.get_comment(&keep.id).await.unwrap().is_some());
}

#[tokio::test]
async fn brief_lines_label_open_threads_with_version_and_pin() {
    let s = store().await;
    artifact(&s, "A").await;
    let v3 = version(&s, "A", "s1").await;
    let mut pinned = new("A", "Button\nlabel is vague", Some(&v3));
    pinned.pin = Some(CommentPin {
        x: 0.42,
        y: 0.1,
        frame: None,
    });
    let first = s.insert_comment(pinned).await.unwrap();
    s.insert_comment(new("A", "Whole page feels busy", None))
        .await
        .unwrap();
    let done = s.insert_comment(new("A", "old note", None)).await.unwrap();
    s.set_comment_resolved(&done.id, Some(("u1", "Ana")))
        .await
        .unwrap();
    let mut reply = new("A", "a reply", None);
    reply.parent_id = Some(first.id.clone());
    s.insert_comment(reply).await.unwrap();

    let lines = brief_for(&s, "A").await;
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(
        lines[0],
        "[C1] on v1 · pinned at 42% across, 10% down · Ana: Button label is vague"
    );
    assert_eq!(
        lines[1],
        "[C2] on the draft · no pin (the whole design) · Ana: Whole page feels busy"
    );
}
