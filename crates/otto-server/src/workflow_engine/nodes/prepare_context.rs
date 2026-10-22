//! The `prepare_context` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall {
        ctx,
        ws,
        user,
        node,
        input,
        env,
        scope,
        session_tx,
        log_tx,
        activity_tx,
        ..
    } = call;
    let run_cwd: &str = env.run_cwd.as_str();
    let p = &node.params;
    let mut jira = serde_json::Map::new();
    match crate::workflow_prepare::extract_jira_key(p, &input) {
        None => {
            jira.insert("found".into(), json!(false));
        }
        Some(key) => {
            jira.insert("found".into(), json!(true));
            jira.insert("key".into(), json!(key));
            let fetched = match crate::workflow_prepare::resolve_jira_accounts(
                ctx,
                &user.id,
                p.get("account_id").and_then(Value::as_str),
            )
            .await
            {
                // Several Jira sites: the first account that knows the key.
                Ok(accounts) => {
                    let mut last = Err(format!("no Jira account could read {key}"));
                    for account in accounts {
                        let token = otto_core::secrets::get_async(&ctx.secrets, &account.token_ref)
                            .await
                            .ok()
                            .flatten();
                        last = match token {
                            Some(t) => otto_issues::Tracker::for_account(&account, &t)
                                .get_issue_full(&key)
                                .await
                                .map_err(|e| e.to_string()),
                            None => Err(format!("missing token for issue account {}", account.id)),
                        };
                        if last.is_ok() {
                            break;
                        }
                    }
                    last
                }
                Err(e) => Err(e),
            };
            match fetched {
                Ok(issue) => {
                    env.files.write_named(
                        &format!("jira-{key}.md"),
                        &crate::workflow_prepare::render_issue_md(&issue),
                    );
                    jira.insert("fetched".into(), json!(true));
                    jira.insert("summary".into(), json!(issue.summary));
                    jira.insert("status".into(), json!(issue.status));
                    jira.insert("url".into(), json!(issue.url));
                }
                Err(e) => {
                    // Indicate loudly, don't die (unless required): every later
                    // step + the human must see the ticket data is missing.
                    env.files.write_named(&format!("jira-{key}.md"),
                                &format!("# ⚠ Could not fetch {key}\n\n{e}\n\nProceed from the prompt/instructions; treat ticket details as UNAVAILABLE.\n"));
                    jira.insert("fetched".into(), json!(false));
                    jira.insert("error".into(), json!(e));
                    if p.get("require").and_then(Value::as_bool).unwrap_or(false) {
                        return Err(otto_core::Error::Invalid(format!(
                            "prepare_context: required Jira fetch failed: {jira:?}"
                        )));
                    }
                }
            }
        }
    }
    let mut out = serde_json::Map::new();
    out.insert("jira".into(), Value::Object(jira));
    // Optional agent phase — mirrors agent_prompt exactly.
    let agent_prompt_txt = p
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let mut logs = vec![format!("prepare_context: jira {}", out["jira"])];
    if !agent_prompt_txt.is_empty() {
        let preamble = env.files.preamble_for(
            scope.step_no,
            node_display_name(node),
            scope.iter,
            scope.inner_idx,
        );
        let full = prepend_skills(
            ctx,
            p,
            &format!(
                "{preamble}{agent_prompt_txt}\n\n[input data]\n{}",
                truncate(&input.to_string(), 4000)
            ),
        );
        let provider = p
            .get("provider")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(env.default_provider.as_str());
        let model = p.get("model").and_then(Value::as_str);
        let acwd = node_cwd(node, &input, run_cwd);
        let done = env
            .files
            .step_md_path(&crate::workflow_context::step_base_name(
                scope.step_no,
                node_display_name(node),
                scope.iter,
                scope.inner_idx,
            ));
        let (reply, sid) = run_node_agent(
            ctx,
            ws,
            user,
            node,
            &env.wf_name,
            &env.run_id,
            provider,
            model,
            &full,
            &acwd,
            done,
            session_tx,
            log_tx,
            activity_tx,
        )
        .await?;
        out.insert("reply".into(), json!(reply));
        out.insert("session_id".into(), json!(sid));
        out.insert("working_directory".into(), json!(acwd));
        for k in ["repo_id", "base", "repos", "worktree", "pr", "pr_branch"] {
            if let Some(v) = input.get(k) {
                if !v.is_null() {
                    out.insert(k.into(), v.clone());
                }
            }
        }
        logs.push("agent phase complete".into());
    }
    Ok((Value::Object(out), logs))
}
