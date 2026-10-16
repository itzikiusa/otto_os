use super::*;
use crate::grid::GridState;

/// Every viewer learns the PTY's real grid and who sizes it (the WS
/// `grid` frame): a remote viewer typing re-sizes the PTY, and the local
/// pane must see that (and see the owner leave) to take the grid back.
#[tokio::test]
async fn grid_feed_tracks_resizes_claims_and_the_owner_leaving() {
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    let spec = CommandSpec {
        program: "/bin/cat".into(),
        args: vec![],
        cwd: Some("/".into()),
        env: vec![],
    };
    let handle = Arc::new(PtyHandle::spawn_sized(&spec, 150, 40).unwrap());
    mgr.live.insert(id.clone(), Arc::clone(&handle));
    let local = mgr.attach(&id);
    let mut rx = mgr.grid_watch(&id);
    assert_eq!(
        *rx.borrow_and_update(),
        GridState {
            cols: 150,
            rows: 40,
            owner: None
        }
    );

    // A remote browser attaches, types (claims) and resizes.
    let remote = mgr.attach(&id);
    mgr.claim_input_authority(&id, remote.conn_id());
    mgr.human_resize(&id, &owner, false, 100, 30).await.unwrap();
    assert!(rx.has_changed().unwrap());
    assert_eq!(
        *rx.borrow_and_update(),
        GridState {
            cols: 100,
            rows: 30,
            owner: Some(remote.conn_id())
        }
    );
    assert!(!mgr.may_resize(&id, local.conn_id()));

    // The remote tab closes: authority stays sticky, but viewers see the
    // grid as un-owned so the local pane may take it back.
    drop(remote);
    assert_eq!(rx.borrow_and_update().owner, None);
    assert!(
        !mgr.may_resize(&id, local.conn_id()),
        "sticky until a claim"
    );

    // The local pane claims and re-sizes: one feed, its own grid again.
    mgr.human_claim_size(&id, &owner, false, local.conn_id())
        .await
        .unwrap();
    mgr.human_resize(&id, &owner, false, 150, 40).await.unwrap();
    assert_eq!(
        *rx.borrow_and_update(),
        GridState {
            cols: 150,
            rows: 40,
            owner: Some(local.conn_id())
        }
    );

    // Same-size resizes and repeated claims publish nothing.
    mgr.human_resize(&id, &owner, false, 150, 40).await.unwrap();
    mgr.claim_input_authority(&id, local.conn_id());
    assert!(!rx.has_changed().unwrap());

    // The last viewer leaving (its receiver gone first, as the WS loop
    // drops it before its guard) drops the feed.
    drop(rx);
    drop(local);
    assert!(!mgr.grids.contains(&id));
    let _ = handle.kill();
}

/// A process that exits under an attached viewer reads as 0×0 (ws.md: "0
/// while no process is live"): the feed used to keep the dead PTY's grid, so
/// a pane went on judging (and claiming against) a grid nothing had.
#[tokio::test]
async fn grid_feed_reads_zero_once_the_process_exits() {
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    let spec = CommandSpec {
        program: "/bin/cat".into(),
        args: vec![],
        cwd: Some("/".into()),
        env: vec![],
    };
    let handle = Arc::new(PtyHandle::spawn_sized(&spec, 120, 40).unwrap());
    mgr.live.insert(id.clone(), Arc::clone(&handle));
    mgr.start_status_task(
        id.clone(),
        ws.id.clone(),
        "shell".into(),
        Arc::clone(&handle),
        false,
    );
    let _viewer = mgr.attach(&id);
    let mut rx = mgr.grid_watch(&id);
    assert_eq!(rx.borrow_and_update().cols, 120);
    let _ = handle.kill();
    tokio::time::timeout(Duration::from_secs(10), async {
        while rx.borrow_and_update().cols != 0 {
            rx.changed().await.unwrap();
        }
    })
    .await
    .expect("the exit publishes 0x0");
    assert_eq!(*rx.borrow(), GridState::default());
}
