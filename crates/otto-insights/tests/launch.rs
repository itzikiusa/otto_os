//! Exercise report admission through the real session cwd guard and PTY launch.
use std::sync::Arc;

use otto_core::domain::Workspace;
use otto_core::event::Event;
use otto_core::Id;
use otto_insights::{InsightsCtx, Kind, RunMode};
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::WorkspacesRepo;
use tokio::sync::broadcast;

#[derive(Clone)]
struct Host {
    library: otto_context::Library,
    manager: Arc<SessionManager>,
    events: broadcast::Sender<Event>,
    workspaces: WorkspacesRepo,
    prompts: broadcast::Sender<String>,
}

impl InsightsCtx for Host {
    fn library(&self) -> &otto_context::Library {
        &self.library
    }
    fn manager(&self) -> &Arc<SessionManager> {
        &self.manager
    }
    fn events(&self) -> &broadcast::Sender<Event> {
        &self.events
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
    async fn resolve_provider(
        &self,
        _: Option<&Workspace>,
        _: Option<&str>,
    ) -> otto_core::Result<String> {
        Ok("insights-test".into())
    }
    async fn submit_prompt(&self, _: &Id, prompt: &str) -> bool {
        self.prompts.send(prompt.into()).unwrap();
        true
    }
}

#[tokio::test]
async fn manual_and_scheduled_runs_launch_in_the_allowed_report_directory() {
    for mode in [RunMode::Manual, RunMode::Scheduled] {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let pool = otto_state::open(&data.join("test.db")).await.unwrap();
        let users = otto_state::UsersRepo::new(pool.clone());
        let user = users
            .create("owner", "unused", "Owner", true)
            .await
            .unwrap();
        let workspaces = WorkspacesRepo::new(pool.clone());
        workspaces
            .create("Reports", temp.path().to_str().unwrap(), &user.id)
            .await
            .unwrap();
        let (events, _) = broadcast::channel(64);
        let (prompts, mut submitted) = broadcast::channel(4);
        // A harmless real PTY, with no paid agent or provider-home writes.
        let providers = ProviderRegistry::new(Some(&serde_json::json!({
            "insights-test": {"cmd": "/bin/cat", "args": []}
        })));
        let host = Host {
            library: otto_context::Library::new(data.join("library")),
            manager: Arc::new(
                SessionManager::new(
                    otto_state::SessionsRepo::new(pool),
                    events.clone(),
                    providers,
                )
                .with_data_dir(&data),
            ),
            events,
            workspaces,
            prompts,
        };
        host.library
            .put_skill("insights", "# Test report skill")
            .unwrap();
        let reports = otto_insights::insights_dir(&host);
        assert!(
            !reports.exists(),
            "first run must create its work directory"
        );
        let result = otto_insights::run_insights(
            &host,
            Kind::Day,
            1,
            chrono::NaiveDate::from_ymd_opt(2026, 10, 11).unwrap(),
            mode,
        )
        .await;
        assert!(
            result.is_ok(),
            "report launch must pass the configured data-directory guard"
        );
        let sid = result.unwrap().expect("installed skill starts a run");
        let session = host.manager.get(&sid).await.unwrap();
        let prompt = tokio::time::timeout(std::time::Duration::from_secs(5), submitted.recv())
            .await
            .unwrap()
            .unwrap();
        host.manager.kill_session(&sid).await.unwrap();
        assert_eq!(std::path::Path::new(&session.cwd), reports);
        assert!(reports.is_dir());
        assert!(prompt.contains("--as-of 2026-10-11"));
        assert!(prompt.contains(reports.to_str().unwrap()));
        assert_eq!(session.meta["source"], "insights");
        assert!(reports
            .join("daily/completion-daily-20261010_20261010.json")
            .exists());
    }
}
