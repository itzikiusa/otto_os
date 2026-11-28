use super::*;
use crate::toolchains::llvm_manifest::LLVM_MANIFEST;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOP: &str = "LLVM-23.1.5-macOS-ARM64";

fn fake_asset(sha256: &'static str, bytes: u64) -> LlvmAsset {
    LlvmAsset {
        sha256,
        bytes,
        ..*LLVM_MANIFEST
            .iter()
            .find(|a| a.top_dir == TOP)
            .expect("macOS arm64 pin")
    }
}

/// A small `.tar.zst` shaped like the real archive: every allowlisted member,
/// plus members that must be skipped.
fn synthetic_archive(extra_symlink: Option<(&str, &str)>) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    let mut file = |path: &str, body: &[u8]| {
        let mut h = tar::Header::new_gnu();
        h.set_size(body.len() as u64);
        h.set_mode(0o755);
        h.set_entry_type(tar::EntryType::Regular);
        h.set_cksum();
        b.append_data(&mut h, format!("{TOP}/{path}"), body)
            .unwrap();
    };
    for p in [
        "bin/clang-23",
        "bin/clangd",
        "bin/clang-format",
        "bin/clang-tidy",
        "bin/clang-apply-replacements",
        "bin/llvm-symbolizer",
        "lib/clang/23/include/stddef.h",
        "bin/lld",
        "bin/lldb",
        "include/c++/v1/vector",
        "lib/liblldb.23.1.5.dylib",
    ] {
        file(p, b"x");
    }
    let mut links = vec![("bin/clang", "clang-23"), ("bin/clang++", "clang")];
    links.extend(extra_symlink);
    for (path, target) in links {
        let mut h = tar::Header::new_gnu();
        h.set_size(0);
        h.set_entry_type(tar::EntryType::Symlink);
        h.set_cksum();
        b.append_link(&mut h, format!("{TOP}/{path}"), target)
            .unwrap();
    }
    let tar_bytes = b.into_inner().unwrap();
    zstd::stream::encode_all(&tar_bytes[..], 3).unwrap()
}

#[test]
fn unpack_writes_only_allowlisted_members_and_keeps_relative_symlinks() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tmp.path().join("a.tar.zst");
    std::fs::write(&archive, synthetic_archive(None)).unwrap();
    let staging = tmp.path().join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let asset = fake_asset("unused", 0);
    unpack_zst(&archive, TOP, &asset.allowlist(), &staging).unwrap();
    let home = staging.join(TOP);
    check_members(&home, &asset).unwrap();
    assert_eq!(
        std::fs::read_link(home.join("bin/clang++")).unwrap(),
        Path::new("clang")
    );
    assert!(home.join("lib/clang/23/include/stddef.h").is_file());
    for skipped in [
        "bin/lld",
        "bin/lldb",
        "include/c++/v1/vector",
        "lib/liblldb.23.1.5.dylib",
    ] {
        assert!(
            !home.join(skipped).exists(),
            "{skipped} must not be unpacked"
        );
    }
}

#[test]
fn unpack_refuses_a_symlink_that_leaves_its_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tmp.path().join("a.tar.zst");
    std::fs::write(
        &archive,
        synthetic_archive(Some(("bin/clangd-evil", "../../../etc/passwd"))),
    )
    .unwrap();
    let mut allow = fake_asset("unused", 0).allowlist();
    allow.push("bin/clangd-evil".into());
    let staging = tmp.path().join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    let err = unpack_zst(&archive, TOP, &allow, &staging).unwrap_err();
    assert!(err.contains("points outside its folder"), "{err}");
}

#[test]
fn a_missing_allowlisted_member_fails_the_install() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join(TOP);
    std::fs::create_dir_all(home.join("bin")).unwrap();
    std::fs::write(home.join("bin/clangd"), b"x").unwrap();
    let err = check_members(&home, &fake_asset("unused", 0)).unwrap_err();
    assert!(err.contains("has no bin/clang"), "{err}");
}

/// BUG-8 / BUG-9: the catalog offers exactly the pinned upstream files (name,
/// size, SHA-256 checked against the GitHub release API on 2026-11-28).
#[test]
fn catalog_offers_only_the_pinned_upstream_files() {
    use crate::toolchains_catalog::{
        clangd_download_url, curated_catalog, resolve_sdk_download, EcosystemKind, TargetArch,
    };
    let mac_arm = TargetPlatform {
        os: TargetOs::Macos,
        arch: TargetArch::Aarch64,
    };
    let cat = curated_catalog(EcosystemKind::Cpp, mac_arm);
    assert_eq!(cat.len(), 1);
    assert_eq!(
        cat[0].url,
        "https://github.com/llvm/llvm-project/releases/download/llvmorg-23.1.5/LLVM-23.1.5-macOS-ARM64.tar.zst"
    );
    assert_eq!(
        cat[0].sha256.as_deref(),
        Some("503de30ef90311e0b143623801b19af2250adac6d69faed65ad0051170202e3b")
    );
    assert_eq!(
        cat[0].size_hint.as_deref(),
        Some("871 MB download · 444 MB on disk")
    );
    assert_eq!(
        resolve_sdk_download(EcosystemKind::Cpp, Some("23.1.5"), mac_arm, &cat).unwrap(),
        cat[0]
    );
    let other =
        resolve_sdk_download(EcosystemKind::Cpp, Some("19.1.7"), mac_arm, &cat).unwrap_err();
    assert!(other.contains("Only LLVM 23.1.5"), "{other}");

    let url = |os, arch| {
        curated_catalog(EcosystemKind::Cpp, TargetPlatform { os, arch })[0]
            .url
            .clone()
    };
    assert!(url(TargetOs::Macos, TargetArch::X86_64)
        .ends_with("/llvmorg-20.1.7/LLVM-20.1.7-macOS-X64.tar.xz"));
    assert!(url(TargetOs::Linux, TargetArch::X86_64)
        .ends_with("/llvmorg-23.1.5/LLVM-23.1.5-Linux-X64.tar.zst"));
    assert!(url(TargetOs::Linux, TargetArch::Aarch64)
        .ends_with("/llvmorg-23.1.5/LLVM-23.1.5-Linux-ARM64.tar.zst"));

    assert_eq!(
        clangd_download_url(mac_arm).unwrap(),
        (
            "https://github.com/clangd/clangd/releases/download/23.1.0/clangd-mac-23.1.0.zip"
                .to_string(),
            "1082e6638223b785ca2daf0939f13afcd0bb95c84ee9a4bbaff4745365159253"
        )
    );
    let lin = TargetPlatform {
        os: TargetOs::Linux,
        arch: TargetArch::X86_64,
    };
    assert_eq!(
        clangd_download_url(lin).unwrap().1,
        "e53b1a96196095faedb7642cf64964f7fb9ad4a0c1f00dd2c172a3d9dcbafdfd"
    );
}

#[test]
fn the_selected_toolchain_supplies_clangd_unless_it_lives_in_the_workspace() {
    use crate::toolchains::discovery::cpp_toolchain_clangd;
    let tmp = tempfile::tempdir().unwrap();
    let tc = tmp.path().join("llvm");
    std::fs::create_dir_all(tc.join("bin")).unwrap();
    let clangd = tc.join("bin/clangd");
    std::fs::write(&clangd, "#!/bin/sh\n").unwrap();
    super::super::installer::make_executable(&clangd).unwrap();
    let tc_s = tc.to_string_lossy().into_owned();
    let elsewhere = tmp.path().join("ws");
    assert_eq!(
        cpp_toolchain_clangd(&tc_s, Some(&elsewhere)),
        Some(clangd.clone())
    );
    assert_eq!(
        cpp_toolchain_clangd(&format!("{tc_s}/bin"), None),
        Some(clangd)
    );
    assert_eq!(
        cpp_toolchain_clangd(&tc_s, Some(tmp.path())),
        None,
        "inside the workspace"
    );
    assert_eq!(cpp_toolchain_clangd("relative/llvm", None), None);
    let bare = tmp.path().join("gcc");
    std::fs::create_dir_all(bare.join("bin")).unwrap();
    assert_eq!(
        cpp_toolchain_clangd(&bare.to_string_lossy(), None),
        None,
        "no clangd"
    );
}

#[test]
fn allowlist_matches_exact_names_subtrees_and_prefixes() {
    let allow: Vec<String> = ["bin/clang", "lib/clang/23/", "lib/liblldb.so*"]
        .map(String::from)
        .to_vec();
    assert!(is_allowed(&allow, "bin/clang"));
    assert!(!is_allowed(&allow, "bin/clang-tblgen"));
    assert!(is_allowed(&allow, "lib/clang/23"));
    assert!(is_allowed(&allow, "lib/clang/23/include/stddef.h"));
    assert!(!is_allowed(&allow, "lib/clang/230/x"));
    assert!(is_allowed(&allow, "lib/liblldb.so.23.1"));
    assert!(!is_allowed(&allow, "lib/libLLVM.so"));
}

#[test]
fn df_free_space_is_read_from_the_posix_output() {
    let out = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk3s5 971350180 918000000 22020096 98% /System/Volumes/Data\n";
    assert_eq!(parse_df_free_bytes(out), Some(22_020_096 * 1024));
    assert_eq!(parse_df_free_bytes("garbage"), None);
}

/// Serve `body` on loopback, with the byte at `flip` inverted when given.
async fn serve(
    body: std::sync::Arc<Vec<u8>>,
    flip: Option<usize>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let h = tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf).await;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(head.as_bytes()).await;
            match flip {
                Some(i) => {
                    let _ = s.write_all(&body[..i]).await;
                    let _ = s.write_all(&[body[i] ^ 0x01]).await;
                    let _ = s.write_all(&body[i + 1..]).await;
                }
                None => {
                    let _ = s.write_all(&body).await;
                }
            }
        }
    });
    (format!("http://{addr}/{TOP}.tar.zst"), h)
}

#[tokio::test]
async fn an_install_removes_staging_an_interrupted_install_left() {
    // SEC6-7: a daemon quit mid-install leaves cpp/.staging-<job>; nothing reads it.
    let tmp = tempfile::tempdir().unwrap();
    let mgr = ToolchainManager::new(tmp.path().join("data"));
    let root = tmp.path().join("toolchains");
    let stale = root.join("cpp/.staging-tcjob_gone/LLVM-23.1.5-macOS-ARM64/bin");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("clang"), "x").unwrap();
    let entry = CatalogSdkEntry {
        version: "0.0.0".into(),
        label: "LLVM 0.0.0".into(),
        lts: false,
        os: String::new(),
        arch: String::new(),
        url: String::new(),
        size_hint: None,
        sha256: None,
    };
    // Any outcome: the stale staging goes first.
    let _ = install_llvm_local(&mgr, "tcjob_now", &root, &entry).await;
    assert!(!root.join("cpp/.staging-tcjob_gone").exists());
    assert!(root.join("cpp").is_dir(), "only the staging folder goes");
}

#[tokio::test]
async fn a_tampered_archive_is_refused_and_nothing_is_installed() {
    let good = synthetic_archive(None);
    let sha: &'static str = Box::leak(hex::encode(Sha256::digest(&good)).into_boxed_str());
    let mid = good.len() / 2;
    let (url, server) = serve(std::sync::Arc::new(good.clone()), Some(mid)).await;

    let tmp = tempfile::tempdir().unwrap();
    let mgr = ToolchainManager::new(tmp.path().join("data"));
    mgr.allow_test_loopback
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let root = mgr.toolchains_dir();
    let asset = fake_asset(sha, good.len() as u64);
    let archive = root.join(".downloads").join(asset.asset);
    let staging = root.join("cpp/.staging-job");
    let err = install_steps(&mgr, "job", &root, &asset, &url, &archive, &staging, None)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        "The downloaded LLVM archive does not match the expected file (SHA-256 mismatch). Nothing was installed."
    );
    assert!(!archive.exists(), "the bad archive is deleted");
    assert!(!root.join("cpp").exists(), "no staging, no SDK");
    server.abort();
}

#[tokio::test]
async fn a_larger_archive_than_pinned_is_refused() {
    let good = synthetic_archive(None);
    let sha: &'static str = Box::leak(hex::encode(Sha256::digest(&good)).into_boxed_str());
    let (url, server) = serve(std::sync::Arc::new(good.clone()), None).await;
    let tmp = tempfile::tempdir().unwrap();
    let mgr = ToolchainManager::new(tmp.path().join("data"));
    mgr.allow_test_loopback
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let root = mgr.toolchains_dir();
    let asset = fake_asset(sha, good.len() as u64 - 1);
    let archive = root.join(".downloads").join(asset.asset);
    let err = install_steps(
        &mgr,
        "job",
        &root,
        &asset,
        &url,
        &archive,
        &root.join("cpp/.staging-job"),
        None,
    )
    .await
    .unwrap_err();
    assert!(err.contains("(size mismatch)"), "{err}");
    server.abort();
}

/// Live, by hand: `OTTO_LLVM_ARCHIVE=<path to the real pinned archive for this
/// host>` serves it from loopback, first with one byte flipped (refused), then
/// intact (installed, self-check passed). No network needed.
#[tokio::test]
#[ignore = "needs the real ~0.9 GB LLVM archive in OTTO_LLVM_ARCHIVE"]
async fn live_real_archive_refuses_a_flipped_byte_then_installs() {
    let path = std::env::var("OTTO_LLVM_ARCHIVE").expect("OTTO_LLVM_ARCHIVE");
    let real = std::sync::Arc::new(tokio::fs::read(&path).await.unwrap());
    let asset = llvm_for(TargetPlatform::host()).expect("pinned host");
    assert_eq!(real.len() as u64, asset.bytes);
    let tmp = tempfile::tempdir().unwrap();
    let mgr = ToolchainManager::new(tmp.path().join("data"));
    mgr.allow_test_loopback
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let root = mgr.toolchains_dir();
    let entry = |url: String| CatalogSdkEntry {
        version: asset.version.into(),
        label: String::new(),
        lts: true,
        os: String::new(),
        arch: String::new(),
        url,
        size_hint: None,
        sha256: None,
    };

    let (url, server) = serve(real.clone(), Some(real.len() / 2)).await;
    let err = install_llvm_local(&mgr, "tamper", &root, &entry(url))
        .await
        .unwrap_err();
    eprintln!("tampered: {err}");
    assert!(err.contains("SHA-256 mismatch"), "{err}");
    assert!(!root.join("cpp/sdks").exists());
    server.abort();

    let (url, server) = serve(real, None).await;
    let t0 = std::time::Instant::now();
    let dest = install_llvm_local(&mgr, "ok", &root, &entry(url))
        .await
        .unwrap();
    eprintln!("installed {dest} in {:?}", t0.elapsed());
    assert!(Path::new(&dest).join("bin/clangd").is_file());
    server.abort();
}
