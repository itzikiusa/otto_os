//! Environment resolution (variables + Keychain secrets) and `$secret` auth markers.

use super::*;

/// Resolve the variable map — explicit `environment_id`, else the workspace's
/// active environment, else empty — together with the selected environment
/// row and its Keychain blob. Keychain-backed secret variables are resolved
/// in-memory here (execute/automation time only — no read path returns them);
/// the row and blob never leave the engine and its route callers.
pub async fn resolve_environment<C: ApiClientCtx>(
    ctx: &C,
    repo: &ApiClientRepo,
    wid: &Id,
    environment_id: Option<&Id>,
) -> EngineResult<(
    serde_json::Map<String, Value>,
    Option<ApiEnvironment>,
    BTreeMap<String, String>,
)> {
    let env = match environment_id {
        Some(eid) => {
            let env = repo.get_environment(eid).await?;
            ensure_in_workspace(&env.workspace_id, wid)?;
            Some(env)
        }
        None => repo.active_environment(wid).await?,
    };
    let Some(env) = env else {
        return Ok((serde_json::Map::new(), None, BTreeMap::new()));
    };
    let mut vars = env.variables.as_object().cloned().unwrap_or_default();
    let blob = if env.secret_keys.is_empty() {
        BTreeMap::new()
    } else {
        api_secrets::load_blob_async(ctx.secrets(), &api_secrets::env_ref(&env.id)).await
    };
    if !env.secret_keys.is_empty() {
        for key in &env.secret_keys {
            if let Some(v) = blob.get(key) {
                vars.insert(key.clone(), Value::String(v.clone()));
            }
        }
    }
    Ok((vars, Some(env), blob))
}

/// Clone `req` with every `{"$secret": …}` auth marker resolved from the
/// Keychain. Marker refs must point at requests in `wid` (checked against the
/// DB), otherwise the whole call is rejected — a foreign ref is an attempt to
/// exfiltrate another workspace's credential through execute.
pub async fn resolve_exec_auth(
    repo: &ApiClientRepo,
    secrets: &Arc<dyn SecretStore>,
    wid: &Id,
    req: &ExecuteApiReq,
) -> Result<ExecuteApiReq, String> {
    let mut out = req.clone();
    let refs: Vec<String> = out
        .auth
        .as_object()
        .map(|o| {
            o.values()
                .filter_map(api_secrets::marker_ref)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if refs.is_empty() {
        return Ok(out);
    }
    let mut allowed: Vec<String> = Vec::new();
    for r in &refs {
        let rid = api_secrets::parse_request_ref(r)
            .ok_or_else(|| "auth contains an unsupported secret ref".to_string())?;
        let request = repo
            .get_request(&rid.to_string())
            .await
            .map_err(|_| "auth references a secret for an unknown request".to_string())?;
        if &request.workspace_id != wid {
            return Err("auth references a secret outside this workspace".to_string());
        }
        allowed.push(rid.to_string());
    }
    api_secrets::resolve_auth_markers_async(secrets, &mut out.auth, &allowed).await?;
    Ok(out)
}
