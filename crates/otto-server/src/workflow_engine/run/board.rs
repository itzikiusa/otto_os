//! `RunBoard`: the node states of a run whose nodes execute side by side
//! (engine.md §3.5). Each node future works on its own copy of its state and
//! publishes it here; the board keeps the latest copy of every node and
//! serializes the writes, so the row's revisions follow snapshot order.

use super::super::*;
use super::schedule::RunCx;

pub(in crate::workflow_engine) struct RunBoard {
    /// Indexed like `graph.nodes`; never held across an await.
    states: std::sync::Mutex<Vec<NodeRunState>>,
    /// Held across persist + emit: two nodes publishing at once must not
    /// leave an older `nodes_json` in the row under a newer `rev`.
    write: tokio::sync::Mutex<()>,
    /// The run row as the run started; every publish computes its overview
    /// from it and the current node states (engine.md §2.3).
    base: Option<WorkflowRun>,
}

impl RunBoard {
    pub(in crate::workflow_engine) fn new(
        states: Vec<NodeRunState>,
        base: Option<WorkflowRun>,
    ) -> Self {
        Self {
            states: std::sync::Mutex::new(states),
            write: tokio::sync::Mutex::new(()),
            base,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<NodeRunState>> {
        self.states.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(in crate::workflow_engine) fn get(&self, idx: usize) -> NodeRunState {
        self.lock()[idx].clone()
    }

    pub(in crate::workflow_engine) fn set(&self, idx: usize, st: &NodeRunState) {
        self.lock()[idx] = st.clone();
    }

    pub(in crate::workflow_engine) fn with<R>(
        &self,
        idx: usize,
        f: impl FnOnce(&mut NodeRunState) -> R,
    ) -> R {
        f(&mut self.lock()[idx])
    }

    pub(in crate::workflow_engine) fn snapshot(&self) -> Vec<NodeRunState> {
        self.lock().clone()
    }

    /// The step numbers of the given nodes (those that have one).
    pub(in crate::workflow_engine) fn step_nos_of(&self, ids: &HashSet<String>) -> HashSet<u32> {
        self.lock()
            .iter()
            .filter(|s| ids.contains(&s.node_id))
            .filter_map(|s| s.step_no)
            .collect()
    }

    pub(in crate::workflow_engine) fn into_states(self) -> Vec<NodeRunState> {
        self.states.into_inner().unwrap_or_else(|e| e.into_inner())
    }

    /// Store `st` as node `idx`'s state, persist every node's state and
    /// push it to the run view, naming node `idx` as the one that changed.
    pub(in crate::workflow_engine) async fn publish(
        &self,
        cx: RunCx<'_>,
        idx: usize,
        st: &NodeRunState,
    ) {
        let _w = self.write.lock().await;
        let states = {
            let mut all = self.lock();
            all[idx] = st.clone();
            all.clone()
        };
        let mut overview = self.base.as_ref().map(|base| {
            let run = WorkflowRun {
                nodes: states.clone(),
                ..base.clone()
            };
            otto_workflows::overview::run_overview(
                &run,
                &cx.workflow.graph,
                None,
                None,
                chrono::Utc::now(),
            )
        });
        let rev = cx
            .repo
            .update_run_progress_with(cx.run_id, &states, overview.as_ref())
            .await
            .unwrap_or(0);
        if let Some(o) = overview.as_mut() {
            o.rev = rev;
        }
        // The live chat card follows every change (the pump coalesces, §9.4).
        if let Some(o) = overview.as_ref().filter(|_| cx.progress.live) {
            let view = super::chat::view(cx.workflow);
            let graph = &cx.workflow.graph;
            cx.progress
                .status(otto_workflows::chat_status::render_status(
                    &view, o, &states, graph,
                ));
        }
        let run = (&cx.workflow.workspace_id, cx.run_id, "running", rev);
        emit_run_updated_with(
            cx.ctx,
            run,
            Some(&states[idx].node_id),
            Some(&states[idx]),
            &states,
            false,
            overview.as_ref(),
        );
    }

    /// [`RunBoard::publish`] of the state the board holds for `idx`.
    pub(in crate::workflow_engine) async fn publish_current(&self, cx: RunCx<'_>, idx: usize) {
        let st = self.get(idx);
        self.publish(cx, idx, &st).await;
    }
}
