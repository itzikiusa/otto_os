//! Local LSP and DAP binary installation routines for [`super::ToolchainManager`].

use std::path::Path;

use crate::toolchains_catalog::{
    codelldb_vsix_url, jq_binary_url, rust_analyzer_gz_url, uv_tarball_url, EcosystemKind,
    TargetPlatform, BASH_LSP_NPM_PACKAGES, DEBUGPY_PKG, DLV_PKG, GOPLS_PKG, JAVA_DEBUG_JAR_NAME,
    JAVA_DEBUG_JAR_URL, JDTLS_TARBALL_URL, JSON_LSP_NPM_PACKAGES, JS_DEBUG_TARBALL_URL,
    PYRIGHT_PKG, TS_LSP_NPM_PACKAGES,
};

use super::installer::{
    download_https_to_file, ensure_local_uv, make_executable, relink_current, resolve_best_sdk_bin,
    run_local_cmd,
};
use super::{
    find_executable_in, is_executable_file, system_search_dirs_with_shell, ToolchainManager,
};

pub(super) async fn install_local_lsp(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    eco: EcosystemKind,
    platform: TargetPlatform,
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
            let go_bin = resolve_best_sdk_bin(mgr, root, EcosystemKind::Go, "go")
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
            let uv_bin = ensure_local_uv(mgr, job_id, root, &uv_tarball_url(platform)).await?;
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
    }
}

pub(super) async fn install_local_dap(
    mgr: &ToolchainManager,
    job_id: &str,
    root: &Path,
    eco: EcosystemKind,
    platform: TargetPlatform,
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
            let go_bin = resolve_best_sdk_bin(mgr, root, EcosystemKind::Go, "go")
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
        EcosystemKind::Rust => {
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
            let uv_bin = ensure_local_uv(mgr, job_id, root, &uv_tarball_url(platform)).await?;
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
    }
}
