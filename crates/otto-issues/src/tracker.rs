//! One issue-tracker interface over every provider an [`IssueAccount`] can be.
//!
//! Callers build a [`Tracker`] from the account + its token and never branch on
//! the provider: the account's configured `provider` decides whether a search,
//! fetch, comment or transition goes to Jira or to Linear. Operations a
//! provider has no equivalent for return [`Error::Invalid`] naming the
//! provider, never a silent empty result.

use otto_core::domain::{
    IssueAccount, IssueDetail, IssueProject, IssueProviderKind, IssueSummary, MyWorkIssue,
};
use otto_core::{Error, Result};

use crate::jira::{
    CommentRef, CreatedIssue, DevStatus, EditableField, IssueComment, IssueFull, JiraClient,
    JiraTransition, JiraUser,
};
use crate::linear::LinearClient;

/// A provider-dispatching issue client for one account.
pub enum Tracker {
    Jira(JiraClient),
    Linear(LinearClient),
}

/// A provider-native raw query: JQL for Jira, an `IssueFilter` JSON object for
/// Linear. Which one a caller sent is checked against the account so a JQL
/// string never reaches Linear (or the reverse) as a confusing upstream error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawQuery<'a> {
    Jql(&'a str),
    LinearFilter(&'a str),
    /// The caller did not say — interpreted in the account's own language.
    Native(&'a str),
}

impl Tracker {
    /// The client for `account`, authenticated with `token`.
    pub fn for_account(account: &IssueAccount, token: &str) -> Self {
        match account.provider {
            IssueProviderKind::Jira => {
                Self::Jira(JiraClient::new(&account.base_url, &account.email, token))
            }
            IssueProviderKind::Linear => Self::Linear(LinearClient::new(token)),
        }
    }

    pub fn provider(&self) -> IssueProviderKind {
        match self {
            Self::Jira(_) => IssueProviderKind::Jira,
            Self::Linear(_) => IssueProviderKind::Linear,
        }
    }

    /// The Jira client, for Jira-only features (dev-status ids, editmeta, …).
    pub fn as_jira(&self) -> Option<&JiraClient> {
        match self {
            Self::Jira(c) => Some(c),
            Self::Linear(_) => None,
        }
    }

    fn unsupported(&self, what: &str) -> Error {
        Error::Invalid(format!(
            "{what} is not available for {} accounts",
            provider_name(self.provider())
        ))
    }

    /// Projects (Jira) or teams (Linear), plus whether the listing was capped.
    pub async fn list_projects_paged(&self) -> Result<(Vec<IssueProject>, bool)> {
        match self {
            Self::Jira(c) => c.list_projects_paged().await,
            Self::Linear(c) => c.list_projects_paged().await,
        }
    }

    /// The picker's free-text / key search, optionally in one project (team).
    pub async fn search(
        &self,
        q: &str,
        project: Option<&str>,
        start_at: u32,
        limit: u32,
    ) -> Result<Vec<IssueSummary>> {
        match self {
            Self::Jira(c) => {
                let jql = crate::jira::build_jql(q, project);
                c.search_jql_window(&jql, start_at, limit).await
            }
            Self::Linear(c) => c.search(q, project, start_at, limit).await,
        }
    }

    /// Run a provider-native raw query.
    pub async fn search_raw(
        &self,
        query: RawQuery<'_>,
        start_at: u32,
        limit: u32,
    ) -> Result<Vec<IssueSummary>> {
        match (self, query) {
            (Self::Jira(c), RawQuery::Jql(q) | RawQuery::Native(q)) => {
                c.search_jql_window(q, start_at, limit).await
            }
            (Self::Linear(c), RawQuery::LinearFilter(f) | RawQuery::Native(f)) => {
                c.search_filter(f, start_at, limit).await
            }
            (Self::Jira(_), RawQuery::LinearFilter(_)) => Err(Error::Invalid(
                "this is a Jira account: use `jql`, not a Linear `filter`".into(),
            )),
            (Self::Linear(_), RawQuery::Jql(_)) => Err(Error::Invalid(
                "this is a Linear account: JQL does not apply — pass `filter`, a Linear \
                 IssueFilter JSON object (e.g. {\"state\":{\"type\":{\"eq\":\"started\"}}})"
                    .into(),
            )),
        }
    }

    pub async fn my_work(&self) -> Result<Vec<MyWorkIssue>> {
        match self {
            Self::Jira(c) => c.my_work().await,
            Self::Linear(c) => c.my_work().await,
        }
    }

    pub async fn get_issue(&self, key: &str) -> Result<IssueDetail> {
        match self {
            Self::Jira(c) => c.get_issue(key).await,
            Self::Linear(c) => c.get_issue(key).await,
        }
    }

    pub async fn get_issue_full(&self, key: &str) -> Result<IssueFull> {
        match self {
            Self::Jira(c) => c.get_issue_full(key).await,
            Self::Linear(c) => c.get_issue_full(key).await,
        }
    }

    pub async fn list_comments(&self, key: &str) -> Result<Vec<IssueComment>> {
        match self {
            Self::Jira(c) => c.list_comments(key).await,
            Self::Linear(c) => c.list_comments(key).await,
        }
    }

    pub async fn list_comments_since(&self, key: &str, since: &str) -> Result<Vec<IssueComment>> {
        match self {
            Self::Jira(c) => c.list_comments_since(key, since).await,
            Self::Linear(c) => c.list_comments_since(key, since).await,
        }
    }

    /// Post a comment; `body` is plain text / Markdown.
    pub async fn add_comment(&self, key: &str, body: &str) -> Result<CommentRef> {
        match self {
            Self::Jira(c) => c.add_comment(key, body).await,
            Self::Linear(c) => c.add_comment(key, body).await,
        }
    }

    /// Available status moves. For Linear each "transition" is a workflow
    /// state of the issue's team (its id is the state id).
    pub async fn list_transitions(&self, key: &str) -> Result<Vec<JiraTransition>> {
        match self {
            Self::Jira(c) => c.list_transitions(key).await,
            Self::Linear(c) => c.list_transitions(key).await,
        }
    }

    pub async fn transition_issue(&self, key: &str, transition_id: &str) -> Result<()> {
        match self {
            Self::Jira(c) => c.transition_issue(key, transition_id).await,
            Self::Linear(c) => c.transition_issue(key, transition_id).await,
        }
    }

    pub async fn list_assignable(&self, key: &str) -> Result<Vec<JiraUser>> {
        match self {
            Self::Jira(c) => c.list_assignable(key).await,
            Self::Linear(c) => c.list_assignable(key).await,
        }
    }

    /// Assign to a user id; `"-1"` unassigns.
    pub async fn assign_issue(&self, key: &str, user_id: &str) -> Result<()> {
        match self {
            Self::Jira(c) => c.assign_issue(key, user_id).await,
            Self::Linear(c) => c.assign_issue(key, user_id).await,
        }
    }

    /// Replace the description from Markdown.
    pub async fn update_description(&self, key: &str, body_md: &str) -> Result<()> {
        match self {
            Self::Jira(c) => c.update_description(key, body_md).await,
            Self::Linear(c) => c.update_description(key, body_md).await,
        }
    }

    /// Create an issue in a project (Jira) or team (Linear).
    pub async fn create_issue(
        &self,
        project_key: &str,
        issue_type: &str,
        summary: &str,
        description_md: &str,
    ) -> Result<CreatedIssue> {
        match self {
            Self::Jira(c) => {
                c.create_issue(project_key, issue_type, summary, description_md)
                    .await
            }
            Self::Linear(c) => {
                c.create_issue(project_key, issue_type, summary, description_md)
                    .await
            }
        }
    }

    pub async fn list_issue_types(&self, project_key: &str) -> Result<Vec<String>> {
        match self {
            Self::Jira(c) => c.list_issue_types(project_key).await,
            Self::Linear(c) => c.list_issue_types(project_key).await,
        }
    }

    /// Linked branches / commits / PRs. `issue_id` is Jira's numeric id when
    /// the caller already has it (skips a lookup); Linear keys on `key`.
    pub async fn dev_status(&self, key: &str, issue_id: Option<&str>) -> Result<DevStatus> {
        match self {
            Self::Jira(c) => {
                let id = match issue_id.filter(|s| !s.is_empty()) {
                    Some(id) => id.to_string(),
                    None => c.get_issue_id(key).await?,
                };
                c.dev_status(&id).await
            }
            Self::Linear(c) => c.dev_status(key).await,
        }
    }

    /// Download an attachment. Linear attachments are links (their URLs are
    /// in the issue's fields), so there is nothing to proxy.
    pub async fn attachment_bytes(&self, attachment_id: &str) -> Result<(String, Vec<u8>)> {
        match self {
            Self::Jira(c) => c.attachment_bytes(attachment_id).await,
            Self::Linear(_) => Err(self.unsupported("attachment download")),
        }
    }

    /// The generic field editor's schema. Linear has no editmeta, so it offers
    /// no generic fields (status, assignee and description have their own
    /// controls and work on both).
    pub async fn editmeta(&self, key: &str) -> Result<Vec<EditableField>> {
        match self {
            Self::Jira(c) => c.editmeta(key).await,
            Self::Linear(_) => Ok(Vec::new()),
        }
    }

    pub async fn update_fields(&self, key: &str, fields: serde_json::Value) -> Result<()> {
        match self {
            Self::Jira(c) => c.update_fields(key, fields).await,
            Self::Linear(_) => Err(self.unsupported("editing Jira-shaped fields")),
        }
    }
}

/// Display name of a provider for messages.
pub fn provider_name(p: IssueProviderKind) -> &'static str {
    match p {
        IssueProviderKind::Jira => "Jira",
        IssueProviderKind::Linear => "Linear",
    }
}

/// Confluence lives on Atlassian sites only: refuse a Linear account up front
/// with a clear message instead of sending its key to an Atlassian path.
pub fn require_confluence(account: &IssueAccount) -> Result<()> {
    match account.provider {
        IssueProviderKind::Jira => Ok(()),
        IssueProviderKind::Linear => Err(Error::Invalid(format!(
            "account \"{}\" is a Linear account — Confluence needs a Jira (Atlassian) account",
            account.label
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn account(provider: IssueProviderKind) -> IssueAccount {
        IssueAccount {
            id: "a1".into(),
            user_id: "u1".into(),
            provider,
            label: "Work".into(),
            email: "me@example.com".into(),
            token_ref: "ref".into(),
            base_url: "https://example.atlassian.net".into(),
            token_expires_at: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn the_account_provider_picks_the_client() {
        let j = Tracker::for_account(&account(IssueProviderKind::Jira), "t");
        assert_eq!(j.provider(), IssueProviderKind::Jira);
        assert!(j.as_jira().is_some());
        let l = Tracker::for_account(&account(IssueProviderKind::Linear), "lin_api_x");
        assert_eq!(l.provider(), IssueProviderKind::Linear);
        assert!(l.as_jira().is_none());
    }

    #[tokio::test]
    async fn a_raw_query_in_the_wrong_language_is_refused_before_any_call() {
        let l = Tracker::for_account(&account(IssueProviderKind::Linear), "lin_api_x");
        let e = l
            .search_raw(RawQuery::Jql("project = X"), 0, 25)
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Invalid(m) if m.contains("Linear account")));
        let j = Tracker::for_account(&account(IssueProviderKind::Jira), "t");
        let e = j
            .search_raw(RawQuery::LinearFilter("{}"), 0, 25)
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Invalid(m) if m.contains("Jira account")));
        // A non-JSON native query on Linear fails validation locally too.
        let e = l
            .search_raw(RawQuery::Native("project = X"), 0, 25)
            .await
            .unwrap_err();
        assert!(matches!(e, Error::Invalid(m) if m.contains("IssueFilter")));
    }

    #[tokio::test]
    async fn jira_only_operations_name_the_provider() {
        let l = Tracker::for_account(&account(IssueProviderKind::Linear), "lin_api_x");
        assert!(matches!(
            l.attachment_bytes("1").await,
            Err(Error::Invalid(m)) if m.contains("Linear")
        ));
        assert!(l.editmeta("ENG-1").await.unwrap().is_empty());
        assert!(require_confluence(&account(IssueProviderKind::Jira)).is_ok());
        assert!(matches!(
            require_confluence(&account(IssueProviderKind::Linear)),
            Err(Error::Invalid(m)) if m.contains("Linear account")
        ));
    }
}
