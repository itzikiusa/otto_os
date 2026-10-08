use super::*;

fn rows(s: &str) -> Vec<String> {
    s.lines().map(str::to_string).collect()
}

#[test]
fn live_draft_takes_the_region_between_echo_and_input_box() {
    let screen = rows(
        "⏺ earlier answer\n\n> option 2, other services still read GSS_games\n\n⏺ Still exploring. Reading the DAO.\n\n⏺ Bash(cd x && grep -n foo)\n  ⎿  3 lines\n\n✻ Cooking… (esc to interrupt)\n\n────────────────────────────\n❯ \n────────────────────────────\n  -- INSERT --",
    );
    let d = live_draft(&screen);
    assert_eq!(
        d,
        "⏺ Still exploring. Reading the DAO.\n\n⏺ Bash(cd x && grep -n foo)\n  ⎿  3 lines"
    );
}

#[test]
fn live_draft_drops_timer_hint_and_tip_rows() {
    let screen = rows(
        "> go\n\n⏺ Bash(cargo test)\n  ⎿  Running… (7m 34s · timeout 10m)\n     (ctrl+b to run in background)\n\n  Tip: Use /clear to start fresh\n\n────────────────\n❯ ",
    );
    assert_eq!(live_draft(&screen), "⏺ Bash(cargo test)");
}

#[test]
fn live_draft_is_empty_right_after_a_prompt_and_tolerates_no_box() {
    assert_eq!(live_draft(&rows("> hi\n\n❯ ")), "");
    assert_eq!(
        live_draft(&rows("plain output\nmore")),
        "plain output\nmore"
    );
    assert_eq!(live_draft(&[]), "");
}

#[test]
fn screen_parts_splits_input_and_status_rows() {
    let screen = rows(
        "> hi\n\n⏺ working\n\n────────────────────────────\n❯ option 2, other services   \n────────────────────────────\n  ~ | Fable 5.1 | ▓▓░░ 11%\n  -- INSERT --  ▶▶ bypass permissions on",
    );
    let p = screen_parts(&screen);
    assert_eq!(p.draft, "⏺ working");
    assert_eq!(p.input, "option 2, other services");
    assert_eq!(
        p.status,
        "~ | Fable 5.1 | ▓▓░░ 11% · -- INSERT -- ▶▶ bypass permissions on"
    );
    // No input box → draft only.
    let p = screen_parts(&rows("just text"));
    assert_eq!(p.input, "");
    assert_eq!(p.status, "");
}

#[test]
fn git_branch_reads_head_and_follows_worktree_gitdir() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feat/x\n").unwrap();
    let sub = repo.join("crates/a");
    std::fs::create_dir_all(&sub).unwrap();
    assert_eq!(git_branch(&sub).as_deref(), Some("feat/x"));
    // Worktree: `.git` is a file pointing at the gitdir.
    let wt = tmp.path().join("wt");
    let gd = tmp.path().join("gitdir");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::create_dir_all(&gd).unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gd.display())).unwrap();
    std::fs::write(gd.join("HEAD"), "0123456789abcdef\n").unwrap();
    assert_eq!(git_branch(&wt).as_deref(), Some("01234567"));
    assert_eq!(git_branch(tmp.path()), None);
}

#[test]
fn live_draft_caps_to_the_tail() {
    let big: Vec<String> = (0..2000)
        .map(|i| format!("line {i} {}", "x".repeat(20)))
        .collect();
    let d = live_draft(&big);
    assert!(d.len() <= LIVE_CAP + 4);
    assert!(d.starts_with('…'));
    assert!(d.ends_with("line 1999 xxxxxxxxxxxxxxxxxxxx"));
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

/// The tail folds a file that grows in odd-sized appends (lines split
/// anywhere, the initial fold landing mid-line) and must end exactly where
/// a one-shot fold of the finished file does — every record once, the
/// partial line carried, the delta cursor contiguous, sub-agent sidecars
/// attached — without re-reading the file.
#[test]
fn oversized_initial_fold_is_rejected_before_reading_its_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oversized.jsonl");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(TAIL_INPUT_CAP as u64 + 1)
        .unwrap();
    assert!(
        matches!(refold_with(Provider::Claude, &path, Default::default()), Err(e) if e.kind() == std::io::ErrorKind::OutOfMemory)
    );
}

#[test]
fn oversized_fallback_coalesces_changes_and_ignores_unchanged_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fallback.jsonl");
    std::fs::write(&path, b"{}\n").unwrap();
    let mut previous = None;
    let start = Instant::now();
    let mut last = start;
    assert!(fallback_step(
        &path,
        &mut previous,
        &mut last,
        start + FALLBACK_REFRESH / 2
    )
    .is_none());
    assert!(
        fallback_step(&path, &mut previous, &mut last, start + FALLBACK_REFRESH)
            .unwrap()
            .oversize
    );
    std::fs::write(&path, b"{}\n{}\n").unwrap();
    assert!(fallback_step(
        &path,
        &mut previous,
        &mut last,
        start + FALLBACK_REFRESH + Duration::from_secs(1)
    )
    .is_none());
    assert_eq!(
        fallback_step(
            &path,
            &mut previous,
            &mut last,
            start + FALLBACK_REFRESH * 2
        )
        .unwrap()
        .cursor,
        "6"
    );
    assert!(fallback_step(
        &path,
        &mut previous,
        &mut last,
        start + FALLBACK_REFRESH * 3
    )
    .is_none());
}

#[test]
fn aggregate_admission_rejects_without_leaking_charge() {
    let total = AtomicUsize::new(0);
    for _ in 0..4 {
        reserve_total(&total, TAIL_BYTES_CAP).unwrap();
    }
    assert_eq!(total.load(Ordering::Acquire), TOTAL_TAIL_BYTES);
    assert!(reserve_total(&total, 1).is_err());
    assert!(reserve_total(&total, usize::MAX).is_err());
    assert_eq!(total.load(Ordering::Acquire), TOTAL_TAIL_BYTES);
    total.fetch_sub(TAIL_BYTES_CAP, Ordering::AcqRel);
    reserve_total(&total, TAIL_BYTES_CAP).unwrap();
}

#[test]
fn growing_past_live_budget_retires_to_an_explicit_refetch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("growing.jsonl");
    std::fs::write(
        &path,
        b"{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();
    let mut st = refold_with(Provider::Claude, &path, Default::default()).unwrap();
    let live = Live {
        provider: Provider::Claude,
        path: path.clone(),
        state: Mutex::new(None),
        settled: tokio::sync::watch::Sender::new(true),
    };
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(TAIL_INPUT_CAP as u64 + 1)
        .unwrap();
    let out = step(
        &|| Default::default(),
        &"budget-test".into(),
        &live,
        &mut st,
        &mut HashSet::new(),
    )
    .unwrap();
    assert!(st.retired);
    assert!(out.oversize && out.turns.is_empty());
    assert_eq!(
        st.folder.record_count(),
        1,
        "oversized bytes were never folded"
    );
}

#[test]
fn per_tail_charge_limit_is_checked_before_global_reservation() {
    let mut budget = TailBudget::default();
    assert!(budget.reserve(TAIL_BYTES_CAP + 1).is_err());
    assert_eq!(budget.bytes, 0);
    budget.reserve(1024).unwrap();
    assert_eq!(budget.bytes, 1024);
}

#[test]
fn stepping_a_growing_file_matches_a_whole_file_fold() {
    use std::io::Write;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../otto-transcript/fixtures");
    for (provider, sub) in [(Provider::Claude, "claude"), (Provider::Codex, "codex-new")] {
        let mut files: Vec<PathBuf> = std::fs::read_dir(root.join(sub))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        files.sort();
        for src in files {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(src.file_name().unwrap());
            let side = src.with_extension("");
            if side.is_dir() {
                copy_dir(&side, &path.with_extension(""));
            }
            let bytes = std::fs::read(&src).unwrap();
            let head = bytes.len() / 3;
            std::fs::write(&path, &bytes[..head]).unwrap();
            let live = Live {
                provider,
                path: path.clone(),
                state: Mutex::new(None),
                settled: tokio::sync::watch::Sender::new(false),
            };
            let mut st = refold_with(provider, &path, Default::default()).unwrap();
            let mut known: HashSet<String> =
                st.folder.artifacts().iter().map(|a| a.id.clone()).collect();
            let opts = || otto_transcript::FoldOpts::default();
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            let mut at = head;
            for n in [1usize, 97, 4096, 13, 20_000, 7].iter().cycle() {
                if at >= bytes.len() {
                    break;
                }
                let end = (at + n).min(bytes.len());
                f.write_all(&bytes[at..end]).unwrap();
                f.flush().unwrap();
                at = end;
                let before = st.folder.record_count();
                if let Some(out) = step(&opts, &"t".into(), &live, &mut st, &mut known) {
                    assert_eq!(
                        out.cursor,
                        st.folder.record_count().saturating_sub(1).to_string()
                    );
                    assert!(st.folder.record_count() > before || out.turns.is_empty());
                }
            }
            let want = otto_transcript::fold_file(provider, &path, {
                let mut o = otto_transcript::FoldOpts::default();
                if provider == Provider::Claude {
                    o.subagents = otto_transcript::read_subagents(&path);
                }
                o
            })
            .unwrap();
            let got = st.folder.snapshot();
            assert_eq!(got.record_count, want.record_count, "{}", src.display());
            assert_eq!(
                serde_json::to_value(got.turns_since(0)).unwrap(),
                serde_json::to_value(want.turns_since(0)).unwrap(),
                "{}",
                src.display()
            );
            let ids = |f: &Folded| f.artifacts.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
            assert_eq!(ids(&got), ids(&want));
            assert_eq!(known.len(), want.artifacts.len());
        }
    }
}

/// A fold of fixture 01 whose every tool result is blown up to `n`
/// bytes of text (+ a patch of the same size) — the shape of a delta
/// carrying several near-cap Read/Bash results.
fn fat_fold(n: usize) -> Folded {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../otto-transcript/fixtures/claude/01-basic-tools.jsonl");
    let mut f = otto_transcript::fold_file(Provider::Claude, &path, Default::default()).unwrap();
    let mut i = 0usize;
    for t in &mut f.turns {
        for b in &mut t.turn.blocks {
            if let otto_transcript::Block::ToolCall { result, .. } = b {
                i += 1;
                let r = result.get_or_insert_with(Default::default);
                // Multi-byte chars so the cut has to find a boundary.
                r.text = Some(format!("{i}é{}", "λx\n".repeat(n / 4)));
                r.patch = Some(format!("@@ -1 +1 @@\n-{}", "y".repeat(n)));
            }
        }
    }
    assert!(i >= 2, "fixture must carry several tool calls");
    f
}

fn tool_results(turns: &[serde_json::Value]) -> Vec<(String, serde_json::Value)> {
    turns
        .iter()
        .flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default())
        .filter(|b| b["kind"] == "tool_call" && b["result"].is_object())
        .map(|b| (b["id"].as_str().unwrap().to_string(), b["result"].clone()))
        .collect()
}

/// SA-04: an over-cap delta is shrunk (tool-result bodies elided to
/// 4 KB and flagged) instead of being dropped to `turns: []`; the lazy
/// tool endpoint's lookup returns the whole stored result.
#[test]
fn oversized_delta_is_trimmed_under_the_cap_and_the_tool_lookup_is_full() {
    let folded = fat_fold(30 * 1024);
    let mut turns: Vec<serde_json::Value> = folded
        .turns_since(0)
        .iter()
        .map(|t| serde_json::to_value(t).unwrap())
        .collect();
    let before = tool_results(&turns);
    assert!(
        json_size(&turns) > EVENT_CAP,
        "fixture must start over the cap"
    );
    let size = trim_oversized(&mut turns);
    assert!(size <= EVENT_CAP, "trimmed delta must fit: {size}");
    assert_eq!(size, json_size(&turns));
    let after = tool_results(&turns);
    assert_eq!(after.len(), before.len());
    for ((id, full), (_, cut)) in before.iter().zip(&after) {
        assert_eq!(cut["elided"], true, "{id} elided");
        let (ft, ct) = (
            full["text"].as_str().unwrap(),
            cut["text"].as_str().unwrap(),
        );
        assert!(
            ct.len() <= TRIM_TEXT && ft.starts_with(ct),
            "{id} text is a prefix"
        );
        // The stored-result facts are untouched.
        assert_eq!(cut["bytes"], full["bytes"]);
        assert_eq!(cut["truncated"], full["truncated"]);
        // The lazy load gets it all back, never elided.
        let block = find_tool_block(&folded, id).expect("tool block");
        let v = serde_json::to_value(&block).unwrap();
        assert_eq!(v["result"]["text"].as_str().unwrap(), ft);
        assert_eq!(v["result"]["patch"], full["patch"]);
        assert!(v["result"].get("elided").is_none());
    }
    assert!(find_tool_block(&folded, "no-such-tool").is_none());
}

/// Text elision alone is tried first: a patch survives whole when the
/// cut texts already fit, and a delta under the cap is never touched.
#[test]
fn trim_prefers_text_and_leaves_small_deltas_alone() {
    let folded = fat_fold(20 * 1024);
    let mut turns: Vec<serde_json::Value> = folded
        .turns_since(0)
        .iter()
        .map(|t| serde_json::to_value(t).unwrap())
        .collect();
    // Shrink the patches so only the texts overflow.
    for t in &mut turns {
        for b in t["blocks"].as_array_mut().unwrap() {
            if b["kind"] == "tool_call" && b["result"].is_object() {
                b["result"]["patch"] = serde_json::json!("@@ -1 +1 @@\n-a\n+b");
            }
        }
    }
    assert!(json_size(&turns) > EVENT_CAP);
    trim_oversized(&mut turns);
    for (_, r) in tool_results(&turns) {
        assert_eq!(r["patch"], "@@ -1 +1 @@\n-a\n+b");
        assert_eq!(r["elided"], true);
    }
    let small = fat_fold(512);
    let mut turns: Vec<serde_json::Value> = small
        .turns_since(0)
        .iter()
        .map(|t| serde_json::to_value(t).unwrap())
        .collect();
    let untouched = turns.clone();
    assert!(trim_oversized(&mut turns) <= EVENT_CAP);
    assert_eq!(turns, untouched);
}

#[test]
fn slot_guard_frees_the_registry_entry_on_drop() {
    let id: Id = "tail-test-slot".into();
    lock().insert(
        id.clone(),
        Entry {
            last_touch: Instant::now(),
            stop: Arc::new(AtomicBool::new(false)),
            live: Arc::new(Live {
                provider: Provider::Claude,
                path: PathBuf::from("/nonexistent.jsonl"),
                state: Mutex::new(None),
                settled: tokio::sync::watch::Sender::new(false),
            }),
        },
    );
    let live = lock().get(&id).unwrap().live.clone();
    assert!(should_continue(&id, &live));
    {
        let _slot = Slot {
            id: id.clone(),
            live: live.clone(),
        };
    }
    assert!(lock().get(&id).is_none());
    // A missing entry (removed by `stop`) ends the loop under the same lock.
    assert!(!should_continue(&id, &live));
    assert_eq!(EVENT_CAP, 65536);
    assert_eq!(POLL, Duration::from_millis(700));
    assert_eq!(MAX_TAILS, 64);
}

/// Register a tail for `path` whose initial fold has not landed yet.
fn pending_tail(id: &Id, path: &Path) -> Arc<Live> {
    let live = Arc::new(Live {
        provider: Provider::Claude,
        path: path.to_path_buf(),
        state: Mutex::new(None),
        settled: tokio::sync::watch::Sender::new(false),
    });
    lock().insert(
        id.clone(),
        Entry {
            last_touch: Instant::now(),
            stop: Arc::new(AtomicBool::new(false)),
            live: live.clone(),
        },
    );
    live
}

#[tokio::test]
async fn r02_touch_replaces_a_changed_transcript_identity() {
    let dir = tempfile::tempdir().unwrap();
    let pool = crate::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    let session = Session {
        id: "r02-tail-recapture".into(),
        workspace_id: "r02-workspace".into(),
        kind: otto_core::domain::SessionKind::Agent,
        provider: "shell".into(),
        title: "nested agent".into(),
        status: otto_core::domain::SessionStatus::Running,
        cwd: dir.path().to_string_lossy().into_owned(),
        provider_session_id: Some("new-conversation".into()),
        connection_id: None,
        created_by: "r02-owner".into(),
        created_at: chrono::Utc::now(),
        last_active_at: chrono::Utc::now(),
        archived: false,
        meta: serde_json::json!({"nested_provider": "codex"}),
    };
    let old_path = dir.path().join("old-claude.jsonl");
    let new_path = dir.path().join("new-codex.jsonl");
    std::fs::write(&old_path, "{}\n").unwrap();
    std::fs::write(&new_path, "{}\n").unwrap();
    touch(&ctx, &session, Provider::Claude, &old_path);
    assert!(live_of(&session.id, Provider::Claude, &old_path).is_some());
    // A second nested CLI in the same shell changes the provider/path;
    // the normal GET and keep-alive routes call touch with that new pair.
    touch(&ctx, &session, Provider::Codex, &new_path);
    let follows_new = live_of(&session.id, Provider::Codex, &new_path).is_some();
    let follows_old = live_of(&session.id, Provider::Claude, &old_path).is_some();
    stop(&session.id);
    assert!(
        follows_new,
        "touch must switch the live tail to the newly captured transcript"
    );
    assert!(
        !follows_old,
        "the prior nested conversation must stop owning this session's live events"
    );
}

#[tokio::test]
async fn a_read_waits_for_the_tails_initial_fold_instead_of_folding_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    std::fs::write(
        &path,
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();
    let id: Id = "tail-test-settle".into();
    let live = pending_tail(&id, &path);
    // Before the fold lands, a plain `live_page` has nothing to serve.
    assert!(live_page(&id, Provider::Claude, &path, None, 60)
        .await
        .is_none());
    let (lv, p) = (live.clone(), path.clone());
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let st = refold_with(Provider::Claude, &p, Default::default()).unwrap();
        *lv.lock() = Some(st);
        lv.settled.send_replace(true);
    });
    let folded = live_page_settled(&id, Provider::Claude, &path, None, 60)
        .await
        .expect("served from the tail once its fold settled");
    assert_eq!(folded.turns.len(), 1);
    // Another file of the same session is not this tail's to serve.
    let other = dir.path().join("other.jsonl");
    assert!(live_page_settled(&id, Provider::Claude, &other, None, 60)
        .await
        .is_none());
    lock().remove(&id);
}

#[tokio::test]
async fn a_failed_initial_fold_releases_the_waiting_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gone.jsonl");
    let id: Id = "tail-test-settle-fail".into();
    let live = pending_tail(&id, &path);
    live.settled.send_replace(true);
    let started = Instant::now();
    assert!(live_page_settled(&id, Provider::Claude, &path, None, 60)
        .await
        .is_none());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "no wait once settled"
    );
    lock().remove(&id);
}

#[test]
fn retired_tail_cannot_remove_its_replacement() {
    let id: Id = "tail-generation-replacement".into();
    let old = pending_tail(&id, Path::new("old.jsonl"));
    let retired_slot = Slot {
        id: id.clone(),
        live: old.clone(),
    };
    let new = pending_tail(&id, Path::new("new.jsonl"));
    assert!(!should_continue(&id, &old));
    assert!(should_continue(&id, &new));
    let mut published = Vec::new();
    assert!(!while_current(&id, &old, || published.push("old")));
    assert!(while_current(&id, &new, || published.push("new")));
    assert_eq!(published, ["new"]);
    drop(retired_slot);
    assert!(should_continue(&id, &new));
    // Reopening the same filename still creates a distinct generation.
    let reopened = pending_tail(&id, Path::new("old.jsonl"));
    assert!(!should_continue(&id, &old));
    assert!(!while_current(&id, &new, || published.push("retired")));
    assert!(while_current(&id, &reopened, || published.push("reopened")));
    assert_eq!(published, ["new", "reopened"]);
    stop(&id);
    assert!(!should_continue(&id, &new));
}
