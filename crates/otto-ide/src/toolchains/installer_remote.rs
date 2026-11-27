//! Remote SSH installer script generation and streaming runner.

use std::collections::VecDeque;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const REMOTE_INSTALL_TIMEOUT: Duration = Duration::from_secs(900);

use crate::toolchains_catalog::{
    clangd_download_url, codelldb_vsix_url, csharp_ls_version_for_dotnet_sdk,
    dotnet_sdk_major_slot, jq_binary_url, kotlin_lsp_zip_url, netcoredbg_download_asset,
    rust_analyzer_gz_url, shfmt_binary_url, CatalogSdkEntry, EcosystemKind, TargetPlatform,
    CSHARP_LS_TOOL, DEBUGPY_PKG, DLV_PKG, FSAUTOCOMPLETE_TOOL, FSAUTOCOMPLETE_VERSION, GOPLS_PKG,
    JAVA_DEBUG_JAR_NAME, JAVA_DEBUG_JAR_URL, JDTLS_TARBALL_URL, JS_DEBUG_TARBALL_URL, PYRIGHT_PKG,
};
use crate::types::{IdeInstallComponent, IdeSshSpec};

use super::installer::InstallerProcessGroupGuard;

pub(super) async fn run_ssh_streaming<F: FnMut(String)>(
    ssh: &IdeSshSpec,
    script: &str,
    mut on_line: F,
) -> std::result::Result<String, String> {
    let (ssh_args, dest) =
        crate::target::ssh_exec::build_ssh_cli_args(ssh, None).map_err(|e| e.to_string())?;
    let mut cmd = Command::new("ssh");
    cmd.args(&ssh_args)
        .arg("--")
        .arg(dest)
        .args(["sh", "-s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().map_err(|e| format!("spawn ssh: {e}"))?;
    let mut pg_guard = InstallerProcessGroupGuard {
        pid: child.id(),
        disarmed: false,
    };
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(script.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
    }
    let stdout = child.stdout.take().ok_or("missing ssh stdout")?;
    let stderr = child.stderr.take().ok_or("missing ssh stderr")?;

    let (line_tx, mut line_rx) = tokio::sync::mpsc::channel::<(bool, String)>(256);
    let tx_out = line_tx.clone();
    let mut out_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        loop {
            match reader.next_line().await {
                Ok(Some(line)) => {
                    if tx_out.send((false, line)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => return Ok::<(), String>(()),
                Err(e) => return Err(format!("read ssh stdout: {e}")),
            }
        }
        Ok(())
    });
    let mut err_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        loop {
            match reader.next_line().await {
                Ok(Some(line)) => {
                    if line_tx.send((true, line)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => return Ok::<(), String>(()),
                Err(e) => return Err(format!("read ssh stderr: {e}")),
            }
        }
        Ok(())
    });

    let mut installed = String::from("~/.otto/toolchains");
    let mut stderr_tail: VecDeque<String> = VecDeque::with_capacity(12);

    let wait_res = tokio::time::timeout(REMOTE_INSTALL_TIMEOUT, async {
        while let Some((is_err, line)) = line_rx.recv().await {
            if !is_err {
                if let Some(p) = line.strip_prefix("INSTALLED_PATH:") {
                    installed = p.trim().to_string();
                    continue;
                }
            } else {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    if stderr_tail.len() >= 10 {
                        stderr_tail.pop_front();
                    }
                    stderr_tail.push_back(trimmed.to_string());
                }
            }
            on_line(line);
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if let Ok(Err(e)) = (&mut out_task).await {
            return Err(e);
        }
        if let Ok(Err(e)) = (&mut err_task).await {
            return Err(e);
        }
        Ok(status)
    })
    .await;

    let status = match wait_res {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            out_task.abort();
            err_task.abort();
            let _ = child.start_kill();
            return Err(format!(
                "remote installer timed out after {}s",
                REMOTE_INSTALL_TIMEOUT.as_secs()
            ));
        }
    };
    pg_guard.disarmed = true;
    if !status.success() {
        let detail: Vec<String> = stderr_tail.into_iter().collect();
        if detail.is_empty() {
            return Err(format!("remote installer exited with {status}"));
        }
        return Err(format!(
            "remote installer exited with {status}: {}",
            detail.join(" | ")
        ));
    }
    Ok(installed)
}

#[cfg(test)]
pub(crate) fn build_remote_install_script(
    eco: EcosystemKind,
    component: IdeInstallComponent,
    platform: TargetPlatform,
    sdk: Option<&CatalogSdkEntry>,
) -> std::result::Result<String, String> {
    build_remote_install_script_with_zls(eco, component, platform, sdk, None)
}

pub(super) fn build_remote_install_script_with_zls(
    eco: EcosystemKind,
    component: IdeInstallComponent,
    platform: TargetPlatform,
    sdk: Option<&CatalogSdkEntry>,
    zls_release: Option<&crate::toolchains_catalog::ZlsReleaseAsset>,
) -> std::result::Result<String, String> {
    let mut s = String::from(
        "set -eu\nTC=\"$HOME/.otto/toolchains\"\nmkdir -p \"$TC/.downloads\"\nMAX_DL_BYTES=1073741824\nverify_sha256() {\n  file=\"$1\"; expected=\"$2\"\n  if [ -z \"$expected\" ]; then return 0; fi\n  if command -v sha256sum >/dev/null 2>&1; then\n    actual=$(sha256sum \"$file\" | awk '{print tolower($1)}')\n  elif command -v shasum >/dev/null 2>&1; then\n    actual=$(shasum -a 256 \"$file\" | awk '{print tolower($1)}')\n  elif command -v openssl >/dev/null 2>&1; then\n    actual=$(openssl dgst -sha256 \"$file\" | awk '{print tolower($NF)}')\n  else\n    rm -f \"$file\"\n    echo \"ERROR: no sha256sum/shasum/openssl found to verify checksum\" >&2\n    exit 1\n  fi\n  exp_lc=$(printf '%s' \"$expected\" | tr 'A-Z' 'a-z')\n  if [ \"$actual\" != \"$exp_lc\" ]; then\n    rm -f \"$file\"\n    echo \"SHA-256 checksum mismatch: expected $exp_lc, got $actual\" >&2\n    exit 1\n  fi\n}\ndownload_and_verify() {\n  url=\"$1\"; dest=\"$2\"; expected=\"${3:-}\"\n  curl --proto '=https' --tlsv1.2 --max-filesize \"$MAX_DL_BYTES\" -fsSL \"$url\" -o \"$dest\"\n  if [ -z \"$expected\" ]; then\n    cand=$(curl --proto '=https' --tlsv1.2 --max-filesize 4096 -fsSL \"${url}.sha256\" 2>/dev/null | awk '{print $1}' | head -n 1 || true)\n    case \"$cand\" in\n      [0-9a-fA-F]*) [ \"${#cand}\" -eq 64 ] && expected=\"$cand\" ;;\n    esac\n  fi\n  verify_sha256 \"$dest\" \"$expected\"\n}\nfor d in \"$TC\"/go/sdks/*/go/bin \"$TC\"/node/sdks/*/bin; do [ -d \"$d\" ] && PATH=\"$d:$PATH\"; done\nexport PATH=\"$TC/java/current/bin:$TC/go/current/bin:$TC/node/current/bin:$TC/rust/cargo/bin:$TC/python/bin:$PATH\"\n",
    );
    if matches!(
        component,
        IdeInstallComponent::Sdk | IdeInstallComponent::All
    ) {
        let entry = sdk.ok_or("missing SDK entry")?;
        let exp_sha = match entry
            .sha256
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
        {
            Some(h) if h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()) => {
                h.to_ascii_lowercase()
            }
            Some(bad) => {
                return Err(format!("invalid SDK SHA-256 digest '{bad}'"));
            }
            None => String::new(),
        };
        let (u, v) = (
            shell_words::quote(&entry.url),
            shell_words::quote(&entry.version),
        );
        match eco {
            EcosystemKind::Java => s.push_str(&format!(
                "DEST=\"$TC/java/jdks/jdk-{0}\"\nARCH=\"$TC/.downloads/temurin-{0}.tar.gz\"\nrm -rf \"$DEST\" && mkdir -p \"$DEST\"\ndownload_and_verify {u} \"$ARCH\" '{exp_sha}'\ntar -xzf \"$ARCH\" -C \"$DEST\"\nrm -f \"$ARCH\"\nJH=\"\"\nfor cand in \"$DEST/Contents/Home\" \"$DEST\" \"$DEST\"/*/Contents/Home \"$DEST\"/*; do\n  if [ -x \"$cand/bin/java\" ]; then JH=\"$cand\"; break; fi\ndone\n[ -n \"$JH\" ] || {{ echo \"JDK archive missing bin/java\" >&2; exit 1; }}\nchmod +x \"$JH/bin/java\"\n\"$JH/bin/java\" -version\nln -sfn \"$JH\" \"$TC/java/current\"\nexport PATH=\"$TC/java/current/bin:$PATH\"\necho \"INSTALLED_PATH:$JH\"\n",
                entry.version
            )),
            EcosystemKind::Go => s.push_str(&format!("DEST=\"$TC/go/sdks/{0}\"\nARCH=\"$TC/.downloads/go-{0}.tar.gz\"\nrm -rf \"$DEST\" && mkdir -p \"$DEST\"\ndownload_and_verify {u} \"$ARCH\" '{exp_sha}'\ntar -xzf \"$ARCH\" -C \"$DEST\"\nrm -f \"$ARCH\"\n\"$DEST/go/bin/go\" version\nln -sfn \"$DEST/go\" \"$TC/go/current\"\nexport PATH=\"$TC/go/current/bin:$PATH\"\necho \"INSTALLED_PATH:$DEST/go\"\n", entry.version)),
            EcosystemKind::Rust => s.push_str(&format!("INIT=\"$TC/.downloads/rustup-init\"\ndownload_and_verify {u} \"$INIT\" '{exp_sha}'\nchmod +x \"$INIT\"\nRUSTUP_HOME=\"$TC/rust/rustup\" CARGO_HOME=\"$TC/rust/cargo\" \"$INIT\" -y --no-modify-path --default-toolchain {v}\nrm -f \"$INIT\"\necho \"INSTALLED_PATH:$TC/rust/cargo\"\n")),
            EcosystemKind::Zig => s.push_str(&format!("DEST=\"$TC/zig/sdks/{0}\"\nSTAGE=\"$TC/.downloads/zig-stage-{0}-$$\"\nARCH=\"$STAGE/zig.tar.xz\"\nrm -rf \"$STAGE\" && mkdir -p \"$STAGE/sdk\" \"$TC/zig/sdks\"\ndownload_and_verify {u} \"$ARCH\" '{exp_sha}'\ntar -xJf \"$ARCH\" --strip-components=1 -C \"$STAGE/sdk\"\nrm -f \"$ARCH\"\n\"$STAGE/sdk/zig\" version\nrm -rf \"$DEST\" && mv \"$STAGE/sdk\" \"$DEST\" && rm -rf \"$STAGE\"\nln -sfn \"$DEST\" \"$TC/zig/current\"\nexport PATH=\"$TC/zig/current:$PATH\"\necho \"INSTALLED_PATH:$DEST\"\n", entry.version)),
            EcosystemKind::Cpp => {
                return Err(
                    "C/C++ compiler toolchains (Clang / GCC / LLVM) are installed via system packages (`apt install clang` / `dnf install clang` / Homebrew), not remote archive scripts"
                        .into(),
                );
            }
            EcosystemKind::Kotlin => s.push_str(&format!("DEST=\"$TC/kotlin/sdks/{0}\"\nARCH=\"$TC/.downloads/kotlin-compiler-{0}.zip\"\nrm -rf \"$DEST\" && mkdir -p \"$DEST\"\ndownload_and_verify {u} \"$ARCH\" '{exp_sha}'\nunzip -q -o \"$ARCH\" -d \"$DEST\"\nrm -f \"$ARCH\"\nKH=\"$DEST\"; [ -x \"$DEST/kotlinc/bin/kotlinc\" ] && KH=\"$DEST/kotlinc\"\nchmod +x \"$KH/bin/kotlinc\"\nln -sfn \"$KH\" \"$TC/kotlin/current\"\nexport PATH=\"$TC/kotlin/current/bin:$PATH\"\necho \"INSTALLED_PATH:$KH\"\n", entry.version)),
            EcosystemKind::Dotnet => s.push_str(&format!("DEST=\"$TC/dotnet/sdks/{0}\"\nARCH=\"$TC/.downloads/dotnet-sdk-{0}.tar.gz\"\nrm -rf \"$DEST\" && mkdir -p \"$DEST\"\ndownload_and_verify {u} \"$ARCH\" '{exp_sha}'\ntar -xzf \"$ARCH\" -C \"$DEST\"\nrm -f \"$ARCH\"\nDOTNET_ROOT=\"$DEST\" \"$DEST/dotnet\" --version\nln -sfn \"$DEST\" \"$TC/dotnet/current\"\nexport DOTNET_ROOT=\"$TC/dotnet/current\"\nexport PATH=\"$TC/dotnet/current:$PATH\"\necho \"INSTALLED_PATH:$DEST\"\n", entry.version)),
            EcosystemKind::Node => s.push_str(&format!("DEST=\"$TC/node/sdks/{0}\"\nARCH=\"$TC/.downloads/node-{0}.tar.gz\"\nrm -rf \"$DEST\" && mkdir -p \"$DEST\"\ndownload_and_verify {u} \"$ARCH\" '{exp_sha}'\ntar -xzf \"$ARCH\" --strip-components=1 -C \"$DEST\"\nrm -f \"$ARCH\"\n\"$DEST/bin/node\" --version\nln -sfn \"$DEST\" \"$TC/node/current\"\nexport PATH=\"$TC/node/current/bin:$PATH\"\necho \"INSTALLED_PATH:$DEST\"\n", entry.version)),
            EcosystemKind::Python => s.push_str(&format!("mkdir -p \"$TC/python/bin\" \"$TC/python/interpreters\"\nARCH=\"$TC/.downloads/uv.tar.gz\"\nUV_SHA='{exp_sha}'\ndownload_and_verify {u} \"$ARCH\" \"$UV_SHA\"\ntar -xzf \"$ARCH\" -C \"$TC/.downloads\"\nrm -f \"$ARCH\"\nUV_BIN=$(find \"$TC/.downloads\" -name uv -type f | head -n 1)\ncp \"$UV_BIN\" \"$TC/python/bin/uv\" && chmod +x \"$TC/python/bin/uv\"\nUV_PYTHON_INSTALL_DIR=\"$TC/python/interpreters\" \"$TC/python/bin/uv\" python install {v}\necho \"INSTALLED_PATH:$TC/python/interpreters\"\n")),
            EcosystemKind::Shell => s.push_str(&format!("mkdir -p \"$TC/shell/bin\"\nSHFMT_TMP=\"$TC/.downloads/shfmt\"\ndownload_and_verify '{}' \"$SHFMT_TMP\" '{exp_sha}'\nmv \"$SHFMT_TMP\" \"$TC/shell/bin/shfmt\" && chmod +x \"$TC/shell/bin/shfmt\"\necho \"INSTALLED_PATH:$TC/shell/bin/shfmt\"\n", shfmt_binary_url(&entry.version, platform))),
            EcosystemKind::Json => s.push_str(&format!("mkdir -p \"$TC/json/bin\"\nJQ_TMP=\"$TC/.downloads/jq\"\ndownload_and_verify '{}' \"$JQ_TMP\" '{exp_sha}'\nmv \"$JQ_TMP\" \"$TC/json/bin/jq\" && chmod +x \"$TC/json/bin/jq\"\necho \"INSTALLED_PATH:$TC/json/bin/jq\"\n", jq_binary_url(&entry.version, platform))),
            EcosystemKind::Ios | EcosystemKind::Android | EcosystemKind::Flutter => {
                return Err(format!("{} SDK is not installed via remote archive scripts", eco.display_name()));
            }
        }
    }
    if matches!(
        component,
        IdeInstallComponent::Lsp | IdeInstallComponent::All
    ) {
        match eco {
            EcosystemKind::Java => s.push_str(&format!("mkdir -p \"$TC/java/lsp\"\nARCH=\"$TC/.downloads/jdtls.tar.gz\"\ndownload_and_verify '{JDTLS_TARBALL_URL}' \"$ARCH\"\ntar -xzf \"$ARCH\" -C \"$TC/java/lsp\" && rm -f \"$ARCH\" && chmod +x \"$TC/java/lsp/bin/jdtls\"\necho \"INSTALLED_PATH:$TC/java/lsp/bin/jdtls\"\n")),
            EcosystemKind::Go => s.push_str(&format!("export PATH=\"$TC/go/current/bin:$PATH\"\nmkdir -p \"$TC/go/bin\"\nGOBIN=\"$TC/go/bin\" GOPATH=\"$TC/go/gopath\" go install '{GOPLS_PKG}'\necho \"INSTALLED_PATH:$TC/go/bin/gopls\"\n")),
            EcosystemKind::Rust => s.push_str(&format!("mkdir -p \"$TC/rust/bin\"\nARCH=\"$TC/.downloads/rust-analyzer.gz\"\ndownload_and_verify '{}' \"$ARCH\"\ngzip -dc \"$ARCH\" > \"$TC/rust/bin/rust-analyzer\" && rm -f \"$ARCH\" && chmod +x \"$TC/rust/bin/rust-analyzer\"\necho \"INSTALLED_PATH:$TC/rust/bin/rust-analyzer\"\n", rust_analyzer_gz_url(platform))),
            EcosystemKind::Zig => {
                let rel = zls_release.ok_or_else(|| {
                    "missing verified ZLS release metadata and SHA-256 digest for remote Zig LSP install".to_string()
                })?;
                let sha = rel.sha256.trim();
                if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(format!("invalid ZLS SHA-256 digest '{sha}'"));
                }
                let sha_lc = sha.to_ascii_lowercase();
                let minor_slot = super::discovery::zig_major_minor(&rel.version)
                    .map(|(maj, min)| format!("{maj}.{min}"))
                    .unwrap_or_else(|| rel.version.clone());
                let zu = shell_words::quote(&rel.url);
                s.push_str(&format!(
                    "STAGE=\"$TC/.downloads/zls-stage-$$\"\nARCH=\"$STAGE/zls.tar.xz\"\nrm -rf \"$STAGE\" && mkdir -p \"$STAGE/extract\" \"$TC/zig/bin\" \"$TC/zig/zls/{minor_slot}\"\ndownload_and_verify {zu} \"$ARCH\" '{sha_lc}'\ntar -xJf \"$ARCH\" -C \"$STAGE/extract\" && rm -f \"$ARCH\"\nZLS_BIN=$(find \"$STAGE/extract\" -name zls -type f | head -n 1)\n[ -n \"$ZLS_BIN\" ] || {{ rm -rf \"$STAGE\"; echo \"ZLS archive missing zls binary\" >&2; exit 1; }}\nZLS_DEST=\"$TC/zig/zls/{minor_slot}/zls\"\nZLS_TMP=\"$TC/zig/zls/{minor_slot}/.zls.$$.tmp\"\nmv \"$ZLS_BIN\" \"$ZLS_TMP\" && chmod +x \"$ZLS_TMP\" && mv -f \"$ZLS_TMP\" \"$ZLS_DEST\"\nln -sfn \"$ZLS_DEST\" \"$TC/zig/bin/zls\"\nrm -rf \"$STAGE\"\necho \"INSTALLED_PATH:$ZLS_DEST\"\n"
                ));
            }
            EcosystemKind::Cpp => {
                let url = clangd_download_url(platform)?;
                s.push_str(&format!("STAGE=\"$TC/.downloads/clangd-stage-$$\"\nARCH=\"$STAGE/clangd.zip\"\nrm -rf \"$STAGE\" && mkdir -p \"$STAGE/extract\" \"$TC/cpp/bin\"\ndownload_and_verify '{url}' \"$ARCH\"\nunzip -q -o \"$ARCH\" -d \"$STAGE/extract\" && rm -f \"$ARCH\"\nCD_BIN=$(find \"$STAGE/extract\" -path '*/bin/clangd' -type f | head -n 1)\n[ -n \"$CD_BIN\" ] || {{ rm -rf \"$STAGE\"; echo \"clangd archive missing bin/clangd\" >&2; exit 1; }}\nCD_ROOT=$(dirname \"$(dirname \"$CD_BIN\")\")\nrm -rf \"$TC/cpp/lsp\" && mv \"$CD_ROOT\" \"$TC/cpp/lsp\" && chmod +x \"$TC/cpp/lsp/bin/clangd\"\nln -sfn \"$TC/cpp/lsp/bin/clangd\" \"$TC/cpp/bin/clangd\" && rm -rf \"$STAGE\"\necho \"INSTALLED_PATH:$TC/cpp/bin/clangd\"\n"));
            }
            EcosystemKind::Kotlin => s.push_str(&format!("rm -rf \"$TC/kotlin/lsp\" && mkdir -p \"$TC/kotlin/lsp\" \"$TC/kotlin/bin\"\nARCH=\"$TC/.downloads/kotlin-language-server.zip\"\ndownload_and_verify '{}' \"$ARCH\"\nunzip -q -o \"$ARCH\" -d \"$TC/kotlin/lsp\" && rm -f \"$ARCH\"\nKLS_BIN=\"$TC/kotlin/lsp/server/bin/kotlin-language-server\"\n[ -f \"$KLS_BIN\" ] || KLS_BIN=\"$TC/kotlin/lsp/bin/kotlin-language-server\"\nchmod +x \"$KLS_BIN\" && ln -sfn \"$KLS_BIN\" \"$TC/kotlin/bin/kotlin-language-server\"\necho \"INSTALLED_PATH:$KLS_BIN\"\n", kotlin_lsp_zip_url())),
            EcosystemKind::Dotnet => {
                let pinned = sdk.map(|e| {
                    (
                        csharp_ls_version_for_dotnet_sdk(Some(&e.version)),
                        dotnet_sdk_major_slot(Some(&e.version)),
                    )
                });
                let ver_block = match pinned {
                    Some((v, slot)) => format!("CSHARP_LS_VER='{v}'\nCSHARP_SLOT='{slot}'\n"),
                    None => concat!(
                        "SDK_MAJOR=$(dotnet --version 2>/dev/null | cut -d. -f1 || true)\n",
                        "case \"$SDK_MAJOR\" in\n",
                        "  6|7|8) CSHARP_LS_VER='0.16.0'; CSHARP_SLOT='net8' ;;\n",
                        "  9) CSHARP_LS_VER='0.20.0'; CSHARP_SLOT='net9' ;;\n",
                        "  10) CSHARP_LS_VER='0.26.0'; CSHARP_SLOT='net10' ;;\n",
                        "  11|12|13) CSHARP_LS_VER='0.29.0'; CSHARP_SLOT='net11' ;;\n",
                        "  *) CSHARP_LS_VER='0.20.0'; CSHARP_SLOT='net9' ;;\n",
                        "esac\n"
                    )
                    .to_string(),
                };
                // BUG-22 + DATA2-1 + RED-3: Stage each tool into a private PID-scoped directory
                // and swap into per-SDK-major slot (`$TC/dotnet/tools/$CSHARP_SLOT`) ONLY after
                // `dotnet tool install` succeeds, preserving working servers if NuGet fails and
                // keeping .NET 8/9/10 csharp-ls slots side by side.
                s.push_str(&format!(
                    "export PATH=\"$TC/dotnet/current:$HOME/.dotnet:$PATH\"\n\
                     if [ -z \"${{DOTNET_ROOT:-}}\" ] && [ -x \"$TC/dotnet/current/dotnet\" ]; then export DOTNET_ROOT=\"$TC/dotnet/current\"; fi\n\
                     mkdir -p \"$TC/dotnet/tools\"\n\
                     {ver_block}\
                     CS_STAGE=\"$TC/.downloads/dotnet-csharp-$$\"\n\
                     rm -rf \"$CS_STAGE\" && mkdir -p \"$CS_STAGE\"\n\
                     dotnet tool install --tool-path \"$CS_STAGE\" --version \"$CSHARP_LS_VER\" '{CSHARP_LS_TOOL}'\n\
                     rm -rf \"$TC/dotnet/tools/$CSHARP_SLOT\" && mv \"$CS_STAGE\" \"$TC/dotnet/tools/$CSHARP_SLOT\"\n\
                     ln -sfn \"$TC/dotnet/tools/$CSHARP_SLOT/{CSHARP_LS_TOOL}\" \"$TC/dotnet/tools/{CSHARP_LS_TOOL}\"\n\
                     FS_STAGE=\"$TC/.downloads/dotnet-fs-$$\"\n\
                     rm -rf \"$FS_STAGE\" && mkdir -p \"$FS_STAGE\"\n\
                     dotnet tool install --tool-path \"$FS_STAGE\" --version '{FSAUTOCOMPLETE_VERSION}' '{FSAUTOCOMPLETE_TOOL}'\n\
                     rm -rf \"$TC/dotnet/tools/fsautocomplete-pkg\" && mv \"$FS_STAGE\" \"$TC/dotnet/tools/fsautocomplete-pkg\"\n\
                     ln -sfn \"$TC/dotnet/tools/fsautocomplete-pkg/{FSAUTOCOMPLETE_TOOL}\" \"$TC/dotnet/tools/{FSAUTOCOMPLETE_TOOL}\"\n\
                     echo \"INSTALLED_PATH:$TC/dotnet/tools/$CSHARP_SLOT/{CSHARP_LS_TOOL}\"\n"
                ));
            }
            EcosystemKind::Node => {
                let pkgs = crate::toolchains_catalog::TS_LSP_NPM_PACKAGES.join(" ");
                s.push_str(&format!(
                    "export PATH=\"$TC/node/current/bin:$PATH\"\nmkdir -p \"$TC/node/lsp\"\nnpm install --prefix \"$TC/node/lsp\" {pkgs}\necho \"INSTALLED_PATH:$TC/node/lsp/node_modules/.bin/typescript-language-server\"\n"
                ));
            }
            EcosystemKind::Python => s.push_str(&format!("UV_TOOL_DIR=\"$TC/python/tools\" UV_TOOL_BIN_DIR=\"$TC/python/bin\" \"$TC/python/bin/uv\" tool install --force '{PYRIGHT_PKG}'\necho \"INSTALLED_PATH:$TC/python/bin/pyright-langserver\"\n")),
            EcosystemKind::Shell => s.push_str("export PATH=\"$TC/node/current/bin:$PATH\"\nmkdir -p \"$TC/shell/lsp\"\nnpm install --prefix \"$TC/shell/lsp\" bash-language-server\necho \"INSTALLED_PATH:$TC/shell/lsp/node_modules/.bin/bash-language-server\"\n"),
            EcosystemKind::Json => s.push_str("export PATH=\"$TC/node/current/bin:$PATH\"\nmkdir -p \"$TC/json/lsp\"\nnpm install --prefix \"$TC/json/lsp\" vscode-langservers-extracted\necho \"INSTALLED_PATH:$TC/json/lsp/node_modules/.bin/vscode-json-language-server\"\n"),
            EcosystemKind::Ios | EcosystemKind::Android | EcosystemKind::Flutter => {
                return Err(format!("{} LSP is provided by the native SDK installation", eco.display_name()));
            }
        }
    }
    if matches!(
        component,
        IdeInstallComponent::Dap | IdeInstallComponent::All
    ) {
        match eco {
            EcosystemKind::Java | EcosystemKind::Kotlin => s.push_str(&format!("mkdir -p \"$TC/java/dap\"\nJAR_TMP=\"$TC/.downloads/{JAVA_DEBUG_JAR_NAME}\"\ndownload_and_verify '{JAVA_DEBUG_JAR_URL}' \"$JAR_TMP\"\nmv \"$JAR_TMP\" \"$TC/java/dap/{JAVA_DEBUG_JAR_NAME}\"\necho \"INSTALLED_PATH:$TC/java/dap/{JAVA_DEBUG_JAR_NAME}\"\n")),
            EcosystemKind::Go => s.push_str(&format!("export PATH=\"$TC/go/current/bin:$PATH\"\nmkdir -p \"$TC/go/bin\"\nGOBIN=\"$TC/go/bin\" GOPATH=\"$TC/go/gopath\" go install '{DLV_PKG}'\necho \"INSTALLED_PATH:$TC/go/bin/dlv\"\n")),
            EcosystemKind::Rust | EcosystemKind::Zig | EcosystemKind::Cpp => s.push_str(&format!(
                "STAGE=\"$TC/.downloads/codelldb-stage-$$\"\n\
                 VSIX=\"$STAGE/codelldb.vsix\"\n\
                 rm -rf \"$STAGE\" && mkdir -p \"$STAGE/extract\" \"$TC/rust\"\n\
                 download_and_verify '{}' \"$VSIX\"\n\
                 unzip -q -o \"$VSIX\" -d \"$STAGE/extract\" && rm -f \"$VSIX\"\n\
                 chmod +x \"$STAGE/extract/extension/adapter/codelldb\"\n\
                 LOCK=\"$TC/.downloads/codelldb.lock\"\n\
                 _w=0\n\
                 while ! mkdir \"$LOCK\" 2>/dev/null; do\n\
                   _w=$((_w + 1))\n\
                   [ \"$_w\" -le 300 ] || {{ rm -rf \"$STAGE\"; echo \"timed out waiting for codelldb install lock\" >&2; exit 1; }}\n\
                   sleep 0.1\n\
                 done\n\
                 trap 'rmdir \"$LOCK\" 2>/dev/null || true' EXIT\n\
                 rm -rf \"$TC/rust/dap\" && mv \"$STAGE/extract\" \"$TC/rust/dap\"\n\
                 ln -sfn \"$TC/rust/dap/extension\" \"$TC/rust/codelldb\"\n\
                 rmdir \"$LOCK\" 2>/dev/null || true\n\
                 trap - EXIT\n\
                 rm -rf \"$STAGE\"\n\
                 echo \"INSTALLED_PATH:$TC/rust/dap/extension/adapter/codelldb\"\n",
                codelldb_vsix_url(platform)
            )),
            EcosystemKind::Dotnet => {
                let asset = netcoredbg_download_asset(platform)?;
                let (arch_name, extract_cmd) = if asset.is_zip {
                    (
                        "netcoredbg.zip",
                        "unzip -q -o \"$ARCH\" -d \"$TC/dotnet/dap\"",
                    )
                } else {
                    (
                        "netcoredbg.tar.gz",
                        "tar -xzf \"$ARCH\" -C \"$TC/dotnet/dap\"",
                    )
                };
                s.push_str(&format!(
                    "rm -rf \"$TC/dotnet/dap\" && mkdir -p \"$TC/dotnet/dap\"\n\
                     ARCH=\"$TC/.downloads/{arch_name}\"\n\
                     download_and_verify '{}' \"$ARCH\"\n\
                     {extract_cmd} && rm -f \"$ARCH\"\n\
                     NC_BIN=$(find \"$TC/dotnet/dap\" -name netcoredbg -type f | head -n 1)\n\
                     chmod +x \"$NC_BIN\"\n\
                     ln -sfn \"$(dirname \"$NC_BIN\")\" \"$TC/dotnet/netcoredbg\"\n\
                     echo \"INSTALLED_PATH:$NC_BIN\"\n",
                    asset.url
                ));
            }
            EcosystemKind::Node => s.push_str(&format!("rm -rf \"$TC/node/dap\" && mkdir -p \"$TC/node/dap\"\nARCH=\"$TC/.downloads/js-debug.tar.gz\"\ndownload_and_verify '{JS_DEBUG_TARBALL_URL}' \"$ARCH\"\ntar -xzf \"$ARCH\" -C \"$TC/node/dap\" && rm -f \"$ARCH\"\nln -sfn \"$TC/node/dap/js-debug\" \"$TC/node/js-debug\"\necho \"INSTALLED_PATH:$TC/node/dap/js-debug/src/dapDebugServer.js\"\n")),
            EcosystemKind::Python => s.push_str(&format!("\"$TC/python/bin/uv\" venv --clear \"$TC/python/dap\" && \"$TC/python/bin/uv\" pip install --python \"$TC/python/dap/bin/python\" '{DEBUGPY_PKG}'\necho \"INSTALLED_PATH:$TC/python/dap/bin/python\"\n")),
            EcosystemKind::Shell => s.push_str("mkdir -p \"$TC/shell/bin\"\nprintf '#!/usr/bin/env bash\\nexec bash -x \"$@\"\\n' > \"$TC/shell/bin/bashdb\" && chmod +x \"$TC/shell/bin/bashdb\"\necho \"INSTALLED_PATH:$TC/shell/bin/bashdb\"\n"),
            EcosystemKind::Json => s.push_str(&format!("mkdir -p \"$TC/json/bin\"\nJQ_TMP=\"$TC/.downloads/jq\"\ndownload_and_verify '{}' \"$JQ_TMP\"\nmv \"$JQ_TMP\" \"$TC/json/bin/jq\" && chmod +x \"$TC/json/bin/jq\"\necho \"INSTALLED_PATH:$TC/json/bin/jq\"\n", jq_binary_url("1.7.1", platform))),
            EcosystemKind::Ios | EcosystemKind::Android | EcosystemKind::Flutter => {
                return Err(format!("{} debugger is provided by the native SDK installation", eco.display_name()));
            }
        }
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolchains::installer::{download_https_to_file, verify_sha256_digest};
    use crate::toolchains::ToolchainManager;

    #[test]
    fn sha256_verifier_accepts_case_insensitive_match_and_rejects_corruption() {
        let actual = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify_sha256_digest(actual, None, "https://go.dev/dl/go.tar.gz").is_ok());
        assert!(verify_sha256_digest(
            actual,
            Some("E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855"),
            "https://go.dev/dl/go.tar.gz"
        )
        .is_ok());
        let err = verify_sha256_digest(
            actual,
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            "https://go.dev/dl/go.tar.gz",
        )
        .unwrap_err();
        assert!(err.contains("SHA-256 checksum mismatch"));
    }

    #[test]
    fn remote_install_script_resolves_nested_jdk_home_and_avoids_hardcoded_sdk_versions() {
        let platform = TargetPlatform::host();
        let java_sdk = CatalogSdkEntry {
            version: "21".into(),
            label: "Temurin 21".into(),
            lts: true,
            os: platform.os.as_str().into(),
            arch: platform.arch.as_str().into(),
            url: "https://api.adoptium.net/jdk21.tar.gz".into(),
            size_hint: None,
            sha256: None,
        };
        let java_script = build_remote_install_script(
            EcosystemKind::Java,
            IdeInstallComponent::Sdk,
            platform,
            Some(&java_sdk),
        )
        .unwrap();
        assert!(java_script.contains("Contents/Home"));
        assert!(java_script.contains("ln -sfn \"$JH\" \"$TC/java/current\""));
        assert!(java_script.contains("INSTALLED_PATH:$JH"));

        let go_lsp = build_remote_install_script(
            EcosystemKind::Go,
            IdeInstallComponent::Lsp,
            platform,
            None,
        )
        .unwrap();
        assert!(go_lsp.contains("$TC/go/current/bin"));
        assert!(!go_lsp.contains(EcosystemKind::Go.default_sdk_version()));

        let shell_lsp = build_remote_install_script(
            EcosystemKind::Shell,
            IdeInstallComponent::Lsp,
            platform,
            None,
        )
        .unwrap();
        assert!(shell_lsp.contains("$TC/node/current/bin"));
    }

    #[test]
    fn remote_install_script_verifies_sha256_when_present_and_rejects_malformed_digest() {
        let platform = TargetPlatform::host();
        let sha = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let py_sdk = CatalogSdkEntry {
            version: "3.13".into(),
            label: "Python 3.13".into(),
            lts: true,
            os: platform.os.as_str().into(),
            arch: platform.arch.as_str().into(),
            url: "https://github.com/astral-sh/uv/releases/download/0.6.9/uv-aarch64-apple-darwin.tar.gz".into(),
            size_hint: None,
            sha256: Some(sha.to_ascii_uppercase()),
        };
        let py_script = build_remote_install_script(
            EcosystemKind::Python,
            IdeInstallComponent::Sdk,
            platform,
            Some(&py_sdk),
        )
        .unwrap();
        assert!(py_script.contains("verify_sha256"));
        assert!(py_script.contains(&format!("UV_SHA='{sha}'")));

        let mut bad_sdk = py_sdk;
        bad_sdk.sha256 = Some("not-a-hex-digest; rm -rf /".into());
        let err = build_remote_install_script(
            EcosystemKind::Python,
            IdeInstallComponent::Sdk,
            platform,
            Some(&bad_sdk),
        )
        .unwrap_err();
        assert!(err.contains("invalid SDK SHA-256 digest"));
    }

    #[tokio::test]
    async fn download_https_to_file_blocks_loopback_and_private_urls_via_netguard() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ToolchainManager::new(tmp.path().join("data"));
        let dest = tmp.path().join("out.tar.gz");
        let err_loopback = download_https_to_file(
            &mgr,
            "job_1",
            "https://127.0.0.1:8443/go.tar.gz",
            &dest,
            None,
            10,
        )
        .await
        .unwrap_err();
        assert!(err_loopback.contains("blocked toolchain URL"));
        let err_private =
            download_https_to_file(&mgr, "job_2", "https://10.0.0.5/go.tar.gz", &dest, None, 10)
                .await
                .unwrap_err();
        assert!(err_private.contains("blocked toolchain URL"));
    }

    #[tokio::test]
    async fn download_https_to_file_capped_rejects_oversized_archive_and_cleans_up() {
        use crate::toolchains::installer::download_https_to_file_capped;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            // Serve a chunked response (no Content-Length) exceeding 512 bytes.
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut req = [0u8; 1024];
            let _ = sock.read(&mut req).await;
            let body = "A".repeat(1024);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        });

        let tmp = tempfile::tempdir().unwrap();
        let mgr = ToolchainManager::new(tmp.path().join("data"));
        mgr.allow_test_loopback
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let dest = tmp.path().join("sdk.tar.gz");
        let url = format!("http://127.0.0.1:{port}/sdk.tar.gz");
        let err = download_https_to_file_capped(&mgr, "job_cap", &url, &dest, None, 10, 512)
            .await
            .unwrap_err();
        assert!(err.contains("exceeded 512 byte limit"), "unexpected: {err}");
        assert!(!dest.exists(), "partial oversized file must be deleted");
        drop(server);
    }

    #[test]
    fn remote_install_scripts_avoid_unverified_curl_pipes_and_cap_downloads() {
        let platform = TargetPlatform::host();
        for (eco, comp) in [
            (EcosystemKind::Java, IdeInstallComponent::Lsp),
            (EcosystemKind::Rust, IdeInstallComponent::Lsp),
            (EcosystemKind::Java, IdeInstallComponent::Dap),
            (EcosystemKind::Rust, IdeInstallComponent::Dap),
            (EcosystemKind::Node, IdeInstallComponent::Dap),
            (EcosystemKind::Json, IdeInstallComponent::Dap),
        ] {
            let script = build_remote_install_script(eco, comp, platform, None).unwrap();
            assert!(
                !script.contains("| tar") && !script.contains("| gzip"),
                "remote {eco:?}/{comp:?} script must not pipe curl directly into tar/gzip:\n{script}"
            );
            assert!(
                script.contains("download_and_verify"),
                "remote {eco:?}/{comp:?} script must verify downloaded artifacts:\n{script}"
            );
            assert!(
                script.contains("--max-filesize"),
                "remote {eco:?}/{comp:?} script must bound download size:\n{script}"
            );
        }
    }

    #[test]
    fn remote_zig_sdk_stages_before_replacing_existing_and_zls_requires_verified_digest() {
        let platform = TargetPlatform::from_uname("Linux", "x86_64").unwrap();
        let zig_sdk = CatalogSdkEntry {
            version: "0.13.0".into(),
            label: "Zig 0.13.0".into(),
            lts: false,
            os: "linux".into(),
            arch: "x86_64".into(),
            url: "https://ziglang.org/download/0.13.0/zig-linux-x86_64-0.13.0.tar.xz".into(),
            size_hint: None,
            sha256: Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into()),
        };
        // DATA-3: download + verify + version check must happen BEFORE removing $DEST.
        let sdk_script = build_remote_install_script_with_zls(
            EcosystemKind::Zig,
            IdeInstallComponent::Sdk,
            platform,
            Some(&zig_sdk),
            None,
        )
        .unwrap();
        let dl_pos = sdk_script.find("download_and_verify").expect("download");
        let ver_pos = sdk_script
            .find("\"$STAGE/sdk/zig\" version")
            .expect("verify");
        let rm_dest_pos = sdk_script.find("rm -rf \"$DEST\"").expect("swap");
        assert!(
            dl_pos < rm_dest_pos && ver_pos < rm_dest_pos,
            "{sdk_script}"
        );

        // BUG-2 + SEC-1: Zig LSP remote install must fail without verified ZlsReleaseAsset.
        assert!(build_remote_install_script_with_zls(
            EcosystemKind::Zig,
            IdeInstallComponent::Lsp,
            platform,
            None,
            None,
        )
        .is_err());

        // BUG-2: Selected Zig 0.13 ZLS asset + required SHA-256 digest are embedded and version-slotted.
        let zls_013 = crate::toolchains_catalog::ZlsReleaseAsset {
            version: "0.13.0".into(),
            url: "https://builds.zigtools.org/zls-linux-x86_64-0.13.0.tar.xz".into(),
            sha256: "ec4c1b45caf88e2bcb9ebb16c670603cc596e4f621b96184dfbe837b39cd8410".into(),
        };
        let lsp_script = build_remote_install_script_with_zls(
            EcosystemKind::Zig,
            IdeInstallComponent::Lsp,
            platform,
            None,
            Some(&zls_013),
        )
        .unwrap();
        assert!(lsp_script.contains("zls-linux-x86_64-0.13.0.tar.xz"));
        assert!(
            lsp_script.contains("ec4c1b45caf88e2bcb9ebb16c670603cc596e4f621b96184dfbe837b39cd8410")
        );
        assert!(lsp_script.contains("$TC/zig/zls/0.13/zls"));
        assert!(!lsp_script.contains("0.14.0"));
    }

    #[test]
    fn remote_dotnet_install_scripts_pin_csharp_ls_install_fsautocomplete_and_use_real_netcoredbg()
    {
        let mac_arm = TargetPlatform::from_uname("Darwin", "arm64").unwrap();
        let mac_x64 = TargetPlatform::from_uname("Darwin", "x86_64").unwrap();
        let lin_x64 = TargetPlatform::from_uname("Linux", "x86_64").unwrap();

        // BUG-20: macOS arm64 uses netcoredbg-osx-arm64.zip + unzip; macOS x86_64 fails clearly;
        // Linux x86_64 uses netcoredbg-linux-amd64.tar.gz + tar.
        let mac_dap = build_remote_install_script(
            EcosystemKind::Dotnet,
            IdeInstallComponent::Dap,
            mac_arm,
            None,
        )
        .unwrap();
        assert!(mac_dap.contains("/3.2.0-1092/netcoredbg-osx-arm64.zip"));
        assert!(mac_dap.contains("unzip -q -o"));
        assert!(!mac_dap.contains("osx-amd64"));

        let err_x64 = build_remote_install_script(
            EcosystemKind::Dotnet,
            IdeInstallComponent::Dap,
            mac_x64,
            None,
        )
        .unwrap_err();
        assert!(err_x64.contains("macOS x86_64"));

        let lin_dap = build_remote_install_script(
            EcosystemKind::Dotnet,
            IdeInstallComponent::Dap,
            lin_x64,
            None,
        )
        .unwrap();
        assert!(lin_dap.contains("/3.2.0-1092/netcoredbg-linux-amd64.tar.gz"));
        assert!(lin_dap.contains("tar -xzf"));

        // BUG-22 + RED-3: .NET 8 SDK pins csharp-ls 0.16.0 and installs fsautocomplete 0.85.4.
        let sdk8 = CatalogSdkEntry {
            version: "8.0.408".into(),
            label: ".NET SDK 8.0.408".into(),
            lts: true,
            os: "linux".into(),
            arch: "x86_64".into(),
            url: "https://builds.dotnet.microsoft.com/dotnet/Sdk/8.0.408/dotnet-sdk-8.0.408-linux-x64.tar.gz".into(),
            size_hint: None,
            sha256: None,
        };
        let lsp8 = build_remote_install_script(
            EcosystemKind::Dotnet,
            IdeInstallComponent::Lsp,
            lin_x64,
            Some(&sdk8),
        )
        .unwrap();
        assert!(lsp8.contains("CSHARP_LS_VER='0.16.0'"), "{lsp8}");
        assert!(lsp8.contains("CSHARP_SLOT='net8'"), "{lsp8}");
        assert!(
            !lsp8.contains("dotnet tool uninstall"),
            "DATA2-1: remote installer must stage and swap, never uninstall first:\n{lsp8}"
        );
        assert!(
            lsp8.contains("--version \"$CSHARP_LS_VER\" 'csharp-ls'"),
            "{lsp8}"
        );
        assert!(
            lsp8.contains("--version '0.85.4' 'fsautocomplete'"),
            "{lsp8}"
        );

        // RED-2: remote Node LSP installs @vue/language-server@2.2.10 (standalone hybridMode=false).
        let node_lsp = build_remote_install_script(
            EcosystemKind::Node,
            IdeInstallComponent::Lsp,
            lin_x64,
            None,
        )
        .unwrap();
        assert!(
            node_lsp.contains("@vue/language-server@2.2.10"),
            "RED-2: remote Node LSP must pin @vue/language-server@2.2.10, got:\n{node_lsp}"
        );
    }
}
