//! otto-state — SQLite persistence: pool bootstrap, migrations, repositories.
//!
//! Repositories map rows to `otto_core::domain` structs.

pub mod activity;
pub mod api_client;
pub mod audit;
pub mod aws_accounts;
pub mod broker_audit;
pub mod broker_clusters;
pub mod broker_ops;
pub mod browser;
pub mod browser_credentials;
pub mod canvas;
pub mod connection_sections;
pub mod connections;
pub mod convert;
pub mod database_changes;
pub mod db;
pub mod db_explorer;
pub mod email_senders;
pub mod eval_lab;
pub mod finding_events;
pub mod git;
pub mod goal_loops;
pub mod grants;
pub mod improvements;
pub mod integrations;
pub mod issues;
pub mod k8s_clusters;
pub mod k8s_monitor;
pub mod k8s_pod_actions;
pub mod maintenance;
pub mod mcp_audit;
pub mod mcp_auto_approve;
pub mod mcp_control;
pub mod mcp_profiles;
pub mod mcp_servers;
pub mod memory;
pub mod name_themes;
pub mod notifications;
pub mod personal_agents;
pub mod plugins;
pub mod product;
pub mod product_attachments;
pub mod product_chat;
pub mod product_mockup;
pub mod product_refinement;
pub mod projects;
pub mod provider_accounts;
pub mod provider_models;
pub mod repo_rules;
pub mod resource_access;
pub mod retention;
pub mod review_findings;
pub mod review_proof_packs;
pub mod reviews;
pub mod scheduled_tasks;
pub mod search;
pub mod sessions;
pub mod settings;
pub mod skill_evals;
pub mod skill_reviews;
pub mod users;
pub mod vault_docs_runs;
pub mod workflow_triggers;
pub mod workflows;
pub mod workspaces;

pub use activity::{normalize_title, ActivityRepo, NewTask, NewTrail, PendingNudge};
pub use api_client::{
    ApiClientRepo, NewApiAutomation, NewApiCollection, NewApiEnvironment, NewApiHistory,
    NewApiRequest,
};
pub use audit::{AuditRepo, NewAuditEntry};
pub use aws_accounts::{AwsAccountPatch, AwsAccountRow, AwsAccountsRepo, NewAwsAccount};
pub use broker_audit::{BrokerAuditRepo, BrokerAuditRow};
pub use broker_clusters::{
    BrokerClusterRow, BrokerClustersRepo, NewBrokerCluster, UpdateBrokerCluster,
};
pub use broker_ops::{BrokerOpsRepo, LagAlertRow, NewLagAlert, ReplayRow};
pub use browser::{
    BrowserAnnotation, BrowserAnnotationsRepo, BrowserTab, BrowserTabsRepo, NewBrowserAnnotation,
    NewBrowserTab,
};
pub use browser_credentials::{
    match_domain, normalize_domain, BrowserCredential, BrowserCredentialPatch,
    BrowserCredentialsRepo, NewBrowserCredential,
};
pub use connection_sections::ConnectionSectionsRepo;
pub use connections::{ConnectionsRepo, NewConnection};
pub use db::open;
// Re-exported so daemon-side background tasks can name the pool type without
// taking a direct sqlx dependency.
pub use db_explorer::{
    Dashboard, DbExplorerRepo, HistoryEntry, NewSavedQuery, NewWidget, SavedQuery, Widget,
};
pub use email_senders::{EmailSender, EmailSendersRepo};
pub use git::{GitStore, NewGitAccount, NewRepo};
pub use goal_loops::{GoalLoopsRepo, NewGoalLoop};
pub use grants::{capability_for_role, GrantCache, GrantsRepo, GRANT_CACHE_TTL};
pub use name_themes::{CustomTheme, NameThemesRepo};
pub use pool::DbPool;
pub use provider_models::{ProviderModel, ProviderModelsRepo};
// The raw sqlx pool type, for callers that build their own (tests, tools);
// daemon code holds a `DbPool`.
pub use canvas::{
    CanvasRepo, CanvasScene, CanvasSceneSummary, CanvasSceneVersion, NewScene, SceneUpdate,
    SCENE_VERSIONS_KEPT, USER_SNAPSHOT_EVERY_SECS,
};
pub use eval_lab::{EvalMatricesRepo, GoldenTaskInput, GoldenTasksRepo};
pub use finding_events::FindingEventsRepo;
pub use improvements::{ImprovementsRepo, NewEdit};
pub use integrations::IntegrationsRepo;
pub use issues::{IssuesRepo, NewIssueAccount};
pub use k8s_clusters::{
    K8sCluster, K8sClusterPatch, K8sClusterSource, K8sClustersRepo, NewK8sCluster,
};
pub use k8s_monitor::{K8sMonitorConfigRow, K8sMonitorRepo, K8sMonitorStatusRow};
pub use k8s_pod_actions::{K8sPodActionsRepo, PodAction, PodActionFilter, UpsertPodAction};
pub use mcp_audit::{McpAuditRepo, McpToolCallRow, NewMcpToolCall};
pub use mcp_auto_approve::{
    AutoApproveRulePatch, McpAutoApproveRepo, McpAutoApproveRule, NewAutoApproveRule,
};
pub use mcp_control::{
    set_approval_change_hook, ApprovalChange, CallLogQuery, DiscoveredTool, McpAllowlistEntry,
    McpAllowlistRepo, McpApproval, McpApprovalRepo, McpCallLogRepo, McpCallLogRow, McpPolicy,
    McpPolicyRepo, McpRegistryRepo, McpServerDetail, McpTool, McpToolStats, McpToolsRepo,
    NewAllowlistEntry, NewApproval, NewCallLog, NewPolicy, NewServerRow,
};
pub use mcp_profiles::{McpProfile, McpProfileFields, McpProfileRepo, FULL_PROFILE_ID};
pub use mcp_servers::{McpServersRepo, NewMcpServer};
pub use memory::{GovernedImport, MemoriesRepo};
pub use notifications::{NewNotice, NoticeAccess, NotificationsRepo};
pub use personal_agents::{
    AgentAutonomy, AgentRule, ProactiveConfig, RuleEnforcement, StandingGoal,
};
pub use personal_agents::{
    AgentRoom, AgentRoomMessage, AgentRoomsRepo, AgentSchedulePatch, FinishAgentRun, NewAgentRun,
    NewAgentSchedule, NewPersonalAgent, NewRoomMessage, PersonalAgent, PersonalAgentPatch,
    PersonalAgentRun, PersonalAgentSchedule, PersonalAgentsRepo, RoomActivity,
};
pub use plugins::{NewPlugin, PluginRecord, PluginsRepo};
pub use product::*;
pub use product_attachments::{
    AttachmentPatch, NewAttachment, ProductAttachment, ProductAttachmentRepo,
};
pub use product_chat::{
    DiscoveryChat, DiscoveryChatMessage, DiscoveryChatRepo, NewDiscoveryChat,
    NewDiscoveryChatMessage,
};
pub use product_mockup::{AnnotationPatch, MockupAnnotation, NewAnnotation, ProductMockupRepo};
pub use product_refinement::{
    NewRefinementMessage, NewRefinementThread, ProductRefinementRepo, RefinementMessage,
    RefinementThread,
};
pub use repo_rules::RepoRulesRepo;
pub use resource_access::ResourceAccessRepo;
pub use retention::{RetentionPolicy, RetentionRepo, RetentionReport};
pub use review_findings::{
    compute_fingerprint, FindingPatch, FindingState, NewFinding, ReviewFindingRow,
    ReviewFindingsRepo,
};
pub use review_proof_packs::ReviewProofPacksRepo;
pub use reviews::{ReviewRunContext, ReviewsRepo};
pub use scheduled_tasks::{
    FinishRun, NewRun as NewScheduledRun, NewScheduledTask, ScheduledTaskPatch, ScheduledTasksRepo,
};
pub use sessions::{
    DormantPass, NewSession, SessionListFilter, SessionScope, SessionsRepo, UsageAttrRow,
    UsageLabelRow,
};
pub use settings::{
    otto_mcp_enabled_for, pr_draft_model_from, SettingsRepo, DEFAULT_PROVIDER_KEY,
    OTTO_MCP_ENABLED_KEY, PR_DRAFT_MODEL_DEFAULT, PR_DRAFT_MODEL_KEY,
};
pub use skill_evals::{SkillEvalSummary, SkillEvalSummaryPage, SkillEvalsRepo};
pub use skill_reviews::SkillReviewsRepo;
pub use sqlx::SqlitePool;
pub use users::{UserRecord, UsersRepo};
pub use vault_docs_runs::{VaultDocsRunRow, VaultDocsRunsRepo};
pub use workflow_triggers::{NewWorkflowTrigger, TriggersRepo, WorkflowTrigger};
pub use workflows::{RunHead, WorkflowsRepo};
pub use workspaces::{Member, WorkspacesRepo};

pub mod api_runs;

pub mod workflow_progress;

pub mod network_profiles;
pub mod pool;

// --- Workbench (scratch files with full history) ------------------------------
pub mod workbench;
pub use workbench::{
    NewWorkbenchDoc, WorkbenchAsset, WorkbenchDiff, WorkbenchDoc, WorkbenchDocFull, WorkbenchPatch,
    WorkbenchRepo, WorkbenchRevision, WorkbenchRevisionDetail,
};

/// Escape a SQLite `LIKE` pattern for `ESCAPE '\\'` (backslash first, then `%` and `_`).
pub fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
