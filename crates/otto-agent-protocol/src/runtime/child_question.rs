//! A subagent's QUESTION goes to the thread that delegated it first (T3
//! Code's `DelegatedTaskQuestionReactor.ts`, MIT, Copyright (c) 2026 T3 Tools
//! Inc.): that thread's agent is woken with the question and answers it with
//! `otto.thread_answer` when the task or the person's instructions settle
//! it; otherwise it asks the person, and the question shows in the parent's
//! dock as "Question from <subagent>". An APPROVAL always goes to the person.

use otto_core::agent_runtime::{ChildOpener, QuestionAnswer, RequestKind};

use super::*;

/// Called when `st`'s thread starts waiting on someone: a subagent the
/// agent opened, waiting on a question, wakes the parent's agent with it.
pub(super) fn on_waiting(inner: &Inner, st: &State) {
    let Some(of) = st.session.child_of().filter(|o| o.by == ChildOpener::Agent) else {
        return;
    };
    let Some(r) = st
        .proj
        .requests
        .values()
        .filter(|r| r.status == RuntimeRequestStatus::Pending && r.kind == RequestKind::Question)
        .max_by_key(|r| r.created_at)
    else {
        return;
    };
    let Some(me) = inner.me.upgrade() else {
        return;
    };
    let text = question_text(&st.session, &r.id, r);
    let child = st.session.id.clone();
    let key: Id = format!("question:{}", r.id);
    let rt = AgentRuntime { inner: me };
    tokio::spawn(async move {
        if let Err(e) = rt.queue_wake(&of.id, &child, &key, text, true).await {
            tracing::warn!(session = %of.id, %child, "could not pass a subagent's question on: {e}");
        }
    });
}

/// What the parent's agent reads: who asks, how to answer, then each
/// question with its options.
fn question_text(child: &RuntimeSession, request_id: &str, r: &RuntimeRequest) -> String {
    let mut body = String::new();
    if let Some(t) = r.question_title.as_deref().filter(|t| !t.trim().is_empty()) {
        body.push_str(t.trim());
        body.push_str("\n\n");
    }
    for q in &r.questions {
        body.push_str(&format!(
            "- {} (question_id {}, {:?})",
            q.prompt.trim(),
            q.id,
            q.kind
        ));
        let opts: Vec<&str> = q.options.iter().map(|o| o.value.as_str()).collect();
        if !opts.is_empty() {
            body.push_str(&format!(": {}", opts.join(" | ")));
        }
        body.push('\n');
    }
    format!(
        "[Question from subagent \"{}\" ({}, id {}): it is blocked until it is answered. \
         Answer it with otto_thread_answer (peer_session_id {}, request_id {request_id}) when \
         its task or the user's instructions settle it; otherwise ask the user — it shows in \
         their dock.]\n\n{}",
        child.title,
        child.provider_slug(),
        child.id,
        child.id,
        body.trim_end()
    )
}

impl AgentRuntime {
    /// `POST /runtime-sessions/{id}/peers/{peer}/answer`: thread `caller`
    /// answers a question its own subagent `peer` asks.
    pub async fn answer_child(
        &self,
        caller: &Id,
        peer: &Id,
        request_id: &str,
        answers: Vec<QuestionAnswer>,
    ) -> Result<()> {
        let c = self.get(caller).await?;
        let p = self.group_member(&c, peer).await?;
        if p.child_of().is_none_or(|of| of.id != c.id) {
            return Err(Error::Forbidden(
                "only the thread that delegated to it answers its questions".into(),
            ));
        }
        self.respond_question(&p.id, request_id, answers).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::agent_runtime::{
        QuestionKind, QuestionOption, ResponseCapability, RuntimeQuestion,
    };

    #[test]
    fn the_parent_reads_the_question_and_how_to_answer_it() {
        let child = RuntimeSession {
            title_auto: false,
            id: "kid".into(),
            workspace_id: "w".into(),
            provider: otto_core::agent_runtime::RuntimeProvider::Codex,
            title: "kid".into(),
            cwd: "/tmp".into(),
            model: None,
            effort: None,
            native_thread_id: None,
            status: RuntimeSessionStatus::Idle,
            access_mode: otto_core::agent_runtime::RuntimeAccessMode::ApprovalRequired,
            created_by: "u".into(),
            created_at: Utc::now(),
            last_active_at: Utc::now(),
            last_sequence: 0,
            parent_id: Some("lead".into()),
            ui_control: None,
            background_tasks: Vec::new(),
            capabilities: Default::default(),
            meta: None,
            last_turn_status: None,
            last_error_class: None,
        };
        let r = RuntimeRequest {
            id: "rq1".into(),
            turn_id: None,
            kind: RequestKind::Question,
            status: RuntimeRequestStatus::Pending,
            response_capability: ResponseCapability::Live,
            created_at: Utc::now(),
            resolved_at: None,
            decision: None,
            cancel_reason: None,
            questions: vec![RuntimeQuestion {
                id: "db".into(),
                header: "DB".into(),
                prompt: "Which database?".into(),
                kind: QuestionKind::Single,
                required: true,
                allow_empty: false,
                allow_free_text: false,
                options: vec![
                    QuestionOption {
                        value: "pg".into(),
                        label: "Postgres".into(),
                        description: None,
                    },
                    QuestionOption {
                        value: "my".into(),
                        label: "MySQL".into(),
                        description: None,
                    },
                ],
            }],
            question_title: None,
            answers: None,
        };
        let t = question_text(&child, "rq1", &r);
        assert!(t.starts_with("[Question from subagent \"kid\""));
        assert!(t.contains("otto_thread_answer (peer_session_id kid, request_id rq1)"));
        assert!(t.contains("- Which database? (question_id db, Single): pg | my"));
    }
}
