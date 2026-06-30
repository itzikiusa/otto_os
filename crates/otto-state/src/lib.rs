//! otto-state — SQLite persistence: pool bootstrap, migrations, repositories.
//!
//! Repositories map rows to `otto_core::domain` structs.

pub mod activity;
pub mod api_client;
pub mod audit;
pub mod canvas;
pub mod broker_audit;
pub mod broker_cluster_sections;
pub mod broker_ops;
pub mod broker_clusters;
pub mod connection_sections;
pub mod connections;
pub mod convert;
pub mod db;
pub mod db_explorer;
pub mod email_senders;
pub mod git;
pub mod eval_lab;
pub mod grants;
pub mod product;
pub mod improvements;
pub mod integrations;
pub mod issues;
pub mod mcp_audit;
pub mod mcp_control;
pub mod goal_loops;
pub mod mcp_servers;
pub mod code_index;
pub mod memory;
pub mod vault_backends;
pub mod name_themes;
pub mod notifications;
pub mod plugins;
pub mod product_attachments;
pub mod product_chat;
pub mod product_discovery;
pub mod product_mockup;
pub mod product_refinement;
pub mod finding_events;
pub mod proof;
pub mod repo_rules;
pub mod review_findings;
pub mod review_proof_packs;
pub mod reviews;
pub mod runs;
pub mod saved_views;
pub mod scheduled_tasks;
pub mod sessions;
pub mod settings;
pub mod skill_evals;
pub mod swarm;
pub mod users;
pub mod workflow_triggers;
pub mod workflows;
pub mod workgraph;
pub mod workspaces;

pub use activity::{ActivityRepo, NewTask, NewTrail};
pub use audit::{AuditRepo, NewAuditEntry};
pub use api_client::{
    ApiClientRepo, NewApiAutomation, NewApiCollection, NewApiEnvironment, NewApiHistory,
    NewApiRequest,
};
pub use broker_audit::{BrokerAuditRepo, BrokerAuditRow};
pub use broker_ops::{BrokerOpsRepo, LagAlertRow, NewLagAlert, ReplayRow};
pub use broker_cluster_sections::{BrokerClusterSectionRow, BrokerClusterSectionsRepo};
pub use broker_clusters::{
    BrokerClusterRow, BrokerClustersRepo, NewBrokerCluster, UpdateBrokerCluster,
};
pub use connection_sections::ConnectionSectionsRepo;
pub use connections::{ConnectionsRepo, NewConnection};
pub use db::open;
// Re-exported so daemon-side background tasks can name the pool type without
// taking a direct sqlx dependency.
pub use sqlx::SqlitePool;
pub use db_explorer::{
    Dashboard, DbExplorerRepo, HistoryEntry, NewSavedQuery, NewWidget, SavedQuery, Widget,
};
pub use email_senders::{EmailSender, EmailSendersRepo};
pub use git::{GitStore, NewGitAccount, NewRepo};
pub use goal_loops::{GoalLoopsRepo, NewGoalLoop};
pub use grants::GrantsRepo;
pub use name_themes::{CustomTheme, NameThemesRepo};
pub use proof::{ProofBlob, ProofRepo, ProofSnapshotRow};
// NewRun/RunPatch/NewRunEvent are referenced via the `runs::` path downstream to
// avoid colliding with swarm's `RunPatch`/`RunFilter` re-exports.
pub use runs::RunsRepo;
pub use scheduled_tasks::{
    FinishRun, NewRun as NewScheduledRun, NewScheduledTask, ScheduledTaskPatch, ScheduledTasksRepo,
};
pub use product::*;
pub use improvements::{ImprovementsRepo, NewEdit};
pub use integrations::IntegrationsRepo;
pub use issues::{IssuesRepo, NewIssueAccount};
pub use mcp_audit::{McpAuditRepo, McpToolCallRow, NewMcpToolCall};
pub use mcp_control::{
    CallLogQuery, DiscoveredTool, McpAllowlistEntry, McpAllowlistRepo, McpApproval, McpApprovalRepo,
    McpCallLogRepo, McpCallLogRow, McpPolicy, McpPolicyRepo, McpRegistryRepo, McpServerDetail,
    McpTool, McpToolStats, McpToolsRepo, NewAllowlistEntry, NewApproval, NewCallLog, NewPolicy,
    NewServerRow,
};
pub use mcp_servers::{McpServersRepo, NewMcpServer};
pub use code_index::{
    CodeEdge, CodeGraph, CodeIndexRepo, CodeNode, CodeRepo, CodeSymbol, NewEdge, NewNode,
};
pub use memory::{GovernedImport, MemoriesRepo};
pub use vault_backends::{VaultBackend, VaultBackendsRepo};
pub use notifications::{NewNotice, NoticeAccess, NotificationsRepo};
pub use plugins::{NewPlugin, PluginRecord, PluginsRepo};
pub use canvas::{CanvasRepo, CanvasScene, CanvasSceneSummary, NewScene, SceneUpdate};
pub use product_attachments::{AttachmentPatch, NewAttachment, ProductAttachment, ProductAttachmentRepo};
pub use product_chat::{
    DiscoveryChat, DiscoveryChatMessage, DiscoveryChatRepo, NewDiscoveryChat,
    NewDiscoveryChatMessage,
};
pub use product_discovery::{DiscoveryRun, NewDiscoveryRun, ProductDiscoveryRepo};
pub use product_mockup::{AnnotationPatch, MockupAnnotation, NewAnnotation, ProductMockupRepo};
pub use product_refinement::{
    NewRefinementMessage, NewRefinementThread, ProductRefinementRepo, RefinementMessage,
    RefinementThread,
};
pub use finding_events::FindingEventsRepo;
pub use repo_rules::RepoRulesRepo;
pub use review_findings::{
    compute_fingerprint, FindingPatch, FindingState, NewFinding, ReviewFindingRow,
    ReviewFindingsRepo,
};
pub use review_proof_packs::ReviewProofPacksRepo;
pub use reviews::ReviewsRepo;
pub use saved_views::{NewSavedView, SavedView, SavedViewsRepo};
pub use sessions::{NewSession, SessionsRepo, UsageAttrRow};
pub use settings::{otto_mcp_enabled_for, SettingsRepo, OTTO_MCP_ENABLED_KEY};
pub use eval_lab::{EvalMatricesRepo, GoldenTaskInput, GoldenTasksRepo};
pub use skill_evals::SkillEvalsRepo;
// Note: `swarm::NewTask` collides with `activity::NewTask`; access it via the
// module path (`otto_state::swarm::NewTask`). The rest are re-exported here.
pub use swarm::{
    AgentPatch, GoalPatch, NewAgent, NewGoal, NewMessage, NewProject, NewRun, NewSwarm, NewTrigger,
    ProjectPatch, RunFilter, RunPatch, Swarm, SwarmAgent, SwarmChannelTrigger, SwarmGoal,
    SwarmMessage, SwarmPatch, SwarmProject, SwarmRepo, SwarmRun, SwarmTask, TaskPatch, TriggerPatch,
};
pub use workflow_triggers::{NewWorkflowTrigger, TriggersRepo, WorkflowTrigger};
pub use workflows::WorkflowsRepo;
pub use workgraph::{
    ApprovalStatus, ArtifactKind, CountBucket, EdgeRelation, EdgeView, GraphEdge, GraphNode,
    GraphView, MissionFilter, MissionSummary, NewArtifact, NewWorkEvent, RiskLevel, UpsertResult,
    WorkActor, WorkApproval, WorkArtifact, WorkEdge, WorkEvent, WorkGraphRepo, WorkItem,
    WorkItemDetail, WorkItemUpsert, WorkKind, WorkStatus,
};
pub use users::{UserRecord, UsersRepo};
pub use workspaces::{Member, WorkspacesRepo};
