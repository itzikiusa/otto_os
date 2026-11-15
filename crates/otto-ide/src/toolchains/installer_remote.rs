//! Remote SSH installer script generation and streaming runner.

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::toolchains_catalog::{
    codelldb_vsix_url, jq_binary_url, rust_analyzer_gz_url, shfmt_binary_url, CatalogSdkEntry,
    EcosystemKind, TargetPlatform, DEBUGPY_PKG, DLV_PKG, GOPLS_PKG, JAVA_DEBUG_JAR_NAME,
    JAVA_DEBUG_JAR_URL, JDTLS_TARBALL_URL, JS_DEBUG_TARBALL_URL, PYRIGHT_PKG,
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
    let mut lines = BufReader::new(stdout).lines();
    let mut installed = String::from("~/.otto/toolchains");
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(p) = line.strip_prefix("INSTALLED_PATH:") {
            installed = p.trim().to_string();
        } else {
            on_line(line);
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    pg_guard.disarmed = true;
    if !status.success() {
        return Err(format!("remote installer exited with {status}"));
    }
    Ok(installed)
}

pub(super) fn build_remote_install_script(
    eco: EcosystemKind,
    component: IdeInstallComponent,
    platform: TargetPlatform,
    sdk: Option<&CatalogSdkEntry>,
) -> std::result::Result<String, String> {
    let mut s = String::from(
        "set -eu\nTC=\"$HOME/.otto/toolchains\"\nmkdir -p \"$TC/.downloads\"\nverify_sha256() {\n  file=\"$1\"; expected=\"$2\"\n  if [ -z \"$expected\" ]; then return 0; fi\n  if command -v sha256sum >/dev/null 2>&1; then\n    actual=$(sha256sum \"$file\" | awk '{print tolower($1)}')\n  elif command -v shasum >/dev/null 2>&1; then\n    actual=$(shasum -a 256 \"$file\" | awk '{print tolower($1)}')\n  elif command -v openssl >/dev/null 2>&1; then\n    actual=$(openssl dgst -sha256 \"$file\" | awk '{print tolower($NF)}')\n  else\n    rm -f \"$file\"\n    echo \"ERROR: no sha256sum/shasum/openssl found to verify checksum\" >&2\n    exit 1\n  fi\n  exp_lc=$(printf '%s' \"$expected\" | tr 'A-Z' 'a-z')\n  if [ \"$actual\" != \"$exp_lc\" ]; then\n    rm -f \"$file\"\n    echo \"SHA-256 checksum mismatch: expected $exp_lc, got $actual\" >&2\n    exit 1\n  fi\n}\nfor d in \"$TC\"/go/sdks/*/go/bin \"$TC\"/node/sdks/*/bin; do [ -d \"$d\" ] && PATH=\"$d:$PATH\"; done\nexport PATH=\"$TC/java/current/bin:$TC/go/current/bin:$TC/node/current/bin:$TC/rust/cargo/bin:$TC/python/bin:$PATH\"\n",
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
                "DEST=\"$TC/java/jdks/jdk-{0}\"\nARCH=\"$TC/.downloads/temurin-{0}.tar.gz\"\nrm -rf \"$DEST\" && mkdir -p \"$DEST\"\ncurl -fsSL {u} -o \"$ARCH\"\nverify_sha256 \"$ARCH\" '{exp_sha}'\ntar -xzf \"$ARCH\" -C \"$DEST\"\nrm -f \"$ARCH\"\nJH=\"\"\nfor cand in \"$DEST/Contents/Home\" \"$DEST\" \"$DEST\"/*/Contents/Home \"$DEST\"/*; do\n  if [ -x \"$cand/bin/java\" ]; then JH=\"$cand\"; break; fi\ndone\n[ -n \"$JH\" ] || {{ echo \"JDK archive missing bin/java\" >&2; exit 1; }}\nchmod +x \"$JH/bin/java\"\n\"$JH/bin/java\" -version\nln -sfn \"$JH\" \"$TC/java/current\"\nexport PATH=\"$TC/java/current/bin:$PATH\"\necho \"INSTALLED_PATH:$JH\"\n",
                entry.version
            )),
            EcosystemKind::Go => s.push_str(&format!("DEST=\"$TC/go/sdks/{0}\"\nARCH=\"$TC/.downloads/go-{0}.tar.gz\"\nrm -rf \"$DEST\" && mkdir -p \"$DEST\"\ncurl -fsSL {u} -o \"$ARCH\"\nverify_sha256 \"$ARCH\" '{exp_sha}'\ntar -xzf \"$ARCH\" -C \"$DEST\"\nrm -f \"$ARCH\"\n\"$DEST/go/bin/go\" version\nln -sfn \"$DEST/go\" \"$TC/go/current\"\nexport PATH=\"$TC/go/current/bin:$PATH\"\necho \"INSTALLED_PATH:$DEST/go\"\n", entry.version)),
            EcosystemKind::Rust => s.push_str(&format!("INIT=\"$TC/.downloads/rustup-init\"\ncurl -fsSL {u} -o \"$INIT\"\nverify_sha256 \"$INIT\" '{exp_sha}'\nchmod +x \"$INIT\"\nRUSTUP_HOME=\"$TC/rust/rustup\" CARGO_HOME=\"$TC/rust/cargo\" \"$INIT\" -y --no-modify-path --default-toolchain {v}\nrm -f \"$INIT\"\necho \"INSTALLED_PATH:$TC/rust/cargo\"\n")),
            EcosystemKind::Node => s.push_str(&format!("DEST=\"$TC/node/sdks/{0}\"\nARCH=\"$TC/.downloads/node-{0}.tar.gz\"\nrm -rf \"$DEST\" && mkdir -p \"$DEST\"\ncurl -fsSL {u} -o \"$ARCH\"\nverify_sha256 \"$ARCH\" '{exp_sha}'\ntar -xzf \"$ARCH\" --strip-components=1 -C \"$DEST\"\nrm -f \"$ARCH\"\n\"$DEST/bin/node\" --version\nln -sfn \"$DEST\" \"$TC/node/current\"\nexport PATH=\"$TC/node/current/bin:$PATH\"\necho \"INSTALLED_PATH:$DEST\"\n", entry.version)),
            EcosystemKind::Python => s.push_str(&format!("mkdir -p \"$TC/python/bin\" \"$TC/python/interpreters\"\nARCH=\"$TC/.downloads/uv.tar.gz\"\ncurl -fsSL {u} -o \"$ARCH\"\nUV_SHA='{exp_sha}'\nif [ -z \"$UV_SHA\" ]; then\n  UV_SHA=$(curl -fsSL {u}.sha256 2>/dev/null | awk '{{print $1}}' | head -n 1 || true)\nfi\nverify_sha256 \"$ARCH\" \"$UV_SHA\"\ntar -xzf \"$ARCH\" -C \"$TC/.downloads\"\nrm -f \"$ARCH\"\nUV_BIN=$(find \"$TC/.downloads\" -name uv -type f | head -n 1)\ncp \"$UV_BIN\" \"$TC/python/bin/uv\" && chmod +x \"$TC/python/bin/uv\"\nUV_PYTHON_INSTALL_DIR=\"$TC/python/interpreters\" \"$TC/python/bin/uv\" python install {v}\necho \"INSTALLED_PATH:$TC/python/interpreters\"\n")),
            EcosystemKind::Shell => s.push_str(&format!("mkdir -p \"$TC/shell/bin\"\nSHFMT_TMP=\"$TC/.downloads/shfmt\"\ncurl -fsSL '{}' -o \"$SHFMT_TMP\"\nverify_sha256 \"$SHFMT_TMP\" '{exp_sha}'\nmv \"$SHFMT_TMP\" \"$TC/shell/bin/shfmt\" && chmod +x \"$TC/shell/bin/shfmt\"\necho \"INSTALLED_PATH:$TC/shell/bin/shfmt\"\n", shfmt_binary_url(&entry.version, platform))),
            EcosystemKind::Json => s.push_str(&format!("mkdir -p \"$TC/json/bin\"\nJQ_TMP=\"$TC/.downloads/jq\"\ncurl -fsSL '{}' -o \"$JQ_TMP\"\nverify_sha256 \"$JQ_TMP\" '{exp_sha}'\nmv \"$JQ_TMP\" \"$TC/json/bin/jq\" && chmod +x \"$TC/json/bin/jq\"\necho \"INSTALLED_PATH:$TC/json/bin/jq\"\n", jq_binary_url(&entry.version, platform))),
        }
    }
    if matches!(
        component,
        IdeInstallComponent::Lsp | IdeInstallComponent::All
    ) {
        match eco {
            EcosystemKind::Java => s.push_str(&format!("mkdir -p \"$TC/java/lsp\"\ncurl -fsSL '{JDTLS_TARBALL_URL}' | tar -xz -C \"$TC/java/lsp\" && chmod +x \"$TC/java/lsp/bin/jdtls\"\necho \"INSTALLED_PATH:$TC/java/lsp/bin/jdtls\"\n")),
            EcosystemKind::Go => s.push_str(&format!("export PATH=\"$TC/go/current/bin:$PATH\"\nmkdir -p \"$TC/go/bin\"\nGOBIN=\"$TC/go/bin\" GOPATH=\"$TC/go/gopath\" go install '{GOPLS_PKG}'\necho \"INSTALLED_PATH:$TC/go/bin/gopls\"\n")),
            EcosystemKind::Rust => s.push_str(&format!("mkdir -p \"$TC/rust/bin\"\ncurl -fsSL '{}' | gzip -d > \"$TC/rust/bin/rust-analyzer\" && chmod +x \"$TC/rust/bin/rust-analyzer\"\necho \"INSTALLED_PATH:$TC/rust/bin/rust-analyzer\"\n", rust_analyzer_gz_url(platform))),
            EcosystemKind::Node => s.push_str("export PATH=\"$TC/node/current/bin:$PATH\"\nmkdir -p \"$TC/node/lsp\"\nnpm install --prefix \"$TC/node/lsp\" typescript typescript-language-server\necho \"INSTALLED_PATH:$TC/node/lsp/node_modules/.bin/typescript-language-server\"\n"),
            EcosystemKind::Python => s.push_str(&format!("UV_TOOL_DIR=\"$TC/python/tools\" UV_TOOL_BIN_DIR=\"$TC/python/bin\" \"$TC/python/bin/uv\" tool install --force '{PYRIGHT_PKG}'\necho \"INSTALLED_PATH:$TC/python/bin/pyright-langserver\"\n")),
            EcosystemKind::Shell => s.push_str("export PATH=\"$TC/node/current/bin:$PATH\"\nmkdir -p \"$TC/shell/lsp\"\nnpm install --prefix \"$TC/shell/lsp\" bash-language-server\necho \"INSTALLED_PATH:$TC/shell/lsp/node_modules/.bin/bash-language-server\"\n"),
            EcosystemKind::Json => s.push_str("export PATH=\"$TC/node/current/bin:$PATH\"\nmkdir -p \"$TC/json/lsp\"\nnpm install --prefix \"$TC/json/lsp\" vscode-langservers-extracted\necho \"INSTALLED_PATH:$TC/json/lsp/node_modules/.bin/vscode-json-language-server\"\n"),
        }
    }
    if matches!(
        component,
        IdeInstallComponent::Dap | IdeInstallComponent::All
    ) {
        match eco {
            EcosystemKind::Java => s.push_str(&format!("mkdir -p \"$TC/java/dap\"\ncurl -fsSL '{JAVA_DEBUG_JAR_URL}' -o \"$TC/java/dap/{JAVA_DEBUG_JAR_NAME}\"\necho \"INSTALLED_PATH:$TC/java/dap/{JAVA_DEBUG_JAR_NAME}\"\n")),
            EcosystemKind::Go => s.push_str(&format!("export PATH=\"$TC/go/current/bin:$PATH\"\nmkdir -p \"$TC/go/bin\"\nGOBIN=\"$TC/go/bin\" GOPATH=\"$TC/go/gopath\" go install '{DLV_PKG}'\necho \"INSTALLED_PATH:$TC/go/bin/dlv\"\n")),
            EcosystemKind::Rust => s.push_str(&format!("rm -rf \"$TC/rust/dap\" && mkdir -p \"$TC/rust/dap\"\ncurl -fsSL '{}' -o \"$TC/.downloads/codelldb.vsix\" && unzip -q -o \"$TC/.downloads/codelldb.vsix\" -d \"$TC/rust/dap\" && chmod +x \"$TC/rust/dap/extension/adapter/codelldb\"\nln -sfn \"$TC/rust/dap/extension\" \"$TC/rust/codelldb\"\necho \"INSTALLED_PATH:$TC/rust/dap/extension/adapter/codelldb\"\n", codelldb_vsix_url(platform))),
            EcosystemKind::Node => s.push_str(&format!("rm -rf \"$TC/node/dap\" && mkdir -p \"$TC/node/dap\"\ncurl -fsSL '{JS_DEBUG_TARBALL_URL}' | tar -xz -C \"$TC/node/dap\"\nln -sfn \"$TC/node/dap/js-debug\" \"$TC/node/js-debug\"\necho \"INSTALLED_PATH:$TC/node/dap/js-debug/src/dapDebugServer.js\"\n")),
            EcosystemKind::Python => s.push_str(&format!("\"$TC/python/bin/uv\" venv --clear \"$TC/python/dap\" && \"$TC/python/bin/uv\" pip install --python \"$TC/python/dap/bin/python\" '{DEBUGPY_PKG}'\necho \"INSTALLED_PATH:$TC/python/dap/bin/python\"\n")),
            EcosystemKind::Shell => s.push_str("mkdir -p \"$TC/shell/bin\"\nprintf '#!/usr/bin/env bash\\nexec bash -x \"$@\"\\n' > \"$TC/shell/bin/bashdb\" && chmod +x \"$TC/shell/bin/bashdb\"\necho \"INSTALLED_PATH:$TC/shell/bin/bashdb\"\n"),
            EcosystemKind::Json => s.push_str(&format!("mkdir -p \"$TC/json/bin\"\ncurl -fsSL '{}' -o \"$TC/json/bin/jq\" && chmod +x \"$TC/json/bin/jq\"\necho \"INSTALLED_PATH:$TC/json/bin/jq\"\n", jq_binary_url("1.7.1", platform))),
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
}
