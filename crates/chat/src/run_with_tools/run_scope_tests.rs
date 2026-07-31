use {super::*, std::task::Poll};

#[tokio::test]
async fn run_scoped_delivery_join_waits_for_completion() {
    let (release, released) = tokio::sync::oneshot::channel();
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let task_events = Arc::clone(&events);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let _ = released.await;
        task_events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push("status");
    });
    let joined = join_channel_delivery_tasks(&mut tasks);
    tokio::pin!(joined);
    std::future::poll_fn(|cx| {
        assert!(matches!(joined.as_mut().poll(cx), Poll::Pending));
        Poll::Ready(())
    })
    .await;

    assert!(release.send(()).is_ok());
    assert!(joined.await);
    events
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push("final");
    assert_eq!(*events.lock().unwrap_or_else(|error| error.into_inner()), [
        "status", "final"
    ]);
}

#[tokio::test]
async fn dropping_run_scoped_delivery_tasks_cancels_late_status_side_effect() {
    struct DropNotice(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for DropNotice {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    let leaked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let leaked_by_task = Arc::clone(&leaked);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let _notice = DropNotice(Some(dropped_tx));
        let _ = started_tx.send(());
        let _ = release_rx.await;
        leaked_by_task.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    assert!(started_rx.await.is_ok());

    drop(tasks);

    assert!(
        tokio::time::timeout(Duration::from_secs(1), dropped_rx)
            .await
            .is_ok()
    );
    assert!(release_tx.send(()).is_err());
    assert!(!leaked.load(std::sync::atomic::Ordering::SeqCst));
}
