//! Shell (`bash` + `zsh`) DAP adapter resolver.

use std::collections::BTreeMap;

use otto_core::{Error, Result};
use serde_json::json;

use super::adapters::{
    locate_binary, normalize_target_path, DapAdapterTransport, ResolvedDapAdapter,
};
use super::shell_bridge::{ShellBridgeSpec, ShellFlavor};
use super::DapStartContext;
use crate::types::{IdeStartDebugReq, IdeTargetKind};

pub(super) async fn resolve_shell_adapter(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let program = normalize_target_path(&req.target, &req.launch.program);
    if program.trim().is_empty() {
        return Err(Error::Invalid(
            "program path (.sh/.bash/.zsh script) is required to debug a shell script".into(),
        ));
    }

    let mut want_zsh = req.language.eq_ignore_ascii_case("zsh") || program.ends_with(".zsh");
    if !want_zsh && req.target.kind == IdeTargetKind::Local {
        if let Ok(head) = tokio::fs::read_to_string(&program).await {
            if let Some(first_line) = head.lines().next() {
                if first_line.starts_with("#!") && first_line.contains("zsh") {
                    want_zsh = true;
                }
            }
        }
    }

    let (flavor, adapter_name, shell_bin) = if want_zsh {
        let zsh_candidates: Vec<String> = [
            "/opt/homebrew/bin/zsh",
            "/usr/local/bin/zsh",
            "/bin/zsh",
            "zsh",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let bin = locate_binary(&req.target, ctx, &env, &zsh_candidates)
            .await
            .ok_or_else(|| Error::NotFound("Zsh shell ('zsh') not found on target".into()))?;
        (ShellFlavor::Zsh, "zsh-trap-dap", bin)
    } else {
        let bash_candidates: Vec<String> = [
            "/opt/homebrew/bin/bash",
            "/usr/local/bin/bash",
            "bash",
            "/bin/bash",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let bin = locate_binary(&req.target, ctx, &env, &bash_candidates)
            .await
            .ok_or_else(|| Error::NotFound("Bash shell ('bash') not found on target".into()))?;
        (ShellFlavor::Bash, "bash-trap-dap", bin)
    };

    Ok(ResolvedDapAdapter {
        adapter_name: adapter_name.into(),
        language: "shell".into(),
        transport: DapAdapterTransport::ShellBridge(ShellBridgeSpec {
            shell_bin,
            flavor,
            program: program.clone(),
            args: req.launch.args.clone(),
            cwd: cwd.to_string(),
            env,
        }),
        initialize_args: json!({
            "adapterID": adapter_name, "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path"
        }),
        launch_command: "launch".into(),
        launch_args: json!({ "program": program, "cwd": cwd, "args": req.launch.args }),
        normalized_cwd: cwd.to_string(),
        initial_output: Vec::new(),
    })
}
