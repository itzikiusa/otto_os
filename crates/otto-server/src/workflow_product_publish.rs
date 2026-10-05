//! Product's preview → human approval → publish workflow handoff.
use super::*;
use otto_product::types::{PublishAsRfcReq, PublishAsStoryReq};

#[derive(serde::Serialize, serde::Deserialize)]
struct Preview {
    story_id: String,
    kind: String,
    request: Value,
    body_md: String,
    account_label: String,
    account_url: String,
}

fn invalid(message: &str) -> otto_core::Error {
    otto_core::Error::Conflict(format!("product_publish: {message}. Connect a preview Product Publish node → Human Approval → live Product Publish, then review again"))
}

fn parse(value: &Value) -> Result<Preview> {
    let preview: Preview = serde_json::from_value(value.clone())
        .map_err(|_| invalid("invalid publication preview"))?;
    if preview.story_id.is_empty() {
        return Err(invalid("missing preview story"));
    }
    let (account, reviewed) = match preview.kind.as_str() {
        "jira" => {
            let req: PublishAsStoryReq = serde_json::from_value(preview.request.clone())
                .map_err(|_| invalid("invalid Jira destination"))?;
            if req.project_key.is_empty()
                || req.issue_type.is_empty()
                || preview.request.get("space_key").is_some()
            {
                return Err(invalid("invalid Jira destination"));
            }
            (req.account_id, req.reviewed_content)
        }
        "rfc" => {
            let req: PublishAsRfcReq = serde_json::from_value(preview.request.clone())
                .map_err(|_| invalid("invalid Confluence destination"))?;
            if req.space_key.is_empty() || preview.request.get("project_key").is_some() {
                return Err(invalid("invalid Confluence destination"));
            }
            (req.account_id, req.reviewed_content)
        }
        _ => return Err(invalid("invalid preview mode")),
    };
    if account.is_empty()
        || reviewed.as_ref().map(|r| r.body_sha256.as_str())
            != Some(otto_core::proof::content_sha256(&preview.body_md).as_str())
    {
        return Err(invalid("preview body does not match its reviewed identity"));
    }
    if preview
        .request
        .get("reviewed_account_url")
        .and_then(Value::as_str)
        != Some(preview.account_url.as_str())
    {
        return Err(invalid("preview account destination changed"));
    }
    Ok(preview)
}

/// Expose the exact gate input while waiting, so the approval banner cannot
/// accidentally show some other branch's successful preview.
pub(super) async fn expose_pending(
    ctx: &ServerCtx,
    run_id: &Id,
    node: &WorkflowNode,
    input: &Value,
) -> Result<()> {
    let Some(preview) = input.get("publication_preview") else {
        return Ok(());
    };
    parse(preview)?;
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let mut run = repo.get_run(run_id).await?;
    let gate = run
        .nodes
        .iter_mut()
        .find(|state| state.node_id == node.id)
        .ok_or_else(|| invalid("approval node is not in this run"))?;
    gate.output = Some(json!({"publication_preview":preview}));
    repo.update_run_progress(run_id, &run.nodes).await?;
    Ok(())
}

pub(super) async fn execute(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    node: &WorkflowNode,
    input: Value,
    run_id: &Id,
) -> Result<(Value, Vec<String>)> {
    let p = &node.params;
    if p.get("dry_run").and_then(Value::as_bool).unwrap_or(true) {
        let story_id = p
            .get("story_id")
            .or_else(|| input.get("story_id"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid("missing story_id"))?
            .to_owned();
        let account_id = p
            .get("account_id")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("preview requires account_id and destination"))?
            .to_owned();
        let story = ctx.product_repo.get_story(&story_id).await?;
        if story.workspace_id != ws.id {
            return Err(otto_core::Error::Forbidden(
                "story belongs to another workspace".into(),
            ));
        }
        ctx.roles
            .check(user, &ws.id, otto_core::domain::WorkspaceRole::Editor)
            .await?;
        ctx.product.authorize_account_id(&account_id, user).await?;
        let account = otto_state::IssuesRepo::new(ctx.pool.clone())
            .get_account(&account_id)
            .await?;
        let (_, body_md, reviewed) = ctx.product.publication_snapshot(&story_id).await?;
        let kind = p.get("kind").and_then(Value::as_str).unwrap_or("rfc");
        let request = match kind {
            "jira" => {
                json!({"account_id":account_id,"project_key":p.get("project_key").and_then(Value::as_str).unwrap_or_default(),
                "issue_type":p.get("issue_type").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("Story"),"reviewed_content":reviewed,"reviewed_account_url":account.base_url})
            }
            "rfc" => {
                json!({"account_id":account_id,"space_key":p.get("space_key").and_then(Value::as_str).unwrap_or_default(),
                "parent_id":p.get("parent_id").and_then(Value::as_str).filter(|s| !s.is_empty()),"title":p.get("title").and_then(Value::as_str).filter(|s| !s.is_empty()),
                "reviewed_content":reviewed,"reviewed_account_url":account.base_url})
            }
            _ => return Err(invalid("kind must be jira or rfc")),
        };
        let preview = json!({"story_id":story_id,"kind":kind,"request":request,"body_md":body_md,"account_label":account.label,"account_url":account.base_url});
        parse(&preview)?;
        return Ok((
            json!({"story_id":story_id,"kind":kind,"dry_run":true,"publication_preview":preview,
            "note":"Connect this preview to Human Approval, then a Product Publish node with dry_run=false. Review the full frozen destination and content in the approval banner."}),
            vec!["product_publish: preview ready for human review".into()],
        ));
    }

    let value = input
        .get("publication_preview")
        .ok_or_else(|| invalid("missing approved publication preview"))?;
    let preview = parse(value)?;
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let run = repo.get_run(run_id).await?;
    let definition = repo.definition_for_run(&run).await?;
    let gate_id = input
        .get("approval_node_id")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing approval node"))?;
    let recorded = run
        .nodes
        .iter()
        .find(|state| state.node_id == gate_id && state.status == NodeStatus::Success)
        .and_then(|state| state.output.as_ref());
    let same_gate = definition
        .graph
        .nodes
        .iter()
        .any(|node| node.id == gate_id && node.kind == "human_approval");
    if !same_gate
        || run.waiting_approval
        || run.approved_at.is_none()
        || run.approved_by.is_none()
        || run.approval_node_id.as_deref() != Some(gate_id)
        || input.get("approved") != Some(&json!(true))
        || input.get("approval_run_id").and_then(Value::as_str) != Some(run_id.as_str())
        || input.get("approved_by").and_then(Value::as_str) != run.approved_by.as_deref()
        || recorded.and_then(|out| out.get("publication_preview")) != Some(value)
        || recorded.and_then(|out| out.get("approved")) != Some(&json!(true))
        || recorded
            .and_then(|out| out.get("approval_run_id"))
            .and_then(Value::as_str)
            != Some(run_id.as_str())
    {
        return Err(invalid(
            "this run has no successful human approval for this exact preview",
        ));
    }
    // A live node inherits its complete destination. Reject overrides rather
    // than silently publish somewhere other than the approved banner showed.
    for key in [
        "story_id",
        "kind",
        "account_id",
        "project_key",
        "issue_type",
        "space_key",
        "parent_id",
        "title",
        "reviewed_content",
        "reviewed_account_url",
    ] {
        if let Some(configured) = p
            .get(key)
            .filter(|v| !v.is_null() && v.as_str() != Some(""))
        {
            let expected = match key {
                "story_id" => json!(preview.story_id),
                "kind" => json!(preview.kind),
                _ => preview.request.get(key).cloned().unwrap_or(Value::Null),
            };
            if configured != &expected {
                return Err(invalid(&format!(
                    "configured {key} differs from the approved preview"
                )));
            }
        }
    }
    let story = ctx.product_repo.get_story(&preview.story_id).await?;
    if story.workspace_id != ws.id {
        return Err(otto_core::Error::Forbidden(
            "story belongs to another workspace".into(),
        ));
    }
    ctx.roles
        .check(user, &ws.id, otto_core::domain::WorkspaceRole::Editor)
        .await?;
    let detail = if preview.kind == "jira" {
        let req: PublishAsStoryReq = serde_json::from_value(preview.request)
            .map_err(|_| invalid("invalid approved Jira request"))?;
        ctx.product
            .authorize_account_id(&req.account_id, user)
            .await?;
        ctx.product
            .publish_as_story(&preview.story_id, &req, &user.id)
            .await?
    } else {
        let req: PublishAsRfcReq = serde_json::from_value(preview.request)
            .map_err(|_| invalid("invalid approved Confluence request"))?;
        ctx.product
            .authorize_account_id(&req.account_id, user)
            .await?;
        ctx.product
            .publish_as_rfc(&preview.story_id, &req, &user.id)
            .await?
    };
    Ok((
        json!({"story_id":preview.story_id,"kind":preview.kind,"dry_run":false,"detail":detail}),
        vec!["product_publish: published the approved snapshot".into()],
    ))
}
