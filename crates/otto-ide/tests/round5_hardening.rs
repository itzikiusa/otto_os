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
