<script lang="ts">
  // Lobby rail: "What Otto learned from your team". The real pending rule
  // proposals for this workspace (Learning v1, suggest-only) with Keep
  // (approve), Dismiss (reject) and Details — each decision asks first — plus
  // the active-rule count and how many patterns are still forming. When
  // nothing is pending it says how rules appear; no invented rules.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { router } from '../../lib/router.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { designBus } from '../../lib/events.svelte';
  import type { DesignLearnedEdit, DesignLearnedResp } from '../../lib/api/types';
  import { getLearned } from './assist/api';
  import { editHeadline } from './assist/model';
  import { approveRule, rejectRule } from './assist/ruleActions';

  let learned = $state<DesignLearnedResp | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let seq = 0;

  async function load(): Promise<void> {
    const w = ws.currentId;
    if (!w) {
      loading = false;
      return;
    }
    const my = ++seq;
    try {
      const l = await getLearned(w);
      if (my !== seq) return;
      learned = l;
      error = null;
    } catch (e) {
      if (my === seq) error = e instanceof Error ? e.message : String(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  $effect(() => {
    void ws.currentId;
    void designBus.resyncTick;
    untrack(() => void load());
  });

  let seen = designBus.seq;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      if (evs.some((e) => e.type === 'design_learning_update' && e.workspace_id === ws.currentId)) void load();
    });
  });

  const pending = $derived(learned?.pending ?? []);
  const forming = $derived((learned?.candidates ?? []).filter((c) => !c.ready).length);
  const canEdit = $derived(auth.can('design', 'edit'));

  async function keep(e: DesignLearnedEdit): Promise<void> {
    busy = e.edit_id;
    if (await approveRule(e, learned?.skill)) await load();
    busy = null;
  }
  async function dismiss(e: DesignLearnedEdit): Promise<void> {
    busy = e.edit_id;
    if (await rejectRule(e)) await load();
    busy = null;
  }
  const details = (e: DesignLearnedEdit) => router.go(`design/learned/pending/${encodeURIComponent(e.edit_id)}`);
</script>

<section class="card rail-card" aria-labelledby="dh-learned-h" data-testid="design-learned-card">
  <header>
    <span class="ico"><Icon name="bulb" size={14} /></span>
    <h2 id="dh-learned-h">What Otto learned from your team</h2>
  </header>
  {#if loading}
    <p class="dim small" role="status">Loading…</p>
  {:else if error}
    <div class="err">
      <Icon name="warning" size={14} />
      <span>Couldn’t load team rules.</span>
      <button class="btn small ghost" onclick={() => void load()}>Retry</button>
    </div>
  {:else if learned}
    <p class="counts">
      {#if pending.length}<span class="chip new">{pending.length} new</span>{/if}
      <span class="dim">{learned.active.length} active rule{learned.active.length === 1 ? '' : 's'}</span>
      {#if learned.mode === 'off'}<span class="dim">· learning off</span>{/if}
    </p>
    {#if pending.length === 0}
      <p class="dim small">
        No rules waiting. Otto proposes one when the same choice repeats 3 times across 2 designs{forming ? ` — ${forming} pattern${forming === 1 ? ' is' : 's are'} forming` : ''}.
      </p>
    {:else}
      <ul class="props">
        {#each pending.slice(0, 3) as e (e.edit_id)}
          <li data-testid="design-learned-proposal">
            <p class="rule">{editHeadline(e)}</p>
            {#if e.rationale}<p class="why"><Icon name="clock" size={11} /> {e.rationale}</p>{/if}
            <div class="acts">
              <button class="btn small" disabled={!canEdit || busy === e.edit_id} onclick={() => void keep(e)} data-testid="design-learned-keep">
                <Icon name="check" size={11} /> Keep
              </button>
              <button class="btn small ghost" disabled={!canEdit || busy === e.edit_id} onclick={() => void dismiss(e)} data-testid="design-learned-dismiss">Dismiss</button>
              <span class="grow"></span>
              <button class="linkbtn" onclick={() => details(e)}>Details</button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
  <a class="more" href={pending.length ? '#/design/learned/pending' : '#/design/learned/rules'}>
    {pending.length ? 'Review all' : 'See team rules'} <Icon name="chevronRight" size={12} />
  </a>
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
  .counts {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-s);
  }
  .new {
    color: var(--accent-text);
    border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .dim {
    color: var(--text-dim);
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
  .props {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .props li {
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .rule {
    margin: 0;
    font-size: var(--fs-s);
    font-weight: 600;
    line-height: 1.35;
  }
  .why {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .why :global(svg) {
    vertical-align: -2px;
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-block-start: 2px;
  }
  .grow {
    flex: 1;
  }
  .linkbtn {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .linkbtn:hover {
    text-decoration: underline;
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
