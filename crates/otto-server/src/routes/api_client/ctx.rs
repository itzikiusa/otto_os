//! `ServerCtx` as the REST engine's [`ApiClientCtx`], and the engine's error
//! as an HTTP error (every variant maps onto the same `otto_core::Error`, so
//! statuses and bodies are what the handlers returned before the move).

use std::sync::Arc;

use otto_core::domain::{Connection, User, WorkspaceRole};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::Id;
use otto_state::ApiClientRepo;
use serde_json::Value;

use super::{ApiClientCtx, EngineError};
use crate::auth::require_ws_role;
use crate::error::ApiError;
use crate::state::ServerCtx;

#[async_trait::async_trait]
impl ApiClientCtx for ServerCtx {
    fn repo(&self) -> ApiClientRepo {
        super::repo(self)
    }

    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }

    async fn workspace_settings(&self, wid: &Id) -> Option<Value> {
        self.workspaces.get(wid).await.ok().map(|ws| ws.settings)
    }

    async fn require_editor(&self, user: &User, wid: &Id) -> otto_core::Result<()> {
        require_ws_role(self, user, wid, WorkspaceRole::Editor)
            .await
            .map_err(|e| e.0)
    }

    async fn tunnel_connection(&self, conn_id: &Id, actor: &Id) -> Result<Connection, String> {
        self.connections
            .authorize(conn_id, actor, "shell")
            .await
            .map_err(|e| e.to_string())?;
        self.connections
            .get(conn_id)
            .await
            .map_err(|_| "SSH tunnel connection not found".to_string())
    }

    fn broadcast(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn park_body(&self, wid: &Id, user: &Id, content_type: Option<String>, raw: Vec<u8>) -> Id {
        crate::routes::api_response_cache::put(wid, user, content_type, raw)
    }
}

impl From<EngineError> for ApiError {
    fn from(e: EngineError) -> Self {
        ApiError(e.into())
    }
}
