//! Linear GraphQL API client (`https://api.linear.app/graphql`).
//!
//! Linear has no REST surface: every read and write is one GraphQL document
//! posted to a single endpoint. Results are mapped onto the same domain types
//! the Jira client returns ([`IssueSummary`], [`IssueFull`], …) so callers go
//! through [`crate::Tracker`] and never branch on the provider.
//!
//! Concept map (Jira → Linear): project → team (`key` like `ENG`), issue key →
//! identifier (`ENG-123`), transition → workflow state (moving to it is an
//! `issueUpdate` with `stateId`), JQL → an `IssueFilter` JSON object, issue
//! type → a label (Linear has no types).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::domain::{IssueDetail, IssueProject, IssueSummary, MyWorkIssue};
use otto_core::{Error, Result};
use serde_json::{json, Value};

use crate::jira::{
    CommentRef, CreatedIssue, DevPr, DevStatus, IssueComment, IssueFull, JiraChangeItem,
    JiraChangelogEntry, JiraField, JiraLink, JiraTransition, JiraUser,
};

/// The one Linear API endpoint (Linear is SaaS-only — no per-site host).
pub const LINEAR_GRAPHQL: &str = "https://api.linear.app/graphql";

/// Linear's connection page maximum.
const PAGE_MAX: u32 = 100;
/// Hard cap on GraphQL calls one offset request may make while walking cursors.
const WALK_CALLS_MAX: u32 = 10;
/// Comment pages read per issue (100 each).
const COMMENT_PAGES_MAX: u32 = 20;
/// Cursor walks kept at once / how long one stays valid.
const WALKS_MAX: usize = 32;
const WALK_TTL: Duration = Duration::from_secs(600);

/// Fields every issue listing selects — what [`IssueSummary`] / [`MyWorkIssue`]
/// are built from.
const ISSUE_BRIEF: &str = "id identifier title url priority priorityLabel updatedAt \
     state { name type } team { key name } assignee { name displayName } \
     parent { identifier title }";

/// The single-issue selection behind [`LinearClient::get_issue_full`].
const ISSUE_FULL: &str = "id identifier title url description priority priorityLabel estimate \
     createdAt updatedAt dueDate branchName \
     state { id name type } team { id key name } \
     assignee { id name displayName avatarUrl } creator { id name displayName avatarUrl } \
     parent { identifier title state { name } } \
     children(first: 50) { nodes { identifier title state { name } } } \
     labels(first: 50) { nodes { name } } \
     project { name } cycle { number name } projectMilestone { name } \
     relations(first: 50) { nodes { type relatedIssue { identifier title state { name } } } } \
     inverseRelations(first: 50) { nodes { type issue { identifier title state { name } } } } \
     attachments(first: 50) { nodes { id title subtitle url sourceType updatedAt metadata } } \
     history(first: 50) { nodes { createdAt actor { name displayName } \
       fromState { name } toState { name } fromAssignee { name } toAssignee { name } \
       fromPriority toPriority fromTitle toTitle } }";

/// The shared client (one pool for every Linear account: the API is one host
/// and the key rides per request) from the bounded transport cache.
fn http_client() -> reqwest::Client {
    crate::atlassian::cached_client("linear\x00https://api.linear.app")
}

/// The `Authorization` value for a Linear credential: a personal API key
/// (`lin_api_…`) is sent bare, anything else is an OAuth access token.
pub(crate) fn auth_value(token: &str) -> String {
    let t = token.trim();
    if t.starts_with("lin_api_") || t.starts_with("Bearer ") {
        t.to_string()
    } else {
        format!("Bearer {t}")
    }
}

/// A thin Linear GraphQL client scoped to one credential.
pub struct LinearClient {
    endpoint: String,
    auth: String,
    http: reqwest::Client,
}

/// Cursor walks: (credential, query) → offset → the `after` cursor that
/// fetches the page starting there. Linear is cursor-paginated only; this
/// keeps the offset-based "Load more" contract without re-walking from 0.
struct Walk {
    cursors: BTreeMap<u32, String>,
    touched: Instant,
}

static WALKS: OnceLock<Mutex<HashMap<u64, Walk>>> = OnceLock::new();

fn walk_key(auth: &str, query: &str, vars: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    auth.hash(&mut h);
    query.hash(&mut h);
    vars.to_string().hash(&mut h);
    h.finish()
}

fn walks<R>(f: impl FnOnce(&mut HashMap<u64, Walk>) -> R) -> R {
    let cache = WALKS.get_or_init(|| Mutex::new(HashMap::new()));
    match cache.lock() {
        Ok(mut map) => {
            map.retain(|_, w| w.touched.elapsed() < WALK_TTL);
            while map.len() > WALKS_MAX {
                let oldest = map.iter().min_by_key(|(_, w)| w.touched).map(|(k, _)| *k);
                match oldest {
                    Some(k) => {
                        map.remove(&k);
                    }
                    None => break,
                }
            }
            f(&mut map)
        }
        Err(_) => f(&mut HashMap::new()),
    }
}

impl LinearClient {
    pub fn new(token: &str) -> Self {
        Self::with_endpoint(LINEAR_GRAPHQL, token)
    }

    /// A client against another endpoint (tests point this at a stub server).
    pub fn with_endpoint(endpoint: &str, token: &str) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            auth: auth_value(token),
            http: http_client(),
        }
    }

    /// Post one GraphQL document and return its `data`, mapping GraphQL
    /// `errors` (which Linear reports with HTTP 200 or 400) onto Otto errors.
    async fn gql(&self, what: &str, query: &str, variables: Value) -> Result<Value> {
        let resp = self
            .http
            .post(&self.endpoint)
            .header("Authorization", &self.auth)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("linear {what} request: {e}")))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(err) = graphql_error(&body) {
            return Err(map_error(what, status.as_u16(), &err));
        }
        if !status.is_success() {
            return Err(Error::Upstream(format!(
                "linear {what} failed ({status}): {}",
                otto_core::text::clip_chars(&text, 400)
            )));
        }
        body.get("data")
            .cloned()
            .filter(|d| !d.is_null())
            .ok_or_else(|| Error::Upstream(format!("linear {what}: response had no data")))
    }

    /// Fetch `limit` nodes starting at `start_at` from a cursor-paginated
    /// connection. `query` must declare `$first: Int` and `$after: String` and
    /// `pick` must return the connection object (`{nodes, pageInfo}`).
    async fn window(
        &self,
        what: &str,
        query: &str,
        vars: Value,
        start_at: u32,
        limit: u32,
        pick: fn(&Value) -> Option<&Value>,
    ) -> Result<Vec<Value>> {
        let limit = limit.clamp(1, PAGE_MAX);
        let key = walk_key(&self.auth, query, &vars);
        let (mut offset, mut cursor) = walks(|w| {
            if start_at == 0 {
                w.remove(&key);
                return (0, None);
            }
            w.get(&key)
                .and_then(|walk| walk.cursors.range(..=start_at).next_back())
                .map(|(o, c)| (*o, Some(c.clone())))
                .unwrap_or((0, None))
        });
        for _ in 0..WALK_CALLS_MAX {
            let skipping = offset < start_at;
            let first = if skipping {
                (start_at - offset).min(PAGE_MAX)
            } else {
                limit
            };
            let mut v = vars.clone();
            v["first"] = json!(first);
            v["after"] = cursor.clone().map_or(Value::Null, Value::String);
            let data = self.gql(what, query, v).await?;
            let conn = pick(&data).cloned().unwrap_or(Value::Null);
            let nodes = conn["nodes"].as_array().cloned().unwrap_or_default();
            let has_next = conn["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false);
            let end = conn["pageInfo"]["endCursor"].as_str().map(str::to_string);
            let got = nodes.len() as u32;
            if let (true, Some(c)) = (has_next && got > 0, end.as_ref()) {
                let at = offset + got;
                walks(|w| {
                    let walk = w.entry(key).or_insert_with(|| Walk {
                        cursors: BTreeMap::new(),
                        touched: Instant::now(),
                    });
                    walk.cursors.insert(at, c.clone());
                    walk.touched = Instant::now();
                });
            }
            if !skipping {
                return Ok(nodes);
            }
            if !has_next || got == 0 {
                return Ok(Vec::new());
            }
            offset += got;
            cursor = end;
        }
        // Too far past the last remembered cursor to walk in one request —
        // an error, so a capped walk never reads as "no more results".
        Err(Error::Invalid(format!(
            "linear {what}: offset {start_at} is too far from the last loaded page — \
             run the search again from the start"
        )))
    }

    /// Teams, as the tracker's "projects" (`key` = team key). `true` when
    /// more than one page of teams exists.
    pub async fn list_projects_paged(&self) -> Result<(Vec<IssueProject>, bool)> {
        let data = self
            .gql(
                "teams",
                "query { teams(first: 100) { nodes { key name } pageInfo { hasNextPage } } }",
                json!({}),
            )
            .await?;
        let mut teams: Vec<IssueProject> = data["teams"]["nodes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|t| IssueProject {
                        key: s(t, "key"),
                        name: s(t, "name"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        teams.sort_by_key(|t| t.name.to_lowercase());
        let more = data["teams"]["pageInfo"]["hasNextPage"]
            .as_bool()
            .unwrap_or(false);
        Ok((teams, more))
    }

    /// The picker's text / identifier search, optionally inside one team:
    /// empty → the caller's recently updated issues, an identifier (`ENG-12`,
    /// or a bare number with a team) → that issue, else Linear full-text search.
    pub async fn search(
        &self,
        q: &str,
        team: Option<&str>,
        start_at: u32,
        limit: u32,
    ) -> Result<Vec<IssueSummary>> {
        let q = q.trim();
        let team_filter = team.map(|k| json!({ "team": { "key": { "eq": k } } }));
        if q.is_empty() {
            let mut filter = json!({ "assignee": { "isMe": { "eq": true } } });
            if let Some(t) = &team_filter {
                filter = json!({ "and": [filter, t] });
            }
            return self.search_filter_value(filter, start_at, limit).await;
        }
        let ident = match team {
            Some(k) if q.chars().all(|c| c.is_ascii_digit()) => Some(format!("{k}-{q}")),
            _ if crate::jira::is_issue_key(&q.to_ascii_uppercase()) => Some(q.to_ascii_uppercase()),
            _ => None,
        };
        if let Some(id) = ident {
            if start_at > 0 {
                return Ok(Vec::new());
            }
            match self.issue_brief(&id).await {
                Ok(hit) => return Ok(vec![hit]),
                Err(Error::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        let query = format!(
            "query($term: String!, $filter: IssueFilter, $first: Int, $after: String) {{ \
             searchIssues(term: $term, filter: $filter, first: $first, after: $after) {{ \
             nodes {{ {ISSUE_BRIEF} }} pageInfo {{ hasNextPage endCursor }} }} }}"
        );
        let vars = json!({ "term": q, "filter": team_filter });
        let nodes = self
            .window("search", &query, vars, start_at, limit, |d| {
                d.get("searchIssues")
            })
            .await?;
        Ok(nodes.iter().map(summary_from).collect())
    }

    /// Run a raw Linear `IssueFilter` (the JSON object Linear's API and its
    /// filter docs use, e.g. `{"state":{"type":{"eq":"started"}}}`), newest
    /// activity first. The Linear counterpart of a raw JQL search.
    pub async fn search_filter(
        &self,
        filter_json: &str,
        start_at: u32,
        limit: u32,
    ) -> Result<Vec<IssueSummary>> {
        let filter = parse_filter(filter_json)?;
        self.search_filter_value(filter, start_at, limit).await
    }

    async fn search_filter_value(
        &self,
        filter: Value,
        start_at: u32,
        limit: u32,
    ) -> Result<Vec<IssueSummary>> {
        let query = format!(
            "query($filter: IssueFilter, $first: Int, $after: String) {{ \
             issues(filter: $filter, first: $first, after: $after, orderBy: updatedAt) {{ \
             nodes {{ {ISSUE_BRIEF} }} pageInfo {{ hasNextPage endCursor }} }} }}"
        );
        let nodes = self
            .window(
                "issues",
                &query,
                json!({ "filter": filter }),
                start_at,
                limit,
                |d| d.get("issues"),
            )
            .await?;
        Ok(nodes.iter().map(summary_from).collect())
    }

    async fn issue_brief(&self, id: &str) -> Result<IssueSummary> {
        let query = format!("query($id: String!) {{ issue(id: $id) {{ {ISSUE_BRIEF} }} }}");
        let data = self.gql("issue", &query, json!({ "id": id })).await?;
        let issue = present(&data["issue"], id)?;
        Ok(summary_from(issue))
    }

    /// The caller's open assigned issues (not completed / canceled), newest
    /// activity first — one page of 100, like the Jira Focus list.
    pub async fn my_work(&self) -> Result<Vec<MyWorkIssue>> {
        let query = format!(
            "query {{ viewer {{ assignedIssues(first: 100, orderBy: updatedAt, \
             filter: {{ state: {{ type: {{ nin: [\"completed\", \"canceled\"] }} }} }}) {{ \
             nodes {{ {ISSUE_BRIEF} }} }} }} }}"
        );
        let data = self.gql("my-work", &query, json!({})).await?;
        Ok(data["viewer"]["assignedIssues"]["nodes"]
            .as_array()
            .map(|a| a.iter().map(my_work_from).collect())
            .unwrap_or_default())
    }

    pub async fn get_issue(&self, key: &str) -> Result<IssueDetail> {
        let data = self
            .gql(
                "issue",
                "query($id: String!) { issue(id: $id) { identifier title url description \
                 state { name } assignee { name displayName } parent { identifier } } }",
                json!({ "id": key }),
            )
            .await?;
        let i = present(&data["issue"], key)?;
        Ok(IssueDetail {
            key: s(i, "identifier"),
            summary: s(i, "title"),
            status: s(&i["state"], "name"),
            issue_type: issue_type(i),
            url: s(i, "url"),
            description: s(i, "description"),
            assignee: user_name(&i["assignee"]).unwrap_or_default(),
        })
    }

    pub async fn get_issue_full(&self, key: &str) -> Result<IssueFull> {
        let query = format!("query($id: String!) {{ issue(id: $id) {{ {ISSUE_FULL} }} }}");
        let data = self.gql("issue", &query, json!({ "id": key })).await?;
        let issue = present(&data["issue"], key)?;
        let mut full = parse_issue_full(issue);
        full.comments = self.list_comments(key).await?;
        Ok(full)
    }

    /// Every comment on the issue, oldest first (capped at
    /// [`COMMENT_PAGES_MAX`] pages of 100).
    pub async fn list_comments(&self, key: &str) -> Result<Vec<IssueComment>> {
        let query = "query($id: String!, $after: String) { issue(id: $id) { \
             comments(first: 100, after: $after) { nodes { id body createdAt \
             user { name displayName } botActor { name } } \
             pageInfo { hasNextPage endCursor } } } }";
        let mut out = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..COMMENT_PAGES_MAX {
            let data = self
                .gql("comments", query, json!({ "id": key, "after": after }))
                .await?;
            let conn = &present(&data["issue"], key)?["comments"];
            if let Some(nodes) = conn["nodes"].as_array() {
                out.extend(nodes.iter().map(comment_from));
            }
            match (
                conn["pageInfo"]["hasNextPage"].as_bool(),
                conn["pageInfo"]["endCursor"].as_str(),
            ) {
                (Some(true), Some(c)) => after = Some(c.to_string()),
                _ => break,
            }
        }
        out.sort_by(|a, b| a.created.cmp(&b.created));
        Ok(out)
    }

    /// Comments created at or after `since` (the story watcher's cursor,
    /// compared lexicographically like the Jira path), oldest first.
    pub async fn list_comments_since(&self, key: &str, since: &str) -> Result<Vec<IssueComment>> {
        let mut all = self.list_comments(key).await?;
        all.retain(|c| c.created.as_str() >= since);
        Ok(all)
    }

    /// Post a Markdown comment (Linear stores Markdown natively).
    pub async fn add_comment(&self, key: &str, body_md: &str) -> Result<CommentRef> {
        let issue_id = self.issue_uuid(key).await?;
        let data = self
            .gql(
                "comment",
                "mutation($input: CommentCreateInput!) { commentCreate(input: $input) { \
                 success comment { id url } } }",
                json!({ "input": { "issueId": issue_id, "body": body_md } }),
            )
            .await?;
        let payload = &data["commentCreate"];
        mutation_ok("comment", payload)?;
        Ok(CommentRef {
            id: s(&payload["comment"], "id"),
            url: payload["comment"]["url"].as_str().map(str::to_string),
        })
    }

    /// The team's workflow states other than the current one — Linear lets an
    /// issue move to any state of its team, so each is one "transition" whose
    /// id is the state id.
    pub async fn list_transitions(&self, key: &str) -> Result<Vec<JiraTransition>> {
        let data = self
            .gql(
                "states",
                "query($id: String!) { issue(id: $id) { state { id } team { \
                 states(first: 100) { nodes { id name type position } } } } }",
                json!({ "id": key }),
            )
            .await?;
        let issue = present(&data["issue"], key)?;
        Ok(transitions_from(issue))
    }

    /// Move the issue to workflow state `state_id` (a [`JiraTransition::id`]
    /// from [`Self::list_transitions`]).
    pub async fn transition_issue(&self, key: &str, state_id: &str) -> Result<()> {
        self.update_issue(key, json!({ "stateId": state_id })).await
    }

    /// Members of the issue's team.
    pub async fn list_assignable(&self, key: &str) -> Result<Vec<JiraUser>> {
        let data = self
            .gql(
                "members",
                "query($id: String!) { issue(id: $id) { team { members(first: 250) { \
                 nodes { id name displayName avatarUrl active } } } } }",
                json!({ "id": key }),
            )
            .await?;
        let issue = present(&data["issue"], key)?;
        Ok(issue["team"]["members"]["nodes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|u| u["active"].as_bool() != Some(false))
                    .filter_map(user_from)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Assign to a user id; `"-1"` or empty unassigns (the Jira convention).
    pub async fn assign_issue(&self, key: &str, user_id: &str) -> Result<()> {
        let assignee = match user_id.trim() {
            "" | "-1" => Value::Null,
            id => json!(id),
        };
        self.update_issue(key, json!({ "assigneeId": assignee }))
            .await
    }

    /// Replace the description (Markdown, stored as-is).
    pub async fn update_description(&self, key: &str, body_md: &str) -> Result<()> {
        self.update_issue(key, json!({ "description": body_md }))
            .await
    }

    async fn update_issue(&self, key: &str, input: Value) -> Result<()> {
        let data = self
            .gql(
                "update",
                "mutation($id: String!, $input: IssueUpdateInput!) { \
                 issueUpdate(id: $id, input: $input) { success } }",
                json!({ "id": key, "input": input }),
            )
            .await?;
        mutation_ok("update", &data["issueUpdate"])
    }

    /// Create an issue in team `team_key`. Linear has no issue types: a
    /// non-generic `issue_type` (e.g. "Bug") is applied as the label of that
    /// name when the workspace has one, and otherwise ignored.
    pub async fn create_issue(
        &self,
        team_key: &str,
        issue_type: &str,
        summary: &str,
        description_md: &str,
    ) -> Result<CreatedIssue> {
        let data = self
            .gql(
                "team",
                "query($key: String!) { teams(filter: { key: { eq: $key } }, first: 1) { \
                 nodes { id } } }",
                json!({ "key": team_key }),
            )
            .await?;
        let team_id = data["teams"]["nodes"][0]["id"]
            .as_str()
            .ok_or_else(|| Error::NotFound(format!("Linear team {team_key}")))?
            .to_string();
        let mut input = json!({
            "teamId": team_id,
            "title": summary,
            "description": description_md,
        });
        let wanted = issue_type.trim();
        if !wanted.is_empty() && !wanted.eq_ignore_ascii_case(GENERIC_TYPE) {
            let labels = self.team_labels(team_key).await.unwrap_or_default();
            if let Some((id, _)) = labels
                .iter()
                .find(|(_, name)| name.eq_ignore_ascii_case(wanted))
            {
                input["labelIds"] = json!([id]);
            }
        }
        let data = self
            .gql(
                "create",
                "mutation($input: IssueCreateInput!) { issueCreate(input: $input) { \
                 success issue { identifier url } } }",
                json!({ "input": input }),
            )
            .await?;
        let payload = &data["issueCreate"];
        mutation_ok("create", payload)?;
        Ok(CreatedIssue {
            key: s(&payload["issue"], "identifier"),
            url: s(&payload["issue"], "url"),
        })
    }

    /// The "issue types" a create form offers: the generic type plus the
    /// labels usable in the team (team labels + workspace labels).
    pub async fn list_issue_types(&self, team_key: &str) -> Result<Vec<String>> {
        let mut names: Vec<String> = self
            .team_labels(team_key)
            .await?
            .into_iter()
            .map(|(_, n)| n)
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        names.insert(0, GENERIC_TYPE.to_string());
        Ok(names)
    }

    /// `(id, name)` of every label usable in `team_key`.
    async fn team_labels(&self, team_key: &str) -> Result<Vec<(String, String)>> {
        let data = self
            .gql(
                "labels",
                "query($key: String!) { issueLabels(first: 250, filter: { or: [\
                 { team: { null: true } }, { team: { key: { eq: $key } } }] }) { \
                 nodes { id name isGroup team { key } } } }",
                json!({ "key": team_key }),
            )
            .await?;
        Ok(data["issueLabels"]["nodes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|l| l["isGroup"].as_bool() != Some(true))
                    .filter(|l| l["team"].is_null() || l["team"]["key"].as_str() == Some(team_key))
                    .map(|l| (s(l, "id"), s(l, "name")))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Pull requests linked to the issue (Linear's GitHub / GitLab
    /// integrations attach them). Branches and commits are not exposed.
    pub async fn dev_status(&self, key: &str) -> Result<DevStatus> {
        let data = self
            .gql(
                "attachments",
                "query($id: String!) { issue(id: $id) { attachments(first: 50) { \
                 nodes { id title subtitle url sourceType updatedAt metadata } } } }",
                json!({ "id": key }),
            )
            .await?;
        let issue = present(&data["issue"], key)?;
        Ok(DevStatus {
            pull_requests: prs_from(&issue["attachments"]),
            ..Default::default()
        })
    }

    /// The issue's UUID (mutations that take an `issueId` input need it; the
    /// `issue(id:)` query accepts the identifier too).
    async fn issue_uuid(&self, key: &str) -> Result<String> {
        let data = self
            .gql(
                "issue",
                "query($id: String!) { issue(id: $id) { id } }",
                json!({ "id": key }),
            )
            .await?;
        Ok(s(present(&data["issue"], key)?, "id"))
    }
}

/// The issue type every Linear issue has (there are no others).
pub const GENERIC_TYPE: &str = "Issue";

// ---------------------------------------------------------------------------
// Pure mapping helpers (unit-tested)
// ---------------------------------------------------------------------------

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The first GraphQL error message, if the response carries any.
fn graphql_error(body: &Value) -> Option<Value> {
    body.get("errors")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
}

fn map_error(what: &str, status: u16, err: &Value) -> Error {
    let msg = err["message"].as_str().unwrap_or("unknown error");
    let code = err["extensions"]["code"]
        .as_str()
        .or_else(|| err["extensions"]["type"].as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let lower = msg.to_ascii_lowercase();
    if lower.contains("entity not found") || code.contains("not_found") {
        return Error::NotFound(format!("linear {what}: {msg}"));
    }
    if status == 401 || code.contains("authentication") {
        return Error::Upstream(format!(
            "linear {what}: authentication failed — check the account's API key ({msg})"
        ));
    }
    if code.contains("invalid_input") || code.contains("graphql_validation_failed") {
        return Error::Invalid(format!("linear {what}: {msg}"));
    }
    Error::Upstream(format!("linear {what}: {msg}"))
}

fn present<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    if v.is_null() {
        Err(Error::NotFound(format!("Linear issue {key}")))
    } else {
        Ok(v)
    }
}

fn mutation_ok(what: &str, payload: &Value) -> Result<()> {
    if payload["success"].as_bool() == Some(true) {
        Ok(())
    } else {
        Err(Error::Upstream(format!("linear {what}: not applied")))
    }
}

/// Parse a raw filter argument: a JSON object (Linear `IssueFilter`).
pub(crate) fn parse_filter(raw: &str) -> Result<Value> {
    match serde_json::from_str::<Value>(raw.trim()) {
        Ok(v) if v.is_object() => Ok(v),
        _ => Err(Error::Invalid(
            "a Linear filter is a JSON IssueFilter object, e.g. \
             {\"state\":{\"type\":{\"eq\":\"started\"}},\"team\":{\"key\":{\"eq\":\"ENG\"}}}"
                .into(),
        )),
    }
}

fn user_name(u: &Value) -> Option<String> {
    if u.is_null() {
        return None;
    }
    let display = s(u, "displayName");
    let name = s(u, "name");
    let pick = if name.is_empty() { display } else { name };
    (!pick.is_empty()).then_some(pick)
}

fn user_from(u: &Value) -> Option<JiraUser> {
    let display_name = user_name(u)?;
    Some(JiraUser {
        account_id: s(u, "id"),
        display_name,
        avatar_url: u["avatarUrl"].as_str().map(str::to_string),
    })
}

fn issue_type(i: &Value) -> String {
    if i["parent"].is_object() {
        "Sub-issue".into()
    } else {
        GENERIC_TYPE.into()
    }
}

fn opt(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

pub(crate) fn summary_from(i: &Value) -> IssueSummary {
    IssueSummary {
        key: s(i, "identifier"),
        summary: s(i, "title"),
        status: s(&i["state"], "name"),
        issue_type: issue_type(i),
        url: s(i, "url"),
        parent_key: opt(&i["parent"]["identifier"]),
        assignee: user_name(&i["assignee"]),
        fix_versions: Vec::new(),
        updated: opt(&i["updatedAt"]),
    }
}

pub(crate) fn my_work_from(i: &Value) -> MyWorkIssue {
    MyWorkIssue {
        key: s(i, "identifier"),
        summary: s(i, "title"),
        status: s(&i["state"], "name"),
        issue_type: issue_type(i),
        url: s(i, "url"),
        project_key: s(&i["team"], "key"),
        project_name: s(&i["team"], "name"),
        parent_key: opt(&i["parent"]["identifier"]),
        parent_summary: opt(&i["parent"]["title"]),
        parent_type: i["parent"].is_object().then(|| GENERIC_TYPE.to_string()),
        // Priority 0 is Linear's "No priority".
        priority: (i["priority"].as_f64().unwrap_or(0.0) > 0.0)
            .then(|| opt(&i["priorityLabel"]))
            .flatten(),
        updated_at: opt(&i["updatedAt"]),
    }
}

fn comment_from(c: &Value) -> IssueComment {
    IssueComment {
        id: s(c, "id"),
        author: user_name(&c["user"])
            .or_else(|| opt(&c["botActor"]["name"]))
            .unwrap_or_default(),
        body_md: s(c, "body"),
        created: s(c, "createdAt"),
    }
}

pub(crate) fn transitions_from(issue: &Value) -> Vec<JiraTransition> {
    let current = issue["state"]["id"].as_str().unwrap_or("");
    let mut states: Vec<&Value> = issue["team"]["states"]["nodes"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    // Linear's board order: by state type (backlog → canceled), then position.
    let rank = |t: &str| match t {
        "triage" => 0,
        "backlog" => 1,
        "unstarted" => 2,
        "started" => 3,
        "completed" => 4,
        "canceled" => 5,
        _ => 6,
    };
    states.sort_by(|a, b| {
        rank(a["type"].as_str().unwrap_or(""))
            .cmp(&rank(b["type"].as_str().unwrap_or("")))
            .then(
                a["position"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .total_cmp(&b["position"].as_f64().unwrap_or(0.0)),
            )
    });
    states
        .into_iter()
        .filter(|st| st["id"].as_str() != Some(current))
        .map(|st| JiraTransition {
            id: s(st, "id"),
            name: s(st, "name"),
            to_status: s(st, "name"),
        })
        .collect()
}

fn prs_from(attachments: &Value) -> Vec<DevPr> {
    attachments["nodes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|at| {
                    let url = at["url"].as_str().unwrap_or("");
                    url.contains("/pull/")
                        || url.contains("/merge_requests/")
                        || url.contains("/pull-requests/")
                })
                .map(|at| DevPr {
                    id: s(at, "id"),
                    name: s(at, "title"),
                    url: s(at, "url"),
                    status: opt(&at["metadata"]["status"])
                        .or_else(|| opt(&at["subtitle"]))
                        .unwrap_or_default(),
                    repo: opt(&at["metadata"]["repoName"]).unwrap_or_default(),
                    last_update: s(at, "updatedAt"),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn link(rel: &str, i: &Value) -> JiraLink {
    JiraLink {
        rel: rel.to_string(),
        key: s(i, "identifier"),
        summary: s(i, "title"),
        status: s(&i["state"], "name"),
        issue_type: GENERIC_TYPE.into(),
    }
}

/// Human labels for Linear relation types, from the issue's own side and from
/// the other side (`inverseRelations`).
fn relation_label(kind: &str, inverse: bool) -> &'static str {
    match (kind, inverse) {
        ("blocks", false) => "blocks",
        ("blocks", true) => "is blocked by",
        ("duplicate", false) => "duplicates",
        ("duplicate", true) => "is duplicated by",
        ("similar", _) => "is similar to",
        _ => "relates to",
    }
}

/// Map one `issue { … }` (the [`ISSUE_FULL`] selection) onto [`IssueFull`].
/// Comments are fetched separately (they page).
pub(crate) fn parse_issue_full(i: &Value) -> IssueFull {
    let labels: Vec<String> = i["labels"]["nodes"]
        .as_array()
        .map(|a| a.iter().map(|l| s(l, "name")).collect())
        .unwrap_or_default();

    let mut fields = Vec::new();
    let mut field = |key: &str, name: &str, value: Option<String>| {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            fields.push(JiraField {
                key: key.into(),
                name: name.into(),
                value,
            });
        }
    };
    field(
        "team",
        "Team",
        opt(&i["team"]["name"]).map(|n| format!("{n} ({})", s(&i["team"], "key"))),
    );
    field("project", "Project", opt(&i["project"]["name"]));
    field(
        "cycle",
        "Cycle",
        i["cycle"].is_object().then(|| {
            opt(&i["cycle"]["name"]).unwrap_or_else(|| {
                format!(
                    "Cycle {}",
                    trim_num(i["cycle"]["number"].as_f64().unwrap_or(0.0))
                )
            })
        }),
    );
    field(
        "projectMilestone",
        "Milestone",
        opt(&i["projectMilestone"]["name"]),
    );
    field("dueDate", "Due date", opt(&i["dueDate"]));
    field("branchName", "Git branch", opt(&i["branchName"]));
    field("createdAt", "Created", opt(&i["createdAt"]));
    field("updatedAt", "Updated", opt(&i["updatedAt"]));
    let attachment_lines: Vec<String> = i["attachments"]["nodes"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|at| format!("{} — {}", s(at, "title"), s(at, "url")))
                .collect()
        })
        .unwrap_or_default();
    field(
        "attachments",
        "Attachments",
        (!attachment_lines.is_empty()).then(|| attachment_lines.join("\n")),
    );

    let mut links = Vec::new();
    if i["parent"].is_object() {
        links.push(link("Parent", &i["parent"]));
    }
    for c in i["children"]["nodes"].as_array().into_iter().flatten() {
        links.push(link("Sub-issue", c));
    }
    for r in i["relations"]["nodes"].as_array().into_iter().flatten() {
        links.push(link(
            relation_label(r["type"].as_str().unwrap_or(""), false),
            &r["relatedIssue"],
        ));
    }
    for r in i["inverseRelations"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
    {
        links.push(link(
            relation_label(r["type"].as_str().unwrap_or(""), true),
            &r["issue"],
        ));
    }

    let mut history: Vec<JiraChangelogEntry> = i["history"]["nodes"]
        .as_array()
        .map(|a| a.iter().filter_map(history_from).collect())
        .unwrap_or_default();
    history.sort_by(|a, b| a.created.cmp(&b.created));

    IssueFull {
        key: s(i, "identifier"),
        id: s(i, "id"),
        summary: s(i, "title"),
        status: s(&i["state"], "name"),
        issue_type: issue_type(i),
        url: s(i, "url"),
        description_md: s(i, "description"),
        assignee: user_from(&i["assignee"]),
        reporter: user_from(&i["creator"]),
        priority: (i["priority"].as_f64().unwrap_or(0.0) > 0.0)
            .then(|| opt(&i["priorityLabel"]))
            .flatten(),
        labels,
        fields,
        comments: Vec::new(),
        history,
        attachments: Vec::new(),
        links,
        estimate: i["estimate"]
            .as_f64()
            .map(|e| format!("{} pts", trim_num(e))),
    }
}

fn trim_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn history_from(h: &Value) -> Option<JiraChangelogEntry> {
    let mut items = Vec::new();
    let mut change = |field: &str, from: Option<String>, to: Option<String>| {
        if from.is_some() || to.is_some() {
            items.push(JiraChangeItem {
                field: field.into(),
                from: from.unwrap_or_default(),
                to: to.unwrap_or_default(),
            });
        }
    };
    change(
        "status",
        opt(&h["fromState"]["name"]),
        opt(&h["toState"]["name"]),
    );
    change(
        "assignee",
        user_name(&h["fromAssignee"]),
        user_name(&h["toAssignee"]),
    );
    change(
        "priority",
        h["fromPriority"].as_f64().map(priority_label),
        h["toPriority"].as_f64().map(priority_label),
    );
    change("summary", opt(&h["fromTitle"]), opt(&h["toTitle"]));
    if items.is_empty() {
        return None;
    }
    Some(JiraChangelogEntry {
        author: user_name(&h["actor"]).unwrap_or_default(),
        created: s(h, "createdAt"),
        items,
    })
}

fn priority_label(p: f64) -> String {
    match p as i64 {
        1 => "Urgent",
        2 => "High",
        3 => "Medium",
        4 => "Low",
        _ => "No priority",
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_keys_go_bare_and_oauth_tokens_get_bearer() {
        assert_eq!(auth_value(" lin_api_abc "), "lin_api_abc");
        assert_eq!(auth_value("oauth-tok"), "Bearer oauth-tok");
        assert_eq!(auth_value("Bearer x"), "Bearer x");
    }

    #[test]
    fn raw_filters_must_be_json_objects() {
        assert!(parse_filter(r#"{"team":{"key":{"eq":"ENG"}}}"#).is_ok());
        for bad in ["", "project = X", "[1]", "\"x\""] {
            assert!(
                matches!(parse_filter(bad), Err(Error::Invalid(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn graphql_errors_map_to_otto_errors() {
        let nf = json!({"message":"Entity not found: Issue","extensions":{"code":"INVALID_INPUT"}});
        assert!(matches!(map_error("issue", 200, &nf), Error::NotFound(_)));
        let auth = json!({"message":"Authentication required","extensions":{"type":"authentication error"}});
        assert!(
            matches!(map_error("issue", 400, &auth), Error::Upstream(m) if m.contains("API key"))
        );
        let bad = json!({"message":"Field 'x' not defined","extensions":{"code":"GRAPHQL_VALIDATION_FAILED"}});
        assert!(matches!(map_error("issues", 400, &bad), Error::Invalid(_)));
        let other = json!({"message":"Rate limited","extensions":{"code":"RATELIMITED"}});
        assert!(matches!(
            map_error("issues", 429, &other),
            Error::Upstream(_)
        ));
    }

    fn brief() -> Value {
        json!({
            "id": "uuid-1", "identifier": "ENG-12", "title": "Fix login",
            "url": "https://linear.app/acme/issue/ENG-12/fix-login",
            "priority": 2.0, "priorityLabel": "High", "updatedAt": "2026-10-01T10:00:00.000Z",
            "state": {"name": "In Progress", "type": "started"},
            "team": {"key": "ENG", "name": "Engineering"},
            "assignee": {"name": "Dana", "displayName": "dana"},
            "parent": {"identifier": "ENG-1", "title": "Auth epic"}
        })
    }

    #[test]
    fn summaries_and_my_work_rows_map_linear_fields() {
        let sm = summary_from(&brief());
        assert_eq!(sm.key, "ENG-12");
        assert_eq!(sm.status, "In Progress");
        assert_eq!(sm.issue_type, "Sub-issue");
        assert_eq!(sm.parent_key.as_deref(), Some("ENG-1"));
        assert_eq!(sm.assignee.as_deref(), Some("Dana"));
        assert_eq!(sm.updated.as_deref(), Some("2026-10-01T10:00:00.000Z"));

        let mw = my_work_from(&brief());
        assert_eq!(mw.project_key, "ENG");
        assert_eq!(mw.project_name, "Engineering");
        assert_eq!(mw.priority.as_deref(), Some("High"));
        assert_eq!(mw.parent_summary.as_deref(), Some("Auth epic"));

        let mut none = brief();
        none["priority"] = json!(0);
        none["priorityLabel"] = json!("No priority");
        none["parent"] = Value::Null;
        none["assignee"] = Value::Null;
        let mw = my_work_from(&none);
        assert_eq!(mw.priority, None);
        assert_eq!(mw.parent_key, None);
        let sm = summary_from(&none);
        assert_eq!(sm.issue_type, GENERIC_TYPE);
        assert_eq!(sm.assignee, None);
    }

    #[test]
    fn transitions_are_the_other_team_states_in_board_order() {
        let issue = json!({
            "state": {"id": "s2"},
            "team": {"states": {"nodes": [
                {"id": "s4", "name": "Done", "type": "completed", "position": 0},
                {"id": "s2", "name": "In Progress", "type": "started", "position": 0},
                {"id": "s1", "name": "Todo", "type": "unstarted", "position": 1},
                {"id": "s0", "name": "Backlog", "type": "backlog", "position": 0},
                {"id": "s3", "name": "In Review", "type": "started", "position": 2}
            ]}}
        });
        let names: Vec<String> = transitions_from(&issue)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["Backlog", "Todo", "In Review", "Done"]);
        let first = &transitions_from(&issue)[0];
        assert_eq!(first.id, "s0");
        assert_eq!(first.to_status, "Backlog");
    }

    #[test]
    fn full_issue_maps_links_fields_history_and_prs() {
        let i = json!({
            "id": "uuid-12", "identifier": "ENG-12", "title": "Fix login",
            "url": "https://linear.app/acme/issue/ENG-12", "description": "# Steps\n1. x",
            "priority": 1, "priorityLabel": "Urgent", "estimate": 3.0,
            "createdAt": "2026-09-01T00:00:00.000Z", "updatedAt": "2026-10-01T00:00:00.000Z",
            "dueDate": "2026-10-20", "branchName": "dana/eng-12-fix-login",
            "state": {"id": "s2", "name": "In Progress", "type": "started"},
            "team": {"id": "t1", "key": "ENG", "name": "Engineering"},
            "assignee": {"id": "u1", "name": "Dana", "displayName": "dana", "avatarUrl": null},
            "creator": {"id": "u2", "name": "Lee", "displayName": "lee", "avatarUrl": "https://a/x.png"},
            "parent": null,
            "children": {"nodes": [{"identifier": "ENG-13", "title": "Sub", "state": {"name": "Todo"}}]},
            "labels": {"nodes": [{"name": "Bug"}, {"name": "Auth"}]},
            "project": {"name": "Q4 Auth"}, "cycle": {"number": 7.0, "name": null},
            "projectMilestone": null,
            "relations": {"nodes": [{"type": "blocks", "relatedIssue": {"identifier": "ENG-20", "title": "Deploy", "state": {"name": "Todo"}}}]},
            "inverseRelations": {"nodes": [{"type": "blocks", "issue": {"identifier": "ENG-5", "title": "Infra", "state": {"name": "Done"}}}]},
            "attachments": {"nodes": [
                {"id": "a1", "title": "PR #44", "subtitle": "Open", "url": "https://github.com/acme/app/pull/44", "sourceType": "github", "updatedAt": "2026-10-02T00:00:00.000Z", "metadata": {"status": "open", "repoName": "app"}},
                {"id": "a2", "title": "Spec", "subtitle": null, "url": "https://docs.example/spec", "sourceType": "api", "updatedAt": "2026-10-02T00:00:00.000Z", "metadata": {}}
            ]},
            "history": {"nodes": [
                {"createdAt": "2026-09-03T00:00:00.000Z", "actor": {"name": "Dana"}, "fromState": {"name": "Todo"}, "toState": {"name": "In Progress"}, "fromAssignee": null, "toAssignee": null, "fromPriority": null, "toPriority": null, "fromTitle": null, "toTitle": null},
                {"createdAt": "2026-09-02T00:00:00.000Z", "actor": null, "fromState": null, "toState": null, "fromAssignee": null, "toAssignee": null, "fromPriority": 3.0, "toPriority": 1.0, "fromTitle": null, "toTitle": null},
                {"createdAt": "2026-09-01T00:00:00.000Z", "actor": {"name": "Lee"}, "fromState": null, "toState": null, "fromAssignee": null, "toAssignee": null, "fromPriority": null, "toPriority": null, "fromTitle": null, "toTitle": null}
            ]}
        });
        let f = parse_issue_full(&i);
        assert_eq!(f.key, "ENG-12");
        assert_eq!(f.id, "uuid-12");
        assert_eq!(f.issue_type, GENERIC_TYPE);
        assert_eq!(f.description_md, "# Steps\n1. x");
        assert_eq!(f.priority.as_deref(), Some("Urgent"));
        assert_eq!(f.estimate.as_deref(), Some("3 pts"));
        assert_eq!(f.labels, ["Bug", "Auth"]);
        assert_eq!(f.assignee.as_ref().unwrap().account_id, "u1");
        assert_eq!(f.reporter.as_ref().unwrap().display_name, "Lee");
        let field = |k: &str| {
            f.fields
                .iter()
                .find(|x| x.key == k)
                .map(|x| x.value.clone())
        };
        assert_eq!(field("team").as_deref(), Some("Engineering (ENG)"));
        assert_eq!(field("cycle").as_deref(), Some("Cycle 7"));
        assert_eq!(field("projectMilestone"), None);
        assert!(field("attachments")
            .unwrap()
            .contains("https://docs.example/spec"));
        let rels: Vec<(String, String)> = f
            .links
            .iter()
            .map(|l| (l.rel.clone(), l.key.clone()))
            .collect();
        assert_eq!(
            rels,
            [
                ("Sub-issue".to_string(), "ENG-13".to_string()),
                ("blocks".to_string(), "ENG-20".to_string()),
                ("is blocked by".to_string(), "ENG-5".to_string()),
            ]
        );
        // Empty history entries are dropped; the rest run oldest first.
        assert_eq!(f.history.len(), 2);
        assert_eq!(f.history[0].items[0].field, "priority");
        assert_eq!(f.history[0].items[0].from, "Medium");
        assert_eq!(f.history[0].items[0].to, "Urgent");
        assert_eq!(f.history[1].items[0].to, "In Progress");

        let prs = prs_from(&i["attachments"]);
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].status, "open");
        assert_eq!(prs[0].repo, "app");
    }

    #[test]
    fn comments_fall_back_to_the_bot_actor() {
        let c = comment_from(&json!({
            "id": "c1", "body": "hi", "createdAt": "2026-10-01T00:00:00.000Z",
            "user": null, "botActor": {"name": "GitHub"}
        }));
        assert_eq!(c.author, "GitHub");
        assert_eq!(c.body_md, "hi");
    }
}
