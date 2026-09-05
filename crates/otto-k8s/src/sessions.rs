//! PTY sessions: `kubectl exec -it` into a pod and the optional k9s tab —
//! both ad-hoc `Spawner::spawn_command(provider = "k8s")` sessions (contract
//! §3.3), so they show up in the session list, stream over the normal terminal
//! WebSocket and are owned by the calling user.
//!
//! The command vectors are built by pure functions ([`exec_spec`], [`k9s_spec`])
//! so tests can pin them; the streaming base flags (no `--request-timeout`)
//! are used because kubectl would otherwise cut the interactive session.

use otto_core::domain::{Session, User};
use otto_core::{Error, Id, Result};
use otto_pty::CommandSpec;
use otto_state::K8sCluster;
use serde::Deserialize;
use serde_json::json;

use crate::cli::{self, Kubectl};
use crate::install::{self, Tool};
use crate::K8sCtx;

/// `POST /k8s/clusters/{id}/exec` body.
#[derive(Debug, Clone, Deserialize)]
pub struct ExecReq {
    pub workspace_id: Id,
    pub ns: String,
    pub pod: String,
    pub container: Option<String>,
    /// Explicit command; default is a bash-or-sh login shell.
    pub command: Option<Vec<String>>,
}

/// `POST /k8s/clusters/{id}/k9s` body.
#[derive(Debug, Clone, Deserialize)]
pub struct K9sReq {
    pub workspace_id: Id,
    pub ns: Option<String>,
}

/// The default shell probe: bash when the image has it, else sh.
pub const DEFAULT_SHELL: &str = "command -v bash >/dev/null && exec bash || exec sh";

/// `kubectl --kubeconfig .. --context .. -n ns exec -it pod [-c c] -- <cmd>`.
pub fn exec_spec(k: &Kubectl, req: &ExecReq) -> Result<CommandSpec> {
    let ns = req.ns.trim();
    let pod = req.pod.trim();
    if ns.is_empty() || pod.is_empty() {
        return Err(Error::Invalid("ns and pod are required".into()));
    }
    if pod.starts_with('-') || pod.contains(char::is_whitespace) {
        return Err(Error::Invalid("invalid pod name".into()));
    }
    let mut args = k.argv_stream(["-n", ns, "exec", "-it", pod]);
    if let Some(c) = req
        .container
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        args.push("-c".into());
        args.push(c.into());
    }
    args.push("--".into());
    match req.command.as_ref().filter(|c| !c.is_empty()) {
        Some(cmd) => args.extend(cmd.iter().cloned()),
        None => args.extend([
            "sh".to_string(),
            "-c".to_string(),
            DEFAULT_SHELL.to_string(),
        ]),
    }
    Ok(CommandSpec {
        program: k.program.clone(),
        args,
        cwd: None,
        env: k.env.clone(),
    })
}

/// `k9s --kubeconfig .. --context .. [-n ns]`.
pub fn k9s_spec(
    k9s_program: &str,
    cluster: &K8sCluster,
    env: Vec<(String, String)>,
    ns: Option<&str>,
) -> CommandSpec {
    let mut args = cli::base_args_stream(cluster);
    if let Some(n) = ns
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .or(cluster.default_namespace.as_deref())
    {
        args.push("-n".into());
        args.push(n.to_string());
    }
    CommandSpec {
        program: k9s_program.to_string(),
        args,
        cwd: None,
        env,
    }
}

/// Spawn the exec PTY session.
pub async fn exec<S: K8sCtx>(
    ctx: &S,
    user: &User,
    cluster: &K8sCluster,
    req: &ExecReq,
) -> Result<Session> {
    crate::access::check(&ctx.pool(), user, &cluster.id, "exec", Some(&req.ns)).await?;
    let k = crate::clusters::kubectl_for(ctx, cluster).await?;
    let spec = exec_spec(&k, req)?;
    let title = format!("{} · {}", req.pod.trim(), req.ns.trim());
    let meta = json!({
        "k8s": {
            "cluster_id": cluster.id,
            "cluster_name": cluster.name,
            "ns": req.ns.trim(),
            "pod": req.pod.trim(),
            "container": req.container.as_deref().map(str::trim).filter(|c| !c.is_empty()),
            "mode": "exec",
        }
    });
    ctx.spawner()
        .spawn_command(&req.workspace_id, &user.id, "k8s", spec, title, Some(meta))
        .await
}

/// Spawn the k9s PTY session (400 `not installed` when k9s is missing).
pub async fn k9s<S: K8sCtx>(
    ctx: &S,
    user: &User,
    cluster: &K8sCluster,
    req: &K9sReq,
) -> Result<Session> {
    crate::access::check_k9s(&ctx.pool(), user, &cluster.id).await?;
    let bin = install::locate(Tool::K9s, ctx.data_dir())
        .ok_or_else(|| Error::Invalid(cli::not_installed_message("k9s")))?;
    let env = crate::clusters::aws_env_for(ctx, cluster).await?;
    let spec = k9s_spec(&bin.to_string_lossy(), cluster, env, req.ns.as_deref());
    let title = format!("k9s · {}", cluster.name);
    let meta = json!({
        "k8s": {
            "cluster_id": cluster.id,
            "cluster_name": cluster.name,
            "ns": req.ns.as_deref().map(str::trim).filter(|n| !n.is_empty()),
            "mode": "k9s",
        }
    });
    ctx.spawner()
        .spawn_command(&req.workspace_id, &user.id, "k8s", spec, title, Some(meta))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use otto_core::domain::Environment;
    use otto_state::K8sClusterSource;

    fn cluster() -> K8sCluster {
        K8sCluster {
            id: "01".into(),
            name: "prod".into(),
            source: K8sClusterSource::Imported,
            kubeconfig_path: Some("/data/kube/01.yaml".into()),
            context_name: "prod-eu".into(),
            default_namespace: Some("shop".into()),
            aws_account_id: None,
            environment: Environment::Prod,
            color: None,
            params: json!({}),
            capabilities: None,
            created_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_used_at: None,
        }
    }

    #[test]
    fn exec_argv_default_shell_and_explicit_command() {
        let k = Kubectl::new(
            "/usr/local/bin/kubectl",
            &cluster(),
            vec![("AWS_PROFILE".into(), "p".into())],
        );
        let spec = exec_spec(
            &k,
            &ExecReq {
                workspace_id: "ws".into(),
                ns: "shop".into(),
                pod: "web-1".into(),
                container: Some("web".into()),
                command: None,
            },
        )
        .unwrap();
        assert_eq!(spec.program, "/usr/local/bin/kubectl");
        assert_eq!(
            spec.args,
            vec![
                "--kubeconfig",
                "/data/kube/01.yaml",
                "--context",
                "prod-eu",
                "-n",
                "shop",
                "exec",
                "-it",
                "web-1",
                "-c",
                "web",
                "--",
                "sh",
                "-c",
                DEFAULT_SHELL
            ]
        );
        assert!(
            !spec.args.contains(&"--request-timeout".to_string()),
            "streams must not carry a request timeout"
        );
        assert_eq!(spec.env, vec![("AWS_PROFILE".to_string(), "p".to_string())]);

        let spec = exec_spec(
            &k,
            &ExecReq {
                workspace_id: "ws".into(),
                ns: "shop".into(),
                pod: "web-1".into(),
                container: None,
                command: Some(vec!["redis-cli".into(), "-n".into(), "2".into()]),
            },
        )
        .unwrap();
        assert_eq!(
            &spec.args[4..],
            &[
                "-n",
                "shop",
                "exec",
                "-it",
                "web-1",
                "--",
                "redis-cli",
                "-n",
                "2"
            ]
        );
        assert!(exec_spec(
            &k,
            &ExecReq {
                workspace_id: "w".into(),
                ns: "".into(),
                pod: "p".into(),
                container: None,
                command: None
            }
        )
        .is_err());
        assert!(exec_spec(
            &k,
            &ExecReq {
                workspace_id: "w".into(),
                ns: "shop".into(),
                pod: "--evil".into(),
                container: None,
                command: None
            }
        )
        .is_err());
    }

    #[test]
    fn k9s_argv() {
        let spec = k9s_spec(
            "/opt/homebrew/bin/k9s",
            &cluster(),
            vec![],
            Some("kube-system"),
        );
        assert_eq!(spec.program, "/opt/homebrew/bin/k9s");
        assert_eq!(
            spec.args,
            vec![
                "--kubeconfig",
                "/data/kube/01.yaml",
                "--context",
                "prod-eu",
                "-n",
                "kube-system"
            ]
        );
        // Falls back to the cluster's default namespace, then to none.
        assert_eq!(
            k9s_spec("k9s", &cluster(), vec![], None).args[4..],
            ["-n", "shop"]
        );
        let mut c = cluster();
        c.default_namespace = None;
        c.kubeconfig_path = None;
        assert_eq!(
            k9s_spec("k9s", &c, vec![], Some(" ")).args,
            vec!["--context", "prod-eu"]
        );
    }
}
