<script lang="ts">
  // Self-improvement page header actions that act on EVERY workspace the
  // caller can change: “Reject all suggestions” and “Turn off everywhere”.
  // Rendered straight into the PageHeader actions row (no wrapper) with the
  // lowest overflow priority, so they fold into ⋯ first. Each shows only
  // when it has something to do.
  import { improveApi } from '../../lib/api/improve';
  import type { BulkImprovementResp, SelfImprovementSummary } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { improvementBus } from '../../lib/events.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { latestOnly } from '../../lib/latest';
  import { bulkResultText, rejectAllMessage, turnOffAllMessage } from './selfImprovementBulk';

  // `watch`: the page's own state (workspace, its pending count, on/off);
  // a change re-reads the summary, so the counts follow single-edit actions.
  let { watch, onchanged }: { watch: string; onchanged: () => void } = $props();

  let summary: SelfImprovementSummary | null = $state(null);
  let busy = $state(false);

  const loads = latestOnly();
  async function refresh(): Promise<void> {
    const t = loads.begin();
    try {
      const s = await improveApi.summary();
      if (t.current) summary = s;
    } catch {
      // The page shows load errors; these actions just stay hidden.
    }
  }
  $effect(() => {
    const _deps = [improvementBus.tick, watch];
    void refresh();
  });

  async function run(
    kind: 'reject' | 'off',
    message: string,
    opts: { title: string; confirmLabel: string; danger?: boolean },
    call: () => Promise<BulkImprovementResp>,
  ): Promise<void> {
    if (!(await confirmer.ask(message, opts))) return;
    busy = true;
    try {
      const r = await call();
      toasts.success(kind === 'reject' ? 'Suggestions rejected' : 'Self-improvement turned off', bulkResultText(r, kind));
      await refresh();
      onchanged();
    } catch (e) {
      toastError(kind === 'reject' ? 'Couldn’t reject the suggestions' : 'Couldn’t turn self-improvement off', e);
    } finally {
      busy = false;
    }
  }
</script>

{#if summary && summary.pending > 0}
  {@const s = summary}
  <button
    class="btn small"
    data-icon="x"
    data-overflow="-1"
    disabled={busy}
    title="Reject every pending suggestion in all your workspaces"
    onclick={() => run('reject', rejectAllMessage(s), { title: 'Reject all suggestions', confirmLabel: 'Reject all', danger: true }, improveApi.rejectAll)}
  >Reject all suggestions ({s.pending})</button>
{/if}
{#if summary && summary.enabled > 0}
  {@const s = summary}
  <button
    class="btn small"
    data-icon="pause"
    data-overflow="-1"
    disabled={busy}
    title="Turn self-improvement off in all your workspaces"
    onclick={() => run('off', turnOffAllMessage(s), { title: 'Turn off self-improvement everywhere', confirmLabel: 'Turn off' }, () => improveApi.setEnabledAll(false))}
  >Turn off everywhere</button>
{/if}
