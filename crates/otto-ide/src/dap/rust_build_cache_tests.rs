use super::*;

#[test]
fn rust_artifact_stamp_hits_and_invalidates_on_source_or_binary_edit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let crate_dir = dir.path().join("demo");
    fs::create_dir_all(crate_dir.join("src")).expect("mkdir src");
    fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    fs::write(crate_dir.join("src/main.rs"), "fn main() {}\n").expect("write main.rs");

    let exe_path = dir.path().join("demo-bin");
    fs::write(&exe_path, b"fake-elf-macho-binary").expect("write binary");

    let args = vec!["build".to_string(), "--message-format=json".to_string()];
    let stamp1 =
        compute_rust_stamp_sync("cargo", &args, &crate_dir, dir.path(), "", false, &[], &[])
            .expect("stamp1");

    let entry_path = dir.path().join("entry.json");
    let exe_meta = fs::metadata(&exe_path).unwrap();
    let mtime = exe_meta
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let entry = CachedRustArtifactEntry {
        stamp: stamp1.stamp.clone(),
        executable: exe_path.to_string_lossy().into_owned(),
        exe_len: exe_meta.len(),
        exe_mtime_nanos: mtime,
    };
    fs::write(&entry_path, serde_json::to_vec(&entry).unwrap()).unwrap();

    assert_eq!(
        read_valid_cached_executable(&entry_path, &stamp1.stamp).as_deref(),
        Some(exe_path.to_string_lossy().as_ref())
    );

    fs::write(
        crate_dir.join("src/main.rs"),
        "fn main() { println!(\"hi\"); }\n",
    )
    .expect("modify main.rs");
    let stamp2 =
        compute_rust_stamp_sync("cargo", &args, &crate_dir, dir.path(), "", false, &[], &[])
            .expect("stamp2");
    assert_ne!(stamp1.stamp, stamp2.stamp);
    assert!(read_valid_cached_executable(&entry_path, &stamp2.stamp).is_none());

    fs::remove_file(&exe_path).unwrap();
    assert!(read_valid_cached_executable(&entry_path, &stamp1.stamp).is_none());
}

#[test]
fn sibling_workspace_crate_edit_invalidates_stamp_via_walk_and_depinfo() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = dir.path().join("ws");
    let cache_dir = dir.path().join("cache");
    let app_dir = ws.join("crates/app");
    let sib_dir = ws.join("crates/common");
    fs::create_dir_all(app_dir.join("src")).unwrap();
    fs::create_dir_all(sib_dir.join("src/sub dir")).unwrap();
    fs::write(
        ws.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/app\", \"crates/common\"]\n",
    )
    .unwrap();
    fs::write(ws.join("Cargo.lock"), "# lock v1\n").unwrap();
    fs::write(
        app_dir.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let main_rs = app_dir.join("src/main.rs");
    fs::write(&main_rs, "fn main() {}\n").unwrap();
    fs::write(
        sib_dir.join("Cargo.toml"),
        "[package]\nname = \"common\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let sib_lib = sib_dir.join("src/lib.rs");
    let sib_spaced = sib_dir.join("src/sub dir/helper.rs");
    fs::write(&sib_lib, "pub fn v() -> u32 { 1 }\n").unwrap();
    fs::write(&sib_spaced, "pub const X: u32 = 1;\n").unwrap();

    let args = vec!["build".to_string(), "--message-format=json".to_string()];
    // 1. Cold workspace-walk mode: editing sibling crate invalidates `app` even when
    // `target_root` was scoped to `app_dir`!
    let s1 = compute_rust_stamp_with_cache_dir(
        &cache_dir,
        "cargo",
        &args,
        &app_dir,
        &app_dir,
        "",
        false,
        &[],
        &[],
    )
    .unwrap();
    fs::write(&sib_lib, "pub fn v() -> u32 { 222 }\n").unwrap();
    let s2 = compute_rust_stamp_with_cache_dir(
        &cache_dir,
        "cargo",
        &args,
        &app_dir,
        &app_dir,
        "",
        false,
        &[],
        &[],
    )
    .unwrap();
    assert_ne!(
        s1.stamp, s2.stamp,
        "sibling crate edit must invalidate workspace walk stamp"
    );

    // 2. `.d` dep-info fast-path mode (including `\ ` escaped spaces):
    let exe = ws.join("target/debug/app");
    fs::create_dir_all(exe.parent().unwrap()).unwrap();
    fs::write(&exe, b"binary-bytes").unwrap();
    let dep_txt = format!(
        "{}: {} \\\n  {} {}\n\n{}:\n",
        exe.display(),
        main_rs.display(),
        sib_lib.display(),
        sib_spaced.to_string_lossy().replace(' ', "\\ "),
        main_rs.display(),
    );
    fs::write(ws.join("target/debug/app.d"), dep_txt).unwrap();
    save_cached_executable_sync(&cache_dir, &s2, &exe.to_string_lossy());
    assert!(cache_dir.join(format!("{}.d", s2.cache_key)).is_file());

    let s_hit = compute_rust_stamp_with_cache_dir(
        &cache_dir,
        "cargo",
        &args,
        &app_dir,
        &app_dir,
        "",
        false,
        &[],
        &[],
    )
    .unwrap();
    let entry_path = cache_dir.join(format!("{}.json", s_hit.cache_key));
    assert_eq!(
        read_valid_cached_executable(&entry_path, &s_hit.stamp).as_deref(),
        Some(exe.to_string_lossy().as_ref()),
        "second lookup must hit cached artifact via .d dep-info"
    );

    // Editing the space-escaped sibling file in `.d` invalidates the stamp.
    fs::write(&sib_spaced, "pub const X: u32 = 9999;\n").unwrap();
    let s_after_sib = compute_rust_stamp_with_cache_dir(
        &cache_dir,
        "cargo",
        &args,
        &app_dir,
        &app_dir,
        "",
        false,
        &[],
        &[],
    )
    .unwrap();
    assert_ne!(s_hit.stamp, s_after_sib.stamp);
    assert!(read_valid_cached_executable(&entry_path, &s_after_sib.stamp).is_none());

    // Deleting a listed `.d` input file forces a cache miss.
    fs::remove_file(&sib_spaced).unwrap();
    let s_after_del = compute_rust_stamp_with_cache_dir(
        &cache_dir,
        "cargo",
        &args,
        &app_dir,
        &app_dir,
        "",
        false,
        &[],
        &[],
    )
    .unwrap();
    assert_ne!(s_hit.stamp, s_after_del.stamp);
    assert!(read_valid_cached_executable(&entry_path, &s_after_del.stamp).is_none());
}

#[test]
fn cargo_lock_and_rustflags_changes_invalidate_stamp() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ws = dir.path().join("ws");
    let cache_dir = dir.path().join("cache");
    let app_dir = ws.join("crates/app");
    fs::create_dir_all(app_dir.join("src")).unwrap();
    fs::write(
        ws.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/app\"]\n",
    )
    .unwrap();
    let lock_path = ws.join("Cargo.lock");
    fs::write(&lock_path, "version = 3\n").unwrap();
    fs::write(
        app_dir.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    fs::write(app_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

    let args = vec!["build".to_string(), "--message-format=json".to_string()];
    let base = compute_rust_stamp_with_cache_dir(
        &cache_dir,
        "cargo",
        &args,
        &app_dir,
        &ws,
        "",
        false,
        &[],
        &[],
    )
    .unwrap();

    // Editing workspace Cargo.lock invalidates stamp.
    fs::write(&lock_path, "version = 3\n# updated lock\n").unwrap();
    let after_lock = compute_rust_stamp_with_cache_dir(
        &cache_dir,
        "cargo",
        &args,
        &app_dir,
        &ws,
        "",
        false,
        &[],
        &[],
    )
    .unwrap();
    assert_ne!(
        base.stamp, after_lock.stamp,
        "Cargo.lock edit must invalidate stamp"
    );

    // Changing RUSTFLAGS invalidates stamp.
    let with_flags = compute_rust_stamp_with_cache_dir(
        &cache_dir,
        "cargo",
        &args,
        &app_dir,
        &ws,
        "",
        false,
        &[("RUSTFLAGS".to_string(), "-C target-cpu=native".to_string())],
        &[],
    )
    .unwrap();
    assert_ne!(
        after_lock.stamp, with_flags.stamp,
        "RUSTFLAGS change must invalidate stamp"
    );
}
