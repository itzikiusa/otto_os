//! Workspace settings an agent credential may not change (`PATCH
//! /workspaces/{id}`, workspace Admin — which an owner's agent has): the ones
//! that switch a guard rail off. Not only API-client logic; it lives here
//! because the API client owns the first such key and its workspace reader.
//!
//! - `api_client.allow_local` ON (F10): opens the LAN and this Mac's loopback
//!   services (the embedded ClickHouse, plugin sidecars) to the API client.
//! - `self_improvement` changed while it stays active: its own route
//!   (`/workspaces/{id}/self-improvement`) is person-only, because autonomy and
//!   the skill allow-list decide what gets auto-applied into shared skills —
//!   the generic settings patch must not be a way around that.
//!
//! Turning either off, and re-sending the value a person already set (a full
//! `settings` replace), passes.

use super::*;

/// The keys the scheduler rewrites on its own; a re-send that differs only
/// there is unchanged.
const SCHEDULER_KEYS: [&str; 2] = ["last_run_at", "next_run_at"];

/// RFC 7396 merge of `patch` onto `target` (what the store's `json_patch` does).
fn merge_patch(target: &Value, patch: &Value) -> Value {
    let Some(fields) = patch.as_object() else {
        return patch.clone();
    };
    let mut out = target.as_object().cloned().unwrap_or_default();
    for (key, value) in fields {
        if value.is_null() {
            out.remove(key);
        } else {
            let merged = merge_patch(out.get(key).unwrap_or(&Value::Null), value);
            out.insert(key.clone(), merged);
        }
    }
    Value::Object(out)
}

/// `self_improvement` without the scheduler-managed keys.
fn user_part(config: Option<&Value>) -> Value {
    let mut config = config.cloned().unwrap_or(Value::Null);
    if let Some(fields) = config.as_object_mut() {
        for key in SCHEDULER_KEYS {
            fields.remove(key);
        }
    }
    config
}

/// What a person must do, when the workspace update `req` (over the current
/// `settings`) changes a person-only setting; `None` when it doesn't.
pub fn person_only_settings_change(
    req: &otto_core::api::UpdateWorkspaceReq,
    current: &Value,
) -> Option<&'static str> {
    let after = match (&req.settings_patch, &req.settings) {
        (Some(patch), _) => merge_patch(current, patch),
        (None, Some(settings)) => settings.clone(),
        (None, None) => return None,
    };
    let allow_local = |s: &Value| s.pointer("/api_client/allow_local") == Some(&Value::Bool(true));
    if allow_local(&after) && !allow_local(current) {
        return Some(
            "an agent session's credential cannot allow private addresses for the API client — \
             a person turns this on in the workspace settings",
        );
    }
    let before = user_part(current.get("self_improvement"));
    let next = user_part(after.get("self_improvement"));
    // Read off the JSON, not a typed parse: a value the config type rejects
    // must not read as "off" and slip through (both default to false).
    let flag = |key: &str| next.get(key).and_then(Value::as_bool) == Some(true);
    if next != before && (flag("enabled") || flag("live_evolve")) {
        return Some(
            "an agent session's credential cannot change self-improvement — a person sets it in \
             the workspace's Self-improvement settings",
        );
    }
    None
}

/// Refuse (403) an AGENT credential's workspace update that changes a
/// person-only setting ([`person_only_settings_change`]). Outside a request
/// scope this fails closed ([`otto_core::auth::request_is_agent`]).
pub async fn refuse_agent_guard_settings<C: ApiClientCtx>(
    ctx: &C,
    wid: &Id,
    req: &otto_core::api::UpdateWorkspaceReq,
) -> otto_core::Result<()> {
    if (req.settings_patch.is_none() && req.settings.is_none())
        || !otto_core::auth::request_is_agent()
    {
        return Ok(());
    }
    let current = ctx.workspace_settings(wid).await.unwrap_or(Value::Null);
    match person_only_settings_change(req, &current) {
        Some(why) => Err(otto_core::Error::Forbidden(why.into())),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(body: Value) -> otto_core::api::UpdateWorkspaceReq {
        serde_json::from_value(body).unwrap()
    }

    #[test]
    fn allow_local_turns_on_only_by_a_person() {
        let off = json!({"api_client": {"allow_local": false}, "notes": "x"});
        let on = json!({"api_client": {"allow_local": true}});
        let patch_on = req(json!({"settings_patch": {"api_client": {"allow_local": true}}}));
        let replace_on = req(json!({"settings": {"api_client": {"allow_local": true}}}));
        assert!(person_only_settings_change(&patch_on, &off).is_some());
        assert!(person_only_settings_change(&replace_on, &Value::Null).is_some());
        // Already on (a person did it): a re-send passes; so does switching off.
        assert!(person_only_settings_change(&replace_on, &on).is_none());
        let patch_off = req(json!({"settings_patch": {"api_client": {"allow_local": false}}}));
        assert!(person_only_settings_change(&patch_off, &on).is_none());
        // Unrelated keys stay an agent's to change.
        let notes = req(json!({"settings_patch": {"notes": "agent"}}));
        assert!(person_only_settings_change(&notes, &off).is_none());
        assert!(person_only_settings_change(&req(json!({"name": "x"})), &on).is_none());
    }

    #[test]
    fn self_improvement_changes_are_person_only_while_active() {
        let current = json!({"self_improvement": {"enabled": true, "autonomy": "propose",
            "skill_allowlist": ["a"], "next_run_at": "2026-11-13T00:00:00Z"}});
        for body in [
            // Turning it on / raising autonomy / widening what it may edit.
            json!({"settings_patch": {"self_improvement": {"autonomy": "auto"}}}),
            json!({"settings_patch": {"self_improvement": {"skill_allowlist": ["a", "b"]}}}),
            json!({"settings_patch": {"self_improvement": {"live_evolve": true}}}),
            json!({"settings": {"self_improvement": {"enabled": true}}}),
        ] {
            let why = person_only_settings_change(&req(body.clone()), &current);
            assert!(
                why.is_some_and(|w| w.contains("self-improvement")),
                "{body}"
            );
        }
        let off = Value::Null;
        let enable = req(json!({"settings_patch": {"self_improvement": {"enabled": true}}}));
        assert!(person_only_settings_change(&enable, &off).is_some());
        // An unchanged re-send (scheduler timestamps aside) and switching it
        // off both pass.
        let resend = req(json!({"settings": {"self_improvement": {"enabled": true,
            "autonomy": "propose", "skill_allowlist": ["a"], "next_run_at": "2026-11-14T00:00:00Z"}}}));
        assert!(person_only_settings_change(&resend, &current).is_none());
        let disable = req(json!({"settings_patch": {"self_improvement": {"enabled": false}}}));
        assert!(person_only_settings_change(&disable, &current).is_none());
        let removed = req(json!({"settings_patch": {"self_improvement": null}}));
        assert!(person_only_settings_change(&removed, &current).is_none());
    }

    #[test]
    fn merge_patch_follows_rfc_7396() {
        let merged = merge_patch(
            &json!({"a": {"b": 1, "c": 2}, "d": 3}),
            &json!({"a": {"b": null, "e": 4}, "d": [1]}),
        );
        assert_eq!(merged, json!({"a": {"c": 2, "e": 4}, "d": [1]}));
    }
}
