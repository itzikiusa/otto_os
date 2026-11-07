<!--
  A personal agent's run result in its chat — the routine's headline as a
  small card with "Open report" (the run's report in the shared viewer)
  instead of a grey footnote line. Parsed by `runNoticeOf` (timeline.ts).
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import { personalAgentsApi } from '../../../lib/api/personalAgents';
  import { loadErrorText } from '../../../lib/loadError';
  import { toasts } from '../../../lib/toast.svelte';
  import type { PersonalAgentRun } from '../../../lib/api/types';
  import type { RunNotice } from './timeline';

  let { notice, agentId = null }: { notice: RunNotice; agentId?: string | null } = $props();

  let run = $state<PersonalAgentRun | null>(null);
  let opening = $state(false);
  let Report = $state<typeof import('../../../lib/components/RunReport.svelte').default | null>(null);

  async function open(): Promise<void> {
    if (!agentId || opening) return;
    opening = true;
    try {
      const [runs, mod] = await Promise.all([
        personalAgentsApi.runs(agentId),
        import('../../../lib/components/RunReport.svelte'),
      ]);
      Report = mod.default;
      const found = runs.find((r) => r.id === notice.runId);
      if (found) run = found;
      else toasts.info('That run is no longer in its history.');
    } catch (e) {
      toasts.error('Couldn’t open the report', loadErrorText(e));
    } finally {
      opening = false;
    }
  }
</script>

<div class="rn" class:failed={notice.outcome === 'failed'} class:stopped={notice.outcome === 'stopped'} data-testid="runtime-run-notice">
  <Icon name={notice.outcome === 'ok' ? 'check' : notice.outcome === 'failed' ? 'warning' : 'x'} size={14} />
  <span class="rn-text" dir="auto">{notice.text}</span>
  {#if agentId}
    <button type="button" class="btn small" disabled={opening} onclick={open}>Open report</button>
  {/if}
</div>
{#if run && Report}
  <Report {run} routine={run.schedule_id ? notice.text.replace(/ (ran|failed|was stopped|finished)( · .*)?$/s, '') : ''} onclose={() => (run = null)} />
{/if}

<style>
  .rn {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--success);
    margin-block: 4px;
  }
  .rn.failed { color: var(--danger); }
  .rn.stopped { color: var(--text-dim); }
  .rn-text { flex: 1; min-width: 0; color: var(--text); overflow-wrap: anywhere; }
</style>
