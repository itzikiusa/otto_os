<script lang="ts">
  // Lobby rail: "What Otto learned". Phase 0 only CAPTURES design signals —
  // nothing is learned from them yet — so the card shows the latest signals and
  // says honestly that rules come later. No invented rules.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { designBus } from '../../lib/events.svelte';
  import { listSignals } from '../../lib/api/design';
  import type { DesignSignal } from '../../lib/api/types';
  import { library } from './library.svelte';
  import { signalLabel, signalSummary, signalTone } from './model';

  let signals = $state<DesignSignal[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let seq = 0;

  async function load(): Promise<void> {
    const my = ++seq;
    try {
      const s = await listSignals({ limit: 5 });
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
    void designBus.resyncTick;
    untrack(() => void load());
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
</script>

<section class="card rail-card" aria-labelledby="dh-learned-h" data-testid="design-learned-card">
  <header>
    <span class="ico"><Icon name="bulb" size={14} /></span>
    <h2 id="dh-learned-h">What Otto learned</h2>
  </header>
  <p class="lead">
    Every approve, restore and edit after an agent draft is captured as a signal. Otto starts proposing team
    rules once enough signals exist — nothing is applied without your approval.
  </p>
  {#if loading}
    <p class="dim small" role="status">Loading signals…</p>
  {:else if error}
    <div class="err">
      <Icon name="warning" size={14} />
      <span>Couldn’t load signals.</span>
      <button class="btn small ghost" onclick={() => void load()}>Retry</button>
    </div>
  {:else if signals.length === 0}
    <p class="dim small">No signals yet. They appear as you review and approve designs.</p>
  {:else}
    <ul class="sigs">
      {#each signals as s (s.id)}
        <li>
          <span class="kind tone-{signalTone(s.kind)}">{signalLabel(s.kind)}</span>
          <a class="what" href={`#/design/a/${encodeURIComponent(s.artifact_id)}`}>{titleOf(s.artifact_id)}</a>
          <span class="sum">{signalSummary(s)}</span>
          <span class="when" title={new Date(s.created_at).toLocaleString()}>{rel(s.created_at)}</span>
        </li>
      {/each}
    </ul>
  {/if}
  <a class="more" href="#/design/learned">Open the learning log <Icon name="chevronRight" size={12} /></a>
</section>

<style>
  .rail-card {
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ico {
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  h2 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .lead {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.45;
  }
  .small {
    margin: 0;
    font-size: var(--fs-s);
  }
  .err {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .err :global(svg) {
    color: var(--danger);
  }
  .sigs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .sigs li {
    display: grid;
    grid-template-columns: auto 1fr auto;
    grid-template-areas: 'kind what when' 'sum sum sum';
    gap: 2px 8px;
    padding: 8px 0;
    border-block-start: 1px solid var(--border);
    align-items: center;
  }
  .kind {
    grid-area: kind;
    font-size: var(--fs-xs);
    font-weight: 500;
    padding: 1px 6px;
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
  .what {
    grid-area: what;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
    text-decoration: none;
    font-size: var(--fs-s);
    font-weight: 500;
  }
  .what:hover {
    color: var(--accent-text);
  }
  .sum {
    grid-area: sum;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .when {
    grid-area: when;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .more {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    align-self: flex-start;
    font-size: var(--fs-s);
    color: var(--accent-text);
    text-decoration: none;
  }
  .more:hover {
    text-decoration: underline;
  }
</style>
