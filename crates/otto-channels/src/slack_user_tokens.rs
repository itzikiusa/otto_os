//! "Act as me in Slack": a person's own Slack **user** tokens (`xoxp-`),
//! one per Slack workspace (team) they belong to, stored per **Otto user**
//! in the Keychain — never per Otto workspace, never per personal agent,
//! never shared between Otto users.
//!
//! Keychain layout (opaque refs, nothing in the DB):
//! * `slack-user-{otto_user}-{team_id}` — the token itself;
//! * `slack-user-{otto_user}` — the JSON index of that person's connected
//!   teams ([`SlackMe`]: team, Slack user, granted scopes). It holds no
//!   token; it lives next to the tokens so add / remove stay one store.
//!
//! A token is validated with `auth.test` at paste time: it must be a user
//! token (no `bot_id`), and its team keys the entry — a second token for an
//! already-connected team is refused unless the caller asks to replace it.

use std::sync::Arc;

use otto_core::secrets::SecretStore;
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::slack_api::{SlackApi, USER_SCOPES};

/// One connected Slack workspace of an Otto user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlackMe {
    pub team_id: String,
    pub team: String,
    /// `https://acme.slack.com/`.
    pub url: String,
    /// The person's Slack member id in that team.
    pub user_id: String,
    pub user: String,
    /// Scopes Slack granted the token (`x-oauth-scopes` at paste / test time).
    #[serde(default)]
    pub scopes: Vec<String>,
    pub added_at: String,
}

/// [`SlackMe`] plus the as-me scopes the token lacks.
#[derive(Debug, Clone, Serialize)]
pub struct SlackMeView {
    #[serde(flatten)]
    pub me: SlackMe,
    pub missing_scopes: Vec<&'static str>,
}

impl From<SlackMe> for SlackMeView {
    fn from(me: SlackMe) -> Self {
        let missing_scopes = missing_scopes(&me.scopes);
        Self { me, missing_scopes }
    }
}

/// Which of the caller's connected teams this credential may act in — the
/// host decides (`ChannelsCtx::slack_as_me`): a person or a session they
/// run directly gets [`AsMe::All`]; a personal agent only the teams its
/// owner ticked ("Can act as me in Slack", default none).
#[derive(Debug, Clone, PartialEq)]
pub enum AsMe {
    All,
    Teams(Vec<String>),
}

impl AsMe {
    pub fn allows(&self, team_id: &str) -> bool {
        match self {
            AsMe::All => true,
            AsMe::Teams(t) => t.iter().any(|x| x == team_id),
        }
    }
}

pub fn missing_scopes(granted: &[String]) -> Vec<&'static str> {
    USER_SCOPES
        .iter()
        .copied()
        .filter(|s| !granted.iter().any(|g| g == s))
        .collect()
}

/// A person's Slack identity stays theirs: while an admin impersonates them,
/// nothing may act as them in Slack or change their tokens / grants.
pub fn refuse_impersonation(auth: &otto_core::auth::AuthContext) -> Result<()> {
    if auth.real_user.id == auth.effective_user.id {
        Ok(())
    } else {
        Err(Error::Forbidden(
            "acting as someone in Slack (or changing their Slack tokens) is refused while \
             impersonating them"
                .into(),
        ))
    }
}

fn index_ref(user: &str) -> String {
    format!("slack-user-{user}")
}

fn token_ref(user: &str, team_id: &str) -> String {
    format!("slack-user-{user}-{team_id}")
}

/// Serializes index read-modify-writes (two quick adds must not drop one).
fn index_lock() -> &'static tokio::sync::Mutex<()> {
    static L: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    L.get_or_init(Default::default)
}

/// The person's connected teams (empty when none / unreadable index).
pub async fn list(secrets: &Arc<dyn SecretStore>, user: &str) -> Result<Vec<SlackMe>> {
    let raw = otto_core::secrets::get_async(secrets, &index_ref(user)).await?;
    Ok(raw
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default())
}

async fn save_index(secrets: &Arc<dyn SecretStore>, user: &str, teams: &[SlackMe]) -> Result<()> {
    if teams.is_empty() {
        return otto_core::secrets::delete_async(secrets, &index_ref(user)).await;
    }
    let raw = serde_json::to_string(teams).map_err(|e| Error::Internal(e.to_string()))?;
    otto_core::secrets::put_async(secrets, &index_ref(user), &raw).await
}

/// The stored token of one connected team.
pub async fn token(secrets: &Arc<dyn SecretStore>, user: &str, team_id: &str) -> Result<String> {
    otto_core::secrets::get_async(secrets, &token_ref(user, team_id))
        .await?
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| {
            Error::NotFound(format!(
                "no Slack user token stored for team {team_id} — reconnect it in Settings → Channels"
            ))
        })
}

/// Is this shaped like a Slack user token (classic, or a rotating one)?
pub fn looks_like_user_token(t: &str) -> bool {
    t.starts_with("xoxp-") || t.starts_with("xoxe.xoxp-")
}

/// Validate `token` with `auth.test` and connect (or, with `replace`,
/// re-connect) its team. `expect_team` pins the team a replace must match.
pub async fn connect(
    secrets: &Arc<dyn SecretStore>,
    api_base: &str,
    user: &str,
    token: &str,
    replace: bool,
    expect_team: Option<&str>,
) -> Result<SlackMe> {
    let token = token.trim();
    if !looks_like_user_token(token) {
        return Err(Error::Invalid(
            "that isn't a Slack user token — paste the xoxp-… token from OAuth & Permissions → \
             User OAuth Token (not the xoxb- bot token)"
                .into(),
        ));
    }
    let id = SlackApi::with_base(token, api_base)
        .auth_test()
        .await
        .map_err(|e| Error::Invalid(format!("Slack refused the token: {e}")))?;
    if id.bot_id.is_some() || id.user_id.is_empty() || id.team_id.is_empty() {
        return Err(Error::Invalid(
            "Slack says this is not a user token (auth.test returned a bot identity)".into(),
        ));
    }
    if let Some(t) = expect_team.filter(|t| *t != id.team_id) {
        return Err(Error::Invalid(format!(
            "this token belongs to Slack workspace '{}' ({}), not {t} — add it as a new workspace instead",
            id.team, id.team_id
        )));
    }
    let _g = index_lock().lock().await;
    let mut teams = list(secrets, user).await?;
    let existing = teams.iter().position(|m| m.team_id == id.team_id);
    if existing.is_some() && !replace {
        return Err(Error::Conflict(format!(
            "Slack workspace '{}' is already connected — use Replace to swap its token",
            id.team
        )));
    }
    let me = SlackMe {
        team_id: id.team_id.clone(),
        team: id.team,
        url: id.url,
        user_id: id.user_id,
        user: id.user,
        scopes: id.scopes,
        added_at: chrono::Utc::now().to_rfc3339(),
    };
    otto_core::secrets::put_async(secrets, &token_ref(user, &me.team_id), token).await?;
    match existing {
        Some(i) => teams[i] = me.clone(),
        None => teams.push(me.clone()),
    }
    save_index(secrets, user, &teams).await?;
    Ok(me)
}

/// Disconnect one team: its token and its index row.
pub async fn remove(secrets: &Arc<dyn SecretStore>, user: &str, team_id: &str) -> Result<()> {
    let _g = index_lock().lock().await;
    let mut teams = list(secrets, user).await?;
    let before = teams.len();
    teams.retain(|m| m.team_id != team_id);
    let _ = otto_core::secrets::delete_async(secrets, &token_ref(user, team_id)).await;
    if teams.len() == before {
        return Err(Error::NotFound(format!("Slack workspace {team_id}")));
    }
    save_index(secrets, user, &teams).await
}

/// Refresh a team's identity + scopes from a live `auth.test`.
pub async fn retest(
    secrets: &Arc<dyn SecretStore>,
    api_base: &str,
    user: &str,
    team_id: &str,
) -> Result<SlackMe> {
    let tok = token(secrets, user, team_id).await?;
    let id = SlackApi::with_base(tok, api_base)
        .auth_test()
        .await
        .map_err(|e| Error::Upstream(e.to_string()))?;
    let _g = index_lock().lock().await;
    let mut teams = list(secrets, user).await?;
    let Some(m) = teams.iter_mut().find(|m| m.team_id == team_id) else {
        return Err(Error::NotFound(format!("Slack workspace {team_id}")));
    };
    m.team = id.team;
    m.url = id.url;
    m.user = id.user;
    m.user_id = id.user_id;
    m.scopes = id.scopes;
    let out = m.clone();
    save_index(secrets, user, &teams).await?;
    Ok(out)
}

/// Pick the team an as-me call acts in: `want` (team id, name or URL
/// subdomain, case-insensitive) among the teams `scope` allows, or the only
/// one when exactly one is allowed. The error lists what is available.
pub fn pick<'a>(teams: &'a [SlackMe], want: Option<&str>, scope: &AsMe) -> Result<&'a SlackMe> {
    let allowed: Vec<&SlackMe> = teams.iter().filter(|m| scope.allows(&m.team_id)).collect();
    let names = || {
        allowed
            .iter()
            .map(|m| format!("{} ({})", m.team, m.team_id))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if allowed.is_empty() {
        return Err(Error::Forbidden(match (scope, teams.is_empty()) {
            (AsMe::All, _) | (_, true) => "no Slack workspace is connected for acting as you — \
                 add your xoxp- user token in Settings → Channels → Act as me in Slack"
                .into(),
            _ => "this personal agent may not act as you in Slack — its owner can allow it \
                  (agent → Autonomy → Can act as me in Slack)"
                .into(),
        }));
    }
    let Some(want) = want.map(str::trim).filter(|w| !w.is_empty()) else {
        return match allowed.as_slice() {
            [one] => Ok(one),
            _ => Err(Error::Invalid(format!(
                "several Slack workspaces are connected — pass `team`: {}",
                names()
            ))),
        };
    };
    let w = want.to_lowercase();
    allowed
        .iter()
        .find(|m| {
            m.team_id.to_lowercase() == w
                || m.team.to_lowercase() == w
                || subdomain(&m.url).is_some_and(|s| s == w)
        })
        .copied()
        .ok_or_else(|| {
            Error::Invalid(format!(
                "no connected Slack workspace '{want}' you may act in — available: {}",
                names()
            ))
        })
}

/// `acme` of `https://acme.slack.com/`.
fn subdomain(url: &str) -> Option<String> {
    let host = url.split("://").nth(1)?.split('/').next()?;
    host.strip_suffix(".slack.com").map(str::to_lowercase)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(id: &str, team: &str) -> SlackMe {
        SlackMe {
            team_id: id.into(),
            team: team.into(),
            url: format!("https://{}.slack.com/", team.to_lowercase()),
            user_id: "U1".into(),
            user: "dana".into(),
            scopes: vec![],
            added_at: String::new(),
        }
    }

    #[test]
    fn the_only_team_is_the_default_and_several_need_a_name() {
        let one = [me("T1", "Acme")];
        assert_eq!(pick(&one, None, &AsMe::All).unwrap().team_id, "T1");
        let two = [me("T1", "Acme"), me("T2", "Globex")];
        let e = pick(&two, None, &AsMe::All).unwrap_err().to_string();
        assert!(e.contains("Acme (T1)") && e.contains("Globex (T2)"), "{e}");
        // Id, name or subdomain, any case.
        for w in ["t2", "GLOBEX", "globex"] {
            assert_eq!(pick(&two, Some(w), &AsMe::All).unwrap().team_id, "T2");
        }
        assert!(matches!(
            pick(&two, Some("initech"), &AsMe::All),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn an_agent_only_reaches_the_teams_its_owner_allowed() {
        let two = [me("T1", "Acme"), me("T2", "Globex")];
        let scope = AsMe::Teams(vec!["T2".into()]);
        // The one allowed team is the default even though two are connected.
        assert_eq!(pick(&two, None, &scope).unwrap().team_id, "T2");
        assert!(pick(&two, Some("acme"), &scope).is_err());
        let e = pick(&two, None, &AsMe::Teams(vec![])).unwrap_err();
        assert!(
            matches!(e, Error::Forbidden(ref m) if m.contains("Can act as me")),
            "{e}"
        );
        let e = pick(&[], None, &AsMe::All).unwrap_err();
        assert!(
            matches!(e, Error::Forbidden(ref m) if m.contains("xoxp-")),
            "{e}"
        );
    }

    #[test]
    fn an_impersonating_admin_never_acts_as_the_person_in_slack() {
        let u = |id: &str| otto_core::domain::User {
            id: id.into(),
            username: id.into(),
            display_name: id.into(),
            is_root: false,
            disabled: false,
            created_at: chrono::Utc::now(),
        };
        let mut auth = otto_core::auth::AuthContext::automation(&u("dana"), "test");
        assert!(refuse_impersonation(&auth).is_ok());
        auth.real_user = u("root");
        assert!(matches!(
            refuse_impersonation(&auth),
            Err(Error::Forbidden(_))
        ));
    }

    #[test]
    fn only_user_tokens_are_accepted_and_scopes_are_diffed() {
        assert!(looks_like_user_token("xoxp-1-2"));
        assert!(looks_like_user_token("xoxe.xoxp-1-2"));
        assert!(!looks_like_user_token("xoxb-1-2"));
        assert!(!looks_like_user_token("xapp-1"));
        let missing = missing_scopes(&["search:read".to_string()]);
        assert!(!missing.contains(&"search:read"));
        assert!(missing.contains(&"im:history"));
        assert_eq!(
            subdomain("https://acme.slack.com/").as_deref(),
            Some("acme")
        );
    }
}
