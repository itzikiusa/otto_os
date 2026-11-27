//! Local LSP and DAP binary installation routines for [`super::ToolchainManager`].

use std::path::{Path, PathBuf};

use crate::toolchains_catalog::{
    clangd_download_url, codelldb_vsix_url, csharp_ls_version_for_dotnet_sdk,
    dotnet_sdk_major_slot, jq_binary_url, kotlin_lsp_zip_url, netcoredbg_download_asset,
    rust_analyzer_gz_url, uv_tarball_url, EcosystemKind, TargetPlatform, BASH_LSP_NPM_PACKAGES,
    CSHARP_LS_TOOL, DEBUGPY_PKG, DLV_PKG, FSAUTOCOMPLETE_TOOL, FSAUTOCOMPLETE_VERSION, GOPLS_PKG,
    JAVA_DEBUG_JAR_NAME, JAVA_DEBUG_JAR_URL, JDTLS_TARBALL_URL, JSON_LSP_NPM_PACKAGES,
    JS_DEBUG_TARBALL_URL, PYRIGHT_PKG, TS_LSP_NPM_PACKAGES,
};

use super::installer::{
    download_https_to_file, ensure_local_uv, make_executable, relink_current, resolve_best_sdk_bin,
    run_local_cmd,
};
use super::{
    find_executable_in, is_executable_file, system_search_dirs_with_shell, ToolchainManager,
};

/// Job-scoped staging directory so concurrent ZLS installs never share archive or
/// extraction files (CONC-3).
pub(super) fn zls_stage_dir(tmp: &Path, job_id: &str) -> PathBuf {
    tmp.join(format!("zls-stage-{job_id}"))
}

/// Job-scoped staging directory so concurrent CodeLLDB installs (Rust/Zig/C++) never
/// share archive or extraction files (CONC-2).
pub(crate) fn codelldb_stage_dir(tmp: &Path, job_id: &str) -> PathBuf {
    tmp.join(format!("codelldb-stage-{job_id}"))
}

/// `<go_root>/bin/go` when the caller named the project's GOROOT and it holds a Go.
fn project_go_bin(go_root: Option<&Path>) -> Option<PathBuf> {
    go_root
        .map(|r| r.join("bin/go"))
        .filter(|b| super::discovery::is_executable_file(b))
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn install_local_lsp(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    eco: EcosystemKind,
    platform: TargetPlatform,
    go_root: Option<&Path>,
    zig_home: Option<&Path>,
    dotnet_root: Option<&Path>,
) -> std::result::Result<String, String> {
    let tmp = root.join(".downloads");
    tokio::fs::create_dir_all(&tmp)
        .await
        .map_err(|e| e.to_string())?;
    match eco {
        EcosystemKind::Java => {
            let archive = tmp.join("jdtls.tar.gz");
            download_https_to_file(mgr, job_id, JDTLS_TARBALL_URL, &archive, None, 65).await?;
            let dest = root.join("java/lsp");
            let _ = tokio::fs::remove_dir_all(&dest).await;
            tokio::fs::create_dir_all(&dest)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                75,
                Path::new("tar"),
                &[
                    "-xzf",
                    &archive.to_string_lossy(),
                    "-C",
                    &dest.to_string_lossy(),
                ],
                &[],
            )
            .await?;
            let _ = tokio::fs::remove_file(&archive).await;
            let bin = dest.join("bin/jdtls");
            make_executable(&bin)?;
            Ok(bin.to_string_lossy().into_owned())
        }
        EcosystemKind::Go => {
            let go_bin = project_go_bin(go_root)
                .or_else(|| resolve_best_sdk_bin(mgr, root, EcosystemKind::Go, "go"))
                .ok_or("Go SDK required first")?;
            let (gobin, gopath) = (root.join("go/bin"), root.join("go/gopath"));
            tokio::fs::create_dir_all(&gobin)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                75,
                &go_bin,
                &["install", GOPLS_PKG],
                &[("GOBIN", &gobin), ("GOPATH", &gopath)],
            )
            .await?;
            Ok(gobin.join("gopls").to_string_lossy().into_owned())
        }
        EcosystemKind::Rust => {
            let gz = tmp.join("rust-analyzer.gz");
            download_https_to_file(mgr, job_id, &rust_analyzer_gz_url(platform), &gz, None, 65)
                .await?;
            let bin_dir = root.join("rust/bin");
            tokio::fs::create_dir_all(&bin_dir)
                .await
                .map_err(|e| e.to_string())?;
            let dest = bin_dir.join("rust-analyzer");
            let _ = tokio::fs::remove_file(&dest).await;
            run_local_cmd(
                mgr,
                job_id,
                75,
                Path::new("gzip"),
                &["-d", "-f", &gz.to_string_lossy()],
                &[],
            )
            .await?;
            tokio::fs::rename(tmp.join("rust-analyzer"), &dest)
                .await
                .map_err(|e| e.to_string())?;
            make_executable(&dest)?;
            Ok(dest.to_string_lossy().into_owned())
        }
        EcosystemKind::Zig => {
            install_local_zig_lsp(mgr, job_id, root, &tmp, platform, zig_home).await
        }
        EcosystemKind::Cpp => install_local_cpp_lsp(mgr, job_id, root, &tmp, platform).await,
        EcosystemKind::Kotlin => install_local_kotlin_lsp(mgr, job_id, root, &tmp).await,
        EcosystemKind::Dotnet => install_local_dotnet_lsp(mgr, job_id, root, dotnet_root).await,
        EcosystemKind::Node => {
            let npm = resolve_best_sdk_bin(mgr, root, EcosystemKind::Node, "npm")
                .ok_or("Node.js/npm required first")?;
            let prefix = root.join("node/lsp");
            tokio::fs::create_dir_all(&prefix)
                .await
                .map_err(|e| e.to_string())?;
            let mut args = vec!["install", "--prefix", prefix.to_str().unwrap_or(".")];
            args.extend_from_slice(TS_LSP_NPM_PACKAGES);
            run_local_cmd(mgr, job_id, 75, &npm, &args, &[]).await?;
            Ok(prefix
                .join("node_modules/.bin/typescript-language-server")
                .to_string_lossy()
                .into_owned())
        }
        EcosystemKind::Python => {
            let uv_bin =
                ensure_local_uv(mgr, job_id, root, &uv_tarball_url(platform), None).await?;
            let (tool_dir, bin_dir) = (root.join("python/tools"), root.join("python/bin"));
            tokio::fs::create_dir_all(&bin_dir)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                75,
                &uv_bin,
                &["tool", "install", "--force", PYRIGHT_PKG],
                &[("UV_TOOL_DIR", &tool_dir), ("UV_TOOL_BIN_DIR", &bin_dir)],
            )
            .await?;
            Ok(bin_dir
                .join("pyright-langserver")
                .to_string_lossy()
                .into_owned())
        }
        EcosystemKind::Shell => {
            let npm = resolve_best_sdk_bin(mgr, root, EcosystemKind::Node, "npm")
                .ok_or("Node.js/npm required to install bash-language-server")?;
            let prefix = root.join("shell/lsp");
            tokio::fs::create_dir_all(&prefix)
                .await
                .map_err(|e| e.to_string())?;
            let mut args = vec!["install", "--prefix", prefix.to_str().unwrap_or(".")];
            args.extend_from_slice(BASH_LSP_NPM_PACKAGES);
            run_local_cmd(mgr, job_id, 75, &npm, &args, &[]).await?;
            Ok(prefix
                .join("node_modules/.bin/bash-language-server")
                .to_string_lossy()
                .into_owned())
        }
        EcosystemKind::Json => {
            let npm = resolve_best_sdk_bin(mgr, root, EcosystemKind::Node, "npm")
                .ok_or("Node.js/npm required to install vscode-json-language-server")?;
            let prefix = root.join("json/lsp");
            tokio::fs::create_dir_all(&prefix)
                .await
                .map_err(|e| e.to_string())?;
            let mut args = vec!["install", "--prefix", prefix.to_str().unwrap_or(".")];
            args.extend_from_slice(JSON_LSP_NPM_PACKAGES);
            run_local_cmd(mgr, job_id, 75, &npm, &args, &[]).await?;
            Ok(prefix
                .join("node_modules/.bin/vscode-json-language-server")
                .to_string_lossy()
                .into_owned())
        }
        EcosystemKind::Ios => Err(
            "sourcekit-lsp ships with Xcode / Command Line Tools (`xcode-select --install`)."
                .into(),
        ),
        EcosystemKind::Android => {
            Err("Install Kotlin Language Server via `brew install kotlin-language-server`.".into())
        }
        EcosystemKind::Flutter => Err(
            "The Dart language server ships inside the Flutter SDK (`flutter/bin/dart`).".into(),
        ),
    }
}

async fn install_local_zig_lsp(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    tmp: &Path,
    platform: TargetPlatform,
    zig_home: Option<&Path>,
) -> std::result::Result<String, String> {
    // Require verified ZLS release metadata + SHA-256 digest matching the active
    // Zig version; never fall back to an unverified download (SEC-1).
    let zig_ver = zig_home
        .and_then(super::discovery::zig_bin_in_home)
        .and_then(|b| super::discovery::read_zig_bin_version(&b))
        .or_else(|| {
            find_executable_in(
                &system_search_dirs_with_shell(mgr.shell_path().as_deref()),
                &["zig"],
            )
            .and_then(|b| super::discovery::read_zig_bin_version(&b))
        });
    let lookup_ver = zig_ver
        .as_deref()
        .unwrap_or_else(|| EcosystemKind::Zig.default_sdk_version());
    let rel = mgr
        .catalog
        .fetch_zls_release(lookup_ver, platform)
        .await
        .ok_or_else(|| {
            format!(
                "Could not fetch verified ZLS release checksum for Zig {lookup_ver} ({}); retry when releases.zigtools.org is reachable",
                platform.zig_index_key()
            )
        })?;
    let zls_ver = rel.version;
    let minor_slot = super::discovery::zig_major_minor(&zls_ver)
        .map(|(maj, min)| format!("{maj}.{min}"))
        .unwrap_or_else(|| zls_ver.clone());
    // Job-scoped staging prevents concurrent LSP-only and Full Setup jobs from
    // clobbering each other's archive or extraction directory (CONC-3).
    let stage_dir = zls_stage_dir(tmp, job_id);
    let _ = tokio::fs::remove_dir_all(&stage_dir).await;
    let extract_dir = stage_dir.join("extract");
    tokio::fs::create_dir_all(&extract_dir)
        .await
        .map_err(|e| e.to_string())?;
    let archive = stage_dir.join(format!("zls-{zls_ver}.tar.xz"));
    let staged_res = async {
        download_https_to_file(
            mgr,
            job_id,
            &rel.url,
            &archive,
            Some(rel.sha256.as_str()),
            65,
        )
        .await?;
        run_local_cmd(
            mgr,
            job_id,
            75,
            Path::new("tar"),
            &[
                "-xJf",
                &archive.to_string_lossy(),
                "-C",
                &extract_dir.to_string_lossy(),
            ],
            &[],
        )
        .await?;
        let src_bin = [extract_dir.join("zls"), extract_dir.join("bin/zls")]
            .into_iter()
            .find(|p| p.is_file())
            .ok_or("ZLS archive missing zls binary")?;
        // Store per Zig major.minor so multiple projects pinned to different Zig
        // versions keep their own compatible ZLS (BUG-4).
        let version_dir = root.join("zig/zls").join(&minor_slot);
        tokio::fs::create_dir_all(&version_dir)
            .await
            .map_err(|e| e.to_string())?;
        let tmp_dest = version_dir.join(format!(".zls.{job_id}.tmp"));
        let dest = version_dir.join("zls");
        let _ = tokio::fs::remove_file(&tmp_dest).await;
        tokio::fs::rename(&src_bin, &tmp_dest)
            .await
            .map_err(|e| e.to_string())?;
        make_executable(&tmp_dest)?;
        tokio::fs::rename(&tmp_dest, &dest)
            .await
            .map_err(|e| e.to_string())?;
        let bin_dir = root.join("zig/bin");
        tokio::fs::create_dir_all(&bin_dir)
            .await
            .map_err(|e| e.to_string())?;
        relink_current(&bin_dir.join("zls"), &dest);
        Ok::<String, String>(dest.to_string_lossy().into_owned())
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&stage_dir).await;
    staged_res
}

async fn install_local_cpp_lsp(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    tmp: &Path,
    platform: TargetPlatform,
) -> std::result::Result<String, String> {
    let url = clangd_download_url(platform)?;
    let stage_dir = tmp.join(format!("clangd-stage-{job_id}"));
    let _ = tokio::fs::remove_dir_all(&stage_dir).await;
    let extract_dir = stage_dir.join("extract");
    tokio::fs::create_dir_all(&extract_dir)
        .await
        .map_err(|e| e.to_string())?;
    let zip = stage_dir.join("clangd.zip");
    let staged_res = async {
        download_https_to_file(mgr, job_id, &url, &zip, None, 65).await?;
        run_local_cmd(
            mgr,
            job_id,
            75,
            Path::new("unzip"),
            &[
                "-q",
                "-o",
                &zip.to_string_lossy(),
                "-d",
                &extract_dir.to_string_lossy(),
            ],
            &[],
        )
        .await?;
        let src_tree = super::read_subdirs(&extract_dir)
            .into_iter()
            .find(|d| d.join("bin/clangd").is_file())
            .or_else(|| {
                extract_dir
                    .join("bin/clangd")
                    .is_file()
                    .then(|| extract_dir.clone())
            })
            .ok_or("clangd archive missing bin/clangd")?;
        // Preserve the full `clangd` tree (`bin/clangd` + `lib/clang/...` builtin
        // headers) under `cpp/lsp` and symlink `cpp/bin/clangd`.
        let lsp_dir = root.join("cpp/lsp");
        let bin_dir = root.join("cpp/bin");
        tokio::fs::create_dir_all(&bin_dir)
            .await
            .map_err(|e| e.to_string())?;
        let _ = tokio::fs::remove_dir_all(&lsp_dir).await;
        tokio::fs::rename(&src_tree, &lsp_dir)
            .await
            .map_err(|e| e.to_string())?;
        let real_bin = lsp_dir.join("bin/clangd");
        make_executable(&real_bin)?;
        let link = bin_dir.join("clangd");
        relink_current(&link, &real_bin);
        Ok::<String, String>(link.to_string_lossy().into_owned())
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&stage_dir).await;
    staged_res
}

async fn install_local_kotlin_lsp(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    tmp: &Path,
) -> std::result::Result<String, String> {
    let zip = tmp.join("kotlin-language-server.zip");
    download_https_to_file(mgr, job_id, &kotlin_lsp_zip_url(), &zip, None, 65).await?;
    let dest = root.join("kotlin/lsp");
    let _ = tokio::fs::remove_dir_all(&dest).await;
    tokio::fs::create_dir_all(&dest)
        .await
        .map_err(|e| e.to_string())?;
    run_local_cmd(
        mgr,
        job_id,
        75,
        Path::new("unzip"),
        &[
            "-q",
            "-o",
            &zip.to_string_lossy(),
            "-d",
            &dest.to_string_lossy(),
        ],
        &[],
    )
    .await?;
    let _ = tokio::fs::remove_file(&zip).await;
    let bin = [
        dest.join("server/bin/kotlin-language-server"),
        dest.join("bin/kotlin-language-server"),
    ]
    .into_iter()
    .find(|p| p.is_file())
    .ok_or("kotlin-language-server archive missing bin/kotlin-language-server")?;
    make_executable(&bin)?;
    let bin_dir = root.join("kotlin/bin");
    tokio::fs::create_dir_all(&bin_dir)
        .await
        .map_err(|e| e.to_string())?;
    relink_current(&bin_dir.join("kotlin-language-server"), &bin);
    Ok(bin.to_string_lossy().into_owned())
}

async fn install_local_dotnet_lsp(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    dotnet_root: Option<&Path>,
) -> std::result::Result<String, String> {
    let dotnet_bin = dotnet_root
        .and_then(super::discovery::dotnet_bin_in_root)
        .or_else(|| resolve_best_sdk_bin(mgr, root, EcosystemKind::Dotnet, "dotnet"))
        .ok_or(".NET SDK required first")?;
    let tools_dir = root.join("dotnet/tools");
    tokio::fs::create_dir_all(&tools_dir)
        .await
        .map_err(|e| e.to_string())?;
    let mut env_pairs: Vec<(&str, &Path)> = Vec::new();
    let resolved_root = super::discovery::dotnet_root_from_bin(&dotnet_bin);
    if let Some(ref dr) = resolved_root {
        env_pairs.push(("DOTNET_ROOT", dr.as_path()));
    }
    let sdk_ver = super::discovery::read_dotnet_bin_version(&dotnet_bin).or_else(|| {
        resolved_root
            .as_deref()
            .map(super::discovery::dotnet_dir_version)
    });
    let csharp_ver = csharp_ls_version_for_dotnet_sdk(sdk_ver.as_deref());
    let csharp_slot = dotnet_sdk_major_slot(sdk_ver.as_deref());
    let safe_job = job_id.replace(|c: char| !c.is_ascii_alphanumeric(), "_");

    // BUG-22 + DATA2-1: Install each tool into a fresh staging directory first and swap
    // into its per-SDK-major slot (`dotnet/tools/<slot>/csharp-ls`) or package directory
    // only after `dotnet tool install` succeeds. That way:
    //  1) An offline / failed upgrade NEVER deletes a working install.
    //  2) Projects pinned to .NET 8 and .NET 9 coexist without clobbering each other.
    for (tool, version, slot_name, progress) in [
        (CSHARP_LS_TOOL, csharp_ver, csharp_slot, 70u8),
        (
            FSAUTOCOMPLETE_TOOL,
            FSAUTOCOMPLETE_VERSION,
            "fsautocomplete-pkg",
            78u8,
        ),
    ] {
        let stage_dir = tools_dir.join(format!(".stage-{safe_job}-{slot_name}"));
        let _ = tokio::fs::remove_dir_all(&stage_dir).await;
        tokio::fs::create_dir_all(&stage_dir)
            .await
            .map_err(|e| e.to_string())?;
        let stage_str = stage_dir.to_string_lossy().into_owned();
        let res = run_local_cmd(
            mgr,
            job_id,
            progress,
            &dotnet_bin,
            &[
                "tool",
                "install",
                "--tool-path",
                &stage_str,
                "--version",
                version,
                tool,
            ],
            &env_pairs,
        )
        .await;
        if let Err(err) = res {
            let _ = tokio::fs::remove_dir_all(&stage_dir).await;
            return Err(err);
        }
        let slot_dir = tools_dir.join(slot_name);
        let backup_dir = tools_dir.join(format!(".old-{safe_job}-{slot_name}"));
        let _ = tokio::fs::remove_dir_all(&backup_dir).await;
        let had_prev = slot_dir.exists();
        if had_prev {
            tokio::fs::rename(&slot_dir, &backup_dir)
                .await
                .map_err(|e| format!("swap backup {}: {e}", slot_dir.display()))?;
        }
        if let Err(e) = tokio::fs::rename(&stage_dir, &slot_dir).await {
            if had_prev {
                let _ = tokio::fs::rename(&backup_dir, &slot_dir).await;
            }
            let _ = tokio::fs::remove_dir_all(&stage_dir).await;
            return Err(format!("activate {}: {e}", slot_dir.display()));
        }
        if had_prev {
            let _ = tokio::fs::remove_dir_all(&backup_dir).await;
        }
        relink_current(&tools_dir.join(tool), &slot_dir.join(tool));
    }
    Ok(tools_dir
        .join(csharp_slot)
        .join(CSHARP_LS_TOOL)
        .to_string_lossy()
        .into_owned())
}

pub(super) async fn install_local_dap(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    eco: EcosystemKind,
    platform: TargetPlatform,
    go_root: Option<&Path>,
) -> std::result::Result<String, String> {
    let tmp = root.join(".downloads");
    tokio::fs::create_dir_all(&tmp)
        .await
        .map_err(|e| e.to_string())?;
    match eco {
        EcosystemKind::Java | EcosystemKind::Kotlin => {
            let jar = root.join("java/dap").join(JAVA_DEBUG_JAR_NAME);
            download_https_to_file(mgr, job_id, JAVA_DEBUG_JAR_URL, &jar, None, 85).await?;
            Ok(jar.to_string_lossy().into_owned())
        }
        EcosystemKind::Go => {
            let go_bin = project_go_bin(go_root)
                .or_else(|| resolve_best_sdk_bin(mgr, root, EcosystemKind::Go, "go"))
                .ok_or("Go SDK required first")?;
            let (gobin, gopath) = (root.join("go/bin"), root.join("go/gopath"));
            tokio::fs::create_dir_all(&gobin)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                92,
                &go_bin,
                &["install", DLV_PKG],
                &[("GOBIN", &gobin), ("GOPATH", &gopath)],
            )
            .await?;
            Ok(gobin.join("dlv").to_string_lossy().into_owned())
        }
        EcosystemKind::Rust | EcosystemKind::Zig | EcosystemKind::Cpp => {
            // Serialize shared CodeLLDB installs across Rust, Zig, and C++ jobs and use
            // job-scoped staging paths so concurrent jobs never clobber each other (CONC-2).
            static CODELLDB_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> =
                std::sync::OnceLock::new();
            let _guard = CODELLDB_LOCK
                .get_or_init(|| tokio::sync::Mutex::new(()))
                .lock()
                .await;
            let stage_dir = codelldb_stage_dir(&tmp, job_id);
            let _ = tokio::fs::remove_dir_all(&stage_dir).await;
            let extract_dir = stage_dir.join("extract");
            tokio::fs::create_dir_all(&extract_dir)
                .await
                .map_err(|e| e.to_string())?;
            let vsix = stage_dir.join("codelldb.vsix");
            let staged_res = async {
                download_https_to_file(mgr, job_id, &codelldb_vsix_url(platform), &vsix, None, 85)
                    .await?;
                run_local_cmd(
                    mgr,
                    job_id,
                    94,
                    Path::new("unzip"),
                    &[
                        "-q",
                        "-o",
                        &vsix.to_string_lossy(),
                        "-d",
                        &extract_dir.to_string_lossy(),
                    ],
                    &[],
                )
                .await?;
                let staged_adapter = extract_dir.join("extension/adapter/codelldb");
                make_executable(&staged_adapter)?;
                promote_staged_codelldb_dir(root, &extract_dir).await
            }
            .await;
            let _ = tokio::fs::remove_dir_all(&stage_dir).await;
            staged_res
        }
        EcosystemKind::Dotnet => {
            let asset = netcoredbg_download_asset(platform)?;
            let archive = tmp.join(if asset.is_zip {
                "netcoredbg.zip"
            } else {
                "netcoredbg.tar.gz"
            });
            download_https_to_file(mgr, job_id, &asset.url, &archive, None, 85).await?;
            let dest = root.join("dotnet/dap");
            let _ = tokio::fs::remove_dir_all(&dest).await;
            tokio::fs::create_dir_all(&dest)
                .await
                .map_err(|e| e.to_string())?;
            if asset.is_zip {
                run_local_cmd(
                    mgr,
                    job_id,
                    94,
                    Path::new("unzip"),
                    &[
                        "-q",
                        "-o",
                        &archive.to_string_lossy(),
                        "-d",
                        &dest.to_string_lossy(),
                    ],
                    &[],
                )
                .await?;
            } else {
                run_local_cmd(
                    mgr,
                    job_id,
                    94,
                    Path::new("tar"),
                    &[
                        "-xzf",
                        &archive.to_string_lossy(),
                        "-C",
                        &dest.to_string_lossy(),
                    ],
                    &[],
                )
                .await?;
            }
            let _ = tokio::fs::remove_file(&archive).await;
            let adapter = [dest.join("netcoredbg/netcoredbg"), dest.join("netcoredbg")]
                .into_iter()
                .find(|p| p.is_file())
                .ok_or("netcoredbg archive missing netcoredbg binary")?;
            make_executable(&adapter)?;
            if let Some(parent) = adapter.parent() {
                relink_current(&root.join("dotnet/netcoredbg"), parent);
            }
            Ok(adapter.to_string_lossy().into_owned())
        }
        EcosystemKind::Node => {
            let archive = tmp.join("js-debug.tar.gz");
            download_https_to_file(mgr, job_id, JS_DEBUG_TARBALL_URL, &archive, None, 85).await?;
            let dest = root.join("node/dap");
            let _ = tokio::fs::remove_dir_all(&dest).await;
            tokio::fs::create_dir_all(&dest)
                .await
                .map_err(|e| e.to_string())?;
            run_local_cmd(
                mgr,
                job_id,
                94,
                Path::new("tar"),
                &[
                    "-xzf",
                    &archive.to_string_lossy(),
                    "-C",
                    &dest.to_string_lossy(),
                ],
                &[],
            )
            .await?;
            let _ = tokio::fs::remove_file(&archive).await;
            relink_current(&root.join("node/js-debug"), &dest.join("js-debug"));
            Ok(dest
                .join("js-debug/src/dapDebugServer.js")
                .to_string_lossy()
                .into_owned())
        }
        EcosystemKind::Python => {
            let uv_bin =
                ensure_local_uv(mgr, job_id, root, &uv_tarball_url(platform), None).await?;
            let dap_venv = root.join("python/dap");
            run_local_cmd(
                mgr,
                job_id,
                90,
                &uv_bin,
                &["venv", "--clear", &dap_venv.to_string_lossy()],
                &[],
            )
            .await?;
            let venv_py = dap_venv.join("bin/python");
            run_local_cmd(
                mgr,
                job_id,
                95,
                &uv_bin,
                &[
                    "pip",
                    "install",
                    "--python",
                    &venv_py.to_string_lossy(),
                    DEBUGPY_PKG,
                ],
                &[],
            )
            .await?;
            Ok(venv_py.to_string_lossy().into_owned())
        }
        EcosystemKind::Shell => {
            let sp = mgr.shell_path();
            if let Some(sys_bashdb) =
                find_executable_in(&system_search_dirs_with_shell(sp.as_deref()), &["bashdb"])
            {
                return Ok(sys_bashdb.to_string_lossy().into_owned());
            }
            let bin_dir = root.join("shell/bin");
            tokio::fs::create_dir_all(&bin_dir)
                .await
                .map_err(|e| e.to_string())?;
            let wrapper = bin_dir.join("bashdb");
            tokio::fs::write(
                &wrapper,
                "#!/usr/bin/env bash\nexport PS4='+ ${BASH_SOURCE}:${LINENO}: '\nexec bash -x \"$@\"\n",
            )
            .await
            .map_err(|e| e.to_string())?;
            make_executable(&wrapper)?;
            Ok(wrapper.to_string_lossy().into_owned())
        }
        EcosystemKind::Json => {
            let bin_dir = root.join("json/bin");
            tokio::fs::create_dir_all(&bin_dir)
                .await
                .map_err(|e| e.to_string())?;
            let jq = bin_dir.join("jq");
            if !is_executable_file(&jq) {
                download_https_to_file(
                    mgr,
                    job_id,
                    &jq_binary_url("1.7.1", platform),
                    &jq,
                    None,
                    85,
                )
                .await?;
                make_executable(&jq)?;
            }
            Ok(jq.to_string_lossy().into_owned())
        }
        EcosystemKind::Ios => Err("lldb-dap ships with Xcode / Command Line Tools.".into()),
        EcosystemKind::Android => Err("adb ships with Android SDK platform-tools.".into()),
        EcosystemKind::Flutter => Err("Flutter debugging ships inside the Flutter SDK.".into()),
    }
}

/// Process-wide mutex serializing promotion into the shared `<toolchains>/rust/dap`
/// CodeLLDB directory so concurrent Rust / Zig / C++ DAP installs never race on
/// `remove_dir_all` + `rename` (CONC-2).
static CODELLDB_PROMOTE_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> =
    std::sync::OnceLock::new();

pub(crate) async fn promote_staged_codelldb_dir(
    root: &Path,
    extract_dir: &Path,
) -> std::result::Result<String, String> {
    let lock = CODELLDB_PROMOTE_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
    let _guard = lock.lock().await;
    let dest = root.join("rust/dap");
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }
    let _ = tokio::fs::remove_dir_all(&dest).await;
    tokio::fs::rename(extract_dir, &dest)
        .await
        .map_err(|e| e.to_string())?;
    let adapter = dest.join("extension/adapter/codelldb");
    relink_current(&root.join("rust/codelldb"), &dest.join("extension"));
    Ok(adapter.to_string_lossy().into_owned())
}

#[cfg(test)]
mod dotnet_installer_tests {
    use super::*;
    use crate::toolchains_catalog::{TargetArch, TargetOs};

    #[tokio::test]
    async fn local_dotnet_lsp_pins_csharp_ls_for_sdk8_and_installs_fsautocomplete_and_dap_rejects_mac_x64(
    ) {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let tc_root = data_dir.join("toolchains");
        let fake_sdk_8 = tmp.path().join("dotnet-sdk-8");
        let fake_sdk_9 = tmp.path().join("dotnet-sdk-9");
        let fake_sdk_fail = tmp.path().join("dotnet-sdk-fail");
        let log_file = tmp.path().join("dotnet-invocations.log");
        for (dir, ver, fail_install) in [
            (&fake_sdk_8, "8.0.408", false),
            (&fake_sdk_9, "9.0.200", false),
            (&fake_sdk_fail, "8.0.408", true),
        ] {
            std::fs::create_dir_all(dir).unwrap();
            let fake_dotnet = dir.join("dotnet");
            let install_body = if fail_install {
                "exit 1".to_string()
            } else {
                // Materialize the installed tool shim inside `--tool-path <dir>` so
                // slot activation and symlink preservation can be verified on disk.
                format!(
                    "TP=''; PREV=''; for a in \"$@\"; do \
                       if [ \"$PREV\" = '--tool-path' ]; then TP=\"$a\"; fi; \
                       PREV=\"$a\"; LAST=\"$a\"; \
                     done; \
                     [ -n \"$TP\" ] && printf '#!/bin/sh\\necho %s-{ver}\\n' \"$LAST\" > \"$TP/$LAST\" && chmod +x \"$TP/$LAST\"; \
                     exit 0"
                )
            };
            std::fs::write(
                &fake_dotnet,
                format!(
                    "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo {ver}; exit 0; fi\necho \"$*\" >> '{}'\n{install_body}\n",
                    log_file.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&fake_dotnet, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mgr = ToolchainManager::new(data_dir);
        let platform = TargetPlatform {
            os: TargetOs::Macos,
            arch: TargetArch::Aarch64,
        };
        let installed8 = install_local_lsp(
            &mgr,
            "job_dotnet_lsp_8",
            &tc_root,
            EcosystemKind::Dotnet,
            platform,
            None,
            None,
            Some(&fake_sdk_8),
        )
        .await
        .expect("local dotnet 8 lsp install");
        assert!(installed8.ends_with("dotnet/tools/net8/csharp-ls"));
        let net8_bin = tc_root.join("dotnet/tools/net8/csharp-ls");
        assert!(net8_bin.is_file(), "net8 slot binary must exist");

        // BUG-22: Installing for .NET 9 must keep the .NET 8 slot intact alongside net9.
        let installed9 = install_local_lsp(
            &mgr,
            "job_dotnet_lsp_9",
            &tc_root,
            EcosystemKind::Dotnet,
            platform,
            None,
            None,
            Some(&fake_sdk_9),
        )
        .await
        .expect("local dotnet 9 lsp install");
        assert!(installed9.ends_with("dotnet/tools/net9/csharp-ls"));
        let net9_bin = tc_root.join("dotnet/tools/net9/csharp-ls");
        assert!(
            net8_bin.is_file() && net9_bin.is_file(),
            "BUG-22: .NET 8 and .NET 9 csharp-ls slots must coexist without clobbering"
        );
        assert!(
            std::fs::read_to_string(&net8_bin)
                .unwrap()
                .contains("8.0.408"),
            "net8 slot must remain intact after net9 install"
        );

        // DATA2-1: A failing reinstall must NEVER uninstall or destroy the existing working tool.
        let err = install_local_lsp(
            &mgr,
            "job_dotnet_lsp_fail",
            &tc_root,
            EcosystemKind::Dotnet,
            platform,
            None,
            None,
            Some(&fake_sdk_fail),
        )
        .await
        .unwrap_err();
        assert!(!err.is_empty());
        assert!(
            net8_bin.is_file()
                && std::fs::read_to_string(&net8_bin)
                    .unwrap()
                    .contains("8.0.408"),
            "DATA2-1: failed reinstall must preserve existing working net8/csharp-ls"
        );

        let log = std::fs::read_to_string(&log_file).unwrap();
        assert!(
            !log.contains("tool uninstall"),
            "DATA2-1: installer must stage and swap instead of calling `dotnet tool uninstall`:\n{log}"
        );
        assert!(
            log.contains("--version 0.16.0 csharp-ls"),
            "BUG-22: expected csharp-ls 0.16.0 for .NET 8 SDK, got:\n{log}"
        );
        assert!(
            log.contains("--version 0.20.0 csharp-ls"),
            "BUG-22: expected csharp-ls 0.20.0 for .NET 9 SDK, got:\n{log}"
        );
        assert!(
            log.contains("--version 0.85.4 fsautocomplete"),
            "RED-3: expected fsautocomplete 0.85.4 installed alongside csharp-ls, got:\n{log}"
        );

        let err_x64 = install_local_dap(
            &mgr,
            "job_dotnet_dap",
            &tc_root,
            EcosystemKind::Dotnet,
            TargetPlatform {
                os: TargetOs::Macos,
                arch: TargetArch::X86_64,
            },
            None,
        )
        .await
        .unwrap_err();
        assert!(
            err_x64.contains("macOS x86_64"),
            "BUG-20: expected clear error on macOS x86_64 netcoredbg install, got: {err_x64}"
        );
    }
}
