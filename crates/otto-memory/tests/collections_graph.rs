//! E2E: collections (code/docs chunking) + graphify graph import & traversal.

use otto_memory::ingest::{GraphifyEdge, GraphifyGraph, GraphifyNode};
use otto_memory::{MemoryQuery, MemoryService, SearchMode};

const GO_SRC: &str = r#"
package wallet

// WithdrawalService handles payouts.
func (s *WithdrawalService) Withdraw(ctx context.Context, amount int64) error {
    if amount > 200000 {
        return ErrManualReviewRequired // amounts over 2000.00 need compliance review
    }
    return s.process(ctx, amount)
}

func (s *WithdrawalService) process(ctx context.Context, amount int64) error {
    // debit ledger, emit settlement webhook
    return nil
}
"#;

#[tokio::test]
async fn code_chunks_are_searchable() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);

    // Add a docs/code haystack, then the needle file.
    for i in 0..30 {
        let body = format!("// file {i}\nfunc helper{i}() {{ log.Print(\"noise {i}\") }}\n");
        svc.ingest_text(&ws, &user, "code", &format!("noise_{i}.go"), &body)
            .await
            .unwrap();
    }
    let n = svc
        .ingest_text(&ws, &user, "code", "wallet/withdrawal.go", GO_SRC)
        .await
        .unwrap();
    assert!(n >= 1, "expected at least one chunk");

    // Needle: find the manual-review rule in code.
    let hits = svc
        .search(
            &ws,
            MemoryQuery {
                text: Some("manual compliance review withdrawal amount".into()),
                collection: Some("code".into()),
                mode: SearchMode::Hybrid,
                k: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        hits.iter().any(|h| h.memory.body.contains("ManualReviewRequired")),
        "should find the withdrawal-review code chunk"
    );
    assert!(hits.iter().all(|h| h.memory.collection == "code"));
    assert!(hits.iter().all(|h| h.memory.record_type == "chunk"));
}

#[tokio::test]
async fn graphify_import_builds_traversable_graph() {
    let (pool, ws, user) = otto_memory::test_support::mem_pool().await;
    let svc = MemoryService::with_defaults(pool);

    let graph = GraphifyGraph {
        nodes: vec![
            GraphifyNode {
                id: "withdrawal_service".into(),
                label: Some("WithdrawalService".into()),
                kind: Some("service".into()),
                summary: Some("Handles payouts and compliance review".into()),
                file: Some("wallet/withdrawal.go".into()),
            },
            GraphifyNode {
                id: "compliance".into(),
                label: Some("Compliance".into()),
                kind: Some("service".into()),
                summary: Some("Reviews large withdrawals".into()),
                file: None,
            },
        ],
        edges: vec![GraphifyEdge {
            source: "withdrawal_service".into(),
            target: "compliance".into(),
            rel: Some("calls".into()),
            certainty: Some("extracted".into()),
        }],
    };

    let stats = svc.import_graph(&ws, &user, "code", graph).await.unwrap();
    assert_eq!(stats.nodes, 2);
    assert_eq!(stats.edges, 1);

    // Find the WithdrawalService entity, then traverse to its neighbor.
    let hits = svc
        .search(
            &ws,
            MemoryQuery {
                text: Some("WithdrawalService payouts".into()),
                mode: SearchMode::Hybrid,
                k: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let entity = hits
        .iter()
        .find(|h| h.memory.kind == "entity")
        .expect("the imported entity should be searchable");

    let (links, neighbors) = svc.entity_graph(&ws, &entity.memory.id).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].rel, "calls");
    assert_eq!(links[0].certainty.as_deref(), Some("extracted"));
    assert!(
        neighbors.iter().any(|m| m.title == "Compliance"),
        "traversal should reach the Compliance entity"
    );
}
