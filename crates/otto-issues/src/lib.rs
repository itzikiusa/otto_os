//! otto-issues — Jira issue-tracking integration: HTTP router + Jira API client.

pub mod adf;
pub mod confluence;
pub mod http;
pub mod jira;

pub use adf::{adf_to_markdown, text_to_adf};
pub use confluence::{
    build_page_cql, markdown_to_storage, storage_to_markdown, ConfluenceClient, ConfluencePage,
    ConfluencePageSummary, ConfluenceSpace, PageComment,
};
pub use http::{router, IssuesCtx};
pub use jira::{
    build_create_issue_body, parse_issue_full, CommentRef, CreatedIssue, DevBranch, DevCommit,
    DevPr, DevStatus, EditableField, FieldOption, IssueFull, IssueComment, JiraAttachment,
    JiraChangeItem, JiraChangelogEntry, JiraClient, JiraField, JiraLink, JiraTransition, JiraUser,
};
