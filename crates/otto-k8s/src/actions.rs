//! Mutating actions (`POST /k8s/clusters/{id}/actions`) — every verb is a
//! kubectl invocation, planned as an argv list first ([`plan`], pure) and only
//! then executed ([`execute`]). The plan is what the unit tests pin down: for
//! each action in contract §4.6 the exact argv, including the patch JSON.
//!
//! Argo CD / Argo Rollouts are driven through their CRDs (no `argocd` CLI, no
//! `kubectl-argo-rollouts` plugin): rollouts' promote/abort/retry write the
//! `status` subresource exactly like the plugin does, and an Argo CD sync is
//! the same `operation` stanza `argocd --core app sync` writes.
//!
//! Destructive verbs (`delete_pod`, `scale` to 0, `rollout_undo`, `argocd_sync`
//! with prune) require `params.confirm_name == name` — the UI collects a typed
//! confirmation and the MCP tool only sets it after an explicit user yes.

use chrono::{DateTime, SecondsFormat, Utc};
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::cli::Kubectl;
use crate::resources::{self, Kind};

/// `POST …/actions` body (contract §3.3 `K8sActionReq`).
#[derive(Debug, Clone, Deserialize)]
pub struct K8sActionReq {
    pub action: String,
    pub kind: String,
    pub ns: String,
    pub name: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct K8sActionResp {
    pub ok: bool,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

/// A planned action: kubectl argv lists (after the base flags) run in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<Vec<String>>,
    pub message: String,
    /// `rollout_status`: a non-zero exit means "not finished yet", not failure.
    pub tolerate_nonzero: bool,
}

/// Inputs the planner needs from the live cluster (fetched by [`execute`]).
#[derive(Debug, Default)]
pub struct PlanInputs<'a> {
    /// The Argo CD Application manifest (for `argocd_sync` / `argocd_app_restart`).
    pub application: Option<&'a Value>,
}

fn params(req: &K8sActionReq) -> &Value {
    static EMPTY: Value = Value::Null;
    req.params.as_ref().unwrap_or(&EMPTY)
}

fn p_str<'a>(req: &'a K8sActionReq, key: &str) -> Option<&'a str> {
    params(req)
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn p_bool(req: &K8sActionReq, key: &str) -> bool {
    match params(req).get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => matches!(s.as_str(), "true" | "1" | "yes"),
        _ => false,
    }
}

fn p_i64(req: &K8sActionReq, key: &str) -> Option<i64> {
    match params(req).get(key) {
        Some(Value::Number(n)) => n.as_i64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

fn require_confirm(req: &K8sActionReq, what: &str) -> Result<()> {
    if p_str(req, "confirm_name") == Some(req.name.as_str()) {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "confirmation required: {what} — set params.confirm_name to \"{}\"",
            req.name
        )))
    }
}

fn target(kind: Kind, name: &str) -> String {
    format!("{}/{}", kind.kubectl_resource(), name)
}

fn ns_args(ns: &str) -> Vec<String> {
    vec!["-n".into(), ns.into()]
}

fn patch(kind: Kind, name: &str, ns: &str, body: Value) -> Vec<String> {
    let mut a: Vec<String> = vec!["patch".into(), target(kind, name)];
    a.extend(ns_args(ns));
    a.extend([
        "--type".into(),
        "merge".into(),
        "-p".into(),
        body.to_string(),
    ]);
    a
}

fn status_patch(kind: Kind, name: &str, ns: &str, body: Value) -> Vec<String> {
    let mut a: Vec<String> = vec!["patch".into(), target(kind, name)];
    a.extend(ns_args(ns));
    a.extend([
        "--subresource=status".into(),
        "--type".into(),
        "merge".into(),
        "-p".into(),
        body.to_string(),
    ]);
    a
}

fn restart_step(kind: Kind, name: &str, ns: &str, now: DateTime<Utc>) -> Result<Vec<String>> {
    Ok(match kind {
        Kind::Deployments | Kind::Statefulsets | Kind::Daemonsets => {
            let mut a: Vec<String> = vec!["rollout".into(), "restart".into(), target(kind, name)];
            a.extend(ns_args(ns));
            a
        }
        Kind::Rollouts => patch(
            kind,
            name,
            ns,
            json!({"spec": {"restartAt": now.to_rfc3339_opts(SecondsFormat::Secs, true)}}),
        ),
        other => return Err(unsupported("restart", other)),
    })
}

fn unsupported(action: &str, kind: Kind) -> Error {
    Error::Invalid(format!(
        "action '{action}' does not apply to {}",
        kind.as_str()
    ))
}

/// Does this action need the Application manifest before planning?
pub fn needs_application(action: &str) -> bool {
    matches!(action, "argocd_sync" | "argocd_app_restart")
}

/// Build the kubectl steps for `req` (contract §4.6). Pure.
pub fn plan(req: &K8sActionReq, now: DateTime<Utc>, inputs: &PlanInputs<'_>) -> Result<Plan> {
    let kind = Kind::parse(&req.kind)
        .ok_or_else(|| Error::Invalid(format!("unknown kind '{}'", req.kind)))?;
    let name = req.name.trim();
    let ns = req.ns.trim();
    if name.is_empty() {
        return Err(Error::Invalid("name is required".into()));
    }
    if ns.is_empty() && kind.namespaced() {
        return Err(Error::Invalid("ns is required".into()));
    }
    if name.contains(char::is_whitespace) || name.starts_with('-') {
        return Err(Error::Invalid("invalid resource name".into()));
    }
    let action = req.action.trim();
    let one = |step: Vec<String>, msg: String| Plan {
        steps: vec![step],
        message: msg,
        tolerate_nonzero: false,
    };
    Ok(match action {
        "restart" => one(
            restart_step(kind, name, ns, now)?,
            format!("restart requested for {}/{name}", kind.as_str()),
        ),
        "scale" => {
            if !matches!(
                kind,
                Kind::Deployments | Kind::Statefulsets | Kind::Rollouts | Kind::Replicasets
            ) {
                return Err(unsupported(action, kind));
            }
            let replicas = p_i64(req, "replicas").ok_or_else(|| {
                Error::Invalid("params.replicas (integer ≥ 0) is required".into())
            })?;
            if !(0..=10_000).contains(&replicas) {
                return Err(Error::Invalid(
                    "params.replicas must be between 0 and 10000".into(),
                ));
            }
            if replicas == 0 {
                require_confirm(req, "scaling to 0 stops every pod")?;
            }
            let mut a: Vec<String> = vec!["scale".into(), target(kind, name)];
            a.extend(ns_args(ns));
            a.push(format!("--replicas={replicas}"));
            one(a, format!("{}/{name} scaled to {replicas}", kind.as_str()))
        }
        "delete_pod" => {
            if kind != Kind::Pods {
                return Err(unsupported(action, kind));
            }
            require_confirm(req, "deleting a pod")?;
            let mut a: Vec<String> = vec!["delete".into(), target(kind, name)];
            a.extend(ns_args(ns));
            if let Some(g) = p_i64(req, "grace") {
                if g < 0 {
                    return Err(Error::Invalid("params.grace must be ≥ 0".into()));
                }
                a.push(format!("--grace-period={g}"));
                if g == 0 {
                    // kubectl refuses --grace-period=0 without --force.
                    a.push("--force".into());
                }
            }
            one(a, format!("pod {name} deleted"))
        }
        "rollout_status" => {
            if !matches!(
                kind,
                Kind::Deployments | Kind::Statefulsets | Kind::Daemonsets
            ) {
                return Err(unsupported(action, kind));
            }
            let mut a: Vec<String> = vec!["rollout".into(), "status".into(), target(kind, name)];
            a.extend(ns_args(ns));
            a.push("--timeout=5s".into());
            Plan {
                steps: vec![a],
                message: format!("rollout status of {}/{name}", kind.as_str()),
                tolerate_nonzero: true,
            }
        }
        "rollout_undo" => {
            if !matches!(
                kind,
                Kind::Deployments | Kind::Statefulsets | Kind::Daemonsets
            ) {
                return Err(unsupported(action, kind));
            }
            require_confirm(req, "rolling back")?;
            let mut a: Vec<String> = vec!["rollout".into(), "undo".into(), target(kind, name)];
            a.extend(ns_args(ns));
            if let Some(r) = p_i64(req, "to_revision") {
                a.push(format!("--to-revision={r}"));
            }
            one(a, format!("{}/{name} rolled back", kind.as_str()))
        }
        "rollout_pause" | "rollout_resume" => {
            let pause = action == "rollout_pause";
            match kind {
                Kind::Deployments => {
                    let mut a: Vec<String> = vec![
                        "rollout".into(),
                        if pause {
                            "pause".into()
                        } else {
                            "resume".into()
                        },
                        target(kind, name),
                    ];
                    a.extend(ns_args(ns));
                    one(
                        a,
                        format!(
                            "deployment {name} {}",
                            if pause { "paused" } else { "resumed" }
                        ),
                    )
                }
                Kind::Rollouts => one(
                    patch(kind, name, ns, json!({"spec": {"paused": pause}})),
                    format!(
                        "rollout {name} {}",
                        if pause { "paused" } else { "resumed" }
                    ),
                ),
                other => return Err(unsupported(action, other)),
            }
        }
        "rollout_promote" => {
            if kind != Kind::Rollouts {
                return Err(unsupported(action, kind));
            }
            let full = p_bool(req, "full");
            // Mirrors `kubectl argo rollouts promote`: clear the pause conditions
            // (status subresource), unpause the spec (an indefinite `pause{}`
            // step sets BOTH), and for --full flag promoteFull.
            let mut steps = vec![status_patch(
                kind,
                name,
                ns,
                json!({"status": {"pauseConditions": null}}),
            )];
            if full {
                steps.push(status_patch(
                    kind,
                    name,
                    ns,
                    json!({"status": {"promoteFull": true}}),
                ));
            }
            steps.push(patch(kind, name, ns, json!({"spec": {"paused": false}})));
            Plan {
                steps,
                message: format!(
                    "rollout {name} promoted{}",
                    if full { " (full)" } else { "" }
                ),
                tolerate_nonzero: false,
            }
        }
        "rollout_abort" | "rollout_retry" => {
            if kind != Kind::Rollouts {
                return Err(unsupported(action, kind));
            }
            let abort = action == "rollout_abort";
            one(
                status_patch(kind, name, ns, json!({"status": {"abort": abort}})),
                format!(
                    "rollout {name} {}",
                    if abort { "aborted" } else { "retrying" }
                ),
            )
        }
        "argocd_sync" => {
            if kind != Kind::Applications {
                return Err(unsupported(action, kind));
            }
            let prune = p_bool(req, "prune");
            if prune {
                require_confirm(req, "sync with prune deletes resources")?;
            }
            let revision = p_str(req, "revision")
                .map(str::to_string)
                .or_else(|| {
                    inputs
                        .application
                        .and_then(|a| a.pointer("/spec/source/targetRevision"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "HEAD".to_string());
            let body = json!({
                "operation": {
                    "initiatedBy": {"username": "otto"},
                    "sync": {
                        "revision": revision,
                        "prune": prune,
                        "syncStrategy": {"hook": {}}
                    }
                }
            });
            one(
                patch(kind, name, ns, body),
                format!(
                    "sync requested for application {name} @ {revision}{}",
                    if prune { " (prune)" } else { "" }
                ),
            )
        }
        "argocd_refresh" => {
            if kind != Kind::Applications {
                return Err(unsupported(action, kind));
            }
            let hard = p_bool(req, "hard");
            let mut a: Vec<String> = vec!["annotate".into(), target(kind, name)];
            a.extend(ns_args(ns));
            a.push(format!(
                "argocd.argoproj.io/refresh={}",
                if hard { "hard" } else { "normal" }
            ));
            a.push("--overwrite".into());
            one(
                a,
                format!(
                    "{}refresh requested for application {name}",
                    if hard { "hard " } else { "" }
                ),
            )
        }
        "argocd_terminate_op" => {
            if kind != Kind::Applications {
                return Err(unsupported(action, kind));
            }
            let mut a: Vec<String> = vec!["patch".into(), target(kind, name)];
            a.extend(ns_args(ns));
            a.extend([
                "--type".into(),
                "json".into(),
                "-p".into(),
                json!([{"op": "remove", "path": "/operation"}]).to_string(),
            ]);
            one(a, format!("operation terminated on application {name}"))
        }
        "argocd_app_restart" => {
            if kind != Kind::Applications {
                return Err(unsupported(action, kind));
            }
            let app = inputs.application.ok_or_else(|| {
                Error::Internal("application manifest required to plan a restart".into())
            })?;
            let want_kind = p_str(req, "resource_kind").map(|k| k.to_ascii_lowercase());
            let want_name = p_str(req, "resource_name");
            let dest_ns = app
                .pointer("/spec/destination/namespace")
                .and_then(Value::as_str)
                .unwrap_or(ns);
            let mut steps = Vec::new();
            let mut restarted = Vec::new();
            for r in app
                .pointer("/status/resources")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                let rk = r.get("kind").and_then(Value::as_str).unwrap_or("");
                let Some(k) = Kind::from_k8s_kind(rk) else {
                    continue;
                };
                if !matches!(
                    k,
                    Kind::Deployments | Kind::Statefulsets | Kind::Daemonsets | Kind::Rollouts
                ) {
                    continue;
                }
                if let Some(w) = &want_kind {
                    if w != &rk.to_ascii_lowercase() && w != k.as_str() {
                        continue;
                    }
                }
                let rn = r.get("name").and_then(Value::as_str).unwrap_or("");
                if rn.is_empty() {
                    continue;
                }
                if let Some(w) = want_name {
                    if w != rn {
                        continue;
                    }
                }
                let rns = r
                    .get("namespace")
                    .and_then(Value::as_str)
                    .unwrap_or(dest_ns);
                steps.push(restart_step(k, rn, rns, now)?);
                restarted.push(format!("{rk}/{rn}"));
            }
            if steps.is_empty() {
                return Err(Error::Invalid(
                    "application has no Deployment/StatefulSet/DaemonSet/Rollout resources to restart".into(),
                ));
            }
            Plan {
                steps,
                message: format!("restarted {}", restarted.join(", ")),
                tolerate_nonzero: false,
            }
        }
        "cronjob_trigger" => {
            if kind != Kind::Cronjobs {
                return Err(unsupported(action, kind));
            }
            let job = manual_job_name(name, now);
            let mut a: Vec<String> = vec![
                "create".into(),
                "job".into(),
                format!("--from=cronjob/{name}"),
                job.clone(),
            ];
            a.extend(ns_args(ns));
            one(a, format!("job {job} created from cronjob {name}"))
        }
        "cronjob_suspend" | "cronjob_resume" => {
            if kind != Kind::Cronjobs {
                return Err(unsupported(action, kind));
            }
            let suspend = action == "cronjob_suspend";
            one(
                patch(kind, name, ns, json!({"spec": {"suspend": suspend}})),
                format!(
                    "cronjob {name} {}",
                    if suspend { "suspended" } else { "resumed" }
                ),
            )
        }
        other => return Err(Error::Invalid(format!("unknown action '{other}'"))),
    })
}

/// `<name>-manual-<unix ts>` trimmed to the 63-char DNS label limit.
pub fn manual_job_name(cronjob: &str, now: DateTime<Utc>) -> String {
    let suffix = format!("-manual-{}", now.timestamp());
    let max_base = 63usize.saturating_sub(suffix.len());
    let base: String = cronjob.chars().take(max_base).collect();
    format!("{}{suffix}", base.trim_end_matches('-'))
}

/// Plan + run. Fetches the Application manifest first when the action needs it.
pub async fn execute(k: &Kubectl, req: &K8sActionReq) -> Result<K8sActionResp> {
    let now = Utc::now();
    let app = if needs_application(req.action.trim()) {
        Some(resources::get_one(k, Kind::Applications, Some(&req.ns), req.name.trim()).await?)
    } else {
        None
    };
    let plan = plan(
        req,
        now,
        &PlanInputs {
            application: app.as_ref(),
        },
    )?;
    let mut outputs = Vec::new();
    let mut ok = true;
    for step in &plan.steps {
        if plan.tolerate_nonzero {
            let argv = k.argv(step.iter().cloned());
            let out =
                crate::cli::run_raw(&k.program, &argv, &k.env, crate::cli::DEFAULT_TIMEOUT, None)
                    .await?;
            if out.status != 0 {
                // Forbidden still maps to 403 even in tolerant mode.
                if out.stderr.to_ascii_lowercase().contains("forbidden") {
                    return Err(crate::cli::classify_failure(&k.program, &out.stderr));
                }
                ok = false;
            }
            let mut text = out.stdout;
            if !out.stderr.trim().is_empty() {
                text.push_str(&crate::cli::redact(out.stderr.trim()));
            }
            outputs.push(text);
        } else {
            let out = k.run(step.iter().cloned()).await?;
            outputs.push(out.stdout);
        }
    }
    let output = outputs
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(K8sActionResp {
        ok,
        message: if ok {
            plan.message
        } else {
            format!("{} — not complete", plan.message)
        },
        output: (!output.is_empty()).then_some(output),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn req(action: &str, kind: &str, name: &str, params: Value) -> K8sActionReq {
        K8sActionReq {
            action: action.into(),
            kind: kind.into(),
            ns: "shop".into(),
            name: name.into(),
            params: Some(params),
        }
    }

    fn steps(action: &str, kind: &str, name: &str, params: Value) -> Vec<Vec<String>> {
        plan(
            &req(action, kind, name, params),
            now(),
            &PlanInputs::default(),
        )
        .unwrap_or_else(|e| panic!("{action}/{kind}: {e}"))
        .steps
    }

    /// Split a patch step into (argv-before-json, parsed json).
    fn split_patch(step: &[String]) -> (Vec<&str>, Value) {
        let (json, head) = step.split_last().unwrap();
        (
            head.iter().map(String::as_str).collect(),
            serde_json::from_str(json).unwrap(),
        )
    }

    #[test]
    fn restart_variants() {
        assert_eq!(
            steps("restart", "deployments", "web", json!({})),
            vec![vec!["rollout", "restart", "deployments/web", "-n", "shop"]]
        );
        assert_eq!(
            steps("restart", "statefulsets", "redis", json!({}))[0],
            vec!["rollout", "restart", "statefulsets/redis", "-n", "shop"]
        );
        assert_eq!(
            steps("restart", "daemonsets", "fluentd", json!({}))[0],
            vec!["rollout", "restart", "daemonsets/fluentd", "-n", "shop"]
        );
        let s = steps("restart", "rollouts", "checkout", json!({}));
        let (head, body) = split_patch(&s[0]);
        assert_eq!(
            head,
            vec![
                "patch",
                "rollouts.argoproj.io/checkout",
                "-n",
                "shop",
                "--type",
                "merge",
                "-p"
            ]
        );
        assert_eq!(body, json!({"spec": {"restartAt": "2026-09-01T12:00:00Z"}}));
        assert!(matches!(
            plan(&req("restart", "pods", "p", json!({})), now(), &PlanInputs::default()),
            Err(Error::Invalid(m)) if m.contains("does not apply")
        ));
    }

    #[test]
    fn scale_requires_replicas_and_confirms_zero() {
        assert_eq!(
            steps("scale", "deployments", "web", json!({"replicas": 3})),
            vec![vec![
                "scale",
                "deployments/web",
                "-n",
                "shop",
                "--replicas=3"
            ]]
        );
        assert_eq!(
            steps("scale", "rollouts", "checkout", json!({"replicas": "2"}))[0],
            vec![
                "scale",
                "rollouts.argoproj.io/checkout",
                "-n",
                "shop",
                "--replicas=2"
            ]
        );
        assert!(matches!(
            plan(&req("scale", "deployments", "web", json!({})), now(), &PlanInputs::default()),
            Err(Error::Invalid(m)) if m.contains("replicas")
        ));
        assert!(matches!(
            plan(&req("scale", "deployments", "web", json!({"replicas": 0})), now(), &PlanInputs::default()),
            Err(Error::Invalid(m)) if m.starts_with("confirmation required")
        ));
        assert_eq!(
            steps(
                "scale",
                "deployments",
                "web",
                json!({"replicas": 0, "confirm_name": "web"})
            )[0],
            vec!["scale", "deployments/web", "-n", "shop", "--replicas=0"]
        );
        assert!(plan(
            &req("scale", "daemonsets", "x", json!({"replicas": 1})),
            now(),
            &PlanInputs::default()
        )
        .is_err());
    }

    #[test]
    fn delete_pod_confirm_and_grace() {
        assert!(matches!(
            plan(&req("delete_pod", "pods", "web-1", json!({})), now(), &PlanInputs::default()),
            Err(Error::Invalid(m)) if m.contains("confirm_name")
        ));
        assert!(matches!(
            plan(
                &req(
                    "delete_pod",
                    "pods",
                    "web-1",
                    json!({"confirm_name": "web-2"})
                ),
                now(),
                &PlanInputs::default()
            ),
            Err(Error::Invalid(_))
        ));
        assert_eq!(
            steps(
                "delete_pod",
                "pods",
                "web-1",
                json!({"confirm_name": "web-1"})
            ),
            vec![vec!["delete", "pods/web-1", "-n", "shop"]]
        );
        assert_eq!(
            steps(
                "delete_pod",
                "pods",
                "web-1",
                json!({"confirm_name": "web-1", "grace": 30})
            )[0],
            vec!["delete", "pods/web-1", "-n", "shop", "--grace-period=30"]
        );
        assert_eq!(
            steps(
                "delete_pod",
                "pods",
                "web-1",
                json!({"confirm_name": "web-1", "grace": 0})
            )[0],
            vec![
                "delete",
                "pods/web-1",
                "-n",
                "shop",
                "--grace-period=0",
                "--force"
            ]
        );
        assert!(plan(
            &req(
                "delete_pod",
                "deployments",
                "web",
                json!({"confirm_name": "web"})
            ),
            now(),
            &PlanInputs::default()
        )
        .is_err());
    }

    #[test]
    fn rollout_verbs_on_workloads() {
        let p = plan(
            &req("rollout_status", "deployments", "web", json!({})),
            now(),
            &PlanInputs::default(),
        )
        .unwrap();
        assert_eq!(
            p.steps,
            vec![vec![
                "rollout",
                "status",
                "deployments/web",
                "-n",
                "shop",
                "--timeout=5s"
            ]]
        );
        assert!(p.tolerate_nonzero);

        assert!(matches!(
            plan(&req("rollout_undo", "deployments", "web", json!({})), now(), &PlanInputs::default()),
            Err(Error::Invalid(m)) if m.starts_with("confirmation required")
        ));
        assert_eq!(
            steps(
                "rollout_undo",
                "deployments",
                "web",
                json!({"confirm_name": "web"})
            ),
            vec![vec!["rollout", "undo", "deployments/web", "-n", "shop"]]
        );
        assert_eq!(
            steps(
                "rollout_undo",
                "statefulsets",
                "db",
                json!({"confirm_name": "db", "to_revision": 4})
            )[0],
            vec![
                "rollout",
                "undo",
                "statefulsets/db",
                "-n",
                "shop",
                "--to-revision=4"
            ]
        );
        assert_eq!(
            steps("rollout_pause", "deployments", "web", json!({})),
            vec![vec!["rollout", "pause", "deployments/web", "-n", "shop"]]
        );
        assert_eq!(
            steps("rollout_resume", "deployments", "web", json!({})),
            vec![vec!["rollout", "resume", "deployments/web", "-n", "shop"]]
        );
        assert!(plan(
            &req("rollout_pause", "statefulsets", "db", json!({})),
            now(),
            &PlanInputs::default()
        )
        .is_err());
    }

    #[test]
    fn argo_rollout_verbs_use_status_subresource() {
        let s = steps("rollout_pause", "rollouts", "checkout", json!({}));
        let (head, body) = split_patch(&s[0]);
        assert_eq!(
            head,
            vec![
                "patch",
                "rollouts.argoproj.io/checkout",
                "-n",
                "shop",
                "--type",
                "merge",
                "-p"
            ]
        );
        assert_eq!(body, json!({"spec": {"paused": true}}));
        let (_, body) = split_patch(&steps("rollout_resume", "rollouts", "checkout", json!({}))[0]);
        assert_eq!(body, json!({"spec": {"paused": false}}));

        let s = steps("rollout_promote", "rollouts", "checkout", json!({}));
        assert_eq!(s.len(), 2);
        let (head, body) = split_patch(&s[0]);
        assert_eq!(
            head,
            vec![
                "patch",
                "rollouts.argoproj.io/checkout",
                "-n",
                "shop",
                "--subresource=status",
                "--type",
                "merge",
                "-p"
            ]
        );
        assert_eq!(body, json!({"status": {"pauseConditions": null}}));
        let (_, body) = split_patch(&s[1]);
        assert_eq!(body, json!({"spec": {"paused": false}}));

        let s = steps(
            "rollout_promote",
            "rollouts",
            "checkout",
            json!({"full": true}),
        );
        assert_eq!(s.len(), 3);
        let (head, body) = split_patch(&s[1]);
        assert!(head.contains(&"--subresource=status"));
        assert_eq!(body, json!({"status": {"promoteFull": true}}));
        let (_, body) = split_patch(&s[2]);
        assert_eq!(body, json!({"spec": {"paused": false}}));

        let abort = steps("rollout_abort", "rollouts", "checkout", json!({}));
        let (head, body) = split_patch(&abort[0]);
        assert!(head.contains(&"--subresource=status"));
        assert_eq!(body, json!({"status": {"abort": true}}));
        let (_, body) = split_patch(&steps("rollout_retry", "rollouts", "checkout", json!({}))[0]);
        assert_eq!(body, json!({"status": {"abort": false}}));
        assert!(plan(
            &req("rollout_abort", "deployments", "web", json!({})),
            now(),
            &PlanInputs::default()
        )
        .is_err());
    }

    #[test]
    fn argocd_sync_refresh_terminate() {
        let app: Value =
            serde_json::from_str(include_str!("../testdata/applications.json")).unwrap();
        let app = &app["items"][0];
        let inputs = PlanInputs {
            application: Some(app),
        };
        let r = K8sActionReq {
            ns: "argocd".into(),
            ..req("argocd_sync", "applications", "shop-prod", json!({}))
        };
        let p = plan(&r, now(), &inputs).unwrap();
        let (head, body) = split_patch(&p.steps[0]);
        assert_eq!(
            head,
            vec![
                "patch",
                "applications.argoproj.io/shop-prod",
                "-n",
                "argocd",
                "--type",
                "merge",
                "-p"
            ]
        );
        assert_eq!(
            body,
            json!({"operation": {"initiatedBy": {"username": "otto"}, "sync": {"revision": "main", "prune": false, "syncStrategy": {"hook": {}}}}})
        );
        // Explicit revision wins; prune needs the typed confirm.
        let r = req(
            "argocd_sync",
            "applications",
            "shop-prod",
            json!({"revision": "v1.2.3", "prune": true}),
        );
        assert!(
            matches!(plan(&r, now(), &inputs), Err(Error::Invalid(m)) if m.starts_with("confirmation required"))
        );
        let r = req(
            "argocd_sync",
            "applications",
            "shop-prod",
            json!({"revision": "v1.2.3", "prune": true, "confirm_name": "shop-prod"}),
        );
        let (_, body) = split_patch(&plan(&r, now(), &inputs).unwrap().steps[0]);
        assert_eq!(body["operation"]["sync"]["revision"], "v1.2.3");
        assert_eq!(body["operation"]["sync"]["prune"], true);
        // No manifest and no revision ⇒ HEAD.
        let (_, body) = split_patch(&steps("argocd_sync", "applications", "x", json!({}))[0]);
        assert_eq!(body["operation"]["sync"]["revision"], "HEAD");

        assert_eq!(
            steps("argocd_refresh", "applications", "shop-prod", json!({})),
            vec![vec![
                "annotate",
                "applications.argoproj.io/shop-prod",
                "-n",
                "shop",
                "argocd.argoproj.io/refresh=normal",
                "--overwrite"
            ]]
        );
        assert_eq!(
            steps(
                "argocd_refresh",
                "applications",
                "shop-prod",
                json!({"hard": true})
            )[0][4],
            "argocd.argoproj.io/refresh=hard"
        );
        let s = steps(
            "argocd_terminate_op",
            "applications",
            "shop-prod",
            json!({}),
        );
        let (head, body) = split_patch(&s[0]);
        assert_eq!(
            head,
            vec![
                "patch",
                "applications.argoproj.io/shop-prod",
                "-n",
                "shop",
                "--type",
                "json",
                "-p"
            ]
        );
        assert_eq!(body, json!([{"op": "remove", "path": "/operation"}]));
        assert!(plan(
            &req("argocd_refresh", "deployments", "web", json!({})),
            now(),
            &PlanInputs::default()
        )
        .is_err());
    }

    #[test]
    fn argocd_app_restart_iterates_status_resources() {
        let app: Value =
            serde_json::from_str(include_str!("../testdata/applications.json")).unwrap();
        let app = &app["items"][0];
        let inputs = PlanInputs {
            application: Some(app),
        };
        let r = K8sActionReq {
            ns: "argocd".into(),
            ..req("argocd_app_restart", "applications", "shop-prod", json!({}))
        };
        let p = plan(&r, now(), &inputs).unwrap();
        assert_eq!(
            p.steps.len(),
            3,
            "Deployment + Rollout + StatefulSet; Service skipped"
        );
        assert_eq!(
            p.steps[0],
            vec!["rollout", "restart", "deployments/web", "-n", "shop"]
        );
        let (head, body) = split_patch(&p.steps[1]);
        assert_eq!(head[1], "rollouts.argoproj.io/checkout");
        assert_eq!(body, json!({"spec": {"restartAt": "2026-09-01T12:00:00Z"}}));
        assert_eq!(
            p.steps[2],
            vec![
                "rollout",
                "restart",
                "statefulsets/redis",
                "-n",
                "shop-data"
            ],
            "resource ns wins over app ns"
        );
        assert_eq!(
            p.message,
            "restarted Deployment/web, Rollout/checkout, StatefulSet/redis"
        );

        let r = K8sActionReq {
            ns: "argocd".into(),
            ..req(
                "argocd_app_restart",
                "applications",
                "shop-prod",
                json!({"resource_kind": "Deployment"}),
            )
        };
        let p = plan(&r, now(), &inputs).unwrap();
        assert_eq!(p.steps.len(), 1);
        let r = K8sActionReq {
            ns: "argocd".into(),
            ..req(
                "argocd_app_restart",
                "applications",
                "shop-prod",
                json!({"resource_kind": "rollouts"}),
            )
        };
        assert_eq!(plan(&r, now(), &inputs).unwrap().steps.len(), 1);
        let r = K8sActionReq {
            ns: "argocd".into(),
            ..req(
                "argocd_app_restart",
                "applications",
                "shop-prod",
                json!({"resource_kind": "Job"}),
            )
        };
        assert!(
            matches!(plan(&r, now(), &inputs), Err(Error::Invalid(m)) if m.contains("no Deployment"))
        );
        assert!(
            needs_application("argocd_app_restart")
                && needs_application("argocd_sync")
                && !needs_application("restart")
        );
    }

    #[test]
    fn cronjob_verbs() {
        assert_eq!(
            steps("cronjob_trigger", "cronjobs", "nightly", json!({})),
            vec![vec![
                "create",
                "job",
                "--from=cronjob/nightly",
                "nightly-manual-1788264000",
                "-n",
                "shop"
            ]]
        );
        let suspend = steps("cronjob_suspend", "cronjobs", "nightly", json!({}));
        let (head, body) = split_patch(&suspend[0]);
        assert_eq!(
            head,
            vec![
                "patch",
                "cronjobs/nightly",
                "-n",
                "shop",
                "--type",
                "merge",
                "-p"
            ]
        );
        assert_eq!(body, json!({"spec": {"suspend": true}}));
        let (_, body) = split_patch(&steps("cronjob_resume", "cronjobs", "nightly", json!({}))[0]);
        assert_eq!(body, json!({"spec": {"suspend": false}}));
        let long = "a".repeat(80);
        let n = manual_job_name(&long, now());
        assert!(n.len() <= 63, "{n}");
        assert!(n.ends_with("-manual-1788264000"));
        assert!(plan(
            &req("cronjob_trigger", "jobs", "j", json!({})),
            now(),
            &PlanInputs::default()
        )
        .is_err());
    }

    #[test]
    fn rejects_unknown_and_malformed() {
        assert!(matches!(
            plan(&req("explode", "pods", "p", json!({})), now(), &PlanInputs::default()),
            Err(Error::Invalid(m)) if m.contains("unknown action")
        ));
        assert!(matches!(
            plan(&req("restart", "widgets", "p", json!({})), now(), &PlanInputs::default()),
            Err(Error::Invalid(m)) if m.contains("unknown kind")
        ));
        assert!(plan(
            &req("restart", "deployments", "--bad", json!({})),
            now(),
            &PlanInputs::default()
        )
        .is_err());
        assert!(plan(
            &req("restart", "deployments", "a b", json!({})),
            now(),
            &PlanInputs::default()
        )
        .is_err());
        let mut r = req("restart", "deployments", "web", json!({}));
        r.ns = "".into();
        assert!(plan(&r, now(), &PlanInputs::default()).is_err());
        // params may be absent entirely.
        let r = K8sActionReq {
            params: None,
            ..req("restart", "deployments", "web", json!({}))
        };
        assert!(plan(&r, now(), &PlanInputs::default()).is_ok());
    }
}
