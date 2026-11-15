use super::*;

#[tokio::test]
async fn dropping_background_cancels_supervisors_and_aborts_owned_tasks() {
    let signal = crate::cancel_signal::CancelSignal::new();
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (finished_tx, finished_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _finished_tx = finished_tx;
        std::future::pending::<()>().await;
    });
    let mut background = Background::default();
    background.keep(CancelOnDrop(signal.clone()));
    background.keep(StopFlagOnDrop(flag.clone()));
    background.keep(AbortOnDrop(task));
    assert!(!signal.is_cancelled());
    assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
    drop(background);
    assert!(signal.is_cancelled());
    assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    assert!(tokio::time::timeout(Duration::from_secs(1), finished_rx)
        .await
        .expect("owned task was aborted")
        .is_err());
}

#[tokio::test]
async fn stop_watcher_drops_handles_before_teardown_even_without_signal() {
    let signal = crate::cancel_signal::CancelSignal::new();
    let mut background = Background::default();
    background.keep(CancelOnDrop(signal.clone()));
    // Mirrors the daemon's ownership: an early server exit must stop
    // admission even if the shutdown watcher never received a signal.
    let watcher = background.stop_on(std::future::pending());
    watcher.abort();
    let _ = watcher.await;
    assert!(signal.is_cancelled(), "teardown sees cancelled admission");
}

#[tokio::test]
async fn stop_watcher_cancels_on_signal_before_it_returns() {
    let signal = crate::cancel_signal::CancelSignal::new();
    let mut background = Background::default();
    background.keep(CancelOnDrop(signal.clone()));
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let watcher = background.stop_on(async move {
        let _ = stopped.await;
    });
    assert!(!signal.is_cancelled());
    stop.send(()).unwrap();
    watcher.await.unwrap();
    assert!(signal.is_cancelled());
}
