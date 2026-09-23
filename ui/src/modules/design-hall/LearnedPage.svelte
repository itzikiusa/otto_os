<script lang="ts">
  // What Otto learned (Phase 0 scaffold). Signals are REAL: the raw, filterable
  // log of design decisions captured by the daemon and the Hall. Rules and
  // Memory are honest empty states — nothing is learned from signals until
  // Phase 1, and nothing will ever be applied without a person approving it.
  import { untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { router } from '../../lib/router.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { designBus } from '../../lib/events.svelte';
  import { listSignals } from '../../lib/api/design';
  import type { DesignSignal, DesignSignalKind } from '../../lib/api/types';
  import { signalLabel, signalSummary, signalTone } from './model';
  import { library } from './library.svelte';

  interface Props {
    tab: 'signals' | 'rules' | 'memory';
  }
  let { tab }: Props = $props();

  const KINDS: DesignSignalKind[] = [
    'variant_chosen',
    'variant_rejected',
    'edit_after_draft',
    'review_comment',
    'critique_finding',
    'a11y_fix',
    'brand_correction',
    'rule_feedback',
    'status_change',
    'shipped',
  ];
  let kind = $state<'' | DesignSignalKind>('');
  let signals = $state<DesignSignal[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let seq = 0;

  async function load(): Promise<void> {
    const my = ++seq;
    loading = true;
    try {
      const s = await listSignals({ limit: 300, ...(kind ? { kind } : {}) });
      if (my !== seq) return;
      signals = s;
      error = null;
    } catch (e) {
      if (my === seq) error = e instanceof Error ? e.message : String(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  $effect(() => {
    void kind;
    void designBus.resyncTick;
    untrack(() => {
      void load();
      if (!library.loaded) void library.load();
    });
  });
  let seen = designBus.seq;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      if (evs.some((e) => e.type === 'design_learning_update')) void load();
    });
  });

  const titleOf = (id: string) => library.hitOf(id)?.artifact.title ?? 'A design';
  function byLabel(s: DesignSignal): string {
    if (s.actor_kind === 'agent') return 'Otto';
    if (s.actor_kind === 'system') return 'System';
    return s.actor_id && s.actor_id === auth.me?.id ? 'You' : 'Teammate';
  }
  function whenLabel(iso: string): string {
    const d = new Date(iso);
    const today = new Date();
    if (d.toDateString() === today.toDateString()) return `Today ${d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}`;
    return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  }

  function go(t: 'signals' | 'rules' | 'memory'): void {
    router.go(t === 'signals' ? 'design/learned' : `design/learned/${t}`);
  }
  // Tablist keys: ←/→ move between tabs (each tab is a route) and focus follows.
  let tablist = $state<HTMLDivElement | null>(null);
  function onTabKey(e: KeyboardEvent): void {
    const order: ('signals' | 'rules' | 'memory')[] = ['signals', 'rules', 'memory'];
    const i = order.indexOf(tab);
    let j = -1;
    if (e.key === 'ArrowRight') j = (i + 1) % order.length;
    else if (e.key === 'ArrowLeft') j = (i + order.length - 1) % order.length;
    else if (e.key === 'Home') j = 0;
    else if (e.key === 'End') j = order.length - 1;
    if (j < 0) return;
    e.preventDefault();
    go(order[j]);
    tablist?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[j]?.focus();
  }
</script>

<PageHeader title="What Otto learned" subtitle="Design decisions your team makes, and what Otto will learn from them"
  crumbs={[{ label: 'Design Hall', onclick: () => router.go('design') }]}>
  {#snippet tabs()}
    <div class="segmented" role="tablist" aria-label="Learning" bind:this={tablist}>
      <button role="tab" aria-selected={tab === 'signals'} class:active={tab === 'signals'} onclick={() => go('signals')} onkeydown={onTabKey}>Signals <span class="n">{signals.length}</span></button>
      <button role="tab" aria-selected={tab === 'rules'} class:active={tab === 'rules'} onclick={() => go('rules')} onkeydown={onTabKey}>Rules</button>
      <button role="tab" aria-selected={tab === 'memory'} class:active={tab === 'memory'} onclick={() => go('memory')} onkeydown={onTabKey}>Memory</button>
    </div>
  {/snippet}
</PageHeader>

<PageBody>
  {#if tab === 'signals'}
    <div class="filters" role="group" aria-label="Filter by kind">
      <button class="pill-toggle" class:on={kind === ''} aria-pressed={kind === ''} onclick={() => (kind = '')}>All</button>
      {#each KINDS as k (k)}
        <button class="pill-toggle" class:on={kind === k} aria-pressed={kind === k} onclick={() => (kind = k)}>{signalLabel(k)}</button>
      {/each}
    </div>
    {#if loading && !signals.length}
      <Skeleton rows={6} height={32} />
    {:else if error}
      <div class="err" role="alert">
        <Icon name="warning" size={14} /> Couldn’t load signals. <span class="dim">{error}</span>
        <button class="btn small" onclick={() => void load()}>Retry</button>
      </div>
    {:else if signals.length === 0}
      <EmptyState
        variant="page"
        icon="bulb"
        title={kind ? `No “${signalLabel(kind)}” signals yet` : 'No signals yet'}
        body="Approving, shipping and restoring versions — and editing a draft an agent wrote — are captured here as signals. Nothing learns from them until Phase 1."
      />
    {:else}
      <div class="table-wrap">
        <table data-testid="design-signals">
          <thead>
            <tr><th>When</th><th>Kind</th><th>Signal</th><th>Design</th><th>By</th></tr>
          </thead>
          <tbody>
            {#each signals as s (s.id)}
              <tr>
                <td class="when" title={new Date(s.created_at).toLocaleString()}>{whenLabel(s.created_at)} <span class="dim">· {rel(s.created_at)}</span></td>
                <td><span class="kind tone-{signalTone(s.kind)}">{signalLabel(s.kind)}</span></td>
                <td class="sum">{signalSummary(s)}</td>
                <td class="art"><a href={`#/design/a/${encodeURIComponent(s.artifact_id)}`}>{titleOf(s.artifact_id)}</a></td>
                <td class="by">{byLabel(s)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="foot dim">Every approve, ship, restore and edit after an agent draft lands here. Signals store version references and short summaries — never design content.</p>
    {/if}
  {:else if tab === 'rules'}
    <EmptyState
      variant="page"
      icon="bulb"
      title="No team rules yet"
      body={'In Phase 1 Otto groups repeated signals into proposed rules — for example “you picked the bolder hero 3 of 4 times” — and shows the evidence. You approve, edit or reject each one; nothing is applied without your approval, and every rule can be rolled back.'}
    />
  {:else}
    <EmptyState
      variant="page"
      icon="book"
      title="No design memory yet"
      body="Approved rules and preferences will be kept in Otto memory (collection: design) on this Mac, where agents recall them for each design turn. It arrives with the learning loop in Phase 1."
    />
  {/if}
</PageBody>

<style>
  .n {
    color: var(--text-dim);
    margin-inline-start: 2px;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-block-end: 14px;
  }
  .table-wrap {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  th {
    position: sticky;
    inset-block-start: 0;
    text-align: start;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
    padding: 8px 12px;
    background: var(--surface);
    border-block-end: 1px solid var(--border);
  }
  td {
    padding: 7px 12px;
    border-block-end: 1px solid var(--border);
    white-space: nowrap;
  }
  tbody tr:last-child td {
    border-block-end: 0;
  }
  tbody tr:hover {
    background: var(--hover);
  }
  td.sum {
    white-space: normal;
    min-width: 240px;
  }
  td.art a {
    color: var(--text);
    text-decoration: none;
  }
  td.art a:hover {
    color: var(--accent-text);
  }
  .kind {
    font-size: var(--fs-xs);
    font-weight: 500;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .tone-ok {
    color: var(--success);
    background: var(--success-soft);
  }
  .tone-bad {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .tone-warn {
    color: var(--warning);
    background: var(--warning-soft);
  }
  .tone-info {
    color: var(--info);
    background: var(--info-soft);
  }
  .dim {
    color: var(--text-dim);
  }
  .foot {
    margin: 12px 0 0;
    font-size: var(--fs-xs);
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-s);
  }
  .err > :global(svg) {
    color: var(--danger);
  }
</style>
