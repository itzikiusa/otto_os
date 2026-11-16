//! Go build cache: staleness from `go list` (embed assets, cgo sources,
//! `build/` package dirs) and size/age/temp-file pruning.

#![allow(clippy::disallowed_methods)] // test fixtures on a temp dir

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use super::*;

fn set_mtime(path: &Path, when: SystemTime) {
    fs::File::options()
        .append(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

fn go_list_json(dir: &Path) -> Vec<u8> {
    let d = dir.to_string_lossy();
    format!(
        r#"{{"ImportPath":"fmt","Dir":"/goroot/src/fmt","Standard":true,"GoFiles":["print.go"]}}
{{"ImportPath":"example.com/svc/internal/build","Dir":"{d}/internal/build","GoFiles":["build.go"],"CgoFiles":["native.go"],"CFiles":["native.c"]}}
{{"ImportPath":"example.com/svc","Dir":"{d}","GoFiles":["main.go"],"EmbedFiles":["templates/index.html"],"Module":{{"Path":"example.com/svc","GoMod":"{d}/go.mod"}}}}
"#
    )
    .into_bytes()
}

fn write_fixture(dir: &Path) {
    fs::create_dir_all(dir.join("internal/build")).unwrap();
    fs::create_dir_all(dir.join("templates")).unwrap();
    fs::write(dir.join("go.mod"), "module example.com/svc\n\ngo 1.20\n").unwrap();
    fs::write(dir.join("main.go"), "package main\n").unwrap();
    fs::write(dir.join("templates/index.html"), "<h1>v1</h1>").unwrap();
    fs::write(dir.join("internal/build/build.go"), "package build\n").unwrap();
    fs::write(dir.join("internal/build/native.go"), "package build\n").unwrap();
    fs::write(dir.join("internal/build/native.c"), "int x;\n").unwrap();
}

fn stamp(dir: &Path) -> Option<String> {
    stamp_go_list_output(Sha256::new(), &go_list_json(dir), dir)
}

#[test]
fn stamp_changes_when_an_embedded_asset_cgo_source_or_build_dir_file_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write_fixture(dir);
    let base = stamp(dir).expect("stamp");
    assert_eq!(stamp(dir).as_deref(), Some(base.as_str()), "stable");

    let edits: [(&str, &str); 4] = [
        ("templates/index.html", "<h1>v2 changed</h1>"),
        ("internal/build/build.go", "package build\n\nvar X = 1\n"),
        ("internal/build/native.c", "int x = 42;\n"),
        ("go.mod", "module example.com/svc\n\ngo 1.21.0\n"),
    ];
    let mut prev = base;
    for (rel, body) in edits {
        fs::write(dir.join(rel), body).unwrap();
        let next = stamp(dir).expect("stamp");
        assert_ne!(
            next, prev,
            "editing {rel} must invalidate the cached binary"
        );
        prev = next;
    }
}

#[test]
fn unreadable_go_list_output_gives_no_stamp_so_the_binary_is_rebuilt() {
    let tmp = tempfile::tempdir().unwrap();
    for bad in [
        &b"not json"[..],
        b"",
        br#"{"ImportPath":"fmt","Standard":true}"#,
    ] {
        assert!(stamp_go_list_output(Sha256::new(), bad, tmp.path()).is_none());
    }
    let truncated = vec![b' '; HELPER_OUTPUT_CAP];
    assert!(stamp_go_list_output(Sha256::new(), &truncated, tmp.path()).is_none());
}

#[test]
fn prune_bounds_bytes_and_age_and_clears_interrupted_temp_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let now = SystemTime::now();
    let mk = |name: &str, len: u64, age_secs: u64| {
        let p = dir.join(name);
        let f = fs::File::create(&p).unwrap();
        f.set_len(len).unwrap(); // sparse: no real disk use
        drop(f);
        set_mtime(&p, now - Duration::from_secs(age_secs));
        fs::write(dir.join(format!("{name}.stamp")), "s\n").ok();
        p
    };
    let gib = 1024 * 1024 * 1024u64;
    let keep = mk("keepkey", 900 * 1024 * 1024, 5);
    let newer = mk("newer", 900 * 1024 * 1024, 60);
    let older = mk("older", 900 * 1024 * 1024, 120);
    let ancient = mk("ancient", 1024, 15 * 24 * 60 * 60);
    let small = mk("small", 1024, 30);
    let stale_tmp = dir.join("abc.tmp.0001");
    fs::write(&stale_tmp, "partial").unwrap();
    set_mtime(&stale_tmp, now - Duration::from_secs(2 * 60 * 60));
    let fresh_tmp = dir.join("abc.tmp.0002");
    fs::write(&fresh_tmp, "building").unwrap();

    prune_go_cache_dir(dir, "keepkey", now);

    assert!(keep.exists(), "the just-built binary is always kept");
    assert!(small.exists());
    assert!(newer.exists(), "fits under {gib} * 2 with keepkey");
    assert!(!older.exists(), "over the byte budget");
    assert!(!dir.join("older.stamp").exists());
    assert!(!ancient.exists(), "older than the age limit");
    assert!(!stale_tmp.exists(), "temp file of an interrupted build");
    assert!(fresh_tmp.exists(), "a build may still be writing it");
}

/// Against a real toolchain: `go list` sees an embedded asset and a package
/// in a directory named `build`. Skipped when `go` is not installed.
#[tokio::test]
async fn live_go_list_stamp_tracks_embed_and_build_dir_edits() {
    let Ok(path) = std::env::var("PATH") else {
        return;
    };
    let Some(go_bin) = std::env::split_paths(&path)
        .map(|d| d.join("go"))
        .find(|p| p.is_file())
    else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(tmp.path()).unwrap();
    fs::create_dir_all(dir.join("internal/build")).unwrap();
    fs::create_dir_all(dir.join("templates")).unwrap();
    fs::write(dir.join("go.mod"), "module example.com/svc\n\ngo 1.20\n").unwrap();
    fs::write(
        dir.join("main.go"),
        "package main\n\nimport (\n\t_ \"embed\"\n\t\"example.com/svc/internal/build\"\n)\n\n//go:embed templates/index.html\nvar page string\n\nfunc main() { println(page, build.Name) }\n",
    )
    .unwrap();
    fs::write(dir.join("templates/index.html"), "<h1>v1</h1>").unwrap();
    fs::write(
        dir.join("internal/build/build.go"),
        "package build\n\nconst Name = \"a\"\n",
    )
    .unwrap();

    let mut env: BTreeMap<String, String> = std::env::vars()
        .filter(|(k, _)| {
            matches!(
                k.as_str(),
                "PATH" | "HOME" | "GOCACHE" | "GOPATH" | "TMPDIR"
            )
        })
        .collect();
    env.insert("GOTOOLCHAIN".into(), "local".into());
    env.insert("GOFLAGS".into(), "-mod=mod".into());
    let go_bin = go_bin.to_string_lossy().into_owned();
    let roots = [dir.clone()];
    let req = GoBuildRequest {
        go_bin: &go_bin,
        cwd: &dir,
        pkg_spec: ".",
        mode: GoBuildMode::Run,
        extra_build_flags: &[],
        env_map: &env,
        allowed_roots: &roots,
    };
    let s1 = compute_go_list_stamp(&req, &dir, &dir, "fp")
        .await
        .expect("go list stamp");
    fs::write(dir.join("templates/index.html"), "<h1>v2 longer</h1>").unwrap();
    let s2 = compute_go_list_stamp(&req, &dir, &dir, "fp").await.unwrap();
    assert_ne!(s1, s2, "embedded asset edit must change the stamp");
    fs::write(
        dir.join("internal/build/build.go"),
        "package build\n\nconst Name = \"bb\"\n",
    )
    .unwrap();
    let s3 = compute_go_list_stamp(&req, &dir, &dir, "fp").await.unwrap();
    assert_ne!(
        s2, s3,
        "edit in a package dir named build must change the stamp"
    );
}
