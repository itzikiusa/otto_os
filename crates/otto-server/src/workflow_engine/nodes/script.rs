//! The `script` workflow node (engine.md §7.2): the step itself is
//! `otto_workflows::script_run`; this hands it the daemon's repo rows, the
//! shell-task sandbox and the cached login `PATH`.

use super::super::*;
use super::NodeCall;

struct Host<'a>(&'a ServerCtx);

#[async_trait::async_trait]
impl otto_workflows::script_run::ScriptHost for Host<'_> {
    async fn repo(&self, id: &str) -> Option<(String, String)> {
        let r = self.0.git_store.get_repo(&id.into()).await.ok()?;
        Some((r.name, r.path))
    }
    async fn sandbox(&self, cwd: &str) -> Option<otto_sandbox::SandboxPolicy> {
        let cwd = std::path::Path::new(cwd);
        self.0.manager.process_sandbox_policy("shell", cwd).await
    }
    fn login_path(&self) -> Option<String> {
        let mut env = self.0.ide.environments.cached_shell_env_snapshot();
        env.remove("PATH")
    }
}

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let (node, env, scope) = (call.node, call.env, call.scope);
    let name = node_display_name(node);
    let base =
        crate::workflow_context::step_base_name(scope.step_no, name, scope.iter, scope.inner_idx);
    let at = otto_workflows::script_run::ScriptRun {
        files: &env.files,
        run_id: env.run_id.as_str(),
        run_cwd: &env.run_cwd,
        step_base: &base,
    };
    otto_workflows::script_run::run(&Host(call.ctx), &at, &node.params)
        .await
        .map_err(otto_core::Error::Upstream)
}

#[cfg(test)]
#[path = "script_tests.rs"]
pub(super) mod tests;
