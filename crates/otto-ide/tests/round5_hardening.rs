//! Round 5 end-to-end integration tests for `otto-ide`:
//! - `GET /ide/fs/git-base`: broad-root rejection, nested subrepo inside parent `.git`,
//!   CRLF & no-trailing-newline preservation, staged `git mv` rename follow, untracked & unborn `HEAD`.
//! - `GET /ide/fs/file` & `PUT /ide/fs/file`: `> 4 MiB` UTF-8 char-boundary truncation
//!   (`read_only: true`, zero `U+FFFD`) and `409 Conflict` overwrite guard unless `allow_overwrite_large: true`.
//! - `POST /ide/lsp/code-actions`, `hover`, `definition`: `\r\n` CRLF preservation and
//!   1-based UTF-16 column accuracy after 4-byte astral emojis (`🚀`).
//! - `POST /ide/gutter`: Go `main.go` alongside sibling `*_test.go` emits `debug_program`
//!   pointing to the `main.go` file (never parent directory).
//! - Plus `round5_env` submodule: `DormantNetBridge` debug restart, non-root `Editor` RBAC &
//!   loader-env hardening, and `module_go_build_tags: []` + Run Config wire parity.

#![allow(clippy::disallowed_methods)]

mod common;
mod round5_env;

use std::path::Path;
use std::process::Command;

use axum::http::{Method, StatusCode};
use common::{build_test_app, send_json, urlencode};
use serde_json::json;

#[allow(clippy::disallowed_methods)]
fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn git_base_handles_broad_roots_nested_subrepo_crlf_renames_and_unborn_head() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    let db_path = tmp.path().join("state.db");
    let (_ctx, app) = build_test_app(data_dir, &db_path).await;

    // 1. Broad non-project root (`/tmp`) returns `in_repo: false`.
    let (st, broad) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/git-base?root={}&path={}",
            urlencode("/tmp"),
            urlencode("/tmp/anything.txt")
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "broad git-base failed: {broad}");
    assert_eq!(broad["in_repo"], json!(false));
    assert_eq!(broad["head_exists"], json!(false));
    assert!(broad["content"].is_null());

    // 2. Unborn HEAD repository returns `in_repo: true`, `head_exists: false`.
    let unborn = tmp.path().join("unborn-repo");
    std::fs::create_dir_all(&unborn).unwrap();
    git(&unborn, &["init"]);
    let unborn_file = unborn.join("draft.rs");
    std::fs::write(&unborn_file, "fn draft() {}\n").unwrap();
    let unborn_root = unborn.to_string_lossy().into_owned();
    let (st, ub_resp) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/git-base?root={}&path={}",
            urlencode(&unborn_root),
            urlencode(&unborn_file.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(ub_resp["in_repo"], json!(true));
    assert_eq!(ub_resp["head_exists"], json!(false));
    assert!(ub_resp["content"].is_null());

    // 3. Parent dir has `.git`, nested `subrepo` has its own `.git`:
    //    `git-base` resolves `subrepo` (never parent) and preserves exact `\r\n` & missing trailing `\n`.
    let umbrella = tmp.path().join("umbrella");
    let subrepo = umbrella.join("subrepo");
    std::fs::create_dir_all(&subrepo).unwrap();
    git(&umbrella, &["init"]);
    git(&subrepo, &["init"]);

    let crlf_bytes = b"line_one\r\nline_two\r\nno_final_newline";
    let crlf_file = subrepo.join("win.txt");
    std::fs::write(&crlf_file, crlf_bytes).unwrap();

    let orig_rename = subrepo.join("old_name.rs");
    std::fs::write(&orig_rename, "pub fn answer() -> i32 {\n    41\n}\n").unwrap();

    git(&subrepo, &["add", "win.txt", "old_name.rs"]);
    git(&subrepo, &["commit", "-m", "initial"]);

    // Modify working tree of `win.txt`, add untracked file, and stage `git mv old_name.rs new_name.rs` + edit.
    std::fs::write(&crlf_file, b"changed_in_wt\r\n").unwrap();
    let untracked = subrepo.join("brand_new.rs");
    std::fs::write(&untracked, "pub fn fresh() {}\n").unwrap();
    git(&subrepo, &["mv", "old_name.rs", "new_name.rs"]);
    let renamed_file = subrepo.join("new_name.rs");
    std::fs::write(&renamed_file, "pub fn answer() -> i32 {\n    42\n}\n").unwrap();

    let sub_root = std::fs::canonicalize(&subrepo)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    // 3a. Exact CRLF + missing trailing newline from `HEAD:win.txt`.
    let (st, crlf_resp) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/git-base?root={}&path={}",
            urlencode(&sub_root),
            urlencode(&crlf_file.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "crlf git-base: {crlf_resp}");
    assert_eq!(crlf_resp["in_repo"], json!(true));
    assert_eq!(crlf_resp["repo_root"], json!(sub_root));
    assert_eq!(crlf_resp["rel_path"], json!("win.txt"));
    assert_eq!(crlf_resp["head_exists"], json!(true));
    assert_eq!(
        crlf_resp["content"].as_str().unwrap().as_bytes(),
        crlf_bytes,
        "git-base must preserve exact CRLF and missing trailing newline"
    );

    // 3b. Staged rename `old_name.rs -> new_name.rs` returns `HEAD:old_name.rs`!
    let (st, ren_resp) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/git-base?root={}&path={}",
            urlencode(&sub_root),
            urlencode(&renamed_file.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "rename git-base: {ren_resp}");
    assert_eq!(ren_resp["in_repo"], json!(true));
    assert_eq!(ren_resp["head_exists"], json!(true));
    assert_eq!(
        ren_resp["content"],
        json!("pub fn answer() -> i32 {\n    41\n}\n"),
        "renamed file must return original HEAD blob so gutter marks only edited lines"
    );

    // 3c. Untracked file returns `in_repo: true`, `head_exists: false`.
    let (st, unt_resp) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/git-base?root={}&path={}",
            urlencode(&sub_root),
            urlencode(&untracked.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(unt_resp["in_repo"], json!(true));
    assert_eq!(unt_resp["head_exists"], json!(false));
    assert!(unt_resp["content"].is_null());
}

#[tokio::test]
async fn truncated_large_file_trims_utf8_boundary_marks_read_only_and_blocks_overwrite() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("state.db")).await;

    // Construct a >4 MiB file where a 4-byte emoji (`🚀` = F0 9F 9A 80) straddles byte 4_194_304!
    // 4_194_302 ASCII bytes + `🚀` (bytes 4_194_302..4_194_306) + tail text.
    let cap = 4 * 1024 * 1024usize;
    let mut raw = vec![b'a'; cap - 2];
    raw.extend_from_slice("🚀TAIL_MUST_SURVIVE\n".as_bytes());
    assert!(raw.len() > cap);
    let big_file = ws.join("huge.txt");
    std::fs::write(&big_file, &raw).unwrap();

    let ws_str = ws.to_string_lossy().into_owned();
    let file_str = big_file.to_string_lossy().into_owned();

    let (st, read_resp) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/file?root={}&path={}",
            urlencode(&ws_str),
            urlencode(&file_str)
        ),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "read huge file: {read_resp}");
    assert_eq!(read_resp["truncated"], json!(true));
    assert_eq!(
        read_resp["read_only"],
        json!(true),
        "truncated >4 MiB file must be marked read_only"
    );
    let content = read_resp["content"].as_str().unwrap();
    assert!(
        !content.contains('\u{FFFD}'),
        "truncated UTF-8 content must trim cleanly at char boundary without U+FFFD"
    );
    assert_eq!(content.len(), cap - 2);

    // Saving without `allow_overwrite_large` MUST return `409 Conflict` and leave disk untouched.
    let (st_conflict, err_body) = send_json(
        &app,
        Method::PUT,
        "/ide/fs/file",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": file_str,
            "content": "accidentally truncated buffer"
        })),
    )
    .await;
    assert_eq!(
        st_conflict,
        StatusCode::CONFLICT,
        "overwriting >4 MiB file without allow_overwrite_large must return 409: {err_body}"
    );
    assert_eq!(std::fs::read(&big_file).unwrap().len(), raw.len());

    // Explicit `allow_overwrite_large: true` permits intentional replacement.
    let (st_ok, _) = send_json(
        &app,
        Method::PUT,
        "/ide/fs/file",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": file_str,
            "content": "intentional small replacement\n",
            "allow_overwrite_large": true
        })),
    )
    .await;
    assert_eq!(st_ok, StatusCode::OK);
    assert_eq!(
        std::fs::read_to_string(&big_file).unwrap(),
        "intentional small replacement\n"
    );
}

#[tokio::test]
async fn crlf_and_utf16_emoji_columns_are_accurate_in_code_actions_hover_and_definition() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws-utf16");
    std::fs::create_dir_all(&ws).unwrap();
    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("state.db")).await;

    let ts_path = ws.join("emoji.ts");
    // Line 1 & 2: unsorted imports with CRLF `\r\n`
    // Line 3: target declaration
    // Line 4: 4-byte astral emoji `🚀` (2 UTF-16 code units, 4 UTF-8 bytes) before `unusedVal` and `computeTarget`!
    let content = concat!(
        "import { z } from \"./z\";\r\n",
        "import { a } from \"./a\";\r\n",
        "export function computeTarget(): number { return 7; }\r\n",
        "const icon = \"🚀\"; const unusedVal = computeTarget();\r\n"
    );
    std::fs::write(&ts_path, content.as_bytes()).unwrap();

    // Compute exact 1-based UTF-16 column of `unusedVal` and `computeTarget` on line 4:
    // `"const icon = \""` = 14 UTF-16 units
    // `"🚀"` = 2 UTF-16 units (cols 15..17)
    // `"\"; const "` = 9 UTF-16 units -> `unusedVal` starts at 1-based UTF-16 column `1 + 14 + 2 + 9 = 26`!
    // `"unusedVal = "` = 12 UTF-16 units -> `computeTarget` starts at 1-based UTF-16 column `26 + 12 = 38`!
    let unused_utf16_col = 26u32;
    let call_utf16_col = 38u32;

    let ws_str = ws.to_string_lossy().into_owned();
    let ts_str = ts_path.to_string_lossy().into_owned();

    // 1. `POST /ide/lsp/code-actions`: preserves `\r\n` CRLF and prefixes `unusedVal` -> `_unusedVal`
    //    at exact UTF-16 column 26 without corrupting `🚀`.
    let (st, ca_resp) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/code-actions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": ts_str,
            "line": 4,
            "column": unused_utf16_col,
            "content": content,
            "diagnostics": [{
                "line": 4,
                "column": unused_utf16_col,
                "message": "'unusedVal' is declared but its value is never read."
            }]
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "code-actions failed: {ca_resp}");
    let actions = ca_resp["actions"].as_array().unwrap();
    assert!(!actions.is_empty(), "expected code actions: {ca_resp}");

    for act in actions {
        if let Some(rep) = act["replacement_content"].as_str() {
            let bare_lf = rep
                .as_bytes()
                .windows(2)
                .filter(|w| w[0] != b'\r' && w[1] == b'\n')
                .count();
            assert_eq!(
                bare_lf, 0,
                "action '{}' stripped CRLF into bare LF: {rep:?}",
                act["title"]
            );
            if !act["title"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains("remove")
            {
                assert!(rep.contains("🚀"), "emoji must remain intact: {rep:?}");
            }
        }
    }
    let prefix_action = actions
        .iter()
        .find(|a| {
            a["replacement_content"]
                .as_str()
                .is_some_and(|r| r.contains("_unusedVal"))
        })
        .unwrap_or_else(|| panic!("expected _unusedVal quick-fix in {actions:?}"));
    assert!(prefix_action["replacement_content"]
        .as_str()
        .unwrap()
        .contains("const icon = \"🚀\"; const _unusedVal = computeTarget();\r\n"));

    // 2. `POST /ide/lsp/hover` & `POST /ide/lsp/definition` at UTF-16 column 38 after `🚀` resolve `computeTarget`!
    let (st_h, hover_resp) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/hover",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": ts_str,
            "line": 4,
            "column": call_utf16_col,
            "content": content
        })),
    )
    .await;
    assert_eq!(st_h, StatusCode::OK);
    assert_eq!(hover_resp["symbol"], json!("computeTarget"));

    let (st_d, def_resp) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/definition",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": ts_str,
            "line": 4,
            "column": call_utf16_col,
            "content": content
        })),
    )
    .await;
    assert_eq!(st_d, StatusCode::OK);
    assert_eq!(def_resp["symbol"], json!("computeTarget"));
    let defs = def_resp["locations"]
        .as_array()
        .expect("definition locations array");
    assert!(
        !defs.is_empty(),
        "expected definition for computeTarget: {def_resp}"
    );
    assert_eq!(defs[0]["line"], json!(3));
}

#[tokio::test]
async fn go_gutter_on_main_go_alongside_test_file_emits_main_go_debug_program() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("gomain");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("go.mod"), "module example.com/gomain\n\ngo 1.23\n").unwrap();
    let main_go = ws.join("main.go");
    let foo_test = ws.join("foo_test.go");
    let main_src = "package main\n\nfunc main() {\n\tprintln(\"hi\")\n}\n";
    std::fs::write(&main_go, main_src).unwrap();
    std::fs::write(
        &foo_test,
        "package main\n\nimport \"testing\"\n\nfunc TestFoo(t *testing.T) {}\n",
    )
    .unwrap();

    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("state.db")).await;
    let ws_str = ws.to_string_lossy().into_owned();
    let main_str = main_go.to_string_lossy().into_owned();

    let (st, resp) = send_json(
        &app,
        Method::POST,
        "/ide/gutter",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": main_str,
            "content": main_src
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "gutter failed: {resp}");
    let items = resp["items"].as_array().unwrap();
    let main_item = items
        .iter()
        .find(|i| i["kind"] == json!("main"))
        .expect("expected main gutter item");
    assert_eq!(main_item["run_spec"]["runner"], json!("go_run"));
    let dbg_prog = main_item["run_spec"]["debug_program"]
        .as_str()
        .expect("GoRun gutter item must carry debug_program");
    assert!(
        dbg_prog.ends_with("/main.go"),
        "debug_program must point to main.go (not parent directory): {dbg_prog}"
    );
}

#[tokio::test]
async fn round6_git_base_secondary_multiroot_worktree_readonly_gitdir_and_cat_file_500() {
    let tmp = tempfile::tempdir().unwrap();
    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("state.db")).await;

    // 1. Primary repo `repo_a` + attached secondary folder `repo_b` (each with its own independent `.git`):
    //    `GET /ide/fs/git-base?root=<repo_a>&path=<repo_b/sec.go>` must resolve `repo_b`'s HEAD!
    let repo_a = tmp.path().join("repo-a");
    let repo_b = tmp.path().join("repo-b");
    std::fs::create_dir_all(&repo_a).unwrap();
    std::fs::create_dir_all(&repo_b).unwrap();
    git(&repo_a, &["init"]);
    git(&repo_b, &["init"]);

    let file_a = repo_a.join("main.go");
    std::fs::write(&file_a, "package main\nfunc main() {}\n").unwrap();
    git(&repo_a, &["add", "main.go"]);
    git(&repo_a, &["commit", "-m", "init a"]);

    let file_b = repo_b.join("sec.go");
    std::fs::write(&file_b, "package sec\n// head-from-repo-b\n").unwrap();
    git(&repo_b, &["add", "sec.go"]);
    git(&repo_b, &["commit", "-m", "init b"]);
    std::fs::write(&file_b, "package sec\n// dirty-working-tree\n").unwrap();

    let a_str = std::fs::canonicalize(&repo_a)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let b_str = std::fs::canonicalize(&repo_b)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let file_b_str = std::fs::canonicalize(&file_b)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let (st_attach, attach_body) = send_json(
        &app,
        Method::POST,
        "/ide/projects/folders",
        Some(json!({
            "target": { "kind": "local", "root": a_str },
            "folder_path": b_str,
        })),
    )
    .await;
    assert_eq!(st_attach, StatusCode::OK, "attach repo_b: {attach_body}");

    let (st_sec, sec_resp) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/git-base?root={}&path={}",
            urlencode(&a_str),
            urlencode(&file_b_str)
        ),
        None,
    )
    .await;
    assert_eq!(st_sec, StatusCode::OK, "secondary git-base: {sec_resp}");
    assert_eq!(sec_resp["in_repo"], json!(true));
    assert_eq!(sec_resp["repo_root"], json!(b_str));
    assert_eq!(sec_resp["rel_path"], json!("sec.go"));
    assert_eq!(sec_resp["head_exists"], json!(true));
    assert_eq!(
        sec_resp["content"],
        json!("package sec\n// head-from-repo-b\n")
    );

    // 2. Linked `git worktree` (`.git` is a regular file `gitdir: ...`):
    //    both `GET /ide/fs/git-base` and `POST /ide/fs/complete` must succeed.
    let wt_dir = tmp.path().join("repo-a-wt");
    git(
        &repo_a,
        &["worktree", "add", wt_dir.to_str().unwrap(), "-b", "wt-feat"],
    );
    assert!(
        wt_dir.join(".git").is_file(),
        "linked git worktree .git must be a regular file"
    );
    let wt_str = std::fs::canonicalize(&wt_dir)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let wt_main = wt_dir.join("main.go");
    std::fs::write(&wt_main, "package main\nfunc main() { println(1) }\n").unwrap();

    let (st_wt, wt_resp) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/git-base?root={}&path={}",
            urlencode(&wt_str),
            urlencode(&wt_main.to_string_lossy())
        ),
        None,
    )
    .await;
    assert_eq!(st_wt, StatusCode::OK, "worktree git-base: {wt_resp}");
    assert_eq!(wt_resp["in_repo"], json!(true));
    assert_eq!(wt_resp["head_exists"], json!(true));
    assert_eq!(wt_resp["content"], json!("package main\nfunc main() {}\n"));

    let (st_wt_comp, wt_comp) = send_json(
        &app,
        Method::POST,
        "/ide/fs/complete",
        Some(json!({
            "target": { "kind": "local", "root": wt_str },
            "query": "@main"
        })),
    )
    .await;
    assert_eq!(st_wt_comp, StatusCode::OK);
    assert!(wt_comp["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == "main.go"));

    // 3. Read-only `.git` directory (`0555`): `GIT_OPTIONAL_LOCKS=0` prevents `index.lock` failure.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(repo_b.join(".git"), std::fs::Permissions::from_mode(0o555))
            .unwrap();
        let (st_ro, ro_resp) = send_json(
            &app,
            Method::GET,
            &format!(
                "/ide/fs/git-base?root={}&path={}",
                urlencode(&b_str),
                urlencode(&file_b_str)
            ),
            None,
        )
        .await;
        std::fs::set_permissions(repo_b.join(".git"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        assert_eq!(st_ro, StatusCode::OK, "0555 .git git-base: {ro_resp}");
        assert_eq!(ro_resp["head_exists"], json!(true));
    }

    // 4. Corrupt loose git blob object: non-missing `git cat-file` failure must return `500`,
    //    never silently reporting `head_exists: false`.
    let blob_sha_out = Command::new("git")
        .arg("-C")
        .arg(&repo_b)
        .args(["rev-parse", "HEAD:sec.go"])
        .output()
        .unwrap();
    let blob_sha = String::from_utf8_lossy(&blob_sha_out.stdout)
        .trim()
        .to_string();
    let obj_path = repo_b
        .join(".git/objects")
        .join(&blob_sha[..2])
        .join(&blob_sha[2..]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&obj_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    std::fs::write(&obj_path, b"corrupted-loose-git-object").unwrap();

    let (st_corrupt, corrupt_resp) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/git-base?root={}&path={}",
            urlencode(&b_str),
            urlencode(&file_b_str)
        ),
        None,
    )
    .await;
    assert_eq!(
        st_corrupt,
        StatusCode::INTERNAL_SERVER_ERROR,
        "corrupt git object must propagate 500 instead of fake head_exists=false: {corrupt_resp}"
    );
}

#[tokio::test]
async fn round6_go_chained_struct_field_receiver_non_utf8_after_8kib_and_spaced_filenames() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws-r6");
    std::fs::create_dir_all(&ws).unwrap();
    let (ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("state.db")).await;
    let ws_str = std::fs::canonicalize(&ws)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    // 1. Go same-package struct field receiver disambiguation (`g.pool.Submit` -> `(*Pool).Submit`,
    //    NOT `(*GoRoutineConcurrencyHelper).Submit`).
    std::fs::write(ws.join("go.mod"), "module example.com/r6\n\ngo 1.23\n").unwrap();
    let helper_src = concat!(
        "package worker\n\n",
        "type GoRoutineConcurrencyHelper struct {\n",
        "\tpool *Pool\n",
        "}\n\n",
        "func (g *GoRoutineConcurrencyHelper) Submit() {}\n\n",
        "func (g *GoRoutineConcurrencyHelper) Run() {\n",
        "\tg.pool.Submit()\n",
        "}\n",
    );
    let pool_src = concat!(
        "package worker\n\n",
        "type Pool struct{}\n\n",
        "func (p *Pool) Submit() {}\n",
    );
    let helper_go = ws.join("helper.go");
    let pool_go = ws.join("pool.go");
    std::fs::write(&helper_go, helper_src).unwrap();
    std::fs::write(&pool_go, pool_src).unwrap();
    let canon_pool_str = std::fs::canonicalize(&pool_go)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    // Line 10 is `\tg.pool.Submit()`: `\t` (col 1), `g` (2), `.` (3), `pool` (4..7), `.` (8), `Submit` starts at col 9.
    let (st_def, def_resp) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/definition",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": helper_go.to_string_lossy(),
            "line": 10,
            "column": 9,
            "content": helper_src,
        })),
    )
    .await;
    assert_eq!(st_def, StatusCode::OK, "go definition: {def_resp}");
    let locs = def_resp["locations"].as_array().unwrap();
    assert!(
        !locs.is_empty(),
        "expected definition for g.pool.Submit: {def_resp}"
    );
    assert_eq!(
        locs[0]["path"],
        json!(canon_pool_str),
        "g.pool.Submit() must resolve (*Pool).Submit in pool.go, not helper.go: {locs:?}"
    );
    assert_eq!(locs[0]["line"], json!(5));

    // 2. Non-UTF-8 byte after 8 KiB (`10 KiB` ASCII + `\xFF\xFE`) marks `binary: true` & `read_only: true`.
    let mut late_non_utf8 = vec![b'x'; 10 * 1024];
    late_non_utf8.extend_from_slice(&[0xFF, 0xFE, b'\n']);
    let bin_tail_path = ws.join("late_latin1.dat");
    std::fs::write(&bin_tail_path, &late_non_utf8).unwrap();

    let (st_bin, bin_resp) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/file?root={}&path=late_latin1.dat",
            urlencode(&ws_str)
        ),
        None,
    )
    .await;
    assert_eq!(st_bin, StatusCode::OK);
    assert_eq!(bin_resp["binary"], json!(true));
    assert_eq!(bin_resp["read_only"], json!(true));

    // 3. Relative filenames with leading/trailing spaces round-trip without `.trim()` mangling.
    let spaced_rel = " padded name.txt ";
    let (st_put_sp, put_sp_resp) = send_json(
        &app,
        Method::PUT,
        "/ide/fs/file",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": spaced_rel,
            "content": "exact spaces preserved\n",
        })),
    )
    .await;
    assert_eq!(st_put_sp, StatusCode::OK, "put spaced file: {put_sp_resp}");
    assert!(
        ws.join(spaced_rel).is_file(),
        "exact leading/trailing spaces must be preserved on disk"
    );

    let (st_get_sp, get_sp_resp) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/fs/file?root={}&path={}",
            urlencode(&ws_str),
            urlencode(spaced_rel)
        ),
        None,
    )
    .await;
    assert_eq!(st_get_sp, StatusCode::OK, "get spaced file: {get_sp_resp}");
    assert_eq!(get_sp_resp["content"], json!("exact spaces preserved\n"));

    // 4. Unified DAP env builder strips dangerous loader/compiler/HOME vars from `.env` and `launch.env`.
    std::fs::write(
        ws.join("debug.vars"),
        "SAFE_DEBUG_KEY=ok\nHOME=/tmp/evil_dap_home\nGOCACHEPROG=/tmp/evil_prog\nCC=/tmp/evil_cc\nCGO_LDFLAGS=-L/tmp/evil\nLD_PRELOAD=/tmp/evil.so\n",
    )
    .unwrap();
    let mut launch_env = std::collections::BTreeMap::new();
    launch_env.insert("SAFE_LAUNCH_KEY".into(), "1".into());
    launch_env.insert("GOTOOLCHAIN".into(), "path".into());
    launch_env.insert("JAVA_OPTS".into(), "-javaagent:/tmp/evil.jar".into());
    let dbg_req = otto_ide::types::IdeStartDebugReq {
        target: otto_ide::types::IdeTargetSpec::local(ws_str.clone()),
        language: "go".into(),
        launch: otto_ide::types::IdeDebugLaunchSpec {
            program: ".".into(),
            cwd: Some(ws_str.clone()),
            env_file: Some("debug.vars".into()),
            env: launch_env,
            ..Default::default()
        },
        ..Default::default()
    };
    let dbg_ctx = otto_ide::dap::DapStartContext::new().with_caller_is_root(false);
    let eff = otto_ide::dap::adapters::build_effective_env(&ctx.data_dir, &dbg_req, &dbg_ctx);
    assert_eq!(eff.get("SAFE_DEBUG_KEY").map(String::as_str), Some("ok"));
    assert_eq!(eff.get("SAFE_LAUNCH_KEY").map(String::as_str), Some("1"));
    for blocked in [
        "GOCACHEPROG",
        "CC",
        "CGO_LDFLAGS",
        "LD_PRELOAD",
        "GOTOOLCHAIN",
        "JAVA_OPTS",
    ] {
        assert!(
            !eff.contains_key(blocked),
            "DAP effective env must strip dangerous key {blocked}: {eff:?}"
        );
    }
    assert_ne!(
        eff.get("HOME").map(String::as_str),
        Some("/tmp/evil_dap_home")
    );
}
