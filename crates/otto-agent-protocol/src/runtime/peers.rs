//! Linked-thread peer operations (`open_peer`, `message_peer`, `close_peer`,
//! `peer_reply`, `latest_reply`) and custom provider slug resolution for
//! [`AgentRuntime`].

use std::time::{Duration, Instant};

use otto_core::agent_runtime::{
    CreateRuntimeSessionReq, OpenPeerReq, PeerInfo, PeerReply, ProviderTurn, RuntimePromptReq,
    RuntimePromptResp, RuntimeProvider, RuntimeSession, RuntimeSessionStatus, TurnItemKind,
};
use otto_core::{Error, Id, Result};

use super::{children, inherit_embedded_origin, is_busy, AgentRuntime, MAX_LINKED_OPEN};

impl AgentRuntime {
    /// Resolve a custom provider slug (e.g. `"claudework"`) via the host's
    /// `ProviderRegistry` when enabled.
    pub fn resolve_custom_provider(&self, slug: &str) -> Option<RuntimeProvider> {
        self.inner.host.resolve_custom_provider(slug)
    }

    /// Resolve a built-in or custom provider slug while enforcing
    /// `RuntimeHost::is_provider_enabled` (so disabled built-ins like `"grok"`
    /// return `None`).
    pub fn resolve_runtime_provider(&self, slug: &str) -> Option<RuntimeProvider> {
        let trimmed = slug.trim();
        if trimmed.is_empty() || !self.inner.host.is_provider_enabled(trimmed) {
            return None;
        }
        self.resolve_custom_provider(trimmed)
            .or_else(|| RuntimeProvider::parse(trimmed))
    }

    /// Resolve an optional `provider_slug` override against built-in and custom
    /// providers, returning `(effective_provider, custom_slug_for_meta)`.
    pub(super) fn resolve_target_provider(
        &self,
        fallback: RuntimeProvider,
        provider_slug: Option<&str>,
    ) -> Result<(RuntimeProvider, Option<String>)> {
        let Some(slug) = provider_slug.map(str::trim).filter(|s| !s.is_empty()) else {
            if !self.inner.host.is_provider_enabled(fallback.as_str()) {
                return Err(Error::Invalid(format!(
                    "unknown or disabled provider '{}'",
                    fallback.as_str()
                )));
            }
            return Ok((fallback, None));
        };
        if !self.inner.host.is_provider_enabled(slug) {
            return Err(Error::Invalid(format!(
                "unknown or disabled provider '{slug}'"
            )));
        }
        if let Some(custom) = self.resolve_custom_provider(slug) {
            let meta_slug = RuntimeProvider::parse(slug)
                .is_none()
                .then(|| slug.to_string());
            return Ok((custom, meta_slug));
        }
        if let Some(builtin) = RuntimeProvider::parse(slug) {
            return Ok((builtin, None));
        }
        Err(Error::Invalid(format!(
            "unknown or disabled provider '{slug}'"
        )))
    }

    /// The linked group `caller` belongs to: its root and every thread opened
    /// under it, same owner, not closed — `caller` first-class among them.
    pub(super) async fn group(&self, caller: &RuntimeSession) -> Result<Vec<RuntimeSession>> {
        let root = caller
            .parent_id
            .clone()
            .unwrap_or_else(|| caller.id.clone());
        Ok(self
            .list(&caller.workspace_id, Some(&caller.created_by))
            .await?
            .into_iter()
            .filter(|s| s.status != RuntimeSessionStatus::Closed)
            .filter(|s| s.id == root || s.parent_id.as_ref() == Some(&root))
            .collect())
    }

    /// 409 when `c`'s linked group has no room for another thread. One
    /// confused agent must not fan out without bound: `group` lists the OPEN
    /// threads of this linked group, the root included.
    pub(super) async fn ensure_group_room(&self, c: &RuntimeSession) -> Result<()> {
        if self.group(c).await?.len() > MAX_LINKED_OPEN {
            return Err(Error::Conflict(format!(
                "this thread group already has {MAX_LINKED_OPEN} open linked threads — close \
                 one (otto_thread_close) before opening another"
            )));
        }
        Ok(())
    }

    pub(super) fn peer_info(s: &RuntimeSession, caller: &Id) -> PeerInfo {
        PeerInfo {
            id: s.id.clone(),
            provider: s.provider,
            title: s.title.clone(),
            status: s.status,
            is_self: &s.id == caller,
        }
    }

    /// `peer` when it is in `caller`'s group (and not `caller` itself).
    pub(super) async fn group_member(
        &self,
        caller: &RuntimeSession,
        peer: &Id,
    ) -> Result<RuntimeSession> {
        if peer == &caller.id {
            return Err(Error::Invalid("that is this thread itself".into()));
        }
        self.group(caller)
            .await?
            .into_iter()
            .find(|s| &s.id == peer)
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "no linked thread '{peer}' (list them with otto_thread_list)"
                ))
            })
    }

    /// The header a peer's message arrives under, so the receiving agent
    /// knows who wrote it and how to answer.
    pub(super) fn peer_header(from: &RuntimeSession, verb: &str) -> String {
        format!(
            "[{verb} thread \"{}\" ({}, id {}). Answer it with the otto_thread_message tool \
             (session_id {}); see every linked thread with otto_thread_list.]",
            from.title,
            from.provider_slug(),
            from.id,
            from.id
        )
    }

    /// Open a linked thread from `caller`: same owner, workspace, cwd and
    /// access mode (never wider than the opener's), grouped under the
    /// caller's root; the opening prompt (if any) is sent framed.
    pub async fn open_peer(&self, caller: &Id, mut req: OpenPeerReq) -> Result<RuntimeSession> {
        let only_slug_meta = req.meta.as_ref().is_none_or(|m| {
            m.as_object().is_some_and(|o| {
                o.keys()
                    .all(|k| k == otto_core::agent_runtime::PROVIDER_SLUG_META_KEY)
            })
        });
        if only_slug_meta {
            let child_obj =
                children::child_meta(caller, otto_core::agent_runtime::ChildOpener::Agent);
            let target = req.meta.get_or_insert_with(|| serde_json::json!({}));
            if let (Some(dst), Some(src)) = (target.as_object_mut(), child_obj.as_object()) {
                for (k, v) in src {
                    dst.insert(k.clone(), v.clone());
                }
            }
        }
        self.open_linked(caller, req, true).await
    }

    /// [`Self::open_peer`]. `framed = false` (daemon-only) sends the opening
    /// prompt as it is, without the "Opened by …" header: a prompt the daemon
    /// built itself (a council synthesis) that no thread's agent wrote.
    pub(super) async fn open_linked(
        &self,
        caller: &Id,
        req: OpenPeerReq,
        framed: bool,
    ) -> Result<RuntimeSession> {
        let c = self.get(caller).await?;
        if c.status == RuntimeSessionStatus::Closed {
            return Err(Error::Conflict("the session is closed".into()));
        }
        let requested_slug = req.provider_slug().to_string();
        let (provider, custom_slug) =
            self.resolve_target_provider(req.provider, Some(&requested_slug))?;
        self.ensure_group_room(&c).await?;
        let root = c.parent_id.clone().unwrap_or_else(|| c.id.clone());
        let mut meta = req.meta;
        if let Some(slug) = custom_slug {
            let m = meta.get_or_insert_with(|| serde_json::json!({}));
            if let Some(o) = m.as_object_mut() {
                o.insert(
                    otto_core::agent_runtime::PROVIDER_SLUG_META_KEY.into(),
                    serde_json::Value::String(slug),
                );
            }
        } else if let Some(o) = meta.as_mut().and_then(|m| m.as_object_mut()) {
            o.remove(otto_core::agent_runtime::PROVIDER_SLUG_META_KEY);
        }
        if c.meta_flag("read_only") {
            let m = meta.get_or_insert_with(|| serde_json::json!({}));
            if let Some(o) = m.as_object_mut() {
                o.insert("read_only".into(), serde_json::Value::Bool(true));
            }
        }
        inherit_embedded_origin(&c, &mut meta);
        let created = self
            .create_linked(
                &c.created_by,
                &c.workspace_id,
                CreateRuntimeSessionReq {
                    provider: Some(provider),
                    cwd: Some(req.cwd.unwrap_or_else(|| c.cwd.clone())),
                    title: req.title,
                    model: req.model,
                    access_mode: Some(c.access_mode),
                    meta,
                },
                Some(root),
            )
            .await?;
        if let Some(p) = req.prompt.filter(|p| !p.trim().is_empty()) {
            let text = if framed {
                format!("{}\n\n{p}", Self::peer_header(&c, "Opened by"))
            } else {
                p
            };
            let mut prompt = RuntimePromptReq::of(&text);
            prompt.effort = req.effort.filter(|e| !e.trim().is_empty());
            self.prompt_with(&created.id, &prompt).await?;
        }
        self.get(&created.id).await
    }

    /// Every thread in `caller`'s linked group.
    pub async fn peers(&self, caller: &Id) -> Result<Vec<PeerInfo>> {
        let c = self.get(caller).await?;
        Ok(self
            .group(&c)
            .await?
            .iter()
            .map(|s| Self::peer_info(s, caller))
            .collect())
    }

    /// Send `text` to linked thread `peer` as its next turn. A peer still
    /// working its turn refuses (409) — read it with [`Self::peer_reply`]
    /// and send again.
    pub async fn message_peer(
        &self,
        caller: &Id,
        peer: &Id,
        text: &str,
    ) -> Result<RuntimePromptResp> {
        if text.trim().is_empty() {
            return Err(Error::Invalid("message text is empty".into()));
        }
        let c = self.get(caller).await?;
        let p = self.group_member(&c, peer).await?;
        let framed = format!("{}\n\n{text}", Self::peer_header(&c, "Message from"));
        self.prompt(&p.id, &framed).await.map_err(|e| match e {
            Error::Conflict(m) if m.contains("turn is already running") => Error::Conflict(format!(
                "thread \"{}\" is busy with a turn — wait for it with otto_thread_read, then send again",
                p.title
            )),
            other => other,
        })
    }

    /// Close linked thread `peer` (stop its agent; it stays in History).
    pub async fn close_peer(&self, caller: &Id, peer: &Id) -> Result<RuntimeSession> {
        let c = self.get(caller).await?;
        let p = self.group_member(&c, peer).await?;
        if p.parent_id.is_none() {
            return Err(Error::Forbidden(
                "that is the group's lead thread — only a person closes it".into(),
            ));
        }
        self.close(&p.id).await
    }

    /// Linked thread `peer`'s latest reply: waits up to `wait` for its
    /// running turn to settle, then returns that turn's assistant text.
    pub async fn peer_reply(&self, caller: &Id, peer: &Id, wait: Duration) -> Result<PeerReply> {
        let c = self.get(caller).await?;
        let p = self.group_member(&c, peer).await?;
        let mut reply = self.latest_reply(&p.id, wait).await?;
        reply.peer.is_self = false;
        Ok(reply)
    }

    /// Session `id`'s latest reply: waits up to `wait` for it to settle.
    pub async fn latest_reply(&self, id: &Id, wait: Duration) -> Result<PeerReply> {
        let deadline = Instant::now() + wait;
        let live = self.live(id).await?;
        loop {
            if Instant::now() >= deadline {
                break;
            }
            if !is_busy(&live.lock()) {
                tokio::time::sleep(Duration::from_millis(300)).await;
                if !is_busy(&live.lock()) {
                    break;
                }
                continue;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let st = live.lock();
        let busy = is_busy(&st);
        let snap = st.proj.snapshot();
        let session = st.session.clone();
        drop(st);
        let mut turns: Vec<&ProviderTurn> = snap.turns.iter().collect();
        turns.sort_by_key(|t| std::cmp::Reverse(t.ordinal));
        let answer = |t: &ProviderTurn| {
            snap.turn_items
                .iter()
                .filter(|i| i.turn_id.as_ref() == Some(&t.id) && i.parent_item_id.is_none())
                .filter_map(|i| match &i.kind {
                    TurnItemKind::AssistantMessage { text, .. } if !text.trim().is_empty() => {
                        Some(text.as_str())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let last = turns.first().copied();
        let text = turns
            .iter()
            .map(|t| answer(t))
            .find(|t| !t.is_empty())
            .unwrap_or_default();
        Ok(PeerReply {
            peer: Self::peer_info(&session, id),
            busy,
            background_tasks: session.background_tasks.len().min(u32::MAX as usize) as u32,
            turn_status: last
                .filter(|t| t.status.is_terminal())
                .and_then(|t| serde_json::to_value(t.status).ok())
                .and_then(|v| v.as_str().map(str::to_string)),
            text,
        })
    }
}
