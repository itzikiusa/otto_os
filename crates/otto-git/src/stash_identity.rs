//! Identity-selected stash deletion. Native ref locks synchronize with external
//! Git and linked worktrees; application mutexes alone cannot do that.
//!
//! All native/filesystem functions below run on the blocking pool. Reflog and
//! reference commits are not crash-atomic, so a durable recovery record and
//! temporary reachability refs precede every nonempty-stack rewrite.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use git2::{Oid, Reflog, Repository};
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};

const STASH: &str = "refs/stash";
const JOURNALS: &str = "otto-stash-drop";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    old: String,
    new: String,
    name: Vec<u8>,
    email: Vec<u8>,
    seconds: i64,
    offset: i32,
    message: Option<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    version: u8,
    operation: String,
    old_target: String,
    before: Vec<Entry>,
    after: Vec<Entry>,
    pins: Vec<(String, String)>,
}

/// System Git owns final-entry deletion (including packed-ref cleanup). The
/// caller must prepare a conditional update-ref deletion, then compare this
/// exact reflog under that prepared lock before sending commit.
pub(crate) enum Rewrite {
    Last {
        expected: String,
        entries: Vec<Entry>,
    },
    Changed {
        journal: PathBuf,
    },
}

fn failure(context: &str, error: impl std::fmt::Display) -> Error {
    Error::Internal(format!("stash {context}: {error}"))
}

fn snapshot(log: &Reflog) -> Vec<Entry> {
    log.iter()
        .map(|entry| {
            let who = entry.committer();
            Entry {
                old: entry.id_old().to_string(),
                new: entry.id_new().to_string(),
                name: who.name_bytes().to_vec(),
                email: who.email_bytes().to_vec(),
                seconds: who.when().seconds(),
                offset: who.when().offset_minutes(),
                message: entry.message_bytes().map(<[u8]>::to_vec),
            }
        })
        .collect()
}

fn open(path: &Path, expected_common: &Path) -> Result<Repository> {
    let repo = Repository::open(path).map_err(|e| failure("open repository", e))?;
    let common = repo
        .commondir()
        .canonicalize()
        .map_err(|e| failure("common directory", e))?;
    if common != expected_common {
        return Err(Error::Conflict(
            "Git repository location changed; refresh and retry".into(),
        ));
    }
    Ok(repo)
}

fn target(repo: &Repository) -> Result<String> {
    repo.find_reference(STASH)
        .map_err(|e| failure("read reference", e))?
        .target()
        .map(|oid| oid.to_string())
        .ok_or_else(|| {
            Error::Conflict(
                "stash reference is symbolic; resolve it with Git before retrying".into(),
            )
        })
}

fn write_journal(common: &Path, journal: &Journal) -> Result<PathBuf> {
    let dir = common.join(JOURNALS);
    std::fs::create_dir_all(&dir).map_err(|e| failure("create recovery directory", e))?;
    std::fs::File::open(common)
        .and_then(|f| f.sync_all())
        .map_err(|e| failure("sync recovery directory creation", e))?;
    let path = dir.join(format!("{}.json", journal.operation));
    // Publish only a complete, synced record. No Git reference file is written
    // here; those are exclusively managed by the native Git backends.
    let mut tmp =
        tempfile::NamedTempFile::new_in(&dir).map_err(|e| failure("create recovery record", e))?;
    serde_json::to_writer(&mut tmp, journal).map_err(|e| failure("encode recovery record", e))?;
    tmp.flush()
        .map_err(|e| failure("flush recovery record", e))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| failure("sync recovery record", e))?;
    tmp.persist_noclobber(&path)
        .map_err(|e| failure("publish recovery record", e))?;
    std::fs::File::open(&dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| failure("sync recovery directory", e))?;
    Ok(path)
}

/// Select the current occurrence only AFTER acquiring the actual ref lock.
/// `requested` is a validated full SHA-1, never a reflog index.
pub(crate) fn rewrite(path: &Path, common: &Path, requested: &str) -> Result<Rewrite> {
    let repo = open(path, common)?;
    let mut tx = repo
        .transaction()
        .map_err(|e| failure("begin transaction", e))?;
    tx.lock_ref(STASH)
        .map_err(|e| Error::Conflict(format!("stash is being changed by Git; retry: {e}")))?;
    let old_target = target(&repo)?;
    let mut log = repo.reflog(STASH).map_err(|e| failure("read reflog", e))?;
    let before = snapshot(&log);
    if before.first().map(|e| e.new.as_str()) != Some(old_target.as_str()) {
        return Err(Error::Conflict(
            "stash reference and history disagree; recovery is required before Drop".into(),
        ));
    }
    let index = before
        .iter()
        .position(|e| e.new == requested)
        .ok_or_else(|| {
            Error::Conflict(format!(
                "stash {requested} no longer exists; refresh the list"
            ))
        })?;
    if before.len() == 1 {
        // No empty-reflog intermediate state: release this lock and let the
        // caller revalidate the complete singleton under system Git's lock.
        return Ok(Rewrite::Last {
            expected: old_target,
            entries: before,
        });
    }
    log.remove(index, true)
        .map_err(|e| failure("remove selected occurrence", e))?;
    let after = snapshot(&log);
    let operation = otto_core::new_id();
    let objects: BTreeSet<_> = before.iter().map(|e| &e.new).cloned().collect();
    let pins = objects
        .into_iter()
        .enumerate()
        .map(|(n, oid)| (format!("refs/otto/stash-drop/{operation}/{n}"), oid))
        .collect();
    let journal = Journal {
        version: 1,
        operation,
        old_target,
        before,
        after,
        pins,
    };
    let journal_path = write_journal(common, &journal)?;
    // Independent stash commits are not an ancestry chain. Pin every object
    // needed for either image before the first destructive write.
    for (name, oid) in &journal.pins {
        repo.reference(
            name,
            Oid::from_str(oid).map_err(|e| failure("recovery object", e))?,
            false,
            "Otto stash recovery",
        )
        .map_err(|e| failure("pin recovery object (original stash unchanged)", e))?;
    }
    let new_target = log
        .get(0)
        .expect("nonempty after removing one of multiple entries")
        .id_new();
    let who = log.get(0).expect("nonempty reflog").committer().to_owned();
    tx.set_target(STASH, new_target, Some(&who), "Otto stash drop")
        .map_err(|e| failure("set target", e))?;
    tx.set_reflog(STASH, log)
        .map_err(|e| failure("set history", e))?;
    if let Err(error) = tx.commit() {
        // A commit error may mean history changed but the target did not. The
        // durable record is intentionally retained for checked forward repair.
        return Err(Error::Conflict(format!(
            "stash update was interrupted: {error}; recovery record retained at {}",
            journal_path.display()
        )));
    }
    Ok(Rewrite::Changed {
        journal: journal_path,
    })
}

/// Check/repair a previous operation under the same native reference lock.
/// Return its recovery refs for expected-OID deletion by system Git. Never
/// restore a stale before-image over a newer external writer's history.
pub(crate) fn recover(
    path: &Path,
    common: &Path,
    journal_path: &Path,
) -> Result<Vec<(String, String)>> {
    let bytes = std::fs::read(journal_path).map_err(|e| failure("read recovery record", e))?;
    let journal: Journal =
        serde_json::from_slice(&bytes).map_err(|e| failure("decode recovery record", e))?;
    if journal.version != 1
        || journal.after.is_empty()
        || journal.before.is_empty()
        || journal.pins.iter().any(|(name, _)| {
            !name.starts_with(&format!("refs/otto/stash-drop/{}/", journal.operation))
        })
    {
        return Err(failure(
            "recovery",
            "unsupported or inconsistent record; retained without mutation",
        ));
    }
    let repo = open(path, common)?;
    let mut tx = repo
        .transaction()
        .map_err(|e| failure("begin recovery", e))?;
    tx.lock_ref(STASH).map_err(|e| {
        Error::Conflict(format!(
            "stash recovery is waiting for another Git writer: {e}"
        ))
    })?;
    let log = repo
        .reflog(STASH)
        .map_err(|e| failure("read recovery history", e))?;
    let current = snapshot(&log);
    let current_target = match repo.find_reference(STASH) {
        Err(e) if e.code() == git2::ErrorCode::NotFound && current.is_empty() => {
            // Native Git cleared the stack after this operation. Its absence
            // is a coherent newer state, not permission to restore our copy.
            return Ok(journal.pins);
        }
        _ => target(&repo)?,
    };
    if current == journal.before && current_target == journal.old_target {
        // The process stopped before mutation; cleanup is safe, retry will
        // select the requested occurrence again from the current history.
        return Ok(journal.pins);
    }
    if current != journal.after
        && current.first().map(|e| e.new.as_str()) == Some(current_target.as_str())
    {
        // A later external Git operation has already produced a coherent
        // stack. Never roll it back to either of our historical images.
        return Ok(journal.pins);
    }
    if current != journal.after {
        return Err(Error::Conflict(format!("stash history changed outside Otto during recovery; no history overwritten; recovery refs retained ({}). Inspect with git reflog before retrying", journal_path.display())));
    }
    let desired = &journal.after[0].new;
    if current_target == *desired {
        return Ok(journal.pins);
    }
    if current_target != journal.old_target {
        return Err(Error::Conflict("stash target changed during recovery; recovery refs retained without overwriting Git changes".into()));
    }
    let who = log
        .get(0)
        .expect("validated nonempty recovery history")
        .committer()
        .to_owned();
    tx.set_target(
        STASH,
        Oid::from_str(desired).map_err(|e| failure("recovery target", e))?,
        Some(&who),
        "Otto stash recovery",
    )
    .map_err(|e| failure("prepare recovery target", e))?;
    tx.set_reflog(STASH, log)
        .map_err(|e| failure("preserve recovery history", e))?;
    tx.commit()
        .map_err(|e| failure("commit recovery; record retained", e))?;
    Ok(journal.pins)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| failure("worker", e))?
}

async fn common_dir(git: &crate::local::LocalGit) -> Result<PathBuf> {
    let raw = git
        .run_read(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .await?;
    tokio::fs::canonicalize(raw.trim_end_matches(['\r', '\n']))
        .await
        .map_err(|e| failure("resolve common directory", e))
}

async fn cleanup(
    git: &crate::local::LocalGit,
    path: &Path,
    common: &Path,
    record: PathBuf,
) -> Result<()> {
    let p = path.to_owned();
    let c = common.to_owned();
    let r = record.clone();
    let deletes = blocking(move || {
        let pins = recover(&p, &c, &r)?;
        let repo = open(&p, &c)?;
        let mut commands = String::from("start\n");
        for (name, expected) in pins {
            match repo.find_reference(&name) {
                Ok(reference)
                    if reference.target().map(|id| id.to_string()) == Some(expected.clone()) =>
                {
                    commands.push_str(&format!("delete {name} {expected}\n"));
                }
                Err(e) if e.code() == git2::ErrorCode::NotFound => {}
                _ => {
                    return Err(Error::Conflict(
                        "stash recovery reference changed; record retained".into(),
                    ))
                }
            }
        }
        commands.push_str("prepare\ncommit\n");
        Ok(commands)
    })
    .await?;
    // System Git owns packed-ref deletion; never use libgit2's packed-delete
    // path for either stash or recovery refs. Expected IDs fence replacements.
    let (ok, out, err, _) = git
        .run_raw_stdin(&["update-ref", "--stdin"], deletes.as_bytes())
        .await?;
    if !ok {
        return Err(failure(
            "release recovery refs (record retained)",
            format!("{err}{out}"),
        ));
    }
    blocking(move || {
        match std::fs::remove_file(&record) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(failure("remove completed recovery record", e)),
        }
        std::fs::File::open(record.parent().expect("journal has parent"))
            .and_then(|f| f.sync_all())
            .map_err(|e| failure("sync completed recovery", e))
    })
    .await
}

/// Called before reading the stash list, so a process interruption is repaired
/// before an Apply/Drop selection is made. Ordinary repositories with no
/// recovery directory incur no native-library open and retain format support.
#[allow(clippy::disallowed_methods)] // read_dir executes inside blocking(), never on a runtime worker
pub(crate) async fn recover_pending(git: &crate::local::LocalGit, path: &Path) -> Result<()> {
    let common = common_dir(git).await?;
    let dir = common.join(JOURNALS);
    let records = blocking(move || {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(failure("list recovery records", e)),
        };
        let mut records = Vec::new();
        for entry in entries {
            let p = entry.map_err(|e| failure("read recovery entry", e))?.path();
            if p.extension().is_some_and(|ext| ext == "json") {
                records.push(p);
            }
        }
        records.sort();
        Ok(records)
    })
    .await?;
    for record in records {
        cleanup(git, path, &common, record).await?;
    }
    Ok(())
}

async fn delete_singleton(
    git: &crate::local::LocalGit,
    path: PathBuf,
    common: PathBuf,
    expected: String,
    entries: Vec<Entry>,
) -> Result<bool> {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut cmd = git.base_cmd();
    cmd.args(["update-ref", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| failure("prepare final deletion", e))?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let mut input = child.stdin.take().expect("piped stdin");
        let mut output = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
        input
            .write_all(
                format!("start\noption no-deref\ndelete {STASH} {expected}\nprepare\n").as_bytes(),
            )
            .await
            .map_err(|e| failure("write final transaction", e))?;
        input
            .flush()
            .await
            .map_err(|e| failure("flush final transaction", e))?;
        let started = output
            .next_line()
            .await
            .map_err(|e| failure("read transaction start", e))?;
        let prepared = output
            .next_line()
            .await
            .map_err(|e| failure("read transaction prepare", e))?;
        if started.as_deref() != Some("start: ok") || prepared.as_deref() != Some("prepare: ok") {
            return Ok(false); // expected target changed or another writer holds it
        }
        // The system Git process now holds the shared ref lock. Comparing the
        // complete entry also rejects same-OID ABA arrivals, not only a new tip.
        let matches = blocking(move || {
            let repo = open(&path, &common)?;
            let log = repo
                .reflog(STASH)
                .map_err(|e| failure("read locked final history", e))?;
            Ok(snapshot(&log) == entries && target(&repo)? == expected)
        })
        .await?;
        let command = if matches { "commit\n" } else { "abort\n" };
        input
            .write_all(command.as_bytes())
            .await
            .map_err(|e| failure("finish final transaction", e))?;
        input
            .flush()
            .await
            .map_err(|e| failure("flush final decision", e))?;
        drop(input);
        let answer = output
            .next_line()
            .await
            .map_err(|e| failure("read final decision", e))?;
        let status = child
            .wait()
            .await
            .map_err(|e| failure("wait final transaction", e))?;
        if !status.success()
            || answer.as_deref() != Some(if matches { "commit: ok" } else { "abort: ok" })
        {
            return Err(Error::Conflict(
                "Git could not finish stash deletion; refresh the list before retrying".into(),
            ));
        }
        Ok(matches)
    })
    .await;
    // This also closes a prepared transaction after failed read/timeout. Never
    // leave an owned child or manually delete its native lock file.
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    result
        .map_err(|_| Error::Conflict("Git stash transaction timed out; refresh and retry".into()))?
}

pub(crate) async fn drop_selected(
    git: &crate::local::LocalGit,
    path: &Path,
    requested: &str,
) -> Result<String> {
    if requested.len() != 40 || !requested.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::Invalid(
            "identity-safe stash Drop currently requires a full SHA-1 object ID".into(),
        ));
    }
    let format = git.run_read(&["rev-parse", "--show-object-format"]).await?;
    let (has_storage, storage, _, _) = git
        .run_raw(&["config", "--get", "extensions.refStorage"], &[])
        .await?;
    if format.trim() != "sha1" || (has_storage && storage.trim() != "files") {
        return Err(Error::Invalid("identity-safe stash Drop supports files/SHA-1 repositories; use native Git for this repository format".into()));
    }
    let common = common_dir(git).await?;
    for _ in 0..3 {
        let p = path.to_owned();
        let c = common.clone();
        let selected = requested.to_owned();
        match blocking(move || rewrite(&p, &c, &selected)).await? {
            Rewrite::Changed { journal } => {
                cleanup(git, path, &common, journal).await?;
                return Ok(format!("Dropped stash {requested}"));
            }
            Rewrite::Last { expected, entries } => {
                if delete_singleton(git, path.to_owned(), common.clone(), expected, entries).await?
                {
                    return Ok(format!("Dropped stash {requested}"));
                }
            }
        }
    }
    Err(Error::Conflict(
        "stash changed while deleting; refresh and retry".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::LocalGit;

    fn git(path: &Path, args: &[&str]) -> String {
        let out = crate::local::hardened_std_command()
            .current_dir(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn fixture(count: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.name", "Stash regression"]);
        git(
            dir.path(),
            &["config", "user.email", "stash@example.invalid"],
        );
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.path().join("file"), "base").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "base"]);
        for n in 0..count {
            std::fs::write(dir.path().join("file"), format!("stash {n}")).unwrap();
            git(dir.path(), &["stash", "push", "-qm", &format!("entry {n}")]);
        }
        dir
    }

    fn entries(path: &Path) -> Vec<Entry> {
        snapshot(&Repository::open(path).unwrap().reflog(STASH).unwrap())
    }

    #[tokio::test]
    async fn drops_top_middle_oldest_and_last_without_losing_other_entries() {
        for (count, index) in [(3, 0), (3, 1), (3, 2), (1, 0)] {
            let dir = fixture(count);
            let before = entries(dir.path());
            let selected = &before[index].new;
            let mut expected = before
                .iter()
                .map(|e| (&e.new, &e.message))
                .collect::<Vec<_>>();
            expected.remove(index);
            let local = LocalGit::new(dir.path());
            drop_selected(&local, dir.path(), selected).await.unwrap();
            let after = entries(dir.path());
            assert_eq!(
                after
                    .iter()
                    .map(|e| (&e.new, &e.message))
                    .collect::<Vec<_>>(),
                expected
            );
            if let Some(first) = after.first() {
                assert_eq!(git(dir.path(), &["rev-parse", STASH]), first.new);
            } else {
                assert!(Repository::open(dir.path())
                    .unwrap()
                    .find_reference(STASH)
                    .is_err());
            }
            assert!(git(
                dir.path(),
                &[
                    "for-each-ref",
                    "--format=%(refname)",
                    "refs/otto/stash-drop/"
                ]
            )
            .is_empty());
        }
    }

    #[tokio::test]
    async fn duplicate_sha_removes_only_newest_occurrence() {
        let dir = fixture(2);
        let original = entries(dir.path());
        let selected = original[1].new.clone();
        git(
            dir.path(),
            &["stash", "store", "-m", "duplicate occurrence", &selected],
        );
        let local = LocalGit::new(dir.path());
        drop_selected(&local, dir.path(), &selected).await.unwrap();
        let after = entries(dir.path());
        assert_eq!(after.len(), 2);
        assert_eq!(
            after
                .iter()
                .map(|e| (&e.new, &e.message))
                .collect::<Vec<_>>(),
            original
                .iter()
                .map(|e| (&e.new, &e.message))
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn final_entry_aba_is_rejected_under_prepared_git_lock() {
        let dir = fixture(1);
        let local = LocalGit::new(dir.path());
        let common = common_dir(&local).await.unwrap();
        let selected = entries(dir.path())[0].new.clone();
        let Rewrite::Last {
            expected,
            entries: old,
        } = rewrite(dir.path(), &common, &selected).unwrap()
        else {
            panic!("singleton");
        };
        std::fs::write(dir.path().join("file"), "external arrival").unwrap();
        git(dir.path(), &["stash", "push", "-qm", "external"]);
        git(
            dir.path(),
            &["stash", "store", "-m", "same oid new occurrence", &selected],
        );
        let before = entries(dir.path());
        assert!(
            !delete_singleton(&local, dir.path().to_owned(), common, expected, old)
                .await
                .unwrap()
        );
        assert_eq!(entries(dir.path()), before);
        assert_eq!(git(dir.path(), &["rev-parse", STASH]), selected);
    }

    #[test]
    fn actual_native_lock_excludes_system_git_in_main_and_linked_worktrees() {
        let dir = fixture(2);
        let linked = dir.path().join("linked");
        git(
            dir.path(),
            &["worktree", "add", "-qb", "linked", linked.to_str().unwrap()],
        );
        let repo = Repository::open(dir.path()).unwrap();
        let before = entries(dir.path());
        let mut tx = repo.transaction().unwrap();
        tx.lock_ref(STASH).unwrap();
        for path in [dir.path(), linked.as_path()] {
            std::fs::write(path.join("file"), "external unsaved work").unwrap();
            for args in [["stash", "drop", "stash@{0}"], ["stash", "push", "-q"]] {
                let out = crate::local::hardened_std_command()
                    .current_dir(path)
                    .args(args)
                    .output()
                    .unwrap();
                assert!(
                    !out.status.success(),
                    "external writer must respect native lock"
                );
                assert_eq!(entries(dir.path()), before);
                assert_eq!(
                    std::fs::read_to_string(path.join("file")).unwrap(),
                    "external unsaved work"
                );
            }
        }
    }

    #[tokio::test]
    async fn interrupted_target_commit_is_repaired_without_recreating_log_entries() {
        let dir = fixture(3);
        let local = LocalGit::new(dir.path());
        let common = common_dir(&local).await.unwrap();
        let repo = Repository::open(dir.path()).unwrap();
        let before = entries(dir.path());
        let mut log = repo.reflog(STASH).unwrap();
        log.remove(0, true).unwrap();
        let after = snapshot(&log);
        let journal = Journal {
            version: 1,
            operation: otto_core::new_id(),
            old_target: before[0].new.clone(),
            before,
            after: after.clone(),
            pins: Vec::new(),
        };
        let record = write_journal(&common, &journal).unwrap();
        // Native history commit without target commit is the documented
        // partial failure boundary; no handwritten Git storage manipulation.
        let mut tx = repo.transaction().unwrap();
        tx.lock_ref(STASH).unwrap();
        tx.set_reflog(STASH, log).unwrap();
        tx.commit().unwrap();
        assert_eq!(git(dir.path(), &["rev-parse", STASH]), journal.old_target);
        cleanup(&local, dir.path(), &common, record.clone())
            .await
            .unwrap();
        assert_eq!(entries(dir.path()), after);
        assert_eq!(git(dir.path(), &["rev-parse", STASH]), journal.after[0].new);
        assert!(!record.exists());
    }

    #[tokio::test]
    async fn packed_stash_and_missing_user_config_preserve_unrelated_refs() {
        for count in [1, 3] {
            let dir = fixture(count);
            git(dir.path(), &["branch", "must-survive"]);
            git(dir.path(), &["pack-refs", "--all"]);
            let branch = git(dir.path(), &["rev-parse", "must-survive"]);
            let selected = entries(dir.path())[0].new.clone();
            git(dir.path(), &["config", "--unset", "user.name"]);
            git(dir.path(), &["config", "--unset", "user.email"]);
            drop_selected(&LocalGit::new(dir.path()), dir.path(), &selected)
                .await
                .unwrap();
            assert_eq!(git(dir.path(), &["rev-parse", "must-survive"]), branch);
            assert_eq!(entries(dir.path()).len(), count - 1);
        }
    }

    #[tokio::test]
    async fn committed_rewrite_survives_gc_before_recovery_cleanup() {
        let dir = fixture(3);
        let local = LocalGit::new(dir.path());
        let common = common_dir(&local).await.unwrap();
        let before = entries(dir.path());
        let Rewrite::Changed { journal } = rewrite(dir.path(), &common, &before[0].new).unwrap()
        else {
            panic!("multiple entries");
        };
        // All refs/data are disposable. Simulate restart before cleanup, with
        // aggressive native GC in between, then recover through the public path.
        git(dir.path(), &["gc", "--prune=now"]);
        for entry in &before {
            assert_eq!(git(dir.path(), &["cat-file", "-t", &entry.new]), "commit");
        }
        recover_pending(&local, dir.path()).await.unwrap();
        assert!(!journal.exists());
        assert!(git(
            dir.path(),
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/otto/stash-drop/"
            ]
        )
        .is_empty());
        let remaining = entries(dir.path());
        assert_eq!(remaining.len(), 2);
        git(dir.path(), &["stash", "apply", &remaining[0].new]);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file")).unwrap(),
            "stash 1"
        );
    }

    #[tokio::test]
    async fn recovery_preserves_later_external_push_and_clear() {
        for clear in [false, true] {
            let dir = fixture(3);
            let local = LocalGit::new(dir.path());
            let common = common_dir(&local).await.unwrap();
            let selected = entries(dir.path())[0].new.clone();
            let Rewrite::Changed { journal } = rewrite(dir.path(), &common, &selected).unwrap()
            else {
                panic!("multiple entries");
            };
            if clear {
                git(dir.path(), &["stash", "clear"]);
            } else {
                std::fs::write(dir.path().join("file"), "new external work").unwrap();
                git(dir.path(), &["stash", "push", "-qm", "later external"]);
            }
            let expected = entries(dir.path());
            cleanup(&local, dir.path(), &common, journal).await.unwrap();
            assert_eq!(entries(dir.path()), expected);
        }
    }

    #[test]
    fn journal_creation_failure_leaves_stash_untouched() {
        let dir = fixture(3);
        let repo = Repository::open(dir.path()).unwrap();
        let common = repo.commondir().canonicalize().unwrap();
        let before = entries(dir.path());
        std::fs::write(common.join(JOURNALS), "not a directory").unwrap();
        assert!(rewrite(dir.path(), &common, &before[0].new).is_err());
        assert_eq!(entries(dir.path()), before);
        assert_eq!(git(dir.path(), &["rev-parse", STASH]), before[0].new);
        assert!(git(
            dir.path(),
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/otto/stash-drop/"
            ]
        )
        .is_empty());
    }

    #[test]
    fn stale_selected_object_never_removes_another_entry() {
        let dir = fixture(3);
        let repo = Repository::open(dir.path()).unwrap();
        let common = repo.commondir().canonicalize().unwrap();
        let before = entries(dir.path());
        let absent = git(dir.path(), &["rev-parse", "HEAD"]);
        assert!(matches!(
            rewrite(dir.path(), &common, &absent),
            Err(Error::Conflict(_))
        ));
        assert_eq!(entries(dir.path()), before);
    }
}
