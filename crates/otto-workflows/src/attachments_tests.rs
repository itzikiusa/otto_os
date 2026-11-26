use super::*;

/// Files as the Slack listener saves them (`$TMPDIR/otto-slack-<id>-<name>`),
/// removed when the test ends.
#[derive(Default)]
struct Listener(std::cell::RefCell<Vec<PathBuf>>);

impl Listener {
    fn downloaded(&self, name: &str, body: &[u8]) -> InboundAttachment {
        let path = std::env::temp_dir().join(format!("otto-slack-{}-x", otto_core::new_id()));
        std::fs::write(&path, body).unwrap();
        self.0.borrow_mut().push(path.clone());
        InboundAttachment {
            name: name.into(),
            mime: "text/plain".into(),
            size_bytes: body.len() as u64,
            path: Some(path),
            error: None,
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        for p in self.0.borrow().iter() {
            let _ = std::fs::remove_file(p);
        }
    }
}

#[test]
fn files_are_copied_listed_and_read_back() {
    let l = Listener::default();
    let ctx = tempfile::tempdir().unwrap();
    let failed = InboundAttachment {
        name: "spec.pdf".into(),
        mime: "application/pdf".into(),
        size_bytes: 0,
        path: None,
        error: Some("could not download: http 404".into()),
    };
    let refs = copy_into(
        ctx.path(),
        &[l.downloaded("spec.txt", b"lorem"), failed],
        AttachmentSource::Start,
    )
    .unwrap();
    assert_eq!(refs[0].path.as_deref(), Some("attachments/spec.txt"));
    assert_eq!(refs[0].size_bytes, Some(5));
    assert_eq!(
        std::fs::read(ctx.path().join("attachments/spec.txt")).unwrap(),
        b"lorem"
    );
    assert_eq!(
        (refs[1].path.as_ref(), refs[1].error.as_deref()),
        (None, Some("could not download: http 404"))
    );
    assert_eq!(read(ctx.path()), refs, "persisted in attachments.json");
    assert_eq!(
        brief_lines(&refs),
        [
            "Attachment from the requester: attachments/spec.txt (text/plain, 5 B)",
            "Attachment not received: spec.pdf — could not download: http 404",
        ]
    );
    // A steering note's files are appended, not replacing the start ones.
    let more = copy_into(
        ctx.path(),
        &[l.downloaded("spec.txt", b"v2")],
        AttachmentSource::Steer,
    )
    .unwrap();
    assert_eq!(more[0].path.as_deref(), Some("attachments/spec-2.txt"));
    let all = read(ctx.path());
    assert_eq!(all.len(), 3);
    assert_eq!(all[2].source, AttachmentSource::Steer);
}

#[test]
fn copies_survive_the_listeners_temp_sweep() {
    let l = Listener::default();
    let ctx = tempfile::tempdir().unwrap();
    let f = l.downloaded("notes.md", b"keep me");
    copy_into(
        ctx.path(),
        std::slice::from_ref(&f),
        AttachmentSource::Start,
    )
    .unwrap();
    std::fs::remove_file(f.path.as_ref().unwrap()).unwrap();
    assert_eq!(
        std::fs::read(ctx.path().join("attachments/notes.md")).unwrap(),
        b"keep me"
    );
}

#[test]
fn names_are_sanitised_confined_and_never_overwrite() {
    let l = Listener::default();
    let ctx = tempfile::tempdir().unwrap();
    std::fs::write(ctx.path().join("repos.json"), "[]").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(ctx.path().join(ATTACHMENTS_DIR)).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("target.txt"),
        ctx.path().join("attachments/a.txt"),
    )
    .unwrap();
    let refs = copy_into(
        ctx.path(),
        &[
            l.downloaded("../repos.json", b"evil"),
            l.downloaded("a.txt", b"one"),
            l.downloaded("a.txt", b"two"),
            l.downloaded("..", b"dots"),
            l.downloaded("rapport été.pdf", b"x"),
        ],
        AttachmentSource::Start,
    )
    .unwrap();
    let paths: Vec<&str> = refs.iter().map(|r| r.path.as_deref().unwrap()).collect();
    assert_eq!(
        paths,
        [
            "attachments/.._repos.json",
            "attachments/a-2.txt",
            "attachments/a-3.txt",
            "attachments/file",
            "attachments/rapport__t_.pdf",
        ]
    );
    assert_eq!(
        std::fs::read_to_string(ctx.path().join("repos.json")).unwrap(),
        "[]"
    );
    assert!(
        !outside.path().join("target.txt").exists(),
        "the symlink was not followed"
    );
}

#[test]
fn only_files_the_listener_saved_in_the_temp_dir_are_copied() {
    let l = Listener::default();
    let ctx = tempfile::tempdir().unwrap();
    let outside_tmp = InboundAttachment {
        path: Some(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/Cargo.toml"
        ))),
        ..l.downloaded("Cargo.toml", b"")
    };
    let dir = tempfile::tempdir().unwrap();
    let a_dir = InboundAttachment {
        path: Some(dir.path().to_path_buf()),
        ..l.downloaded("d", b"")
    };
    let link = std::env::temp_dir().join(format!("otto-slack-{}-link", otto_core::new_id()));
    std::os::unix::fs::symlink(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"), &link).unwrap();
    let via_link = InboundAttachment {
        path: Some(link.clone()),
        ..l.downloaded("l", b"")
    };
    let refs = copy_into(
        ctx.path(),
        &[outside_tmp, a_dir, via_link],
        AttachmentSource::Start,
    )
    .unwrap();
    for r in &refs {
        assert!(r.path.is_none(), "{r:?}");
        assert_eq!(
            r.error.as_deref(),
            Some("not a file Otto received for this message")
        );
    }
    let _ = std::fs::remove_file(link);
}

#[test]
fn the_temp_dir_is_compared_canonicalized() {
    let l = Listener::default();
    // On macOS TMPDIR is /var/folders/… and canonicalizes to /private/var/…;
    // a listener file must still be accepted.
    let f = l.downloaded("x.txt", b"x");
    let raw = std::env::temp_dir();
    assert_eq!(
        f.path.as_ref().unwrap().canonicalize().unwrap().parent(),
        Some(raw.canonicalize().unwrap().as_path())
    );
    let ctx = tempfile::tempdir().unwrap();
    let refs = copy_into(ctx.path(), &[f], AttachmentSource::Start).unwrap();
    assert!(refs[0].path.is_some(), "{refs:?}");
}

#[test]
fn the_per_run_cap_is_ten_files_and_fifty_mib() {
    let l = Listener::default();
    let ctx = tempfile::tempdir().unwrap();
    let files: Vec<_> = (0..11)
        .map(|i| l.downloaded(&format!("f{i}.txt"), b"x"))
        .collect();
    let refs = copy_into(ctx.path(), &files, AttachmentSource::Start).unwrap();
    assert!(refs[..10].iter().all(|r| r.path.is_some()));
    assert_eq!(refs[10].error.as_deref(), Some(OVER_LIMIT));
    // The cap counts the run's earlier files too.
    let late = copy_into(
        ctx.path(),
        &[l.downloaded("g.txt", b"x")],
        AttachmentSource::Steer,
    )
    .unwrap();
    assert_eq!(late[0].error.as_deref(), Some(OVER_LIMIT));

    let ctx = tempfile::tempdir().unwrap();
    let big = l.downloaded("big.bin", &vec![0u8; 30 << 20]);
    let second = l.downloaded("big2.bin", &vec![0u8; 30 << 20]);
    let refs = copy_into(ctx.path(), &[big, second], AttachmentSource::Start).unwrap();
    assert!(refs[0].path.is_some());
    assert_eq!(refs[1].error.as_deref(), Some(OVER_LIMIT));
}
