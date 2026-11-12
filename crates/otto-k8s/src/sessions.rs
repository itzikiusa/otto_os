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
    /// On a PROD cluster: must equal the pod name ([`check_interactive`]).
    #[serde(default)]
    pub confirm_name: Option<String>,
}

/// `POST /k8s/clusters/{id}/k9s` body.
#[derive(Debug, Clone, Deserialize)]
pub struct K9sReq {
    pub workspace_id: Id,
    pub ns: Option<String>,
    /// On a PROD cluster k9s starts `--readonly` unless a person explicitly
    /// asks for write mode (an agent credential's `write` is ignored).
    #[serde(default)]
    pub write: bool,
    /// Write mode on a PROD cluster: must equal the cluster name.
    #[serde(default)]
    pub confirm_name: Option<String>,
}

/// Whether k9s starts `--readonly`: always on a prod cluster, unless a
/// person (never an agent credential, S6-304) asked for write mode.
pub fn k9s_readonly(env: otto_core::domain::Environment, write: bool) -> bool {
    env == otto_core::domain::Environment::Prod && !(write && otto_core::auth::request_is_person())
}

/// Opening an interactive terminal on a cluster (a shell in a pod, k9s): an
/// agent credential never may — there is no governed exec tool, so no
/// person-approved path exists, and an agent could otherwise type into a prod
/// container over REST (S6-304). On a PROD cluster a person must also send
/// `confirm_name == target` (the pod; the cluster for k9s write mode) when
/// `confirm_needed`.
pub fn check_interactive(
    env: otto_core::domain::Environment,
    confirm_needed: bool,
    confirm_name: Option<&str>,
    target: &str,
    what: &str,
) -> Result<()> {
    if otto_core::auth::request_is_agent() {
        return Err(Error::Forbidden(format!(
            "an agent session's credential cannot open {what} — a person signed in to Otto must \
             do this"
        )));
    }
    if env == otto_core::domain::Environment::Prod
        && confirm_needed
        && confirm_name.map(str::trim) != Some(target)
    {
        return Err(Error::Invalid(format!(
            "confirmation required: {what} on a prod cluster — set confirm_name to \"{target}\""
        )));
    }
    Ok(())
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
    // Flag-shaped names (`--context=…`, `-A`) would re-target kubectl.
    crate::resources::validate_name("namespace", ns)?;
    crate::resources::validate_name("pod", pod)?;
    let mut args = k.argv_stream(["-n", ns, "exec", "-it", pod]);
    if let Some(c) = req
        .container
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        crate::resources::validate_name("container", c)?;
        args.push(format!("--container={c}"));
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
    check_interactive(
        cluster.environment,
        true,
        req.confirm_name.as_deref(),
        req.pod.trim(),
        "a shell in a pod",
    )?;
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
    check_interactive(
        cluster.environment,
        req.write,
        req.confirm_name.as_deref(),
        &cluster.name,
        "k9s with write access",
    )?;
    let bin = install::locate(Tool::K9s, ctx.data_dir())
        .ok_or_else(|| Error::Invalid(cli::not_installed_message("k9s")))?;
    // S11-303: k9s runs the kubeconfig's credential plugin itself — refuse a
    // cluster whose plugin is not allow-listed (the same derivation kubectl
    // calls go through; its overlay is unused here).
    crate::clusters::kubectl_for(ctx, cluster).await?;
    let env = crate::clusters::aws_env_for(ctx, cluster).await?;
    let mut spec = k9s_spec(&bin.to_string_lossy(), cluster, env, req.ns.as_deref());
    if k9s_readonly(cluster.environment, req.write) {
        spec.args.push("--readonly".into());
    }
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
            known_namespaces: Vec::new(),
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
                confirm_name: None,
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
                "--container=web",
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
                confirm_name: None,
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
                command: None,
                confirm_name: None,
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
                command: None,
                confirm_name: None,
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

    #[tokio::test]
    async fn k9s_on_prod_is_readonly_unless_a_person_asks_for_write() {
        use otto_core::auth::{with_request_credential, RequestCredential};
        let person = RequestCredential::default();
        let agent = RequestCredential {
            agent: true,
            root_withheld: false,
        };
        let ro = |cred: RequestCredential, env: Environment, write: bool| {
            with_request_credential(cred, async move { k9s_readonly(env, write) })
        };
        assert!(ro(person, Environment::Prod, false).await);
        assert!(!ro(person, Environment::Prod, true).await);
        assert!(
            ro(agent, Environment::Prod, true).await,
            "agent write ignored"
        );
        assert!(!ro(person, Environment::Dev, false).await);
    }

    /// An agent credential never opens a shell or k9s (S6-304); on prod a
    /// person must type the target, off prod nobody has to.
    #[tokio::test]
    async fn interactive_terminals_are_a_persons_and_prod_needs_the_name() {
        use otto_core::auth::{with_request_credential, RequestCredential};
        let person = RequestCredential::default();
        let agent = RequestCredential {
            agent: true,
            root_withheld: false,
        };
        let check = |cred: RequestCredential, env: Environment, confirm: Option<&'static str>| {
            with_request_credential(cred, async move {
                check_interactive(env, true, confirm, "web-1", "a shell in a pod")
            })
        };
        let refused = check(agent, Environment::Prod, Some("web-1")).await;
        assert!(matches!(refused, Err(Error::Forbidden(_))), "{refused:?}");
        let refused = check(agent, Environment::Dev, None).await;
        assert!(matches!(refused, Err(Error::Forbidden(_))), "{refused:?}");
        assert!(check(person, Environment::Prod, Some("web-1"))
            .await
            .is_ok());
        let missing = check(person, Environment::Prod, None).await;
        assert!(matches!(missing, Err(Error::Invalid(_))), "{missing:?}");
        assert!(check(person, Environment::Dev, None).await.is_ok());
        // k9s read-only (no write) needs no typed name, even on prod.
        let ro = with_request_credential(person, async {
            check_interactive(
                Environment::Prod,
                false,
                None,
                "prod",
                "k9s with write access",
            )
        })
        .await;
        assert!(ro.is_ok());
    }
}
