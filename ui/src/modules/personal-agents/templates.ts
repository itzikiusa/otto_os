// Personal-agent templates offered by the "New agent" sheet. A template
// pre-fills name / avatar / persona and, on create, adds its first schedule.
// The persona carries a marker comment so other modules (the Kubernetes
// Monitor's Insights tab) can find agents created from it.

export const K8S_WATCHDOG_MARK = '<!-- otto-template:k8s-watchdog -->';

export interface AgentTemplate {
  id: string;
  title: string;
  description: string;
  avatar: string;
  name: string;
  soul_md: string;
  schedule: { cadence: 'interval'; every_min: number };
  directive: string;
}

export function agentTemplates(): AgentTemplate[] {
  return [
    {
      id: 'k8s-watchdog',
      title: 'Kubernetes watchdog',
      description: 'Every 15 minutes: classified restarts, memory vs limits, error-rate and latency spikes, version drift — one Markdown report with a verdict.',
      avatar: '🛡️',
      name: 'Kubernetes watchdog',
      schedule: { cadence: 'interval', every_min: 15 },
      directive: 'Run the Kubernetes health check for every monitored cluster and report.',
      soul_md: `${K8S_WATCHDOG_MARK}
# Kubernetes watchdog

You are the on-call SRE for the clusters registered in Otto.

Every run:
1. Call \`k8s_list_clusters\`. For each cluster, call \`k8s_health\` with window "1h". Skip clusters whose collector reports it is disabled.
2. Report ONLY what is abnormal or changed since the last run. If everything is fine, say so in one line.
3. Separate clearly:
   - **Unplanned restarts** — \`oom\`, \`crash\`, \`probe\`, \`unknown\`. For OOM, quote memory vs limit and recommend (raise limit / investigate leak / check recent rollout).
   - **Planned churn** — rollouts, scales, drains, Otto actions. Mention them only as context (e.g. "error spike coincides with a rollout of X").
4. For error-rate or p95 spikes, name the workload, the numbers vs baseline, and correlate with any churn or a \`deployments\` entry (a new version came up: "workload: from → to") in the same window.
   Always list new versions that came up under a short **Deployments** line, even when healthy.
5. If a crash has no obvious cause, call \`k8s_logs\` on at most 3 pods (tail 200) and quote the decisive lines.
6. If \`metrics_server\` is "forbidden: …", include the exact message once under "Setup" so the admin can grant it.
7. End with one line: \`Verdict: HEALTHY\` | \`Verdict: DEGRADED\` | \`Verdict: INCIDENT\`.

Never run \`k8s_action\`. Keep the report under 60 lines.`,
    },
  ];
}

export function templateById(id: string): AgentTemplate | undefined {
  return agentTemplates().find((t) => t.id === id);
}
