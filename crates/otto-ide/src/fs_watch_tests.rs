use super::*;

fn lease(roots: &[&str], dirs: &[(&str, &str)]) -> Lease {
    Lease {
        roots: roots.iter().map(PathBuf::from).collect(),
        dirs: dirs
            .iter()
            .map(|(c, s)| (PathBuf::from(c), s.to_string()))
            .collect(),
        touched: Instant::now(),
    }
}

fn dirs_of(ev: &Event) -> (&str, &[String]) {
    match ev {
        Event::IdeFsChanged {
            client_id, dirs, ..
        } => (client_id.as_str(), dirs.as_slice()),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn only_listing_changes_count() {
    use notify::event::{AccessKind, CreateKind, DataChange, MetadataKind, RemoveKind, RenameMode};
    assert!(changes_listing(&EventKind::Create(CreateKind::File)));
    assert!(changes_listing(&EventKind::Remove(RemoveKind::Folder)));
    assert!(changes_listing(&EventKind::Modify(ModifyKind::Name(
        RenameMode::Any
    ))));
    assert!(!changes_listing(&EventKind::Modify(ModifyKind::Data(
        DataChange::Content
    ))));
    assert!(!changes_listing(&EventKind::Modify(ModifyKind::Metadata(
        MetadataKind::Any
    ))));
    assert!(!changes_listing(&EventKind::Access(AccessKind::Any)));
}

#[test]
fn client_spelling_maps_under_the_canonical_root() {
    let roots = vec![
        ("/var/p".to_string(), PathBuf::from("/private/var/p")),
        ("/var/p/sub".to_string(), PathBuf::from("/real/sub")),
    ];
    assert_eq!(
        canonical_dir("/var/p", &roots),
        Some(PathBuf::from("/private/var/p"))
    );
    assert_eq!(
        canonical_dir("/var/p/src/", &roots),
        Some(PathBuf::from("/private/var/p/src"))
    );
    // The longest matching root wins (an attached folder inside the primary).
    assert_eq!(
        canonical_dir("/var/p/sub/x", &roots),
        Some(PathBuf::from("/real/sub/x"))
    );
    assert_eq!(canonical_dir("/var/pother", &roots), None);
    assert_eq!(canonical_dir("/elsewhere", &roots), None);
}

#[test]
fn a_flush_names_each_window_its_own_changed_folders() {
    let mut leases = HashMap::new();
    leases.insert(
        ("u1".to_string(), "a".to_string()),
        lease(
            &["/r"],
            &[("/r", "/r"), ("/r/src", "/r/src"), ("/r/doc", "/r/doc")],
        ),
    );
    // Another window spells the same root differently.
    leases.insert(
        ("u1".to_string(), "b".to_string()),
        lease(&["/r"], &[("/r/src", "/link/src")]),
    );
    // A window on another project hears nothing.
    leases.insert(
        ("u2".to_string(), "c".to_string()),
        lease(&["/q"], &[("/q", "/q")]),
    );

    let changed: HashSet<PathBuf> = [PathBuf::from("/r/src")].into();
    let mut evs = events_for(&leases, Path::new("/r"), &changed, false);
    evs.sort_by(|x, y| dirs_of(x).0.cmp(dirs_of(y).0));
    assert_eq!(evs.len(), 2);
    assert_eq!(dirs_of(&evs[0]), ("a", &["/r/src".to_string()][..]));
    assert_eq!(dirs_of(&evs[1]), ("b", &["/link/src".to_string()][..]));

    // A folder nobody expanded sends nothing.
    let other: HashSet<PathBuf> = [PathBuf::from("/r/target/debug")].into();
    assert!(events_for(&leases, Path::new("/r"), &other, false).is_empty());

    // A rescan re-lists every expanded folder under the root, still one event per window.
    let evs = events_for(&leases, Path::new("/r"), &HashSet::new(), true);
    let a = evs.iter().find(|e| dirs_of(e).0 == "a").unwrap();
    assert_eq!(dirs_of(a).1, ["/r", "/r/doc", "/r/src"]);
}

async fn next_dirs(rx: &mut broadcast::Receiver<Event>, within: Duration) -> Option<Vec<String>> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(Event::IdeFsChanged { dirs, .. })) => return Some(dirs),
            Ok(Ok(_)) => continue,
            _ => return None,
        }
    }
}

async fn drain(rx: &mut broadcast::Receiver<Event>) {
    while next_dirs(rx, Duration::from_millis(700)).await.is_some() {}
}

fn spelled(p: &Path, rel: &str) -> String {
    if rel.is_empty() {
        p.to_string_lossy().into_owned()
    } else {
        p.join(rel).to_string_lossy().into_owned()
    }
}

async fn set(
    reg: &Arc<FsWatchRegistry>,
    client: &str,
    root: &Path,
    dirs: Vec<String>,
    tx: &broadcast::Sender<Event>,
) -> Vec<String> {
    let (reg, client, tx) = (reg.clone(), client.to_string(), tx.clone());
    let roots = if root.as_os_str().is_empty() {
        vec![]
    } else {
        vec![(root.to_string_lossy().into_owned(), root.to_path_buf())]
    };
    tokio::task::spawn_blocking(move || reg.set_lease(&"u1".to_string(), &client, roots, &dirs, tx))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn outside_changes_reach_expanded_folders_only_and_coalesce() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("target/debug")).unwrap();
    std::fs::write(root.join("src/keep.rs"), b"x").unwrap();
    let (tx, mut rx) = broadcast::channel(256);
    let reg = Arc::new(FsWatchRegistry::new());
    let watched = set(
        &reg,
        "w1",
        &root,
        vec![spelled(&root, ""), spelled(&root, "src")],
        &tx,
    )
    .await;
    assert_eq!(watched, vec![spelled(&root, "")]);
    assert_eq!(reg.watched_roots(), 1);
    // Let the OS stream start; drop anything arming produced.
    tokio::time::sleep(Duration::from_millis(400)).await;
    drain(&mut rx).await;

    // A create in an expanded folder arrives quickly, naming that folder.
    let t0 = Instant::now();
    std::fs::write(root.join("src/new.rs"), b"x").unwrap();
    let dirs = next_dirs(&mut rx, Duration::from_secs(5))
        .await
        .expect("an event for the create");
    assert_eq!(dirs, vec![spelled(&root, "src")]);
    assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
    drain(&mut rx).await;

    // A build churning a collapsed folder sends nothing.
    for i in 0..300 {
        std::fs::write(root.join(format!("target/debug/o{i}.o")), b"x").unwrap();
    }
    assert_eq!(next_dirs(&mut rx, Duration::from_millis(1500)).await, None);

    // A 2000-file burst (a branch switch) is a couple of events, each naming only the expanded folder.
    for i in 0..2000 {
        std::fs::write(root.join(format!("src/f{i}.rs")), b"x").unwrap();
    }
    let mut events = 0;
    while let Some(dirs) = next_dirs(&mut rx, Duration::from_millis(1500)).await {
        events += 1;
        assert_eq!(dirs, vec![spelled(&root, "src")]);
    }
    assert!((1..=6).contains(&events), "burst produced {events} events");

    // Remove + rename at the root level name the root.
    std::fs::remove_file(root.join("src/keep.rs")).unwrap();
    std::fs::rename(root.join("src"), root.join("src2")).unwrap();
    let dirs = next_dirs(&mut rx, Duration::from_secs(5))
        .await
        .expect("an event for the rename");
    assert!(dirs.contains(&spelled(&root, "")), "{dirs:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_watcher_per_root_released_on_close_and_on_expiry() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let other_tmp = tempfile::tempdir().unwrap();
    let other = std::fs::canonicalize(other_tmp.path()).unwrap();
    let (tx, mut rx) = broadcast::channel(64);
    let reg = Arc::new(FsWatchRegistry::new());

    // Two windows on one project share one watcher.
    set(&reg, "w1", &root, vec![spelled(&root, "")], &tx).await;
    set(&reg, "w2", &root, vec![spelled(&root, "")], &tx).await;
    assert_eq!(reg.watched_roots(), 1);
    // Switching w2 to another project moves its watcher.
    set(&reg, "w2", &other, vec![spelled(&other, "")], &tx).await;
    assert_eq!(reg.watched_roots(), 2);
    // Closing w1's project stops that root's watcher; w2 keeps its own.
    set(&reg, "w1", Path::new(""), vec![], &tx).await;
    assert_eq!(reg.watched_roots(), 1);

    // A released root sends nothing any more.
    tokio::time::sleep(Duration::from_millis(300)).await;
    drain(&mut rx).await;
    std::fs::write(root.join("gone.txt"), b"x").unwrap();
    assert_eq!(next_dirs(&mut rx, Duration::from_millis(1200)).await, None);

    // A window that stops re-posting (closed tab, crash) expires.
    reg.reap_at(Instant::now() + LEASE_TTL);
    assert_eq!(reg.watched_roots(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn broad_roots_are_never_watched() {
    let (tx, _rx) = broadcast::channel(4);
    let reg = Arc::new(FsWatchRegistry::new());
    assert!(
        set(&reg, "w", Path::new("/Users"), vec!["/Users".into()], &tx)
            .await
            .is_empty()
    );
    assert_eq!(reg.watched_roots(), 0);
}
