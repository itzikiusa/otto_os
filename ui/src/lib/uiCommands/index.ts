// Agent UI control — every module's handlers. The shell (App.svelte) imports
// this once per document, before the events socket sends its `hello`, so
// `capabilities` lists them all.
//
// Only the shell's own handlers (`./nav`) load eagerly. Every other module is
// registered by NAME here and its handler file — which pulls in that module's
// store — is imported on the first command or `ui_state` read (perf F2: the
// shell no longer evaluates the database / apiClient / k8s / … stores before
// first paint). Add a module here when you add `uiCommands/<module>.ts`;
// unit/uiCommands.test.ts fails when a file's registered names and its entry
// below disagree, and on catalog drift.

import { describeDocument, ensureUiModuleState, registerLazyUiCommands } from '../uiCommands';

// ── Shell: state / open / focus (Agent 3) ──
import './nav';

// ── Database Explorer (Agent 4) ──
registerLazyUiCommands(
  'connections',
  ['db_list_connections', 'db_open_connection', 'db_new_tab', 'db_set_statement', 'db_run_query', 'db_get_result', 'db_page', 'db_set_view', 'db_open_object', 'db_explain', 'db_stop', 'db_export'],
  () => import('./database'),
  'connections',
);
// ── API client, Git, Browser (Agent 2) ──
registerLazyUiCommands(
  'api',
  ['api_state', 'api_open_request', 'api_set_draft', 'api_import_curl', 'api_select_env', 'api_send', 'api_get_response', 'api_save_request'],
  () => import('./api'),
  'api',
);
registerLazyUiCommands(
  'git',
  ['git_list_repos', 'git_open_repo', 'git_tab', 'git_status', 'git_diff', 'git_stage', 'git_commit', 'git_fetch', 'git_pull', 'git_push'],
  () => import('./git'),
  'git',
);
registerLazyUiCommands(
  'browser',
  ['browser_state', 'browser_open', 'browser_navigate', 'browser_select_tab', 'browser_set_mode', 'browser_get_page', 'browser_annotate', 'browser_summarize'],
  () => import('./browser'),
  'browser',
);
// ── Kubernetes, AWS, Brokers, Vault, Workflows, … (Agent 5) ──
registerLazyUiCommands(
  'kubernetes',
  ['k8s_list_clusters', 'k8s_open', 'k8s_list_resources', 'k8s_select', 'k8s_describe', 'k8s_logs', 'k8s_action'],
  () => import('./k8s'),
  'kubernetes',
);
registerLazyUiCommands(
  'aws',
  ['aws_list_accounts', 'aws_open', 'aws_s3_browse', 'aws_s3_preview', 'aws_sqs_list_queues', 'aws_ec2_list', 'aws_sqs_send'],
  () => import('./aws'),
  'aws',
);
registerLazyUiCommands(
  'connections',
  ['brokers_list_clusters', 'brokers_open', 'brokers_list_topics', 'brokers_list_groups', 'brokers_open_topic', 'brokers_peek', 'brokers_produce'],
  () => import('./brokers'),
);
registerLazyUiCommands(
  'vault',
  ['vault_list', 'vault_open_note', 'vault_search', 'vault_write_note'],
  () => import('./vault'),
  'vault',
);
registerLazyUiCommands(
  'workflows',
  ['wf_list', 'wf_open', 'wf_list_runs', 'wf_open_run', 'wf_run', 'wf_cancel_run'],
  () => import('./workflows'),
  'workflows',
);
registerLazyUiCommands(
  'scheduled-tasks',
  ['sched_list', 'sched_show_runs', 'sched_run_now', 'sched_set_enabled'],
  () => import('./scheduled'),
  'scheduled-tasks',
);
registerLazyUiCommands(
  'home',
  ['home_state', 'home_go_to_view', 'home_zoom_box', 'home_add_box', 'home_remove_box'],
  () => import('./home'),
  'home',
);
registerLazyUiCommands(
  'swarm',
  ['swarm_list', 'swarm_open', 'swarm_list_tasks', 'swarm_list_runs', 'swarm_run_task', 'swarm_stop_run'],
  () => import('./swarm'),
  'swarm',
);
registerLazyUiCommands(
  'loops',
  ['loops_list', 'loops_open', 'loops_control'],
  () => import('./loops'),
  'loops',
);

// The module on screen gets its handler file shortly after it shows (its
// store is already loaded by the page, so this is one small chunk): its
// `registerUiState` must answer SYNCHRONOUSLY when the host window reads a
// side pane's `__ottoUiState`, and the first command then runs at once.
if (typeof window !== 'undefined') {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const warm = (): void => {
    clearTimeout(timer);
    timer = setTimeout(() => void ensureUiModuleState(describeDocument().module), 1_500);
  };
  window.addEventListener('hashchange', warm);
  warm();
}

// The catalog JSON (55 KB) is a dev-time cross-check only — never in the
// production shell chunk.
if (import.meta.env.DEV) {
  void import('./catalog').then(({ catalogDrift }) =>
    import('../uiCommands').then(({ uiCapabilities, uiCommandModule }) => {
      const drift = catalogDrift(uiCapabilities().map((name) => ({ name, module: uiCommandModule(name) ?? '' })));
      if (drift.missing.length || drift.extra.length || drift.misplaced.length) {
        console.warn('[uiCommands] handlers and docs/contracts/ui-commands.json disagree', drift);
      }
    }),
  );
}
