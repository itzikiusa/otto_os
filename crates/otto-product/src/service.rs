//! ProductService — orchestrates multi-step product story workflows.

use std::sync::Arc;

use otto_core::domain::IssueAccount;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_issues::{markdown_to_storage, storage_to_markdown, CommentRef, ConfluenceClient, JiraClient, PageComment, IssueComment};
use otto_state::{IssuesRepo, NewEvent, NewStory, NewVersion, ProductQuestion, ProductRepo, QuestionPatch, StoryPatch};
use tracing;

use crate::types::{ImportStoryReq, InjectBundle, InjectSection, ProductStoryDetail, StoryCounts};

/// High-level service for product story analysis workflows.
///
/// Holds repos and the secret store; all complex multi-step operations
/// (import, refresh, post questions, publish) are wired here in later phases.
pub struct ProductService {
    pub(crate) repo: ProductRepo,
    pub(crate) issues: IssuesRepo,
    pub(crate) secrets: Arc<dyn SecretStore>,
}

/// A single comment fetched from the issue tracker (Jira or Confluence).
#[derive(Clone)]
pub struct CommentInfo {
    pub id: String,
    pub author: String,
    pub body_md: String,
    pub created: String,
}

impl CommentInfo {
    fn from_jira(c: IssueComment) -> Self {
        Self {
            id: c.id,
            author: c.author,
            body_md: c.body_md,
            created: c.created,
        }
    }

    fn from_confluence(c: PageComment) -> Self {
        Self {
            id: c.id,
            author: c.author,
            body_md: c.body_md,
            created: c.created,
        }
    }
}

/// Intermediate representation of a fetched source document.
pub struct FetchedSource {
    pub title: String,
    pub url: String,
    pub body_md: String,
    pub raw_json: Option<String>,
    pub issue_type: Option<String>,
}

impl ProductService {
    pub fn new(repo: ProductRepo, issues: IssuesRepo, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            repo,
            issues,
            secrets,
        }
    }

    // ---------------------------------------------------------------------------
    // Internal helpers
    // ---------------------------------------------------------------------------

    /// Load the API token for `account` from the secret store.
    fn account_token(&self, account: &IssueAccount) -> Result<String> {
        self.secrets
            .get(&account.token_ref)?
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "missing token for issue account {}",
                    account.id
                ))
            })
    }

    /// Load the source document from the issue tracker and convert to Markdown.
    ///
    /// Supports `source_kind` values: `"jira"` and `"confluence"`.
    pub(crate) async fn fetch_source(
        &self,
        account: &IssueAccount,
        source_kind: &str,
        source_key: &str,
    ) -> Result<FetchedSource> {
        let token = self.account_token(account)?;

        match source_kind {
            "jira" => {
                let client = JiraClient::new(&account.base_url, &account.email, &token);
                let issue = client.get_issue(source_key).await?;
                let raw_json = serde_json::to_string(&serde_json::json!({
                    "key": issue.key,
                    "summary": issue.summary,
                    "status": issue.status,
                    "issue_type": issue.issue_type,
                    "url": issue.url,
                    "description": issue.description,
                    "assignee": issue.assignee
                }))
                .ok();
                Ok(FetchedSource {
                    title: issue.summary,
                    url: issue.url,
                    body_md: issue.description,
                    raw_json,
                    issue_type: if issue.issue_type.is_empty() {
                        None
                    } else {
                        Some(issue.issue_type)
                    },
                })
            }
            "confluence" => {
                let client = ConfluenceClient::new(&account.base_url, &account.email, &token);
                let page = client.get_page(source_key).await?;
                let body_md = storage_to_markdown(&page.body_storage);
                let raw_json = serde_json::to_string(&serde_json::json!({
                    "id": page.id,
                    "title": page.title,
                    "body_storage": page.body_storage,
                    "version": page.version,
                    "space_key": page.space_key,
                    "url": page.url
                }))
                .ok();
                Ok(FetchedSource {
                    title: page.title,
                    url: page.url,
                    body_md,
                    raw_json,
                    issue_type: Some("page".into()),
                })
            }
            other => Err(Error::Invalid(format!(
                "unsupported source_kind: {other}"
            ))),
        }
    }

    /// Build and return a full `ProductStoryDetail` for a given story ID.
    pub(crate) async fn story_detail(&self, story_id: &Id) -> Result<ProductStoryDetail> {
        let story = self.repo.get_story(story_id).await?;
        let source = self.repo.latest_source_version(story_id).await?;
        let versions = self.repo.list_versions(story_id).await?;
        let analyses = self.repo.list_analyses(story_id).await?;
        let questions = self.repo.list_questions(story_id).await?;
        let notes = self.repo.list_notes(story_id).await?;
        let runs = self.repo.list_testcase_runs(story_id).await?;
        let open_questions = questions.iter().filter(|q| q.status == "open").count() as i64;
        let mut testcase_count: i64 = 0;
        for run in &runs {
            let tcs = self.repo.list_testcases(&run.id).await?;
            testcase_count += tcs.len() as i64;
        }
        Ok(ProductStoryDetail {
            story,
            source,
            counts: StoryCounts {
                versions: versions.len() as i64,
                analyses: analyses.len() as i64,
                open_questions,
                notes: notes.len() as i64,
                testcases: testcase_count,
            },
        })
    }

    // ---------------------------------------------------------------------------
    // Internal record-writing path (factored for testability)
    // ---------------------------------------------------------------------------

    /// Create a new story record + initial source version + 'imported' event.
    ///
    /// Separated from network I/O so it can be tested in isolation.
    pub(crate) async fn record_import(
        &self,
        workspace_id: &Id,
        req: &ImportStoryReq,
        user_id: &Id,
        src: FetchedSource,
    ) -> Result<ProductStoryDetail> {
        // 1. Create the story row.
        let story = self
            .repo
            .create_story(NewStory {
                workspace_id: workspace_id.clone(),
                source_kind: req.source_kind.clone(),
                account_id: req.account_id.clone(),
                source_key: req.source_key.clone(),
                title: src.title.clone(),
                url: src.url.clone(),
                issue_type: src.issue_type.clone(),
                stage: "imported".into(),
                cwd: req.cwd.clone(),
                created_by: user_id.clone(),
            })
            .await?;

        // 2. If watch_enabled was requested, apply it now.
        if req.watch_enabled == Some(true) {
            self.repo
                .update_story(
                    &story.id,
                    StoryPatch {
                        watch_enabled: Some(true),
                        ..Default::default()
                    },
                )
                .await?;
        }

        // 3. Create the first 'source' version.
        self.repo
            .add_version(NewVersion {
                story_id: story.id.clone(),
                kind: "source".into(),
                title: src.title.clone(),
                body_md: src.body_md,
                raw_json: src.raw_json,
                change_notes: None,
                created_by: user_id.clone(),
            })
            .await?;

        // 4. Emit an 'imported' event.
        self.repo
            .add_event(NewEvent {
                story_id: story.id.clone(),
                section: "source".into(),
                kind: "imported".into(),
                summary: format!(
                    "Imported {} {}",
                    req.source_kind, req.source_key
                ),
                actor_id: Some(user_id.clone()),
                meta_json: None,
            })
            .await?;

        self.story_detail(&story.id).await
    }

    // ---------------------------------------------------------------------------
    // Public API
    // ---------------------------------------------------------------------------

    /// Import a story from an issue-tracker account.
    /// Fetches the source, creates the story + first source version + 'imported' event.
    pub async fn import_story(
        &self,
        workspace_id: &Id,
        req: &ImportStoryReq,
        user_id: &Id,
    ) -> Result<ProductStoryDetail> {
        let account = self.issues.get_account(&req.account_id).await?;
        let src = self.fetch_source(&account, &req.source_kind, &req.source_key).await?;
        self.record_import(workspace_id, req, user_id, src).await
    }

    /// Refresh a story from its issue-tracker source.
    ///
    /// Re-fetches the source document. If the body changed (or there is no prior
    /// source version), adds a new `'source'` version and a `'refreshed'` event.
    pub async fn refresh_story(
        &self,
        story_id: &Id,
        user_id: &Id,
    ) -> Result<ProductStoryDetail> {
        let story = self.repo.get_story(story_id).await?;
        let account = self.issues.get_account(&story.account_id).await?;
        let src = self
            .fetch_source(&account, &story.source_kind, &story.source_key)
            .await?;

        let latest = self.repo.latest_source_version(story_id).await?;
        let changed = match &latest {
            None => true,
            Some(v) => v.body_md != src.body_md,
        };

        if changed {
            self.repo
                .add_version(NewVersion {
                    story_id: story_id.clone(),
                    kind: "source".into(),
                    title: src.title.clone(),
                    body_md: src.body_md,
                    raw_json: src.raw_json,
                    change_notes: None,
                    created_by: user_id.clone(),
                })
                .await?;

            self.repo
                .add_event(NewEvent {
                    story_id: story_id.clone(),
                    section: "source".into(),
                    kind: "refreshed".into(),
                    summary: format!(
                        "Refreshed {} {}",
                        story.source_kind, story.source_key
                    ),
                    actor_id: Some(user_id.clone()),
                    meta_json: None,
                })
                .await?;
        }

        self.story_detail(story_id).await
    }

    // ---------------------------------------------------------------------------
    // Watch: list new comments since a cursor
    // ---------------------------------------------------------------------------

    /// List comments on the story's source (Jira issue or Confluence page)
    /// that were created **after** `since` (RFC3339 lexicographic comparison).
    /// If `since` is `None`, all comments are returned. Results are sorted
    /// ascending by `created`.
    pub async fn list_new_comments(
        &self,
        story_id: &Id,
        since: Option<&str>,
    ) -> Result<Vec<CommentInfo>> {
        let story = self.repo.get_story(story_id).await?;
        let account = self.issues.get_account(&story.account_id).await?;
        let token = self.account_token(&account)?;

        let mut comments: Vec<CommentInfo> = match story.source_kind.as_str() {
            "jira" => {
                let client = JiraClient::new(&account.base_url, &account.email, &token);
                client
                    .list_comments(&story.source_key)
                    .await?
                    .into_iter()
                    .map(CommentInfo::from_jira)
                    .collect()
            }
            "confluence" => {
                let client = ConfluenceClient::new(&account.base_url, &account.email, &token);
                client
                    .list_comments(&story.source_key)
                    .await?
                    .into_iter()
                    .map(CommentInfo::from_confluence)
                    .collect()
            }
            other => {
                return Err(Error::Invalid(format!(
                    "list_new_comments: unsupported source_kind: {other}"
                )))
            }
        };

        // Filter to those strictly after `since` (RFC3339 strings sort lexicographically).
        if let Some(cursor) = since {
            comments.retain(|c| c.created.as_str() > cursor);
        }

        // Sort ascending by created.
        comments.sort_by(|a, b| a.created.cmp(&b.created));

        Ok(comments)
    }

    // ---------------------------------------------------------------------------
    // Pure helper (factored for unit testing)
    // ---------------------------------------------------------------------------

    /// Build the markdown comment body for a batch of questions.
    ///
    /// Produces:
    /// ```text
    /// Clarifying questions from Otto:
    ///
    /// 1. Question text one
    /// 2. Question text two
    /// ```
    pub(crate) fn build_questions_comment(questions: &[ProductQuestion]) -> String {
        let mut out = "Clarifying questions from Otto:\n\n".to_string();
        for (i, q) in questions.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, q.text));
        }
        out
    }

    /// Post selected questions to the issue tracker.
    ///
    /// Builds ONE combined comment with a numbered list of all selected questions,
    /// posts it to the story's Jira issue or Confluence page, then marks every
    /// question as `status = "posted"` with `posted_ref` set to the comment URL
    /// (or id if URL is absent) and emits a `question_posted` event.
    pub async fn post_questions(
        &self,
        story_id: &Id,
        ids: &[Id],
        _format: Option<&str>,
        user_id: &Id,
    ) -> Result<Vec<(Id, CommentRef)>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }

        // 1. Load story + account.
        let story = self.repo.get_story(story_id).await?;
        let account = self.issues.get_account(&story.account_id).await?;
        let token = self.account_token(&account)?;

        // 2. Load each requested question (skipping any whose story_id doesn't match).
        let mut questions: Vec<ProductQuestion> = Vec::with_capacity(ids.len());
        for id in ids {
            let q = self.repo.get_question(id).await?;
            if &q.story_id != story_id {
                continue; // defensive: skip questions belonging to a different story
            }
            questions.push(q);
        }

        if questions.is_empty() {
            return Ok(vec![]);
        }

        // 3. Build the combined comment body.
        let body_md = Self::build_questions_comment(&questions);

        // 4. Post the comment to the appropriate tracker.
        let comment_ref = match story.source_kind.as_str() {
            "jira" => {
                let client = JiraClient::new(&account.base_url, &account.email, &token);
                client.add_comment(&story.source_key, &body_md).await?
            }
            "confluence" => {
                let client = ConfluenceClient::new(&account.base_url, &account.email, &token);
                client
                    .add_comment(&story.source_key, &markdown_to_storage(&body_md))
                    .await?
            }
            other => {
                return Err(Error::Invalid(format!(
                    "post_questions: unsupported source_kind: {other}"
                )))
            }
        };

        // 5. Compute the posted_ref (prefer URL, fall back to id).
        let posted_ref_value = comment_ref
            .url
            .clone()
            .unwrap_or_else(|| comment_ref.id.clone());

        // 6. Update every question: status → "posted", posted_ref set.
        let mut results: Vec<(Id, CommentRef)> = Vec::with_capacity(questions.len());
        for q in &questions {
            self.repo
                .update_question(
                    &q.id,
                    QuestionPatch {
                        text: None,
                        rationale: None,
                        category: None,
                        status: Some("posted".into()),
                        answer: None,
                        posted_ref: Some(Some(posted_ref_value.clone())),
                    },
                )
                .await?;
            results.push((q.id.clone(), comment_ref.clone()));
        }

        // 7. Emit a single event.
        self.repo
            .add_event(NewEvent {
                story_id: story_id.clone(),
                section: "questions".into(),
                kind: "question_posted".into(),
                summary: format!(
                    "Posted {} question(s) to {} {}",
                    questions.len(),
                    story.source_kind,
                    story.source_key
                ),
                actor_id: Some(user_id.clone()),
                meta_json: None,
            })
            .await?;

        Ok(results)
    }

    /// Publish a suggested version to the issue tracker.
    ///
    /// - Jira: updates the issue description with the version body.
    /// - Confluence: fetches the current page version, then updates the page body.
    ///
    /// On success, records a `published` version and a `publish` event, and
    /// returns `(url, ref)` where `ref` is the story's `source_key`.
    pub async fn publish_version(
        &self,
        version_id: &Id,
        by: &Id,
    ) -> Result<(String, String)> {
        // 1. Load version and its story.
        let version = self.repo.get_version(version_id).await?;
        let story = self.repo.get_story(&version.story_id).await?;
        let account = self.issues.get_account(&story.account_id).await?;
        let token = self.account_token(&account)?;

        // 2. Push to the issue tracker.
        match story.source_kind.as_str() {
            "jira" => {
                let client = JiraClient::new(&account.base_url, &account.email, &token);
                client
                    .update_description(&story.source_key, &version.body_md)
                    .await?;
            }
            "confluence" => {
                let client = ConfluenceClient::new(&account.base_url, &account.email, &token);
                let page = client.get_page(&story.source_key).await?;
                client
                    .update_page(
                        &story.source_key,
                        &page.title,
                        &markdown_to_storage(&version.body_md),
                        page.version,
                    )
                    .await?;
            }
            other => {
                return Err(Error::Invalid(format!(
                    "publish_version: unsupported source_kind: {other}"
                )));
            }
        }

        // 3. Record a 'published' version row.
        self.repo
            .add_version(NewVersion {
                story_id: story.id.clone(),
                kind: "published".into(),
                title: version.title.clone(),
                body_md: version.body_md.clone(),
                raw_json: None,
                change_notes: version.change_notes.clone(),
                created_by: by.clone(),
            })
            .await?;

        // 4. Emit a 'published' event.
        self.repo
            .add_event(NewEvent {
                story_id: story.id.clone(),
                section: "publish".into(),
                kind: "published".into(),
                summary: format!(
                    "Published version '{}' to {} {}",
                    version.title, story.source_kind, story.source_key
                ),
                actor_id: Some(by.clone()),
                meta_json: None,
            })
            .await?;

        Ok((story.url.clone(), story.source_key.clone()))
    }

    /// Render a list of (pre-filtered, approved) test cases as Markdown.
    ///
    /// Groups by category in canonical order: happy → validation → error → edge.
    /// Other categories appear alphabetically after. Parses each case's `steps_json`
    /// (a JSON object `{preconditions:[…], steps:[…], expected:"…"}`) to render
    /// structured sections; tolerates parse failure (renders title at minimum).
    ///
    /// This is a pure function exposed for unit testing.
    pub fn render_testcases_markdown(
        story_title: &str,
        cases: &[otto_state::ProductTestcase],
    ) -> String {
        #[derive(serde::Deserialize)]
        struct StepsJson {
            #[serde(default)]
            preconditions: Vec<String>,
            #[serde(default)]
            steps: Vec<String>,
            #[serde(default)]
            expected: String,
        }

        // Canonical category order.
        const ORDER: &[&str] = &["happy", "validation", "error", "edge"];

        // Collect unique categories in stable order (canonical first, then alpha).
        let mut categories: Vec<String> = {
            let mut seen = std::collections::BTreeSet::new();
            let mut ordered: Vec<String> = Vec::new();
            // First, insert canonical categories that are present.
            for &cat in ORDER {
                if cases.iter().any(|c| c.category == cat) && seen.insert(cat.to_string()) {
                    ordered.push(cat.to_string());
                }
            }
            // Then, any remaining categories alphabetically.
            for c in cases {
                if seen.insert(c.category.clone()) {
                    ordered.push(c.category.clone());
                }
            }
            ordered
        };
        // Sort any extra (non-canonical) categories alphabetically, keeping canonical order first.
        let canonical_count = ORDER
            .iter()
            .filter(|&&cat| cases.iter().any(|c| c.category == cat))
            .count();
        if categories.len() > canonical_count {
            categories[canonical_count..].sort();
        }

        let mut out = format!("# Test Cases — {story_title}\n");

        for category in &categories {
            let mut cat_cases: Vec<&otto_state::ProductTestcase> =
                cases.iter().filter(|c| &c.category == category).collect();
            // Sort within category by order_idx then title for determinism.
            cat_cases.sort_by(|a, b| a.order_idx.cmp(&b.order_idx).then(a.title.cmp(&b.title)));

            let cat_label = {
                let mut s = category.clone();
                if let Some(first) = s.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                s
            };
            out.push_str(&format!("\n## {cat_label}\n"));

            for tc in cat_cases {
                out.push_str(&format!("\n### {}\n", tc.title));
                out.push_str(&format!("\n**Priority:** {}\n", tc.priority));

                // Try to parse steps_json.
                match serde_json::from_str::<StepsJson>(&tc.steps_json) {
                    Ok(parsed) => {
                        if !parsed.preconditions.is_empty() {
                            out.push_str("\n**Preconditions:**\n");
                            for p in &parsed.preconditions {
                                out.push_str(&format!("- {p}\n"));
                            }
                        }
                        if !parsed.steps.is_empty() {
                            out.push_str("\n**Steps:**\n");
                            for (i, s) in parsed.steps.iter().enumerate() {
                                out.push_str(&format!("{}. {s}\n", i + 1));
                            }
                        }
                        if !parsed.expected.is_empty() {
                            out.push_str(&format!("\n**Expected:** {}\n", parsed.expected));
                        }
                    }
                    Err(_) => {
                        // Tolerate parse failure — skip structured section.
                    }
                }
            }
        }

        out
    }

    /// Publish approved test cases from a run to a Confluence page.
    ///
    /// - Loads the run and its story.
    /// - Filters testcases to those with `status == "approved"`.
    /// - Renders them as Markdown and converts to Confluence storage format.
    /// - Resolves the Confluence space: from the `space_key` param, or (for Confluence
    ///   stories) from the source page, or errors for Jira stories without a space_key.
    /// - Creates a new page (or updates the existing one if `run.confluence_page_id` is set).
    /// - Stores the page id and URL on the run via `set_testcase_run`.
    /// - If the story is a Jira issue, adds a best-effort comment with the page URL.
    /// - Updates the story's `confluence_tests_page_id`/`confluence_tests_url` fields.
    /// - Emits a `tests_published` event.
    /// - Returns the page URL.
    pub async fn publish_testcases(
        &self,
        run_id: &Id,
        by: &Id,
        space_key: Option<&str>,
        parent_id: Option<&str>,
    ) -> Result<String> {
        // 1. Load run + story + account.
        let run = self.repo.get_testcase_run(run_id).await?;
        let story = self.repo.get_story(&run.story_id).await?;
        let account = self.issues.get_account(&story.account_id).await?;
        let token = self.account_token(&account)?;

        // 2. Load and filter testcases to approved only.
        let all_cases = self.repo.list_testcases(run_id).await?;
        let approved: Vec<otto_state::ProductTestcase> = all_cases
            .into_iter()
            .filter(|c| c.status == "approved")
            .collect();
        if approved.is_empty() {
            return Err(Error::Invalid(
                "no approved test cases to publish".into(),
            ));
        }

        // 3. Render content.
        let md = Self::render_testcases_markdown(&story.title, &approved);
        let storage = markdown_to_storage(&md);
        let page_title = format!("Test Cases — {}", story.title);

        // 4. Determine Confluence space key.
        let resolved_space: String = if let Some(sk) = space_key {
            sk.to_string()
        } else if story.source_kind == "confluence" {
            let conf_client =
                ConfluenceClient::new(&account.base_url, &account.email, &token);
            let src_page = conf_client.get_page(&story.source_key).await?;
            src_page.space_key
        } else {
            return Err(Error::Invalid(
                "space_key required to publish a Jira story's test cases".into(),
            ));
        };

        // 5. Create or update the Confluence page.
        let conf_client = ConfluenceClient::new(&account.base_url, &account.email, &token);
        let page = if let Some(ref existing_pid) = run.confluence_page_id {
            // Update existing page.
            let existing_page = conf_client.get_page(existing_pid).await?;
            conf_client
                .update_page(
                    existing_pid,
                    &page_title,
                    &storage,
                    existing_page.version,
                )
                .await?
        } else {
            // Create a new page.
            conf_client
                .create_page(&resolved_space, &page_title, &storage, parent_id)
                .await?
        };

        // 6. Persist page id + url on the run.
        self.repo
            .set_testcase_run(run_id, Some("published"), Some(&page.id), Some(&page.url))
            .await?;

        // 7. If the story is a Jira issue, add a best-effort comment.
        if story.source_kind == "jira" {
            let jira_client = JiraClient::new(&account.base_url, &account.email, &token);
            let comment_body =
                format!("Otto test cases published: {}", page.url);
            if let Err(e) = jira_client.add_comment(&story.source_key, &comment_body).await {
                tracing::warn!(
                    "publish_testcases: failed to add Jira comment on {}: {e}",
                    story.source_key
                );
            }
        }

        // 8. Update story-level Confluence test page cache.
        self.repo
            .update_story(
                &story.id,
                otto_state::StoryPatch {
                    confluence_tests_page_id: Some(Some(page.id.clone())),
                    confluence_tests_url: Some(Some(page.url.clone())),
                    ..Default::default()
                },
            )
            .await?;

        // 9. Emit event.
        self.repo
            .add_event(NewEvent {
                story_id: story.id.clone(),
                section: "publish".into(),
                kind: "tests_published".into(),
                summary: format!(
                    "Test cases published to Confluence: {}",
                    page.url
                ),
                actor_id: Some(by.clone()),
                meta_json: None,
            })
            .await?;

        Ok(page.url)
    }

    /// Build and return the inject bundle for agent injection.
    ///
    /// Assembles six sections (each skipped when empty):
    /// 1. Story — title + the most-recent CONTENT version's body_md
    ///    (latest `suggested` if any, else latest `source`).
    /// 2. Analysis Summary — most-recent analysis's summary.
    /// 3. Answered Questions — questions with status=="answered".
    /// 4. Approved Test Cases — latest testcase run's approved cases.
    /// 5. Relevant Learnings — active workspace learnings grouped by kind.
    /// 6. Implementation Plan — latest `kind="plan"` version's body_md.
    pub async fn build_inject_bundle(&self, story_id: &Id) -> Result<InjectBundle> {
        let story = self.repo.get_story(story_id).await?;

        let mut sections: Vec<InjectSection> = Vec::new();

        // --- Section 1: Story ---
        // Find the best version: highest version_no with kind=="suggested",
        // else highest version_no with kind=="source".
        let all_versions = self.repo.list_versions(story_id).await?;
        let best_version_id = {
            let suggested = all_versions.iter().find(|v| v.kind == "suggested");
            let source = all_versions.iter().find(|v| v.kind == "source");
            // list_versions returns DESC by version_no so .find() gives the highest.
            suggested.or(source).map(|v| v.id.clone())
        };

        if let Some(vid) = best_version_id {
            let version = self.repo.get_version(&vid).await?;
            if !version.body_md.trim().is_empty() {
                let body = format!("# {}\n\n{}", story.title, version.body_md.trim());
                sections.push(InjectSection {
                    heading: "Story".into(),
                    body,
                });
            }
        }

        // --- Section 2: Analysis Summary ---
        let analyses = self.repo.list_analyses(story_id).await?;
        // list_analyses returns DESC by created_at; first is the most recent.
        if let Some(analysis) = analyses.first() {
            let summary = analysis.summary.trim().to_string();
            if !summary.is_empty() {
                sections.push(InjectSection {
                    heading: "Analysis Summary".into(),
                    body: summary,
                });
            }
        }

        // --- Section 3: Answered Questions ---
        let questions = self.repo.list_questions(story_id).await?;
        let answered: Vec<_> = questions
            .iter()
            .filter(|q| q.status == "answered")
            .collect();
        if !answered.is_empty() {
            let mut body = String::new();
            for q in &answered {
                let answer = q.answer.as_deref().unwrap_or("").trim();
                body.push_str(&format!(
                    "**Q:** {}\n**A:** {}\n\n",
                    q.text.trim(),
                    answer
                ));
            }
            let body = body.trim_end().to_string();
            sections.push(InjectSection {
                heading: "Answered Questions".into(),
                body,
            });
        }

        // --- Section 4: Approved Test Cases ---
        let runs = self.repo.list_testcase_runs(story_id).await?;
        // list_testcase_runs returns DESC by created_at; first is most recent.
        if let Some(run) = runs.first() {
            let all_cases = self.repo.list_testcases(&run.id).await?;
            let approved: Vec<otto_state::ProductTestcase> = all_cases
                .into_iter()
                .filter(|c| c.status == "approved")
                .collect();
            if !approved.is_empty() {
                let md = Self::render_testcases_markdown(&story.title, &approved);
                sections.push(InjectSection {
                    heading: "Approved Test Cases".into(),
                    body: md,
                });
            }
        }

        // --- Section 5: Relevant Learnings ---
        let learnings = self
            .repo
            .list_learnings(&story.workspace_id, true)
            .await?;
        if !learnings.is_empty() {
            let patterns: Vec<_> = learnings.iter().filter(|l| l.kind == "pattern").collect();
            let avoids: Vec<_> = learnings.iter().filter(|l| l.kind == "avoid").collect();

            let mut body = String::new();
            if !patterns.is_empty() {
                body.push_str("### Patterns to follow\n\n");
                for l in &patterns {
                    body.push_str(&format!("**{}**\n\n{}\n\n", l.title, l.body.trim()));
                }
            }
            if !avoids.is_empty() {
                body.push_str("### Cases to avoid\n\n");
                for l in &avoids {
                    body.push_str(&format!("**{}**\n\n{}\n\n", l.title, l.body.trim()));
                }
            }
            let body = body.trim_end().to_string();
            if !body.is_empty() {
                sections.push(InjectSection {
                    heading: "Relevant Learnings".into(),
                    body,
                });
            }
        }

        // --- Section 6: Implementation Plan ---
        // Latest kind=="plan" version (list_versions omits body_md, so fetch the
        // full row via get_version) — so agents attached via attach-product see it.
        if let Some(plan) = all_versions.iter().find(|v| v.kind == "plan") {
            let version = self.repo.get_version(&plan.id).await?;
            if !version.body_md.trim().is_empty() {
                sections.push(InjectSection {
                    heading: "Implementation Plan".into(),
                    body: version.body_md.trim().to_string(),
                });
            }
        }

        // --- Assemble markdown ---
        let intro = format!(
            "Context bundle for story {} ({}) — refined picture for implementation.",
            story.title, story.source_key
        );
        let mut parts = vec![intro];
        for sec in &sections {
            parts.push(format!("## {}\n\n{}", sec.heading, sec.body));
        }
        let markdown = parts.join("\n\n");

        Ok(InjectBundle { markdown, sections })
    }

    // ---------------------------------------------------------------------------
    // Agent context document (written to a temp file, read by lens/rewrite/test agents)
    // ---------------------------------------------------------------------------

    /// Build a Markdown document describing the story in full detail.
    ///
    /// Agents read this from a temp file (never inlined in the prompt) to keep
    /// prompts short while giving agents the richest possible context.
    ///
    /// Contents:
    /// - Story heading, source reference, url, stage.
    /// - `## Story` — latest CONTENT version body (prefer newest `suggested`, else newest `source`).
    /// - `## Full Jira context` (only when `source_kind == "jira"`) — fetched via
    ///   `JiraClient::get_issue_full`; includes status, assignee, reporter, priority,
    ///   labels, custom fields table, linked issues, comments (last 20), change history
    ///   (last 20), and attachment list. On any Jira fetch error: log + skip section.
    /// - `## Learnings` — active workspace learnings grouped by kind.
    /// - `## FOCUS` (near the top, before Story) when `focus` is non-empty.
    pub async fn build_agent_context(
        &self,
        story_id: &Id,
        focus: Option<&str>,
    ) -> Result<String> {
        let story = self.repo.get_story(story_id).await?;

        // Best content version: newest suggested, else newest source.
        let all_versions = self.repo.list_versions(story_id).await?;
        let body_md = {
            let suggested = all_versions.iter().find(|v| v.kind == "suggested");
            let source = all_versions.iter().find(|v| v.kind == "source");
            match suggested.or(source) {
                Some(v) => {
                    let ver = self.repo.get_version(&v.id).await?;
                    ver.body_md
                }
                None => String::new(),
            }
        };

        let mut doc = String::new();

        // --- Title heading ---
        doc.push_str("# ");
        doc.push_str(&story.title);
        doc.push('\n');
        doc.push_str(&format!(
            "Source: {} `{}`  \nURL: {}  \nStage: {}\n\n",
            story.source_kind, story.source_key, story.url, story.stage
        ));

        // --- FOCUS (near top so agents see it early) ---
        if let Some(f) = focus {
            if !f.trim().is_empty() {
                doc.push_str("## FOCUS — pay special attention to:\n");
                doc.push_str(f.trim());
                doc.push_str("\n\n");
            }
        }

        // --- Story body ---
        doc.push_str("## Story\n\n");
        if !body_md.trim().is_empty() {
            doc.push_str(body_md.trim());
        } else {
            doc.push_str("*(no story body available)*");
        }
        doc.push_str("\n\n");

        // --- Full Jira context (only for jira stories) ---
        if story.source_kind == "jira" {
            // Load account + token; tolerate missing account gracefully.
            let jira_section: Option<String> = async {
                let account = self.issues.get_account(&story.account_id).await.ok()?;
                let token = self.account_token(&account).ok()?;
                let client = JiraClient::new(&account.base_url, &account.email, &token);
                match client.get_issue_full(&story.source_key).await {
                    Err(e) => {
                        tracing::warn!(
                            "build_agent_context: get_issue_full({}) failed: {e}",
                            story.source_key
                        );
                        None
                    }
                    Ok(full) => {
                        let mut s = String::from("## Full Jira context\n\n");

                        // Basic metadata
                        s.push_str(&format!("**Status:** {}  \n", full.status));
                        if let Some(ref a) = full.assignee {
                            s.push_str(&format!("**Assignee:** {}  \n", a.display_name));
                        }
                        if let Some(ref r) = full.reporter {
                            s.push_str(&format!("**Reporter:** {}  \n", r.display_name));
                        }
                        if let Some(ref p) = full.priority {
                            s.push_str(&format!("**Priority:** {}  \n", p));
                        }
                        if !full.labels.is_empty() {
                            s.push_str(&format!("**Labels:** {}  \n", full.labels.join(", ")));
                        }
                        s.push('\n');

                        // Custom / non-empty fields table
                        let nonempty_fields: Vec<&otto_issues::JiraField> =
                            full.fields.iter().filter(|f| !f.value.trim().is_empty()).collect();
                        if !nonempty_fields.is_empty() {
                            s.push_str("### Fields\n\n");
                            s.push_str("| Field | Value |\n|---|---|\n");
                            for f in nonempty_fields.iter().take(50) {
                                // Escape pipes so the table doesn't break
                                let val = f.value.replace('|', "\\|").replace('\n', " ");
                                s.push_str(&format!("| {} | {} |\n", f.name, val));
                            }
                            s.push('\n');
                        }

                        // Linked issues
                        if !full.links.is_empty() {
                            s.push_str("### Linked issues\n\n");
                            for l in &full.links {
                                s.push_str(&format!(
                                    "- **{}** {} — {} *({})*\n",
                                    l.rel, l.key, l.summary, l.status
                                ));
                            }
                            s.push('\n');
                        }

                        // Comments (most recent 20)
                        if !full.comments.is_empty() {
                            s.push_str("### Comments\n\n");
                            let start = full.comments.len().saturating_sub(20);
                            for c in &full.comments[start..] {
                                s.push_str(&format!(
                                    "**{}** ({}):\n{}\n\n",
                                    c.author, c.created, c.body_md
                                ));
                            }
                        }

                        // Change history (last 20 entries)
                        if !full.history.is_empty() {
                            s.push_str("### Change history\n\n");
                            let start = full.history.len().saturating_sub(20);
                            for entry in &full.history[start..] {
                                for item in &entry.items {
                                    s.push_str(&format!(
                                        "- {} changed **{}**: `{}` → `{}` ({})\n",
                                        entry.author, item.field, item.from, item.to, entry.created
                                    ));
                                }
                            }
                            s.push('\n');
                        }

                        // Attachments (list only — no binary content)
                        if !full.attachments.is_empty() {
                            s.push_str("### Attachments\n\n");
                            for att in &full.attachments {
                                s.push_str(&format!(
                                    "- {} · {} · {} bytes\n",
                                    att.filename, att.mime, att.size
                                ));
                            }
                            s.push('\n');
                        }

                        Some(s)
                    }
                }
            }
            .await;

            if let Some(section) = jira_section {
                doc.push_str(&section);
            }
        }

        // --- Active learnings ---
        let learnings = self
            .repo
            .list_learnings(&story.workspace_id, true)
            .await
            .unwrap_or_default();
        if !learnings.is_empty() {
            doc.push_str("## Learnings\n\n");
            for l in &learnings {
                doc.push_str(&format!("### [{}] {}\n\n{}\n\n", l.kind, l.title, l.body.trim()));
            }
        }

        // --- Transcripts ---
        let transcripts = self.repo.list_transcripts(story_id).await.unwrap_or_default();
        if !transcripts.is_empty() {
            doc.push_str("## Transcripts\n\n");
            for t in &transcripts {
                let heading = if t.title.is_empty() {
                    "(untitled)".to_string()
                } else {
                    t.title.clone()
                };
                doc.push_str(&format!("### {heading}\n\n{}\n\n", t.body.trim()));
            }
        }

        Ok(doc)
    }

    // ---------------------------------------------------------------------------
    // Discovery mode helpers
    // ---------------------------------------------------------------------------

    /// Pick the best content version for publishing or agent context.
    /// Priority: newest `suggested` > newest `draft` > newest `source`.
    async fn best_content_version(&self, story_id: &Id) -> Result<Option<String>> {
        let all_versions = self.repo.list_versions(story_id).await?;
        // list_versions returns DESC by version_no, so .find() gives the newest.
        let preferred = all_versions
            .iter()
            .find(|v| v.kind == "suggested")
            .or_else(|| all_versions.iter().find(|v| v.kind == "draft"))
            .or_else(|| all_versions.iter().find(|v| v.kind == "source"));
        match preferred {
            Some(v) => {
                let full = self.repo.get_version(&v.id).await?;
                Ok(Some(full.body_md))
            }
            None => Ok(None),
        }
    }

    // ---------------------------------------------------------------------------
    // Discovery mode: create blank draft
    // ---------------------------------------------------------------------------

    /// Create a blank draft story (not linked to any issue tracker yet).
    pub async fn create_draft(
        &self,
        ws: &Id,
        by: &Id,
        title: Option<&str>,
    ) -> Result<ProductStoryDetail> {
        let draft_title = title.unwrap_or("Untitled draft");

        // 1. Create the story row with source_kind="draft".
        let story = self
            .repo
            .create_story(otto_state::NewStory {
                workspace_id: ws.clone(),
                source_kind: "draft".into(),
                account_id: String::new(),
                source_key: String::new(),
                title: draft_title.into(),
                url: String::new(),
                issue_type: None,
                stage: "draft".into(),
                cwd: None,
                created_by: by.clone(),
            })
            .await?;

        // 2. Create the first blank draft version.
        self.repo
            .add_version(otto_state::NewVersion {
                story_id: story.id.clone(),
                kind: "draft".into(),
                title: draft_title.into(),
                body_md: String::new(),
                raw_json: None,
                change_notes: None,
                created_by: by.clone(),
            })
            .await?;

        // 3. Emit draft_created event.
        self.repo
            .add_event(otto_state::NewEvent {
                story_id: story.id.clone(),
                section: "source".into(),
                kind: "draft_created".into(),
                summary: format!("Draft created: {}", draft_title),
                actor_id: Some(by.clone()),
                meta_json: None,
            })
            .await?;

        self.story_detail(&story.id).await
    }

    // ---------------------------------------------------------------------------
    // Discovery mode: update draft body (in-place edit)
    // ---------------------------------------------------------------------------

    /// Update the draft body in-place (edit the existing draft version, or create
    /// one if missing). Also syncs the story title.
    pub async fn update_draft_body(
        &self,
        story_id: &Id,
        title: &str,
        body_md: &str,
        by: &Id,
    ) -> Result<ProductStoryDetail> {
        // Find the existing draft version, or create one.
        let all_versions = self.repo.list_versions(story_id).await?;
        let draft_version_id = all_versions
            .iter()
            .find(|v| v.kind == "draft")
            .map(|v| v.id.clone());

        let version_id = match draft_version_id {
            Some(vid) => vid,
            None => {
                // Create a new draft version.
                let v = self
                    .repo
                    .add_version(otto_state::NewVersion {
                        story_id: story_id.clone(),
                        kind: "draft".into(),
                        title: title.into(),
                        body_md: body_md.into(),
                        raw_json: None,
                        change_notes: None,
                        created_by: by.clone(),
                    })
                    .await?;
                v.id
            }
        };

        // Edit the version in-place.
        self.repo
            .update_version_body(&version_id, title, body_md)
            .await?;

        // Sync the story title.
        self.repo
            .update_story(
                story_id,
                otto_state::StoryPatch {
                    title: Some(title.into()),
                    ..Default::default()
                },
            )
            .await?;

        // Emit draft_edited event.
        self.repo
            .add_event(otto_state::NewEvent {
                story_id: story_id.clone(),
                section: "source".into(),
                kind: "draft_edited".into(),
                summary: format!("Draft edited: {}", title),
                actor_id: Some(by.clone()),
                meta_json: None,
            })
            .await?;

        self.story_detail(story_id).await
    }

    // ---------------------------------------------------------------------------
    // Discovery mode: publish as Confluence RFC
    // ---------------------------------------------------------------------------

    /// Publish a draft (or any story) to Confluence as an RFC page.
    ///
    /// If the story's source_kind is "draft", the story is REBOUND to the new
    /// Confluence page (it becomes the RFC). Otherwise a published copy is recorded.
    pub async fn publish_as_rfc(
        &self,
        story_id: &Id,
        account_id: &Id,
        space_key: &str,
        parent_id: Option<&str>,
        title: Option<&str>,
        by: &Id,
    ) -> Result<ProductStoryDetail> {
        let story = self.repo.get_story(story_id).await?;
        let account = self.issues.get_account(account_id).await?;
        let token = self.account_token(&account)?;

        // Best content to publish.
        let content_md = self
            .best_content_version(story_id)
            .await?
            .unwrap_or_default();

        let page_title = title.unwrap_or(&story.title);
        let conf_client = ConfluenceClient::new(&account.base_url, &account.email, &token);

        // Create the Confluence page.
        let page = conf_client
            .create_page(
                space_key,
                page_title,
                &markdown_to_storage(&content_md),
                parent_id,
            )
            .await?;

        // Record a published version.
        self.repo
            .add_version(otto_state::NewVersion {
                story_id: story_id.clone(),
                kind: "published".into(),
                title: page_title.into(),
                body_md: content_md,
                raw_json: None,
                change_notes: None,
                created_by: by.clone(),
            })
            .await?;

        // Emit published_rfc event.
        self.repo
            .add_event(otto_state::NewEvent {
                story_id: story_id.clone(),
                section: "publish".into(),
                kind: "published_rfc".into(),
                summary: format!("Published RFC to Confluence: {}", page.url),
                actor_id: Some(by.clone()),
                meta_json: None,
            })
            .await?;

        // If the story was a draft, REBIND it to the Confluence page.
        if story.source_kind == "draft" {
            self.repo
                .update_story(
                    story_id,
                    otto_state::StoryPatch {
                        source_kind: Some("confluence".into()),
                        account_id: Some(account_id.clone()),
                        source_key: Some(page.id.clone()),
                        url: Some(page.url.clone()),
                        stage: Some("refined".into()),
                        title: Some(page_title.into()),
                        ..Default::default()
                    },
                )
                .await?;
        }

        self.story_detail(story_id).await
    }

    // ---------------------------------------------------------------------------
    // Discovery mode: publish as Jira story (or convert RFC → Jira story)
    // ---------------------------------------------------------------------------

    /// Publish a draft/RFC to Jira as a new issue.
    ///
    /// - If source_kind=="draft": REBIND the same story to the new Jira issue.
    /// - Else (Confluence RFC or existing Jira): create a NEW Jira-backed story,
    ///   cross-link back to the original, and return the new story's detail.
    pub async fn publish_as_story(
        &self,
        story_id: &Id,
        account_id: &Id,
        project_key: &str,
        issue_type: &str,
        by: &Id,
    ) -> Result<ProductStoryDetail> {
        let story = self.repo.get_story(story_id).await?;
        let account = self.issues.get_account(account_id).await?;
        let token = self.account_token(&account)?;

        // Best content to publish.
        let content_md = self
            .best_content_version(story_id)
            .await?
            .unwrap_or_default();

        // If original is a Confluence RFC, prepend a reference line.
        let description = if story.source_kind == "confluence" && !story.url.is_empty() {
            format!("> RFC: {}\n\n{}", story.url, content_md)
        } else {
            content_md.clone()
        };

        let jira_client = JiraClient::new(&account.base_url, &account.email, &token);
        let created = jira_client
            .create_issue(project_key, issue_type, &story.title, &description)
            .await?;

        let issue_key = created.key.clone();
        let issue_url = created.url.clone();

        if story.source_kind == "draft" {
            // REBIND the draft story to the new Jira issue.
            self.repo
                .update_story(
                    story_id,
                    otto_state::StoryPatch {
                        source_kind: Some("jira".into()),
                        account_id: Some(account_id.clone()),
                        source_key: Some(issue_key.clone()),
                        url: Some(issue_url.clone()),
                        issue_type: Some(Some(issue_type.into())),
                        stage: Some("refined".into()),
                        ..Default::default()
                    },
                )
                .await?;

            self.repo
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "publish".into(),
                    kind: "published_story".into(),
                    summary: format!("Draft published as Jira issue: {issue_key}"),
                    actor_id: Some(by.clone()),
                    meta_json: Some(
                        serde_json::json!({ "key": issue_key, "url": issue_url }).to_string(),
                    ),
                })
                .await?;

            self.story_detail(story_id).await
        } else {
            // CONVERT: create a NEW Jira-backed story.
            let new_story = self
                .repo
                .create_story(otto_state::NewStory {
                    workspace_id: story.workspace_id.clone(),
                    source_kind: "jira".into(),
                    account_id: account_id.clone(),
                    source_key: issue_key.clone(),
                    title: story.title.clone(),
                    url: issue_url.clone(),
                    issue_type: Some(issue_type.into()),
                    stage: "imported".into(),
                    cwd: story.cwd.clone(),
                    created_by: by.clone(),
                })
                .await?;

            // Record a source version on the new story.
            self.repo
                .add_version(otto_state::NewVersion {
                    story_id: new_story.id.clone(),
                    kind: "source".into(),
                    title: story.title.clone(),
                    body_md: content_md,
                    raw_json: None,
                    change_notes: None,
                    created_by: by.clone(),
                })
                .await?;

            // Emit event on the original story.
            self.repo
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "publish".into(),
                    kind: "converted_to_story".into(),
                    summary: format!("Converted to Jira story {issue_key}"),
                    actor_id: Some(by.clone()),
                    meta_json: Some(
                        serde_json::json!({
                            "new_story_id": new_story.id,
                            "key": issue_key,
                        })
                        .to_string(),
                    ),
                })
                .await?;

            // Emit event on the new story.
            self.repo
                .add_event(otto_state::NewEvent {
                    story_id: new_story.id.clone(),
                    section: "source".into(),
                    kind: "imported_from_rfc".into(),
                    summary: format!("Imported from RFC story {story_id}"),
                    actor_id: Some(by.clone()),
                    meta_json: Some(
                        serde_json::json!({ "source_story_id": story_id.as_str() }).to_string(),
                    ),
                })
                .await?;

            // If original is Confluence, add a best-effort comment with the Jira link.
            if story.source_kind == "confluence" {
                let conf_client =
                    ConfluenceClient::new(&account.base_url, &account.email, &token);
                let comment_md = format!("Linked Jira story: {issue_url}");
                if let Err(e) = conf_client
                    .add_comment(&story.source_key, &markdown_to_storage(&comment_md))
                    .await
                {
                    tracing::warn!(
                        "publish_as_story: failed to add Confluence comment on {}: {e}",
                        story.source_key
                    );
                }
            }

            self.story_detail(&new_story.id).await
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use otto_core::domain::{IssueAccount, IssueProviderKind};
    use otto_core::secrets::SecretStore;
    use otto_core::{new_id, Id, Result};
    use otto_state::{IssuesRepo, ProductRepo};
    use sqlx::SqlitePool;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::types::ImportStoryReq;

    // -----------------------------------------------------------------------
    // In-memory pool + schema
    // -----------------------------------------------------------------------

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        pool
    }

    // Seed a user row so FK constraints are satisfied.
    async fn seed_user(pool: &SqlitePool) -> Id {
        let uid = new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&uid)
        .bind("testuser")
        .bind("hash")
        .bind("Test User")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        uid
    }

    // Seed a workspace row.
    async fn seed_workspace(pool: &SqlitePool) -> Id {
        let wid = new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&wid)
        .bind("ws")
        .bind("/tmp")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        wid
    }

    // -----------------------------------------------------------------------
    // Stubs
    // -----------------------------------------------------------------------

    /// SecretStore that always returns a fixed token for any key.
    struct FixedSecret(String);
    impl SecretStore for FixedSecret {
        fn put(&self, _k: &str, _v: &str) -> Result<()> {
            Ok(())
        }
        fn get(&self, _k: &str) -> Result<Option<String>> {
            Ok(Some(self.0.clone()))
        }
        fn delete(&self, _k: &str) -> Result<()> {
            Ok(())
        }
    }

    /// Seed a real issue account that JiraClient will use.
    /// The account's base_url is set to the wiremock server URL.
    async fn seed_issue_account(pool: &SqlitePool, user_id: &Id, base_url: &str) -> IssueAccount {
        let repo = IssuesRepo::new(pool.clone());
        repo.create_account(otto_state::NewIssueAccount {
            user_id: user_id.clone(),
            provider: IssueProviderKind::Jira,
            label: "Test Jira".into(),
            email: "test@example.com".into(),
            token_ref: "test-token-ref".into(),
            base_url: base_url.into(),
            token_expires_at: None,
        })
        .await
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // Canned Jira issue JSON (a realistic ADF description is included)
    // -----------------------------------------------------------------------

    fn canned_jira_issue_json(key: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "10001",
            "key": key,
            "fields": {
                "summary": "Allow users to reset their password",
                "status": {"name": "In Progress"},
                "issuetype": {"name": "Story"},
                "assignee": {"displayName": "Alice"},
                "description": {
                    "type": "doc",
                    "version": 1,
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [
                                {"type": "text", "text": "As a user I want to reset my password."}
                            ]
                        }
                    ]
                }
            }
        })
    }

    // -----------------------------------------------------------------------
    // Test 1: import_story creates story + version + event (Jira, ADF→md)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn import_story_creates_story_version_event_with_adf_body() {
        // Start a mock HTTP server.
        let server = MockServer::start().await;

        // Register the Jira issue endpoint.
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/PROJ-1"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(canned_jira_issue_json("PROJ-1")),
            )
            .mount(&server)
            .await;

        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws_id = seed_workspace(&pool).await;

        // Use the mock server URL as the Jira base_url.
        let account = seed_issue_account(&pool, &user_id, &server.uri()).await;

        let repo = ProductRepo::new(pool.clone());
        let issues = IssuesRepo::new(pool.clone());
        let secrets: Arc<dyn SecretStore> = Arc::new(FixedSecret("test-api-token".into()));
        let svc = ProductService::new(repo.clone(), issues, secrets);

        let req = ImportStoryReq {
            source_kind: "jira".into(),
            account_id: account.id.clone(),
            source_key: "PROJ-1".into(),
            cwd: None,
            watch_enabled: None,
        };

        let detail = svc.import_story(&ws_id, &req, &user_id).await.unwrap();

        // ---- Story assertions ----
        assert_eq!(detail.story.title, "Allow users to reset their password");
        assert_eq!(detail.story.stage, "imported");
        assert_eq!(detail.story.source_kind, "jira");
        assert_eq!(detail.story.source_key, "PROJ-1");
        assert_eq!(detail.story.issue_type, Some("Story".into()));

        // ---- Version assertions ----
        let source_ver = detail.source.as_ref().expect("should have source version");
        assert_eq!(source_ver.kind, "source");
        assert_eq!(source_ver.version_no, 1);
        // ADF → markdown conversion should have produced the paragraph text.
        assert!(
            source_ver.body_md.contains("As a user I want to reset my password."),
            "body_md did not contain expected text; got: {:?}",
            source_ver.body_md
        );

        // ---- Counts assertions ----
        assert_eq!(detail.counts.versions, 1);

        // ---- Event assertions ----
        let events = repo
            .list_events(&detail.story.id, Some("source"))
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "imported");
        assert!(events[0].summary.contains("PROJ-1"));
    }

    // -----------------------------------------------------------------------
    // Test 2: record_import (no network) — tests pure record-writing path
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn record_import_stores_story_version_event_without_network() {
        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws_id = seed_workspace(&pool).await;
        let account_id = new_id(); // arbitrary id — not used by record_import

        let repo = ProductRepo::new(pool.clone());
        let issues = IssuesRepo::new(pool.clone());
        let secrets: Arc<dyn SecretStore> = Arc::new(FixedSecret("unused".into()));
        let svc = ProductService::new(repo.clone(), issues, secrets);

        let req = ImportStoryReq {
            source_kind: "jira".into(),
            account_id: account_id.clone(),
            source_key: "TEST-42".into(),
            cwd: None,
            watch_enabled: None,
        };

        let src = FetchedSource {
            title: "My Test Story".into(),
            url: "https://jira.example.com/TEST-42".into(),
            body_md: "## Summary\n\nThis is the story body.".into(),
            raw_json: Some("{\"key\":\"TEST-42\"}".into()),
            issue_type: Some("Epic".into()),
        };

        let detail = svc
            .record_import(&ws_id, &req, &user_id, src)
            .await
            .unwrap();

        assert_eq!(detail.story.stage, "imported");
        assert_eq!(detail.story.source_key, "TEST-42");
        assert_eq!(detail.story.issue_type, Some("Epic".into()));

        let ver = detail.source.as_ref().expect("version");
        assert_eq!(ver.kind, "source");
        assert!(ver.body_md.contains("This is the story body."));

        let events = repo
            .list_events(&detail.story.id, Some("source"))
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "imported");
    }

    // -----------------------------------------------------------------------
    // Test 3: refresh_story adds a new version when body changes
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn refresh_story_adds_version_when_body_changes() {
        let server = MockServer::start().await;

        // First call: original body.
        let issue_v1 = serde_json::json!({
            "id": "10001",
            "key": "PROJ-5",
            "fields": {
                "summary": "Login feature",
                "status": {"name": "Open"},
                "issuetype": {"name": "Story"},
                "assignee": null,
                "description": {
                    "type": "doc",
                    "version": 1,
                    "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Version 1"}]}]
                }
            }
        });

        // Second call: changed body.
        let issue_v2 = serde_json::json!({
            "id": "10001",
            "key": "PROJ-5",
            "fields": {
                "summary": "Login feature",
                "status": {"name": "Open"},
                "issuetype": {"name": "Story"},
                "assignee": null,
                "description": {
                    "type": "doc",
                    "version": 1,
                    "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Version 2 - updated"}]}]
                }
            }
        });

        // Mount the mock to respond twice in sequence.
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/PROJ-5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(issue_v1.clone()))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/PROJ-5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(issue_v2))
            .mount(&server)
            .await;

        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws_id = seed_workspace(&pool).await;
        let account = seed_issue_account(&pool, &user_id, &server.uri()).await;

        let repo = ProductRepo::new(pool.clone());
        let issues = IssuesRepo::new(pool.clone());
        let secrets: Arc<dyn SecretStore> = Arc::new(FixedSecret("tok".into()));
        let svc = ProductService::new(repo.clone(), issues, secrets);

        // Import first.
        let req = ImportStoryReq {
            source_kind: "jira".into(),
            account_id: account.id.clone(),
            source_key: "PROJ-5".into(),
            cwd: None,
            watch_enabled: None,
        };
        let detail1 = svc.import_story(&ws_id, &req, &user_id).await.unwrap();
        let story_id = detail1.story.id.clone();
        assert_eq!(detail1.counts.versions, 1);

        // Refresh — body changed, expect new version.
        let detail2 = svc.refresh_story(&story_id, &user_id).await.unwrap();
        assert_eq!(detail2.counts.versions, 2);

        // Check the latest source version has the updated body.
        let ver = detail2.source.as_ref().unwrap();
        assert!(
            ver.body_md.contains("Version 2 - updated"),
            "expected updated body; got: {:?}",
            ver.body_md
        );

        // A 'refreshed' event should exist.
        let events = repo.list_events(&story_id, Some("source")).await.unwrap();
        assert_eq!(events.len(), 2); // 'imported' + 'refreshed'
        assert_eq!(events[1].kind, "refreshed");
    }

    // -----------------------------------------------------------------------
    // Test 4: refresh_story does NOT add a version when body is unchanged
    // -----------------------------------------------------------------------
    // (Tests 5 and 6 are for post_questions — see below after test 4)

    #[tokio::test]
    async fn refresh_story_no_new_version_when_body_unchanged() {
        let server = MockServer::start().await;

        let issue_json = canned_jira_issue_json("PROJ-7");

        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/PROJ-7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(issue_json))
            .mount(&server)
            .await;

        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws_id = seed_workspace(&pool).await;
        let account = seed_issue_account(&pool, &user_id, &server.uri()).await;

        let repo = ProductRepo::new(pool.clone());
        let issues = IssuesRepo::new(pool.clone());
        let secrets: Arc<dyn SecretStore> = Arc::new(FixedSecret("tok".into()));
        let svc = ProductService::new(repo.clone(), issues, secrets);

        let req = ImportStoryReq {
            source_kind: "jira".into(),
            account_id: account.id.clone(),
            source_key: "PROJ-7".into(),
            cwd: None,
            watch_enabled: None,
        };
        let detail1 = svc.import_story(&ws_id, &req, &user_id).await.unwrap();
        let story_id = detail1.story.id.clone();

        // Refresh — same body, no new version expected.
        let detail2 = svc.refresh_story(&story_id, &user_id).await.unwrap();
        assert_eq!(detail2.counts.versions, 1, "should still be 1 version");

        let events = repo.list_events(&story_id, Some("source")).await.unwrap();
        // Only the 'imported' event, no 'refreshed'.
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "imported");
    }

    // -----------------------------------------------------------------------
    // Test 5: build_questions_comment — pure unit test (no I/O)
    // -----------------------------------------------------------------------

    #[test]
    fn build_questions_comment_formats_correctly() {
        use chrono::Utc;
        let now = Utc::now();
        let dummy_id = new_id();
        let make_q = |text: &str| ProductQuestion {
            id: new_id(),
            story_id: dummy_id.clone(),
            analysis_id: None,
            text: text.to_string(),
            rationale: String::new(),
            category: "general".into(),
            status: "open".into(),
            answer: None,
            posted_ref: None,
            created_by: dummy_id.clone(),
            created_at: now,
            updated_at: now,
        };

        let q1 = make_q("What is the acceptance criteria?");
        let q2 = make_q("Who is the target user?");
        let body = ProductService::build_questions_comment(&[q1, q2]);

        // Header present
        assert!(
            body.contains("Clarifying questions from Otto:"),
            "missing header; got: {body:?}"
        );
        // Both questions appear, numbered
        assert!(body.contains("1. What is the acceptance criteria?"), "q1 missing; got: {body:?}");
        assert!(body.contains("2. Who is the target user?"), "q2 missing; got: {body:?}");
        // Correct ordering (1 before 2)
        let pos1 = body.find("1.").unwrap();
        let pos2 = body.find("2.").unwrap();
        assert!(pos1 < pos2, "questions out of order");
    }

    // -----------------------------------------------------------------------
    // Test 6: post_questions — full integration with wiremock Jira
    // -----------------------------------------------------------------------

    fn canned_comment_response(id: &str, url: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "self": url,
        })
    }

    #[tokio::test]
    async fn post_questions_posts_combined_comment_and_updates_status() {
        let server = MockServer::start().await;

        // Stub the Jira issue fetch (import needs it).
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/PROJ-10"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(canned_jira_issue_json("PROJ-10")),
            )
            .mount(&server)
            .await;

        // Stub the Jira add_comment endpoint.
        let comment_url = format!("{}/rest/api/3/issue/PROJ-10/comment/42", server.uri());
        Mock::given(method("POST"))
            .and(path("/rest/api/3/issue/PROJ-10/comment"))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(canned_comment_response("42", &comment_url)),
            )
            .mount(&server)
            .await;

        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws_id = seed_workspace(&pool).await;
        let account = seed_issue_account(&pool, &user_id, &server.uri()).await;

        let repo = ProductRepo::new(pool.clone());
        let issues = IssuesRepo::new(pool.clone());
        let secrets: Arc<dyn SecretStore> = Arc::new(FixedSecret("test-api-token".into()));
        let svc = ProductService::new(repo.clone(), issues, secrets);

        // Import the story so we have a real story row.
        let req = ImportStoryReq {
            source_kind: "jira".into(),
            account_id: account.id.clone(),
            source_key: "PROJ-10".into(),
            cwd: None,
            watch_enabled: None,
        };
        let detail = svc.import_story(&ws_id, &req, &user_id).await.unwrap();
        let story_id = detail.story.id.clone();

        // Seed two questions directly via the repo.
        let q1 = repo
            .create_question(otto_state::NewQuestion {
                story_id: story_id.clone(),
                analysis_id: None,
                text: "What is the rollback plan?".into(),
                rationale: String::new(),
                category: "risk".into(),
                created_by: user_id.clone(),
            })
            .await
            .unwrap();
        let q2 = repo
            .create_question(otto_state::NewQuestion {
                story_id: story_id.clone(),
                analysis_id: None,
                text: "How will success be measured?".into(),
                rationale: String::new(),
                category: "scope".into(),
                created_by: user_id.clone(),
            })
            .await
            .unwrap();

        // Call post_questions with both question ids.
        let results = svc
            .post_questions(&story_id, &[q1.id.clone(), q2.id.clone()], None, &user_id)
            .await
            .unwrap();

        // Should return 2 entries, one per question.
        assert_eq!(results.len(), 2, "expected 2 result entries");

        // Both should reference the same comment id "42".
        for (_, cref) in &results {
            assert_eq!(cref.id, "42");
            assert!(cref.url.as_deref().unwrap_or("").contains("42"));
        }

        // Both questions should now be status="posted" with posted_ref set.
        let updated_q1 = repo.get_question(&q1.id).await.unwrap();
        assert_eq!(updated_q1.status, "posted", "q1 status should be posted");
        assert!(
            updated_q1.posted_ref.is_some(),
            "q1 posted_ref should be set"
        );

        let updated_q2 = repo.get_question(&q2.id).await.unwrap();
        assert_eq!(updated_q2.status, "posted", "q2 status should be posted");
        assert!(
            updated_q2.posted_ref.is_some(),
            "q2 posted_ref should be set"
        );

        // A single 'question_posted' event should exist in the "questions" section.
        let events = repo.list_events(&story_id, Some("questions")).await.unwrap();
        assert_eq!(events.len(), 1, "expected exactly one question_posted event");
        assert_eq!(events[0].kind, "question_posted");
        assert!(
            events[0].summary.contains("PROJ-10"),
            "event summary should mention the issue key"
        );
        assert!(
            events[0].summary.contains("2"),
            "event summary should mention question count"
        );

        // Verify wiremock received exactly one POST to the comment endpoint.
        let received = server.received_requests().await.unwrap();
        let comment_posts: Vec<_> = received
            .iter()
            .filter(|r| r.method == wiremock::http::Method::POST)
            .collect();
        assert_eq!(comment_posts.len(), 1, "should have called add_comment exactly once");

        // The request body should contain BOTH question texts.
        let body_bytes = &comment_posts[0].body;
        let body_str = std::str::from_utf8(body_bytes).unwrap();
        assert!(
            body_str.contains("rollback plan") || body_str.contains("What is the rollback plan"),
            "comment body should contain q1 text; got: {body_str}"
        );
        assert!(
            body_str.contains("success") || body_str.contains("How will success"),
            "comment body should contain q2 text; got: {body_str}"
        );
    }

    // -----------------------------------------------------------------------
    // Helpers for render_testcases_markdown tests
    // -----------------------------------------------------------------------

    fn make_testcase(
        title: &str,
        category: &str,
        priority: &str,
        steps_json: &str,
        order_idx: i64,
    ) -> otto_state::ProductTestcase {
        let now = Utc::now();
        let dummy = new_id();
        otto_state::ProductTestcase {
            id: new_id(),
            run_id: dummy.clone(),
            story_id: dummy.clone(),
            title: title.to_string(),
            category: category.to_string(),
            priority: priority.to_string(),
            steps_json: steps_json.to_string(),
            status: "approved".to_string(),
            review_note: None,
            order_idx,
            created_at: now,
            updated_at: now,
        }
    }

    // -----------------------------------------------------------------------
    // Test 7: render_testcases_markdown — two categories, valid steps_json
    // -----------------------------------------------------------------------

    #[test]
    fn render_testcases_markdown_groups_by_category_and_renders_steps() {
        let story_title = "Login Flow";

        let tc_happy = make_testcase(
            "Happy path login",
            "happy",
            "high",
            r#"{"preconditions":["User is registered"],"steps":["Open login page","Enter credentials","Click submit"],"expected":"User is logged in"}"#,
            0,
        );
        let tc_error = make_testcase(
            "Wrong password",
            "error",
            "medium",
            r#"{"preconditions":["User exists"],"steps":["Enter wrong password","Click submit"],"expected":"Error message shown"}"#,
            0,
        );

        let md = ProductService::render_testcases_markdown(story_title, &[tc_happy, tc_error]);

        // Header
        assert!(
            md.contains("# Test Cases — Login Flow"),
            "missing page header; got:\n{md}"
        );

        // Category headings (happy before error in canonical order)
        let pos_happy = md.find("## Happy").expect("no Happy category heading");
        let pos_error = md.find("## Error").expect("no Error category heading");
        assert!(
            pos_happy < pos_error,
            "happy should appear before error; got:\n{md}"
        );

        // Test case titles
        assert!(md.contains("### Happy path login"), "tc title missing; got:\n{md}");
        assert!(md.contains("### Wrong password"), "tc title missing; got:\n{md}");

        // Priority
        assert!(md.contains("**Priority:** high"), "priority missing; got:\n{md}");

        // Preconditions
        assert!(md.contains("- User is registered"), "precondition missing; got:\n{md}");

        // Steps (numbered)
        assert!(md.contains("1. Open login page"), "step 1 missing; got:\n{md}");
        assert!(md.contains("2. Enter credentials"), "step 2 missing; got:\n{md}");
        assert!(md.contains("3. Click submit"), "step 3 missing; got:\n{md}");

        // Expected
        assert!(md.contains("**Expected:** User is logged in"), "expected missing; got:\n{md}");
        assert!(md.contains("**Expected:** Error message shown"), "expected missing; got:\n{md}");
    }

    // -----------------------------------------------------------------------
    // Test 8: render_testcases_markdown — malformed steps_json does not panic
    // -----------------------------------------------------------------------

    #[test]
    fn render_testcases_markdown_tolerates_malformed_steps_json() {
        let tc_bad = make_testcase(
            "Edge case with bad json",
            "edge",
            "low",
            "NOT_VALID_JSON{{{",
            0,
        );

        // Must not panic.
        let md = ProductService::render_testcases_markdown("My Story", &[tc_bad]);

        // Title should still appear.
        assert!(
            md.contains("### Edge case with bad json"),
            "title missing even with bad json; got:\n{md}"
        );
        // Category heading should appear.
        assert!(md.contains("## Edge"), "category heading missing; got:\n{md}");
        // Priority should appear.
        assert!(md.contains("**Priority:** low"), "priority missing; got:\n{md}");
    }

    // -----------------------------------------------------------------------
    // Test 9: build_inject_bundle — assembles all sections, excludes discarded
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn build_inject_bundle_assembles_content_and_excludes_discarded_questions() {
        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws_id = seed_workspace(&pool).await;

        let repo = ProductRepo::new(pool.clone());
        let issues = IssuesRepo::new(pool.clone());
        let secrets: Arc<dyn SecretStore> = Arc::new(FixedSecret("unused".into()));
        let svc = ProductService::new(repo.clone(), issues, secrets);

        // Create a story.
        let story = repo
            .create_story(otto_state::NewStory {
                workspace_id: ws_id.clone(),
                source_kind: "jira".into(),
                account_id: new_id(),
                source_key: "BUNDLE-1".into(),
                title: "Bundle Test Story".into(),
                url: "https://example.com/BUNDLE-1".into(),
                issue_type: Some("Story".into()),
                stage: "imported".into(),
                cwd: None,
                created_by: user_id.clone(),
            })
            .await
            .unwrap();

        // Add a source version.
        repo.add_version(otto_state::NewVersion {
            story_id: story.id.clone(),
            kind: "source".into(),
            title: "Bundle Test Story".into(),
            body_md: "As a user I want the bundle to work correctly.".into(),
            raw_json: None,
            change_notes: None,
            created_by: user_id.clone(),
        })
        .await
        .unwrap();

        // Add an analysis with a non-empty summary.
        let analysis = repo
            .create_analysis(otto_state::NewAnalysis {
                story_id: story.id.clone(),
                source_version_id: None,
                status: "done".into(),
                created_by: user_id.clone(),
            })
            .await
            .unwrap();
        repo.set_analysis_status(&analysis.id, "done", Some("Key insight: needs testing."), true)
            .await
            .unwrap();

        // Add an answered question.
        let q_answered = repo
            .create_question(otto_state::NewQuestion {
                story_id: story.id.clone(),
                analysis_id: None,
                text: "What is the acceptance criterion?".into(),
                rationale: "".into(),
                category: "general".into(),
                created_by: user_id.clone(),
            })
            .await
            .unwrap();
        repo.update_question(
            &q_answered.id,
            otto_state::QuestionPatch {
                text: None,
                rationale: None,
                category: None,
                status: Some("answered".into()),
                answer: Some(Some("The system must pass all integration tests.".into())),
                posted_ref: None,
            },
        )
        .await
        .unwrap();

        // Add a discarded question (should NOT appear in the bundle).
        let q_discarded = repo
            .create_question(otto_state::NewQuestion {
                story_id: story.id.clone(),
                analysis_id: None,
                text: "This question should be discarded and hidden.".into(),
                rationale: "".into(),
                category: "general".into(),
                created_by: user_id.clone(),
            })
            .await
            .unwrap();
        repo.update_question(
            &q_discarded.id,
            otto_state::QuestionPatch {
                text: None,
                rationale: None,
                category: None,
                status: Some("discarded".into()),
                answer: None,
                posted_ref: None,
            },
        )
        .await
        .unwrap();

        // Add a testcase run with one approved case.
        let run = repo.create_testcase_run(&story.id, &user_id).await.unwrap();
        let tc = repo
            .add_testcase(otto_state::NewTestcase {
                run_id: run.id.clone(),
                story_id: story.id.clone(),
                title: "Approved test case title".into(),
                category: "happy".into(),
                priority: "high".into(),
                steps_json: r#"{"preconditions":[],"steps":["Do the thing"],"expected":"It works"}"#.into(),
                order_idx: 0,
            })
            .await
            .unwrap();
        // Approve the testcase.
        repo.update_testcase(
            &tc.id,
            otto_state::TestcasePatch {
                title: None,
                category: None,
                priority: None,
                steps_json: None,
                status: Some("approved".into()),
                review_note: None,
                order_idx: None,
            },
        )
        .await
        .unwrap();

        // Add an active learning (pattern kind).
        repo.create_learning(otto_state::NewLearning {
            workspace_id: ws_id.clone(),
            kind: "pattern".into(),
            title: "Always write tests first".into(),
            body: "TDD leads to better design.".into(),
            tags: "".into(),
            refs_json: "[]".into(),
            source_story_id: None,
            created_by: user_id.clone(),
        })
        .await
        .unwrap();

        // Build the bundle.
        let bundle = svc.build_inject_bundle(&story.id).await.unwrap();
        let md = &bundle.markdown;

        // Story title appears.
        assert!(md.contains("Bundle Test Story"), "story title missing;\n{md}");

        // The answered question's answer appears.
        assert!(
            md.contains("The system must pass all integration tests."),
            "answer text missing;\n{md}"
        );

        // The approved testcase title appears.
        assert!(
            md.contains("Approved test case title"),
            "approved testcase title missing;\n{md}"
        );

        // The active learning appears.
        assert!(
            md.contains("Always write tests first"),
            "learning title missing;\n{md}"
        );

        // The discarded question text does NOT appear.
        assert!(
            !md.contains("This question should be discarded and hidden."),
            "discarded question text should not be in bundle;\n{md}"
        );
    }

    // -----------------------------------------------------------------------
    // Test D1: create_draft — creates draft story + version + event
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn create_draft_makes_draft_story_and_version() {
        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws_id = seed_workspace(&pool).await;

        let repo = ProductRepo::new(pool.clone());
        let issues = IssuesRepo::new(pool.clone());
        let secrets: Arc<dyn SecretStore> = Arc::new(FixedSecret("unused".into()));
        let svc = ProductService::new(repo.clone(), issues, secrets);

        let detail = svc
            .create_draft(&ws_id, &user_id, Some("My Draft RFC"))
            .await
            .unwrap();

        assert_eq!(detail.story.source_kind, "draft");
        assert_eq!(detail.story.stage, "draft");
        assert_eq!(detail.story.title, "My Draft RFC");
        assert_eq!(detail.story.source_key, "");
        assert_eq!(detail.counts.versions, 1);

        let events = repo
            .list_events(&detail.story.id, Some("source"))
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "draft_created");
    }

    // -----------------------------------------------------------------------
    // Test D2: update_draft_body — edits in place, no new version
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn update_draft_body_edits_in_place_and_syncs_title() {
        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws_id = seed_workspace(&pool).await;

        let repo = ProductRepo::new(pool.clone());
        let issues = IssuesRepo::new(pool.clone());
        let secrets: Arc<dyn SecretStore> = Arc::new(FixedSecret("unused".into()));
        let svc = ProductService::new(repo.clone(), issues, secrets);

        let detail = svc
            .create_draft(&ws_id, &user_id, Some("Initial Title"))
            .await
            .unwrap();
        let story_id = detail.story.id.clone();
        assert_eq!(detail.counts.versions, 1);

        let detail2 = svc
            .update_draft_body(&story_id, "Updated Title", "# New body\n\nSome content.", &user_id)
            .await
            .unwrap();

        assert_eq!(detail2.counts.versions, 1);
        assert_eq!(detail2.story.title, "Updated Title");

        let versions = repo.list_versions(&story_id).await.unwrap();
        let full_ver = repo.get_version(&versions[0].id).await.unwrap();
        assert!(
            full_ver.body_md.contains("Some content."),
            "body not updated; got: {:?}",
            full_ver.body_md
        );
    }

    // -----------------------------------------------------------------------
    // Test D3: build_agent_context includes seeded transcript body
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn build_agent_context_includes_transcript_body() {
        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws_id = seed_workspace(&pool).await;

        let repo = ProductRepo::new(pool.clone());
        let issues = IssuesRepo::new(pool.clone());
        let secrets: Arc<dyn SecretStore> = Arc::new(FixedSecret("unused".into()));
        let svc = ProductService::new(repo.clone(), issues, secrets);

        let detail = svc
            .create_draft(&ws_id, &user_id, Some("Context Test"))
            .await
            .unwrap();
        let story_id = detail.story.id.clone();

        repo.create_transcript(otto_state::NewTranscript {
            story_id: story_id.clone(),
            title: "Planning meeting".into(),
            body: "We discussed requirements and agreed on scope XYZ.".into(),
            created_by: user_id.clone(),
        })
        .await
        .unwrap();

        let ctx = svc.build_agent_context(&story_id, None).await.unwrap();

        assert!(
            ctx.contains("Transcripts"),
            "context missing Transcripts section;\n{ctx}"
        );
        assert!(
            ctx.contains("We discussed requirements and agreed on scope XYZ."),
            "transcript body missing;\n{ctx}"
        );
        assert!(
            ctx.contains("Planning meeting"),
            "transcript title missing;\n{ctx}"
        );
    }

    // -----------------------------------------------------------------------
    // Test D4: rebind logic — draft is rebound to jira after publish_as_story
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn publish_as_story_rebinds_draft_story_to_jira() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/rest/api/3/issue"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "10099",
                "key": "NEW-1",
                "self": format!("{}/rest/api/3/issue/10099", server.uri()),
            })))
            .mount(&server)
            .await;

        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws_id = seed_workspace(&pool).await;
        let account = seed_issue_account(&pool, &user_id, &server.uri()).await;

        let repo = ProductRepo::new(pool.clone());
        let issues = IssuesRepo::new(pool.clone());
        let secrets: Arc<dyn SecretStore> = Arc::new(FixedSecret("tok".into()));
        let svc = ProductService::new(repo.clone(), issues, secrets);

        let detail = svc
            .create_draft(&ws_id, &user_id, Some("My Draft Feature"))
            .await
            .unwrap();
        let story_id = detail.story.id.clone();
        assert_eq!(detail.story.source_kind, "draft");

        let result = svc
            .publish_as_story(&story_id, &account.id, "NEW", "Story", &user_id)
            .await
            .unwrap();

        assert_eq!(result.story.id, story_id);
        assert_eq!(result.story.source_kind, "jira");
        assert_eq!(result.story.source_key, "NEW-1");
        assert_eq!(result.story.stage, "refined");
        assert!(result.story.issue_type.as_deref() == Some("Story"));

        let events = repo
            .list_events(&story_id, Some("publish"))
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "published_story");
    }
}
