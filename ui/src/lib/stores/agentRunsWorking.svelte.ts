// Personal agents that are working — a run in flight (from
// `personal_agent_run_updated`) or a turn in their chat (a run's follow-up),
// kept apart from the Personal Agents store so the status bar's "N working"
// counts them without loading that page. Only what this window has seen.

class AgentRunsWorking {
  /** `<agentId> <source>` → working. */
  private on: Record<string, boolean> = $state({});
  count: number = $derived(new Set(Object.keys(this.on).filter((k) => this.on[k]).map((k) => k.split(' ')[0])).size);

  set(agentId: string, working: boolean, source = 'run'): void {
    const k = `${agentId} ${source}`;
    if (!!this.on[k] !== working) this.on = { ...this.on, [k]: working };
  }
}

export const agentRunsWorking = new AgentRunsWorking();
