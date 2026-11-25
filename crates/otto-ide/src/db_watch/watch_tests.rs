//! Watch engine unit tests against a fake executor (no database).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_core::auth::BoxFuture;
use otto_core::domain::User;
use otto_core::event::Event;
use otto_core::{Error, Result};
use serde_json::{json, Value};
use tokio::sync::watch;

use super::*;
use crate::db_watch::{DbWatchExecOutcome, DbWatchOp, DbWatchQueryExec};

/// columns + rows of one fake table.
type Table = (Vec<String>, Vec<Vec<Value>>);

#[derive(Default)]
struct Fake {
    tables: Mutex<HashMap<String, Table>>,
    latency: Duration,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    reads: AtomicUsize,
    events: Mutex<Vec<(u32, String)>>,
}

impl Fake {
    fn set(&self, table: &str, columns: &[&str], rows: Vec<Vec<Value>>) {
        lock(&self.tables).insert(
            table.into(),
            (columns.iter().map(|c| c.to_string()).collect(), rows),
        );
    }
}

impl WatchHost for Fake {
    fn exec<'a>(
        &'a self,
        _: &'a User,
        exec: DbWatchQueryExec,
    ) -> BoxFuture<'a, Result<DbWatchExecOutcome>> {
        Box::pin(async move {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(self.latency).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            let table = lock(&self.tables).get(&exec.object_name).cloned();
            let (columns, rows) =
                table.ok_or_else(|| Error::NotFound(format!("table {}", exec.object_name)))?;
            Ok(match exec.op {
                DbWatchOp::Describe => DbWatchExecOutcome {
                    primary_key: vec![columns[0].clone()],
                    row_estimate: Some(rows.len() as i64),
                    ..Default::default()
                },
                _ => {
                    self.reads.fetch_add(1, Ordering::SeqCst);
                    DbWatchExecOutcome {
                        columns,
                        rows,
                        ..Default::default()
                    }
                }
            })
        })
    }

    fn emit(&self, event: Event) {
        if let Event::IdeDbWatchCheckpoint { seq, reason, .. } = event {
            lock(&self.events).push((seq, reason));
        }
    }
}

fn user(id: &str) -> User {
    User {
        id: id.into(),
        username: id.into(),
        display_name: id.into(),
        is_root: true,
        disabled: false,
        created_at: chrono::Utc::now(),
    }
}

fn target(local: bool) -> WatchTarget {
    WatchTarget {
        info: IdeDbWatchTargetInfo {
            kind: "saved".into(),
            connection_id: Some("c1".into()),
            ephemeral_id: None,
            engine: IdeDbEngine::Mysql,
            label: "staging".into(),
            environment: Some("dev".into()),
            local,
        },
        exec: DbWatchQueryTarget::Saved {
            connection_id: "c1".into(),
        },
    }
}

fn items(names: &[&str]) -> Vec<IdeDbWatchItemSpec> {
    names
        .iter()
        .map(|n| IdeDbWatchItemSpec {
            object: n.to_string(),
            filter: None,
        })
        .collect()
}

fn as_host(f: &Arc<Fake>) -> Arc<dyn WatchHost> {
    Arc::clone(f) as Arc<dyn WatchHost>
}

#[tokio::test]
async fn compares_since_the_last_stop_and_since_start() {
    let fake = Arc::new(Fake::default());
    fake.set(
        "orders",
        &["id", "status"],
        vec![vec![json!(1), json!("new")]],
    );
    let host = as_host(&fake);
    let w = Watch::new(
        &user("u"),
        "/p",
        target(true),
        Some("db".into()),
        items(&["orders"]),
    )
    .unwrap();
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Start),
        true,
    )
    .await;
    let first = w.read(IdeDbWatchCompare::PreviousStop, false, false);
    assert_eq!(first.from_seq, None, "one checkpoint: nothing to compare");
    assert_eq!(
        first.items[0].current.as_ref().unwrap().rows.len(),
        1,
        "rows at the baseline"
    );
    assert_eq!(w.view().items[0].primary_key, vec!["id".to_string()]);

    fake.set(
        "orders",
        &["id", "status"],
        vec![vec![json!(2), json!("new")], vec![json!(1), json!("paid")]],
    );
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Stop),
        true,
    )
    .await;
    fake.set(
        "orders",
        &["id", "status"],
        vec![
            vec![json!(2), json!("new")],
            vec![json!(1), json!("shipped")],
        ],
    );
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Stop),
        true,
    )
    .await;

    let last = w.read(IdeDbWatchCompare::PreviousStop, false, false);
    assert_eq!((last.from_seq, last.to_seq), (Some(1), Some(2)));
    let it = &last.items[0];
    assert_eq!(
        (it.added_count, it.modified_count, it.unchanged_count),
        (0, 1, 1)
    );
    assert_eq!(it.diffs[0].changes[0].before, json!("paid"));
    assert_eq!(it.diffs[0].changes[0].after, json!("shipped"));
    assert!(
        it.executed_query.contains("ORDER BY `id` DESC LIMIT 150"),
        "{}",
        it.executed_query
    );
    assert_eq!(it.window_note, "all 2 rows");

    let start = w.read(IdeDbWatchCompare::Start, false, false);
    assert_eq!((start.from_seq, start.to_seq), (Some(0), Some(2)));
    assert_eq!(
        (start.items[0].added_count, start.items[0].modified_count),
        (1, 1)
    );
    assert_eq!(
        lock(&fake.events).iter().map(|e| e.0).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
}

#[tokio::test]
async fn a_missing_table_fails_alone() {
    let fake = Arc::new(Fake::default());
    fake.set("orders", &["id"], vec![vec![json!(1)]]);
    let host = as_host(&fake);
    let w = Watch::new(
        &user("u"),
        "/p",
        target(true),
        Some("pr_bo_1".into()),
        items(&["orders", "ghost"]),
    )
    .unwrap();
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Start),
        true,
    )
    .await;
    let v = w.view();
    assert_eq!(v.latest.as_ref().unwrap().failed_items, 1);
    assert_eq!(
        v.items[1].error.as_deref(),
        Some("ghost: not found in pr_bo_1")
    );
    let r = w.read(IdeDbWatchCompare::PreviousStop, false, false);
    assert!(r.items[0].error.is_none());
    assert_eq!(r.items[0].current.as_ref().unwrap().rows.len(), 1);
    // Created later (TestMain made it): the next checkpoint describes it again.
    fake.set("ghost", &["id"], vec![]);
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Stop),
        true,
    )
    .await;
    assert!(w.view().items[1].error.is_none());
}

#[tokio::test]
async fn an_agent_reads_masked_values_and_a_person_raw_ones() {
    let fake = Arc::new(Fake::default());
    fake.set(
        "users",
        &["id", "api_secret", "note"],
        vec![vec![json!(1), json!("s3cr3t"), json!("hi")]],
    );
    let host = as_host(&fake);
    let w = Watch::new(
        &user("u"),
        "/p",
        target(true),
        Some("db".into()),
        items(&["users"]),
    )
    .unwrap();
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Start),
        true,
    )
    .await;
    fake.set(
        "users",
        &["id", "api_secret", "note"],
        vec![vec![json!(1), json!("n3w"), json!("hi")]],
    );
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Stop),
        true,
    )
    .await;

    let person = w.read(IdeDbWatchCompare::PreviousStop, true, false);
    let change = &person.items[0].diffs[0].changes[0];
    assert_eq!(
        (change.before.clone(), change.after.clone(), change.masked),
        (json!("s3cr3t"), json!("n3w"), false)
    );

    let agent = w.read(IdeDbWatchCompare::PreviousStop, true, true);
    let d = &agent.items[0].diffs[0];
    assert!(d.masked);
    assert_eq!(d.changes[0].column, "api_secret");
    assert!(d.changes[0].masked, "changed · hidden");
    assert_eq!(d.changes[0].after, json!("[redacted]"));
    assert_eq!(
        agent.items[0].current.as_ref().unwrap().rows[0][1],
        json!("[redacted]")
    );
    assert_eq!(
        agent.items[0].current.as_ref().unwrap().rows[0][2],
        json!("hi")
    );
    let text = serde_json::to_string(&agent).unwrap();
    assert!(!text.contains("s3cr3t") && !text.contains("n3w"), "{text}");
}

#[test]
fn a_redis_value_under_a_secret_key_is_masked() {
    let cols: Vec<String> = ["key", "type", "value", "ttl"]
        .iter()
        .map(|c| c.to_string())
        .collect();
    let (row, hit) = mask_row(
        &cols,
        &[
            json!("auth:token:42"),
            json!("string"),
            json!("abc"),
            json!(30),
        ],
    );
    assert!(hit);
    assert_eq!(row[2], json!("[redacted]"));
    let (row, hit) = mask_row(
        &cols,
        &[
            json!("cart:42"),
            json!("string"),
            json!("3 items"),
            json!(30),
        ],
    );
    assert!(!hit);
    assert_eq!(row[2], json!("3 items"));
}

/// 20 stops in 2 s against a database answering in 500 ms: one checkpoint
/// at a time (never a queue), ≤ 2 queries in flight on a saved target, seq
/// in order, and the last checkpoint is taken after the last stop.
#[tokio::test]
async fn rapid_stepping_coalesces_into_ordered_checkpoints() {
    let fake = Arc::new(Fake {
        latency: Duration::from_millis(500),
        ..Fake::default()
    });
    for t in ["a", "b", "c"] {
        fake.set(t, &["id"], vec![vec![json!(1)]]);
    }
    let host = as_host(&fake);
    let w = Watch::new(
        &user("u"),
        "/p",
        target(false),
        Some("db".into()),
        items(&["a", "b", "c"]),
    )
    .unwrap();
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Start),
        true,
    )
    .await;
    let (tx, rx) = watch::channel(DapStopSignal::default());
    let where_now = Arc::new(Mutex::new(0u32));
    let loc = Arc::clone(&where_now);
    let task = bind_debug(&w, Arc::clone(&host), rx, move || {
        (Some("dao.go".into()), Some(*lock(&loc)))
    });
    for stop in 1..=20u32 {
        *lock(&where_now) = stop;
        tx.send_replace(DapStopSignal {
            stop_seq: stop,
            ended: false,
        });
        fake.set("a", &["id"], (0..stop).map(|i| vec![json!(i)]).collect());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tx.send_replace(DapStopSignal {
        stop_seq: 20,
        ended: true,
    });
    tokio::time::timeout(Duration::from_secs(20), async {
        while w.view().bound_ended_at_ms.is_none() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("session end checkpoint");
    drop(task);

    let seqs: Vec<u32> = lock(&fake.events).iter().map(|e| e.0).collect();
    assert_eq!(
        seqs,
        (0..seqs.len() as u32).collect::<Vec<_>>(),
        "in order, no gaps"
    );
    assert!(seqs.len() <= 10, "coalesced, not 21 checkpoints: {seqs:?}");
    assert!(
        fake.max_in_flight.load(Ordering::SeqCst) <= 2,
        "saved pool cap"
    );
    let reasons: Vec<String> = lock(&fake.events).iter().map(|e| e.1.clone()).collect();
    assert_eq!(reasons.last().map(String::as_str), Some("session_end"));
    // The checkpoint before the end saw the newest stop's rows.
    let r = w.read(IdeDbWatchCompare::Start, false, false);
    assert_eq!(r.items[0].added_count, 19);
}

#[tokio::test]
async fn a_frame_selection_or_output_takes_no_checkpoint() {
    let fake = Arc::new(Fake::default());
    fake.set("a", &["id"], vec![]);
    let host = as_host(&fake);
    let w = Watch::new(&user("u"), "/p", target(true), None, items(&["a"])).unwrap();
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Start),
        true,
    )
    .await;
    let (tx, rx) = watch::channel(DapStopSignal {
        stop_seq: 3,
        ended: false,
    });
    let _task = bind_debug(&w, Arc::clone(&host), rx, || (None, None));
    tx.send_replace(DapStopSignal {
        stop_seq: 3,
        ended: false,
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(w.view().checkpoint_count, 1);
    tx.send_replace(DapStopSignal {
        stop_seq: 4,
        ended: false,
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let v = w.view();
    assert_eq!(v.checkpoint_count, 2);
    assert_eq!(v.latest.unwrap().reason, IdeDbWatchCheckpointReason::Stop);
}

#[tokio::test]
async fn new_items_get_a_baseline_and_kept_ones_keep_theirs() {
    let fake = Arc::new(Fake::default());
    fake.set("a", &["id"], vec![vec![json!(1)]]);
    fake.set("b", &["id"], vec![vec![json!(1)]]);
    let host = as_host(&fake);
    let w = Watch::new(&user("u"), "/p", target(true), None, items(&["a"])).unwrap();
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Start),
        true,
    )
    .await;
    fake.set("a", &["id"], vec![vec![json!(1)], vec![json!(2)]]);
    w.set_items(items(&["a", "b"])).await.unwrap();
    w.baseline_new_items(&host).await;
    assert_eq!(w.view().checkpoint_count, 1, "no new seq");
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Stop),
        true,
    )
    .await;
    let r = w.read(IdeDbWatchCompare::Start, false, false);
    assert_eq!(r.items[0].added_count, 1, "a kept its baseline");
    assert!(Watch::new(&user("u"), "/p", target(true), None, items(&["a", "a"])).is_err());
    assert!(Watch::new(
        &user("u"),
        "/p",
        target(true),
        None,
        items(&["1", "2", "3", "4", "5", "6", "7", "8", "9"])
    )
    .is_err());
}

#[test]
fn the_registry_is_per_owner_and_one_watch_per_root() {
    let reg = WatchRegistry::new();
    let (alice, bob) = (user("alice"), user("bob"));
    let w1 = Watch::new(&alice, "/p", target(true), None, items(&["a"])).unwrap();
    reg.insert(Arc::clone(&w1));
    assert!(
        reg.get(&bob, &w1.id).is_err(),
        "another user's watch is not found"
    );
    assert!(reg.remove(&bob, &w1.id).is_err());
    let w2 = Watch::new(&alice, "/p", target(true), None, items(&["b"])).unwrap();
    reg.insert(Arc::clone(&w2));
    assert!(
        reg.get(&alice, &w1.id).is_err(),
        "a new watch of the root replaced it"
    );
    for r in ["/q", "/r", "/s", "/t"] {
        reg.insert(Watch::new(&alice, r, target(true), None, items(&["a"])).unwrap());
    }
    assert_eq!(reg.list(&alice, None).len(), 4, "≤ 4 per user");
    assert!(
        reg.get(&alice, &w2.id).is_err(),
        "the least recently used went"
    );
    assert_eq!(reg.list(&alice, Some("/t")).len(), 1);
    assert!(reg.list(&bob, None).is_empty());
}

#[test]
fn an_ended_watch_expires_after_ten_minutes() {
    let reg = WatchRegistry::new();
    let w = Watch::new(&user("u"), "/p", target(true), None, items(&["a"])).unwrap();
    reg.insert(Arc::clone(&w));
    lock(&w.state).bound_ended_at_ms = Some(now_ms() - 9 * 60_000);
    assert!(reg.get(&user("u"), &w.id).is_ok());
    lock(&w.state).bound_ended_at_ms = Some(now_ms() - 11 * 60_000);
    assert!(reg.get(&user("u"), &w.id).is_err());
}

#[test]
fn the_window_note_is_honest() {
    let snap = |n: usize| Snap {
        seq: 0,
        window_full: n >= 150,
        data: Arc::new(IdeDbWatchSnapshotItem {
            columns: vec!["id".into()],
            rows: vec![vec![json!(1)]; n],
        }),
    };
    let mut info = Item::new(items(&["t"]).remove(0), IdeDbEngine::Mysql).info;
    assert_eq!(
        window_note(IdeDbEngine::Mysql, &info, Some(&snap(37))),
        "all 37 rows"
    );
    info.primary_key = vec!["id".into()];
    info.row_estimate = Some(1234);
    assert_eq!(
        window_note(IdeDbEngine::Mysql, &info, Some(&snap(150))),
        "the 150 rows with the highest id of ~1.2k — older rows not compared"
    );
    info.primary_key.clear();
    assert!(window_note(IdeDbEngine::Mysql, &info, Some(&snap(150))).contains("no primary key"));
}

#[test]
fn a_huge_text_cell_keeps_a_hash_of_the_whole_value() {
    let a = cap_cell(json!("x".repeat(10_000)));
    let b = cap_cell(json!(format!("{}y", "x".repeat(9_999))));
    assert!(a.as_str().unwrap().len() < 4_200);
    assert_ne!(a, b, "a change past the prefix is still a change");
    assert_eq!(cap_cell(json!("short")), json!("short"));
}

#[tokio::test]
async fn binding_a_session_that_already_ended_takes_its_last_checkpoint() {
    let fake = Arc::new(Fake::default());
    fake.set("a", &["id"], vec![]);
    let host = as_host(&fake);
    let w = Watch::new(&user("u"), "/p", target(true), None, items(&["a"])).unwrap();
    w.checkpoint(
        &host,
        Checkpoint::new(IdeDbWatchCheckpointReason::Start),
        true,
    )
    .await;
    let (_tx, rx) = watch::channel(DapStopSignal {
        stop_seq: 2,
        ended: true,
    });
    let _task = bind_debug(&w, Arc::clone(&host), rx, || (None, None));
    tokio::time::timeout(Duration::from_secs(2), async {
        while w.view().bound_ended_at_ms.is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("ended at once");
    assert_eq!(
        w.view().latest.unwrap().reason,
        IdeDbWatchCheckpointReason::SessionEnd
    );
}
