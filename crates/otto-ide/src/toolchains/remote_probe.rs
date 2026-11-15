//! Batched remote SSH probe script and parser for SDK, LSP, and DAP discovery.

use std::collections::HashMap;
use std::path::Path;

use crate::toolchains_catalog::{EcosystemKind, TargetPlatform};
use crate::types::{IdeInstalledSdk, IdeToolBinaryStatus};

use super::discovery::dir_name_version;

pub(super) const REMOTE_PROBE_SCRIPT: &str = r#"
echo "UNAME:$(uname -s) $(uname -m)"
export PATH="$HOME/.otto/toolchains/java/current/bin:$HOME/.otto/toolchains/go/current/bin:$HOME/.otto/toolchains/node/current/bin:$HOME/.otto/toolchains/go/bin:$HOME/.otto/toolchains/rust/bin:$HOME/.otto/toolchains/rust/cargo/bin:$HOME/.otto/toolchains/python/bin:$HOME/.otto/toolchains/shell/bin:$HOME/.otto/toolchains/json/bin:$HOME/.cargo/bin:$HOME/go/bin:/opt/homebrew/bin:/usr/local/go/bin:/usr/local/bin:$PATH"
resolve_real() {
  readlink -f "$1" 2>/dev/null || realpath "$1" 2>/dev/null || python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$1" 2>/dev/null || echo "$1"
}
for d in "$HOME"/.otto/toolchains/java/jdks/*; do
  [ -d "$d" ] || continue
  jh=""
  for cand in "$d/Contents/Home" "$d" "$d"/*/Contents/Home "$d"/*; do
    if [ -x "$cand/bin/java" ]; then jh="$cand"; break; fi
  done
  if [ -n "$jh" ]; then
    jv=$(sed -n 's/^JAVA_VERSION="\?\([^"]*\)"\?.*/\1/p' "$jh/release" 2>/dev/null | head -n 1)
    [ -n "$jv" ] || jv=$(basename "$d" | sed 's/^jdk-//')
    echo "SDK:java:otto:$jv:$jh"
  fi
done
for d in "$HOME"/.otto/toolchains/go/sdks/*; do
  gr=""
  if [ -x "$d/go/bin/go" ]; then gr="$d/go"; elif [ -x "$d/bin/go" ]; then gr="$d"; fi
  if [ -n "$gr" ]; then
    gv=$(head -n 1 "$gr/VERSION" 2>/dev/null | sed 's/^go//')
    [ -n "$gv" ] || gv=$(basename "$d" | sed 's/^go//')
    echo "SDK:go:otto:$gv:$gr"
  fi
done
for d in "$HOME"/.otto/toolchains/rust/rustup/toolchains/*; do
  if [ -x "$d/bin/rustc" ]; then
    rv=$("$d/bin/rustc" --version 2>/dev/null | awk '{print $2}')
    [ -n "$rv" ] || rv=$(basename "$d")
    echo "SDK:rust:otto:$rv:$d"
  fi
done
for d in "$HOME"/.otto/toolchains/node/sdks/*; do
  if [ -x "$d/bin/node" ]; then
    nv=$("$d/bin/node" --version 2>/dev/null | sed 's/^v//')
    [ -n "$nv" ] || nv=$(basename "$d" | sed 's/^v//')
    echo "SDK:node:otto:$nv:$d"
  fi
done
for d in "$HOME"/.otto/toolchains/python/interpreters/*; do
  if [ -x "$d/bin/python3" ]; then
    pv=$("$d/bin/python3" --version 2>/dev/null | awk '{print $2}')
    [ -n "$pv" ] || pv=$(basename "$d" | sed 's/^cpython-//' | cut -d- -f1)
    echo "SDK:python:otto:$pv:$d/bin/python3"
  fi
done
if [ -x "$HOME/.otto/toolchains/shell/bin/shfmt" ]; then
  sv=$("$HOME/.otto/toolchains/shell/bin/shfmt" --version 2>/dev/null | head -n 1)
  echo "SDK:shell:otto:${sv:-3.10.0}:$HOME/.otto/toolchains/shell/bin/shfmt"
fi
if [ -x "$HOME/.otto/toolchains/json/bin/jq" ]; then
  qv=$("$HOME/.otto/toolchains/json/bin/jq" --version 2>/dev/null | sed 's/^jq-//')
  echo "SDK:json:otto:${qv:-1.7.1}:$HOME/.otto/toolchains/json/bin/jq"
fi
for pair in "java:java" "go:go" "rust:rustc" "node:node" "python:python3" "shell:bash" "json:jq"; do
  e="${pair%%:*}"; b="${pair#*:}"
  bin=$(command -v "$b" 2>/dev/null || true)
  [ -n "$bin" ] || continue
  real=$(resolve_real "$bin")
  ver=""
  sdk_path="$real"
  case "$e" in
    java)
      sdk_path=$(/usr/libexec/java_home 2>/dev/null || dirname "$(dirname "$real")")
      ver=$("$bin" -version 2>&1 | head -n 1 | sed -n 's/.*version "\([^"]*\)".*/\1/p')
      ;;
    go)
      sdk_path=$("$bin" env GOROOT 2>/dev/null || dirname "$(dirname "$real")")
      ver=$("$bin" version 2>/dev/null | awk '{print $3}' | sed 's/^go//')
      ;;
    rust)
      sdk_path=$("$bin" --print sysroot 2>/dev/null || dirname "$(dirname "$real")")
      ver=$("$bin" --version 2>/dev/null | awk '{print $2}')
      ;;
    node)
      sdk_path=$(dirname "$(dirname "$real")")
      ver=$("$bin" --version 2>/dev/null | sed 's/^v//')
      ;;
    python)
      ver=$("$bin" --version 2>/dev/null | awk '{print $2}')
      ;;
    shell)
      ver=$("$bin" --version 2>/dev/null | head -n 1 | sed -n 's/.*version \([0-9][0-9.]*\).*/\1/p')
      ;;
    json)
      ver=$("$bin" --version 2>/dev/null | sed 's/^jq-//')
      ;;
  esac
  [ -n "$ver" ] || ver="system"
  echo "SDK:$e:system:$ver:$sdk_path"
done
for pair in "java:jdtls:$HOME/.otto/toolchains/java/lsp/bin/jdtls" "go:gopls:$HOME/.otto/toolchains/go/bin/gopls" "rust:rust-analyzer:$HOME/.otto/toolchains/rust/bin/rust-analyzer" "node:typescript-language-server:$HOME/.otto/toolchains/node/lsp/node_modules/.bin/typescript-language-server" "python:pyright-langserver:$HOME/.otto/toolchains/python/bin/pyright-langserver" "shell:bash-language-server:$HOME/.otto/toolchains/shell/lsp/node_modules/.bin/bash-language-server" "json:vscode-json-language-server:$HOME/.otto/toolchains/json/lsp/node_modules/.bin/vscode-json-language-server"; do
  e="${pair%%:*}"; r="${pair#*:}"; b="${r%%:*}"; m="${r#*:}"
  if [ -x "$m" ]; then echo "LSP:$e:$m"; elif command -v "$b" >/dev/null 2>&1; then echo "LSP:$e:$(command -v "$b")"; fi
done
[ -f "$HOME/.otto/toolchains/java/dap/com.microsoft.java.debug.plugin-0.53.1.jar" ] && echo "DAP:java:$HOME/.otto/toolchains/java/dap/com.microsoft.java.debug.plugin-0.53.1.jar" || true
command -v dlv >/dev/null 2>&1 && echo "DAP:go:$(command -v dlv)" || true
if [ -x "$HOME/.otto/toolchains/rust/dap/extension/adapter/codelldb" ]; then echo "DAP:rust:$HOME/.otto/toolchains/rust/dap/extension/adapter/codelldb"; elif command -v codelldb >/dev/null 2>&1; then echo "DAP:rust:$(command -v codelldb)"; fi
[ -f "$HOME/.otto/toolchains/node/dap/js-debug/src/dapDebugServer.js" ] && echo "DAP:node:$HOME/.otto/toolchains/node/dap/js-debug/src/dapDebugServer.js" || true
[ -x "$HOME/.otto/toolchains/python/dap/bin/python" ] && echo "DAP:python:$HOME/.otto/toolchains/python/dap/bin/python" || true
if [ -x "$HOME/.otto/toolchains/shell/bin/bashdb" ]; then echo "DAP:shell:$HOME/.otto/toolchains/shell/bin/bashdb"; elif command -v bashdb >/dev/null 2>&1; then echo "DAP:shell:$(command -v bashdb)"; fi
if [ -x "$HOME/.otto/toolchains/json/bin/jq" ]; then echo "DAP:json:$HOME/.otto/toolchains/json/bin/jq"; elif command -v jq >/dev/null 2>&1; then echo "DAP:json:$(command -v jq)"; fi
"#;

#[derive(Default)]
pub(crate) struct ProbedRemoteEcosystem {
    pub installed: Vec<IdeInstalledSdk>,
    pub lsp: Option<IdeToolBinaryStatus>,
    pub dap: Option<IdeToolBinaryStatus>,
}

fn normalize_remote_sdk_path(eco: EcosystemKind, raw_path: &str) -> String {
    let trimmed = raw_path.trim();
    match eco {
        EcosystemKind::Java | EcosystemKind::Go | EcosystemKind::Rust | EcosystemKind::Node => {
            let p = Path::new(trimmed);
            if let (Some(parent), Some(bin_dir)) =
                (p.parent(), p.parent().and_then(Path::file_name))
            {
                if bin_dir == "bin" {
                    if let Some(home) = parent.parent() {
                        return home.to_string_lossy().into_owned();
                    }
                }
            }
            trimmed.to_string()
        }
        EcosystemKind::Python | EcosystemKind::Shell | EcosystemKind::Json => trimmed.to_string(),
    }
}

fn fallback_remote_sdk_version(eco: EcosystemKind, source: &str, raw_path: &str) -> String {
    let candidate = dir_name_version(Path::new(raw_path));
    if source == "system"
        && matches!(
            candidate.as_str(),
            "" | "unknown"
                | "bin"
                | "usr"
                | "local"
                | "java"
                | "go"
                | "rustc"
                | "node"
                | "python3"
                | "bash"
                | "jq"
        )
    {
        return match eco {
            EcosystemKind::Python => "3".into(),
            _ => "system".into(),
        };
    }
    candidate
}

pub(crate) fn parse_ssh_probe_output(
    raw: &str,
) -> (
    TargetPlatform,
    HashMap<EcosystemKind, ProbedRemoteEcosystem>,
) {
    let mut platform = TargetPlatform::host();
    let mut map: HashMap<EcosystemKind, ProbedRemoteEcosystem> = HashMap::new();
    for line in raw.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("UNAME:") {
            if let Some(p) = TargetPlatform::from_combined_uname(rest) {
                platform = p;
            }
        } else if let Some(rest) = line.strip_prefix("SDK:") {
            let four: Vec<&str> = rest.splitn(4, ':').collect();
            let parsed = if four.len() == 4
                && !four[2].starts_with('/')
                && !four[2].starts_with('~')
            {
                EcosystemKind::parse(four[0]).map(|eco| {
                    let source = four[1];
                    let path = normalize_remote_sdk_path(eco, four[3]);
                    let ver = if four[2].trim().is_empty() {
                        fallback_remote_sdk_version(eco, source, &path)
                    } else {
                        four[2].trim().to_string()
                    };
                    (eco, source, ver, path)
                })
            } else {
                let three: Vec<&str> = rest.splitn(3, ':').collect();
                (three.len() == 3)
                    .then(|| EcosystemKind::parse(three[0]).map(|eco| (eco, three[1], three[2])))
                    .flatten()
                    .map(|(eco, source, raw_path)| {
                        let path = normalize_remote_sdk_path(eco, raw_path);
                        let ver = fallback_remote_sdk_version(eco, source, raw_path);
                        (eco, source, ver, path)
                    })
            };
            if let Some((eco, source, version, path)) = parsed {
                let entry = map.entry(eco).or_default();
                if !entry.installed.iter().any(|s| s.path == path) {
                    entry.installed.push(IdeInstalledSdk {
                        id: format!("{}-{}-{}", eco.as_str(), source, entry.installed.len() + 1),
                        version,
                        path,
                        source: source.to_string(),
                        active: false,
                    });
                }
            }
        } else if let Some((eco_s, path)) =
            line.strip_prefix("LSP:").and_then(|r| r.split_once(':'))
        {
            if let Some(eco) = EcosystemKind::parse(eco_s) {
                map.entry(eco).or_default().lsp = Some(IdeToolBinaryStatus {
                    name: eco.lsp_name().to_string(),
                    available: true,
                    path: Some(path.to_string()),
                });
            }
        } else if let Some((eco_s, path)) =
            line.strip_prefix("DAP:").and_then(|r| r.split_once(':'))
        {
            if let Some(eco) = EcosystemKind::parse(eco_s) {
                map.entry(eco).or_default().dap = Some(IdeToolBinaryStatus {
                    name: eco.dap_name().to_string(),
                    available: true,
                    path: Some(path.to_string()),
                });
            }
        }
    }
    (platform, map)
}
