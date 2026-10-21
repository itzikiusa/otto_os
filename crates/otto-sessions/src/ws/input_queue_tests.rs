//! Perf 01 N7: the per-connection input queue never loses keystrokes. It
//! coalesces what queued behind a slow write and bounds the backlog by
//! BYTES with backpressure, where round 1 dropped every frame past 256.

use super::*;
use std::sync::Mutex;

type Writes = Arc<Mutex<Vec<(Vec<u8>, bool)>>>;
type Reverts = Arc<Mutex<Vec<Option<u64>>>>;
type Results = tokio::sync::mpsc::Receiver<std::result::Result<(), String>>;

/// A queue whose writer blocks until `gate` has a permit per write.
fn gated(gate: Arc<Semaphore>, fail: bool) -> (InputQueue, Results, Writes, Reverts) {
    let writes: Writes = Arc::default();
    let reverts: Reverts = Arc::default();
    let (w, r) = (writes.clone(), reverts.clone());
    let (q, res) = spawn_input_queue(
        move |bytes: Vec<u8>, user: bool| {
            let gate = gate.clone();
            let w = w.clone();
            async move {
                gate.acquire().await.unwrap().forget();
                w.lock().unwrap().push((bytes, user));
                if fail {
                    Err("not live".to_string())
                } else {
                    Ok(())
                }
            }
        },
        move |prev| r.lock().unwrap().push(prev),
    );
    (q, res, writes, reverts)
}

/// What the socket loop does per frame: queue it, or park it and wait.
async fn push(q: &InputQueue, bytes: Vec<u8>, user: bool, prev: Option<u64>) {
    if let Err(job) = q.try_push(bytes, user, prev) {
        InputQueue::reserve(q.budget.clone(), job.cost).await;
        q.send(job);
    }
}

async fn settle(writes: &Writes, total: usize) {
    for _ in 0..500 {
        if writes
            .lock()
            .unwrap()
            .iter()
            .map(|w| w.0.len())
            .sum::<usize>()
            >= total
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("writer never delivered {total} bytes");
}

/// Review S1-25: input already queued when the viewer loses input
/// authority (re-auth narrowing, eviction) is discarded, not written.
#[tokio::test]
async fn revoked_queue_discards_input_already_queued() {
    let gate = Arc::new(Semaphore::new(0));
    let (q, _res, writes, _) = gated(gate.clone(), false);
    push(&q, b"first".to_vec(), true, None).await;
    // The first write is stuck in the child; more queues up behind it.
    tokio::time::sleep(Duration::from_millis(20)).await;
    push(&q, b"queued-after".to_vec(), true, None).await;
    q.revoke();
    gate.add_permits(10);
    settle(&writes, 5).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let delivered: Vec<u8> = writes
        .lock()
        .unwrap()
        .iter()
        .flat_map(|w| w.0.clone())
        .collect();
    assert_eq!(
        delivered, b"first",
        "nothing queued after the revoke is written"
    );
    // The budget is returned: the queue does not wedge.
    assert_eq!(q.budget.available_permits(), INPUT_BUDGET_BYTES);
}

#[tokio::test]
async fn thousands_of_keystrokes_behind_a_stuck_write_are_all_delivered_in_order() {
    let gate = Arc::new(Semaphore::new(0));
    let (q, _res, writes, _) = gated(gate.clone(), false);
    // 5000 one-byte keystrokes while the child is not reading (the first
    // write is stuck): 20× the old 256-frame cap. None may be dropped.
    let expected: Vec<u8> = (0..5000u32).map(|i| b'a' + (i % 26) as u8).collect();
    for b in &expected {
        push(&q, vec![*b], true, None).await;
    }
    gate.add_permits(Semaphore::MAX_PERMITS / 2);
    settle(&writes, expected.len()).await;
    let writes = writes.lock().unwrap();
    let got: Vec<u8> = writes.iter().flat_map(|w| w.0.clone()).collect();
    assert_eq!(got, expected, "every keystroke arrives, in order");
    // Coalesced: the stuck first write, then the backlog in few chunks.
    assert!(
        writes.len() <= 1 + 5000usize.div_ceil(INPUT_COALESCE_BYTES) + 1,
        "the backlog is coalesced, got {} writes",
        writes.len()
    );
}

#[tokio::test]
async fn a_full_byte_budget_waits_instead_of_dropping() {
    let gate = Arc::new(Semaphore::new(0));
    let (q, _res, writes, _) = gated(gate.clone(), false);
    let chunk = INPUT_BUDGET_BYTES / 4;
    // Four quarter-budget pastes fill the budget (the first sits in the
    // stuck write and still holds its share).
    for i in 0..4u8 {
        assert!(q.try_push(vec![i; chunk], true, None).is_ok());
    }
    let Err(job) = q.try_push(vec![9; chunk], true, None) else {
        panic!("a fifth quarter must not fit the byte budget");
    };
    // …so it waits, it is not dropped:
    let wait = InputQueue::reserve(q.budget.clone(), job.cost);
    tokio::pin!(wait);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut wait)
            .await
            .is_err(),
        "the parked frame waits while the budget is full"
    );
    // The child reads: budget frees, the parked frame goes through.
    gate.add_permits(Semaphore::MAX_PERMITS / 2);
    tokio::time::timeout(Duration::from_secs(5), wait)
        .await
        .expect("budget frees once the writer drains");
    q.send(job);
    settle(&writes, 5 * chunk).await;
    let got: Vec<u8> = writes
        .lock()
        .unwrap()
        .iter()
        .flat_map(|w| w.0.clone())
        .collect();
    assert_eq!(got.len(), 5 * chunk);
    assert_eq!(got[4 * chunk], 9, "the waited frame lands last, in order");
}

/// Perf3 R6: with the input budget full behind a child that is not
/// reading its stdin, the loop must keep reading client frames. A paste
/// interleaved with `ack`s still frees credit, so held output flows, and
/// once the child reads every deferred byte arrives, in order.
///
/// This drives the socket loop's own input/read arms (`DeferredInput`
/// guard + reserve/release) over a scripted client; before the fix the
/// read arm was disabled while a frame was parked, so the `ack` behind
/// the paste was never read and this timed out.
#[tokio::test]
async fn a_full_input_budget_still_reads_acks_so_output_keeps_flowing() {
    let gate = Arc::new(Semaphore::new(0));
    let (q, _res, writes, _) = gated(gate.clone(), false);
    let now = tokio::time::Instant::now();
    let window = CREDIT_WINDOW_MIN;
    let mut credit = CreditGate::new(window);
    // Output producer: two windows of output, the second held for credit.
    let mut delivered = 0usize;
    for chunk in [vec![b'o'; window as usize], vec![b'p'; window as usize]] {
        if let CreditStep::Send(b) = credit.push(chunk, now) {
            delivered += b.len();
        }
    }
    assert_eq!(delivered, window as usize, "one window out, one held");

    // Client: six quarter-budget paste frames (four fill the budget, two
    // are deferred), then the ack for the first window, a probe-like
    // no-op resize, and one more keystroke.
    let quarter = INPUT_BUDGET_BYTES / 4;
    let mut expected: Vec<u8> = Vec::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    for i in 0..6u8 {
        let data = vec![b'a' + i; quarter];
        expected.extend_from_slice(&data);
        let frame = serde_json::json!({"type": "input", "data": B64.encode(&data)});
        tx.send(frame.to_string()).unwrap();
    }
    tx.send(format!(r#"{{"type":"ack","bytes":{window}}}"#))
        .unwrap();
    tx.send(r#"{"type":"resize","cols":80,"rows":24}"#.into())
        .unwrap();
    tx.send(serde_json::json!({"type": "input", "data": B64.encode(b"z")}).to_string())
        .unwrap();
    expected.push(b'z');
    drop(tx);

    // The loop's two input-related arms, as in `serve_terminal`.
    let mut deferred = DeferredInput::default();
    let (mut acks, mut resizes) = (0, 0);
    let run = async {
        loop {
            tokio::select! {
                _ = InputQueue::reserve(q.budget.clone(), deferred.front_cost().unwrap_or(1)),
                    if deferred.front_cost().is_some() => deferred.release(&q),
                msg = rx.recv(), if deferred.reading() => {
                    let Some(text) = msg else { break };
                    match serde_json::from_str::<ClientFrame>(&text).unwrap() {
                        ClientFrame::Input { data, user } => {
                            let bytes = B64.decode(data.as_bytes()).unwrap();
                            deferred.push(&q, bytes, user, None);
                        }
                        ClientFrame::Ack { bytes } => {
                            acks += 1;
                            if let CreditStep::Send(b) = credit.ack(bytes, now) {
                                delivered += b.len();
                            }
                        }
                        ClientFrame::Resize { .. } => resizes += 1,
                        _ => {}
                    }
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .expect("client frames behind a full input budget are still read");
    assert_eq!((acks, resizes), (1, 1), "ack and resize processed");
    assert_eq!(
        delivered,
        2 * window as usize,
        "the ack freed credit: the held window went out"
    );
    assert!(
        deferred.front_cost().is_some(),
        "input past the budget is deferred, not dropped"
    );
    assert!(
        writes.lock().unwrap().is_empty(),
        "the child has not read yet"
    );

    // The child starts reading stdin: every deferred frame drains, in order.
    gate.add_permits(Semaphore::MAX_PERMITS / 2);
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(cost) = deferred.front_cost() {
            InputQueue::reserve(q.budget.clone(), cost).await;
            deferred.release(&q);
        }
    })
    .await
    .expect("deferred input drains once the child reads");
    settle(&writes, expected.len()).await;
    let got: Vec<u8> = writes
        .lock()
        .unwrap()
        .iter()
        .flat_map(|w| w.0.clone())
        .collect();
    assert_eq!(got.len(), expected.len(), "nothing dropped");
    assert!(got == expected, "all input arrives in order");
    assert_eq!(deferred.bytes, 0);
}

/// The deferral itself is bounded: past INPUT_DEFER_BYTES the loop stops
/// reading (TCP backpressure) rather than buffering without limit.
#[tokio::test]
async fn deferred_input_is_bounded_and_keeps_order() {
    let gate = Arc::new(Semaphore::new(0));
    let (q, _res, _writes, _) = gated(gate, false);
    let mut d = DeferredInput::default();
    d.push(&q, vec![0; INPUT_BUDGET_BYTES], true, None); // takes the whole budget
    assert!(d.front_cost().is_none() && d.reading());
    // Everything behind a deferred frame is deferred too: order is kept.
    d.push(&q, vec![1; INPUT_DEFER_BYTES - 1], true, None);
    d.push(&q, vec![2], true, None);
    assert_eq!(d.jobs.len(), 2);
    assert!(!d.reading(), "the deferral bound pauses reading");
}

#[tokio::test]
async fn emulator_replies_never_coalesce_with_typing_and_a_failed_typing_write_reverts() {
    let gate = Arc::new(Semaphore::new(0));
    let (q, mut res, writes, reverts) = gated(gate.clone(), true);
    push(&q, b"x".to_vec(), true, Some(7)).await; // stuck write
                                                  // Let the writer pick it up and block in the write.
    tokio::time::sleep(Duration::from_millis(30)).await;
    push(&q, b"a".to_vec(), true, Some(7)).await;
    push(&q, b"b".to_vec(), true, Some(7)).await;
    push(&q, b"\x1b[1;1R".to_vec(), false, None).await; // DSR reply
    push(&q, b"c".to_vec(), true, Some(7)).await;
    gate.add_permits(100);
    settle(&writes, 3 + 6 + 1).await;
    let kinds: Vec<(Vec<u8>, bool)> = writes.lock().unwrap().clone();
    assert_eq!(
        kinds,
        vec![
            (b"x".to_vec(), true),
            (b"ab".to_vec(), true),
            (b"\x1b[1;1R".to_vec(), false),
            (b"c".to_vec(), true),
        ]
    );
    for _ in 0..4 {
        assert!(res.recv().await.unwrap().is_err());
    }
    // Only the three failed TYPING writes revert the size claim.
    assert_eq!(*reverts.lock().unwrap(), vec![Some(7); 3]);
}
