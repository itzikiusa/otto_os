//! otto-state — SQLite persistence: pool bootstrap, migrations, repositories.
//!
//! Repositories map rows to `otto_core::domain` structs.

pub mod activity;
pub mod api_client;
pub mod audit;
pub mod broker_audit;
pub mod broker_cluster_sections;
pub mod broker_clusters;
pub mod connection_sections;
pub mod connections;
pub mod convert;
pub mod db;
pub mod db_explorer;
pub mod email_senders;
pub mod git;
pub mod grants;
pub mod product;
pub mod improvements;
pub mod integrations;
pub mod issues;
pub mod mcp_servers;
pub mod memory;
pub mod notifications;
pub mod reviews;
pub mod sessions;
pub mod settings;
pub mod skill_evals;
pub mod swarm;
pub mod users;
pub mod workflows;
pub mod workspaces;

pub use activity::{ActivityRepo, NewTask, NewTrail};
pub use audit::{AuditRepo, NewAuditEntry};
pub use api_client::{
    ApiClientRepo, NewApiAutomation, NewApiCollection, NewApiEnvironment, NewApiHistory,
    NewApiRequest,
};
pub use broker_audit::{BrokerAuditRepo, BrokerAuditRow};
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
pub use grants::GrantsRepo;
pub use product::*;
pub use improvements::{ImprovementsRepo, NewEdit};
pub use integrations::IntegrationsRepo;
pub use issues::{IssuesRepo, NewIssueAccount};
pub use mcp_servers::{McpServersRepo, NewMcpServer};
pub use memory::MemoriesRepo;
pub use notifications::{NewNotice, NoticeAccess, NotificationsRepo};
pub use reviews::ReviewsRepo;
pub use sessions::{NewSession, SessionsRepo, UsageAttrRow};
pub use settings::SettingsRepo;
pub use skill_evals::SkillEvalsRepo;
// Note: `swarm::NewTask` collides with `activity::NewTask`; access it via the
// module path (`otto_state::swarm::NewTask`). The rest are re-exported here.
pub use swarm::{
    AgentPatch, NewAgent, NewMessage, NewProject, NewRun, NewSwarm, ProjectPatch, RunFilter,
    RunPatch, Swarm, SwarmAgent, SwarmMessage, SwarmPatch, SwarmProject, SwarmRepo, SwarmRun,
    SwarmTask, TaskPatch,
};
pub use workflows::WorkflowsRepo;
pub use users::{UserRecord, UsersRepo};
pub use workspaces::{Member, WorkspacesRepo};
