//! Provider-owned subscription logins. These records never contain credentials.
use crate::Id;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAccount {
    pub id: Id,
    pub provider: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProviderAccount {
    pub provider: String,
    pub label: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginProviderAccount {
    pub workspace_id: Id,
}
