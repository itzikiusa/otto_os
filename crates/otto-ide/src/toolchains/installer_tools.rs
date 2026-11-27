//! Local LSP and DAP binary installation routines for [`super::ToolchainManager`].

use std::path::{Path, PathBuf};

use crate::toolchains_catalog::{
    codelldb_vsix_url, jq_binary_url, netcoredbg_tarball_url, rust_analyzer_gz_url, uv_tarball_url,
    zls_tarball_url, EcosystemKind, TargetPlatform, BASH_LSP_NPM_PACKAGES, CSHARP_LS_TOOL,
    DEBUGPY_PKG, DLV_PKG, GOPLS_PKG, JAVA_DEBUG_JAR_NAME, JAVA_DEBUG_JAR_URL, JDTLS_TARBALL_URL,
    JSON_LSP_NPM_PACKAGES, JS_DEBUG_TARBALL_URL, PYRIGHT_PKG, TS_LSP_NPM_PACKAGES,
};

use super::installer::{
    download_https_to_file, ensure_local_uv, make_executable, relink_current, resolve_best_sdk_bin,
    run_local_cmd,
};
use super::{
    find_executable_in, is_executable_file, system_search_dirs_with_shell, ToolchainManager,
};

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
            // Match ZLS release tag (`0.14.0`, `0.13.0`, ...) to the active Zig major.minor.
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
            let zls_ver = zig_ver
                .as_deref()
                .and_then(super::discovery::zig_major_minor)
                .map(|(maj, min)| format!("{maj}.{min}.0"))
                .unwrap_or_else(|| EcosystemKind::Zig.default_sdk_version().to_string());
            let archive = tmp.join(format!("zls-{zls_ver}.tar.xz"));
            download_https_to_file(
                mgr,
                job_id,
                &zls_tarball_url(&zls_ver, platform),
                &archive,
                None,
                65,
            )
            .await?;
            let bin_dir = root.join("zig/bin");
            tokio::fs::create_dir_all(&bin_dir)
                .await
                .map_err(|e| e.to_string())?;
            let extract_dir = tmp.join("zls-extract");
            let _ = tokio::fs::remove_dir_all(&extract_dir).await;
            tokio::fs::create_dir_all(&extract_dir)
                .await
                .map_err(|e| e.to_string())?;
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
            let _ = tokio::fs::remove_file(&archive).await;
            let src_bin = [extract_dir.join("zls"), extract_dir.join("bin/zls")]
                .into_iter()
                .find(|p| p.is_file())
                .ok_or("ZLS archive missing zls binary")?;
            let dest = bin_dir.join("zls");
            let _ = tokio::fs::remove_file(&dest).await;
            tokio::fs::rename(&src_bin, &dest)
                .await
                .map_err(|e| e.to_string())?;
            let _ = tokio::fs::remove_dir_all(&extract_dir).await;
            make_executable(&dest)?;
            Ok(dest.to_string_lossy().into_owned())
        }
        EcosystemKind::Dotnet => {
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
            let tool_dest = tools_dir.join(CSHARP_LS_TOOL);
            let subcmd = if tool_dest.is_file() {
                "update"
            } else {
                "install"
            };
            run_local_cmd(
                mgr,
                job_id,
                75,
                &dotnet_bin,
                &[
                    "tool",
                    subcmd,
                    "--tool-path",
                    &tools_dir.to_string_lossy(),
                    CSHARP_LS_TOOL,
                ],
                &env_pairs,
            )
            .await?;
            Ok(tool_dest.to_string_lossy().into_owned())
        }
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
        EcosystemKind::Java => {
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
        EcosystemKind::Rust | EcosystemKind::Zig => {
            let vsix = tmp.join("codelldb.vsix");
            download_https_to_file(mgr, job_id, &codelldb_vsix_url(platform), &vsix, None, 85)
                .await?;
            let dest = root.join("rust/dap");
            let _ = tokio::fs::remove_dir_all(&dest).await;
            tokio::fs::create_dir_all(&dest)
                .await
                .map_err(|e| e.to_string())?;
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
                    &dest.to_string_lossy(),
                ],
                &[],
            )
            .await?;
            let _ = tokio::fs::remove_file(&vsix).await;
            let adapter = dest.join("extension/adapter/codelldb");
            make_executable(&adapter)?;
            relink_current(&root.join("rust/codelldb"), &dest.join("extension"));
            Ok(adapter.to_string_lossy().into_owned())
        }
        EcosystemKind::Dotnet => {
            let archive = tmp.join("netcoredbg.tar.gz");
            download_https_to_file(
                mgr,
                job_id,
                &netcoredbg_tarball_url(platform),
                &archive,
                None,
                85,
            )
            .await?;
            let dest = root.join("dotnet/dap");
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
