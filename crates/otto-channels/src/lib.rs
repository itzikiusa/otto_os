//! otto-channels — per-workspace Slack/Telegram integration config, storage and HTTP API.
//! Stage 2 adds a runtime channel manager (Telegram-first, Slack-ready).

pub mod adapter;
pub mod admission;
pub mod attach_guard;
pub mod bridge;
pub mod email;
pub mod gfm;
pub mod health;
pub mod http;
pub mod improve_notify;
pub mod manager;
pub mod mirror;
pub mod secrets_redact;
pub mod seed;
pub mod slack;
pub mod slack_api;
pub mod slack_as_me;
pub mod slack_directory;
pub mod slack_tools;
pub mod slack_user_tokens;
pub mod telegram;
pub mod transcript;
pub mod webhook;
pub mod workflow_trigger;

pub use adapter::{Adapter, Inbound};
pub use bridge::Bridge;
pub use email::GmailSender;
pub use http::{router, ChannelsCtx};
pub use manager::ChannelManager;
pub use mirror::{InteractionImprover, Mirror};
pub use webhook::WebhookAdapter;
