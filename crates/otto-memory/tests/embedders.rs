//! E2E: the remote (OpenAI/Voyage) embedder posts the right request and parses
//! the response — exercised against a mock server (no keys/network).

use otto_memory::embed::{Embedder, RemoteEmbedder};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn remote_embedder_posts_and_parses() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [
                { "embedding": [0.1, 0.2, 0.3] },
                { "embedding": [0.4, 0.5, 0.6] }
            ]
        })))
        .mount(&server)
        .await;

    let e = RemoteEmbedder::with(
        "test-key".into(),
        server.uri(),
        "text-embedding-3-small".into(),
        3,
    );
    let out = e.embed(&["hello".into(), "world".into()]).await.unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0], vec![0.1, 0.2, 0.3]);
    assert_eq!(out[1], vec![0.4, 0.5, 0.6]);
    assert_eq!(e.model_id(), "text-embedding-3-small");
    assert_eq!(e.dim(), 3);
}

#[tokio::test]
async fn remote_embedder_surfaces_http_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let e = RemoteEmbedder::with("bad".into(), server.uri(), "m".into(), 3);
    assert!(e.embed(&["x".into()]).await.is_err());
}

#[tokio::test]
async fn service_with_remote_embedder_indexes_and_recalls() {
    use otto_memory::{MemoryQuery, MemoryService, NewMemory, Scope, SearchMode};
    use std::sync::Arc;

    let server = MockServer::start().await;
    // Deterministic 3-d embedding so the needle and its query land near each other.
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [ { "embedding": [1.0, 0.0, 0.0] } ]
        })))
        .mount(&server)
        .await;

    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let embedder = Arc::new(RemoteEmbedder::with(
        "k".into(),
        server.uri(),
        "text-embedding-3-small".into(),
        3,
    ));
    let svc = MemoryService::with_embedder(pool, embedder);

    svc.save(
        &ws,
        &user,
        vec![NewMemory {
            collection: "product".into(),
            record_type: "item".into(),
            scope: Scope::Workspace,
            story_id: None,
            kind: "fact".into(),
            title: "Settlement".into(),
            body: "settlement happens via webhook".into(),
            entities: vec![],
            tags: vec![],
            source_kind: "manual".into(),
            source_ref: None,
            refs: vec![],
            confidence: None,
            salience: None,
        }],
    )
    .await
    .unwrap();

    // Semantic mode goes through the (mock) remote embedder for the query too.
    let hits = svc
        .search(
            &ws,
            MemoryQuery {
                text: Some("how does settlement work".into()),
                mode: SearchMode::Semantic,
                k: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(hits.iter().any(|h| h.memory.body.contains("webhook")));
}
