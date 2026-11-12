<script lang="ts">
  // Bulk triage for the findings board: one confirm naming the count, then the
  // same per-finding endpoints, one after another (each writes its own audit
  // event and declines the finding's draft comment server-side). Findings the
  // action can't apply to are skipped and counted. Wraps at phone width.
  import type { Finding } from '../../lib/api/types';
  import { acceptFinding, falsePositiveFinding, waiveFinding } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { plural } from '../../lib/plural';
  import { announce } from '../../lib/announce.svelte';
  import { canBulk, type BulkAction } from './findingsTriage';

  interface Props {
    selected: Finding[];
    onupdated: (f: Finding) => void;
    onclear: () => void;
  }
  let { selected, onupdated, onclear }: Props = $props();
  let busy = $state(false);

  const ACTIONS: BulkAction[] = ['accept', 'false_positive', 'waive'];
  const LABEL: Record<BulkAction, string> = { accept: 'Accept', false_positive: 'Not a problem', waive: 'Won’t fix' };
  const CALL: Record<BulkAction, (id: string) => Promise<Finding>> = {
    accept: (id) => acceptFinding(id),
    false_positive: (id) => falsePositiveFinding(id, 'Bulk triage: not a problem'),
    waive: (id) => waiveFinding(id, 'Bulk triage: won’t fix'),
  };

  async function run(action: BulkAction): Promise<void> {
    if (busy) return;
    const eligible = selected.filter((f) => canBulk(f, action));
    if (eligible.length === 0) {
      toasts.warn(`None of the selected findings can be marked “${LABEL[action]}”`);
      return;
    }
    const skipped = selected.length - eligible.length;
    const ok = await confirmer.ask(
      `${LABEL[action]}: ${plural(eligible.length, 'finding')}${skipped ? ` (${skipped} skipped — their status doesn’t allow it)` : ''}.${action === 'accept' ? '' : ' Their draft comments are declined, so Post all won’t send them.'}`,
      { title: `${LABEL[action]} — ${plural(eligible.length, 'finding')}?`, confirmLabel: `${LABEL[action]} ${eligible.length}`, danger: false },
    );
    if (!ok) return;
    busy = true;
    announce(`Updating ${plural(eligible.length, 'finding')}…`);
    let failed = 0;
    for (const f of eligible) {
      try {
        onupdated(await CALL[action](f.id));
      } catch {
        failed++;
      }
    }
    busy = false;
    if (failed) toasts.error(`${failed} of ${eligible.length} couldn’t be updated`, 'The rest were saved — try those again.');
    else toasts.success(`${plural(eligible.length, 'finding')} updated`);
    onclear();
  }
</script>

{#if selected.length > 0}
  <div class="fbb" role="group" aria-label="Bulk actions" data-testid="findings-bulk-bar">
    <span class="fbb-count">{selected.length} selected</span>
    {#each ACTIONS as a (a)}
      <button class="btn small" disabled={busy} onclick={() => void run(a)}>{LABEL[a]}</button>
    {/each}
    <button class="btn small ghost" disabled={busy} onclick={onclear}>Clear</button>
    {#if busy}<span class="dim fbb-busy">Updating…</span>{/if}
  </div>
{/if}

<style>
  .fbb {
    display: flex; flex-wrap: wrap; align-items: center; gap: 6px;
    padding: 6px 8px; border: 1px solid var(--accent-line); border-radius: var(--radius-m);
    background: var(--accent-soft); position: sticky; inset-block-start: 0; z-index: 1;
  }
  .fbb-count { font-size: var(--fs-s); font-weight: 600; margin-inline-end: auto; }
  .fbb-busy { font-size: var(--fs-xs); }
  .dim { color: var(--text-dim); }
</style>
