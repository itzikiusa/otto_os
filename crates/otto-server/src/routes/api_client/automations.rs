//! API-client automation (collection runner) CRUD + run routes (split out of
//! `routes/api_client.rs`).

use super::*;

/// `GET /workspaces/{wid}/api-client/automations`
pub async fn list_automations(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ApiAutomation>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).list_automations(&wid).await?))
}

/// `POST /workspaces/{wid}/api-client/automations`
pub async fn create_automation(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiAutomationReq>,
) -> ApiResult<Json<ApiAutomation>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(Error::Invalid("automation name must not be empty".into()).into());
    }
    let auto = repo(&ctx)
        .create_automation(NewApiAutomation {
            workspace_id: wid,
            name: req.name.trim().to_string(),
            steps: normalize_json_array(req.steps),
        })
        .await?;
    notify_changed(
        &ctx,
        &auto.workspace_id,
        "automation",
        Some(&auto.id),
        false,
    );
    Ok(Json(auto))
}

/// `PATCH /workspaces/{wid}/api-client/automations/{id}`
pub async fn update_automation(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpsertApiAutomationReq>,
) -> ApiResult<Json<ApiAutomation>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_automation(&id).await?.workspace_id, &wid)?;
    let steps = normalize_json_array(req.steps);
    let auto = repo
        .update_automation(&id, Some(req.name.trim()), Some(&steps))
        .await?;
    notify_changed(&ctx, &wid, "automation", Some(&id), false);
    Ok(Json(auto))
}

/// `DELETE /workspaces/{wid}/api-client/automations/{id}`
pub async fn delete_automation(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let repo = repo(&ctx);
    ensure_in_workspace(&repo.get_automation(&id).await?.workspace_id, &wid)?;
    repo.delete_automation(&id).await?;
    notify_changed(&ctx, &wid, "automation", Some(&id), true);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /workspaces/{wid}/api-client/automations/{id}/run`
///
/// Runs each step in order against its saved request. The effective variable
/// map starts from the workspace's active environment and is overlaid with
/// variables extracted from prior steps (chaining). Every step is attempted —
/// a failing step is recorded but never aborts the run, and a failure to load a
/// request, send it, or evaluate assertions never panics.
pub async fn run_automation(
    Path((wid, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    headers: HeaderMap,
    options: Option<Json<otto_core::api::StartApiAutomationRunReq>>,
) -> ApiResult<Json<ApiRunResult>> {
    let Json(run) = crate::routes::api_automation_runs::start(
        Path((wid.clone(), id)),
        State(ctx.clone()),
        CurrentUser(user),
        CurrentAuthContext(auth),
        headers,
        Json(options.map(|o| o.0).unwrap_or_default()),
    )
    .await?;
    // Wait on the run's progress channel (closed once the final record is
    // saved) instead of decoding the run from the DB every 100 ms (perf F2).
    if let Some(mut progress) = crate::routes::api_automation_runs::subscribe(&run.id) {
        while progress.changed().await.is_ok() {}
    }
    let reports = otto_state::api_runs::ApiRunsRepo(ctx.pool);
    loop {
        let report = reports
            .get(&wid, &run.id)
            .await
            .map_err(|e| ApiError(Error::Internal(e.to_string())))?;
        if report.status != "running" {
            return Ok(Json(report.report));
        }
        // Only reachable if the channel closed without a final status (a
        // failed finalize write) — a slow safety net, not the wait path.
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod binding_tests {
    //! TQ-5-05: the agent-run secret↔host binding exercised through
    //! `run_step` itself, not just its helper — the block must sit AFTER the
    //! pre-request script (which can retarget the URL) and must fire only for
    //! the binding `api_automation_runs::start` builds for an agent caller.
    use super::*;
    use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};
    use wiremock::matchers::{header, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A human-saved request on `http://127.0.0.1:{A}` with a bearer token
    /// (moved to the Keychain as a `$secret` marker on save) and a pre-request
    /// script that points the send at `http://localhost:{B}` — a different
    /// HOST, which is what the binding compares.
    async fn saved_retargeting_request(
        ctx: &ServerCtx,
        wid: &Id,
        a: &MockServer,
        b: &MockServer,
    ) -> ApiRequest {
        let b_url = format!("http://localhost:{}/steal", b.address().port());
        let req: UpsertApiRequestReq = serde_json::from_value(json!({
            "name": "owner",
            "method": "GET",
            "url": format!("{}/me", a.uri()),
            "auth": {"type": "bearer", "token": "tk-secret"},
            "extras": {"v": 1, "scripts": {"pre": format!("pm.request.url = '{b_url}';")}},
        }))
        .expect("upsert dto");
        let Json(saved) = create_request(
            Path(wid.clone()),
            State(ctx.clone()),
            CurrentUser(root_user()),
            HeaderMap::new(),
            Json(req),
        )
        .await
        .expect("save request");
        assert!(
            api_secrets::marker_ref(&saved.auth["token"]).is_some(),
            "the bearer must be stored as a $secret marker: {}",
            saved.auth
        );
        saved
    }

    #[tokio::test]
    async fn agent_run_step_refuses_a_script_retargeted_secret_and_a_person_run_sends_it() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = mem_pool().await;
        let wid: Id = "ws-step-binding".into();
        seed_workspace(&pool, &wid).await;
        // Both mocks are on loopback: let the API client reach them.
        sqlx::query("UPDATE workspaces SET settings_json = ? WHERE id = ?")
            .bind(json!({"api_client": {"allow_local": true}}).to_string())
            .bind(wid.as_str())
            .execute(&pool)
            .await
            .unwrap();
        let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
        let user = root_user();
        let repo = repo(&ctx);
        let a = MockServer::start().await;
        let b = MockServer::start().await;
        let saved = saved_retargeting_request(&ctx, &wid, &a, &b).await;

        // Agent run: the binding `start` builds, through the same constructor
        // (TQ-6-06) — a person gets none, an agent binds the saved request's
        // host (A), so the script's retarget to B is refused.
        {
            let _guard = Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200))
                .expect(0)
                .mount_as_scoped(&b)
                .await;
            let (env_vars, environment, env_blob) =
                resolve_environment(&ctx, &repo, &wid, None).await.unwrap();
            let person = StepSecretBinding::for_caller(
                false,
                &repo,
                &wid,
                &env_vars,
                environment.as_ref(),
                &env_blob,
            )
            .await
            .unwrap();
            assert!(person.is_none(), "a person's run carries no binding");
            let binding = StepSecretBinding::for_caller(
                true,
                &repo,
                &wid,
                &env_vars,
                environment.as_ref(),
                &env_blob,
            )
            .await
            .unwrap()
            .expect("an agent's run is bound");
            assert!(
                binding.bound_hosts.contains("127.0.0.1"),
                "the secret is bound to its saved request's host: {:?}",
                binding.bound_hosts
            );
            assert!(!binding.bound_hosts.contains("localhost"));
            let mut vars = env_vars;
            let step = run_step(
                &ctx,
                &repo,
                &wid,
                &json!({"request_id": saved.id}),
                &mut vars,
                Some(saved.clone()),
                &user.id,
                Some(&binding),
            )
            .await;
            assert!(!step.ok, "an agent run must not send the secret to B");
            let error = step.error.unwrap_or_default();
            assert!(
                error.contains("'localhost'"),
                "error names B's host: {error}"
            );
            assert!(error.contains("needs_confirm=new_host"), "{error}");
        } // `.expect(0)` on B is verified here.

        // A person's run (`binding: None`) is theirs to aim: it is sent, with
        // the resolved bearer.
        let _guard = Mock::given(method("GET"))
            .and(header("authorization", "Bearer tk-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount_as_scoped(&b)
            .await;
        let step = run_step(
            &ctx,
            &repo,
            &wid,
            &json!({"request_id": saved.id}),
            &mut serde_json::Map::new(),
            Some(saved),
            &user.id,
            None,
        )
        .await;
        assert_eq!(step.status, Some(200), "{:?}", step.error);
    }
}
