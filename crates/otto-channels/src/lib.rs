//! otto-channels — per-workspace Slack/Telegram integration config, storage and HTTP API.
//! Stage 2 adds a runtime channel manager (Telegram-first, Slack-ready).

pub mod adapter;
pub mod bridge;
pub mod email;
pub mod http;
pub mod improve_notify;
pub mod manager;
pub mod mirror;
pub mod seed;
pub mod slack;
pub mod telegram;
pub mod transcript;

pub use email::GmailSender;
pub use http::{router, ChannelsCtx};
pub use manager::ChannelManager;
