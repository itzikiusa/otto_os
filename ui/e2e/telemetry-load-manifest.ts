/** Module-level navigation coverage comes from SIDEBAR_MODULES automatically.
 * These populated workloads complement it; a new shared heavy component must
 * add a scenario here and run with the telemetry load report. */
export const TELEMETRY_LOAD_SCENARIOS = [
  { component: 'conversation', spec: 'desktop-conversation-load-perf.spec.ts', workload: 'large transcripts, concurrent streams and scrolling' },
  { component: 'terminal', spec: 'desktop-terminal-compact-queue-perf.spec.ts', workload: 'PTY bursts and bounded queued output' },
  { project: 'desktop-webkit', component: 'database.results', spec: 'desktop-db-results-perf.spec.ts', workload: 'large result tables and virtualized scrolling' },
  { project: 'desktop-webkit', component: 'database.editor', spec: 'desktop-db-editor-perf.spec.ts', workload: 'large query editing and completion' },
  { component: 'git.graph', spec: 'desktop-git-graph-perf.spec.ts', workload: 'large commit graph navigation' },
  { component: 'git.diff', spec: 'desktop-review5-shared-diff-perf.spec.ts', workload: 'equal, sparse and fully rewritten large diffs' },
  { component: 'api.history', spec: 'desktop-api-history-performance.spec.ts', workload: 'large request history paging' },
  { component: 'vault', spec: 'desktop-vault-scale-perf.spec.ts', workload: 'large note and graph collections' },
  { component: 'canvas', spec: 'desktop-canvas-scale-perf.spec.ts', workload: 'large canvas scenes' },
  { component: 'design', spec: 'desktop-design-hall-scale-perf.spec.ts', workload: 'large artifact catalog' },
  { component: 'agents', spec: 'desktop-agents-scale-perf.spec.ts', workload: 'large session/history lists and transcript collections' },
  { component: 'agents.workflows', spec: 'desktop-performance-agents-workflows.spec.ts', workload: 'History reader recovery and bounded workflow polling/lazy bodies' },
  { component: 'infrastructure', spec: 'desktop-infra-perf.spec.ts', workload: 'Kubernetes and infrastructure collections' },
  { component: 'transport', spec: 'desktop-transport-perf.spec.ts', workload: 'interactive requests under background saturation' },
] as const;
