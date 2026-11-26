<script lang="ts">
  // Optimize (addendum-chat-optimize.md §2.4–2.5; a lazy chunk): Otto's
  // rule-based draft of a faster version of this workflow — the expected
  // saving from its own runs, what was checked, the changes grouped by what
  // they may change (ticked as the server pre-selects them), what was not
  // offered and why, Before | After and what each step reads and writes.
  // Ticking a change re-asks the server for that selection's draft (the
  // client never builds a graph). Nothing is saved until "Apply as version
  // N…" confirms; a person's credential only (an agent gets 403).
  import { untrack } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import PageHeader from '../../../lib/components/PageHeader.svelte';
  import PageBody from '../../../lib/components/PageBody.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Tabs from '../../../lib/components/Tabs.svelte';
  import { api, ApiError } from '../../../lib/api/client';
  import type { OptimizeDraft, Workflow } from '../../../lib/api/types';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { defaultAgentProvider } from '../../../lib/providers';
  import { router } from '../../../lib/router.svelte';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import EditorCanvas from '../editor/EditorCanvas.svelte';
  import { clockTime } from '../runs/runWords';
  import { afterMarks, canApply, estimateLines, groupChanges, initialSelection, savingText } from './optimizeView';

  let { workflowId }: { workflowId: string } = $props();
  const path = $derived(`/workflows/${encodeURIComponent(workflowId)}/optimize`);
  const phone = $derived(viewport.isPhone);

  let wf = $state<Workflow | null>(null);
  /** The pre-selected answer: the changes and what was not offered. */
  let first = $state.raw<OptimizeDraft | null>(null);
  /** The answer for the current selection: draft graph, estimate, checks. */
  let draft = $state.raw<OptimizeDraft | null>(null);
  let selected = $state<string[]>([]);
  let error = $state<string | null>(null);
  let stale = $state<string | null>(null);
  let rejected = $state(false);
  let applying = $state(false);
  let view = $state<'before' | 'after'>('after');
  let notOfferedOpen = $state(false);

  async function load(): Promise<void> {
    error = null;
    stale = null;
    rejected = false;
    try {
      const [w, d] = await Promise.all([api.get<Workflow>(`/workflows/${encodeURIComponent(workflowId)}`), api.post<OptimizeDraft>(path, {})]);
      wf = w;
      first = d;
      draft = d;
      selected = initialSelection(d.changes);
    } catch (e) {
      error = loadErrorText(e);
    }
  }
  $effect(() => {
    void workflowId;
    untrack(() => void load());
  });

  const key = (ids: readonly string[]) => [...ids].sort().join(',');
  // Re-ask for the ticked selection's draft (debounced 300 ms).
  $effect(() => {
    const ids = [...selected];
    const base = first;
    if (!base) return;
    if (key(ids) === key(initialSelection(base.changes))) {
      draft = base;
      return;
    }
    const t = setTimeout(() => {
      api.post<OptimizeDraft>(path, { change_ids: ids }).then(
        (d) => { if (key(selected) === key(ids)) draft = d; },
        (e) => toastError('Couldn’t check this selection', e),
      );
    }, 300);
    return () => clearTimeout(t);
  });

  const toggle = (id: string) => (selected = selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id]);
  const next = $derived((draft?.base_version ?? wf?.version ?? 0) + 1);
  const names = $derived(new Map([...(wf?.graph.nodes ?? []), ...(draft?.draft_graph.nodes ?? [])].map((n) => [n.id, n.name || n.id])));
  const marks = $derived(wf && draft && first ? afterMarks(wf.graph, draft.draft_graph, first.changes, selected) : {});
  const est = $derived(draft ? estimateLines(draft.estimate) : null);
  const groups = $derived(first ? groupChanges(first.changes) : []);
  const renamed = $derived(new Map((draft?.checks.renumbered ?? []).map((r) => [r.node_id, r])));

  async function apply(): Promise<void> {
    if (!wf || !draft || !canApply(selected)) return;
    const base = draft.base_version;
    const n = selected.length;
    const ok = await confirmer.ask(
      `${n} ${n === 1 ? 'change' : 'changes'}. Version ${base} stays in Versions and can be restored. Runs already started keep version ${base}; the next run — including from Slack — uses version ${next}.`,
      { title: `Save as version ${next} of ${wf.name}?`, confirmLabel: `Save version ${next}` },
    );
    if (!ok) return;
    applying = true;
    try {
      const saved = await api.post<Workflow>(`${path}/apply`, { base_version: base, change_ids: selected });
      toasts.success(`Saved version ${saved?.version ?? next}`);
      router.go(`workflows/${workflowId}`);
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
        const now = await api.get<Workflow>(`/workflows/${encodeURIComponent(workflowId)}`).catch(() => null);
        stale = now ? `${now.name} changed (version ${now.version} saved at ${clockTime(now.updated_at)}).` : `${wf.name} changed since this draft.`;
      } else if (e instanceof ApiError && e.status === 422) rejected = true;
      else if (e instanceof ApiError && e.status === 403) toasts.error('Only a person can apply this draft.');
      else toastError('Couldn’t save the optimized version', e);
    } finally {
      applying = false;
    }
  }
  const back = () => router.go(`workflows/${workflowId}`);
</script>

<PageHeader title="Optimize" crumbs={phone || !wf ? [] : [{ label: wf.name, onclick: back }]}
  more={phone ? () => [{ label: 'Discard', icon: 'x', action: back }] : undefined}>
  {#snippet leading()}
    {#if phone}<button type="button" class="icon-btn" aria-label="Back to the workflow" title="Back to the workflow" onclick={back}><Icon name="chevronLeft" size={16} /></button>{/if}
  {/snippet}
  {#snippet actions()}
    {#if !phone}<button type="button" class="btn" onclick={back}>Discard</button>{/if}
    <button type="button" class="btn primary" data-testid="wf-opt-apply" disabled={!draft || !canApply(selected) || !!stale || applying}
      title={!canApply(selected) ? 'Tick at least one change' : stale ? 'Check again first' : undefined} onclick={() => void apply()}>
      {applying ? 'Saving…' : `Apply as version ${next}…`}
    </button>
  {/snippet}
</PageHeader>
<PageBody fill padded={false}>
  {#if !wf || !draft || !first}
    <LoadState what="this workflow’s draft" errorTitle="Couldn’t check this workflow" variant="page" loading={!error} {error} empty onretry={() => void load()} />
  {:else}
    <div class="opt" data-testid="wf-opt-page">
      <div class="side">
        {#if stale || rejected}
          <p class="banner" use:announceShown={{ text: stale ?? 'A change no longer passes its checks.' }}>{stale ?? 'A change no longer passes its checks.'} <button type="button" class="btn small" onclick={() => void load()}>Check again</button></p>
        {/if}
        <section class="opt-card" data-testid="wf-opt-estimate">
          {#if est?.none}<p class="dim">{est.none}</p>
          {:else if est}<p class="big">Expected: {est.typical}</p><p class="dim">{[est.slow, est.basis].filter(Boolean).join(' · ')}</p>{/if}
          {#if draft.estimate.note}<p class="dim">{draft.estimate.note}</p>{/if}
        </section>
        <p class="checks" data-testid="wf-opt-checks">
          <Icon name="check" size={12} />Checked: valid · step numbers that prompts mention are unchanged · every file a step reads is still written before it · outward steps keep their order.
          {#each draft.checks.warnings as w, i (i)}<span class="warn">{w}</span>{/each}
        </p>
        {#if !groups.length}
          <EmptyState variant="panel" icon="zap" title="No faster version found" body="Each step needs the one before it, and no step matches a pattern Otto can replace with a built-in step." />
        {/if}
        {#each groups as g (g.effect)}
          <h3>{g.label}<span class="count">{g.changes.length}</span></h3>
          <ul class="changes">
            {#each g.changes as c (c.id)}
              <li data-testid="wf-opt-change" data-change-id={c.id} data-rule={c.rule} data-effect={c.effect} data-preselected={c.preselected}>
                <label class="opt-row">
                  <input type="checkbox" checked={selected.includes(c.id)} onchange={() => toggle(c.id)} />
                  <span class="title" dir="auto">{c.title}</span>
                  {#if savingText(c.saving_ms)}<span class="saving">{savingText(c.saving_ms)}</span>{/if}
                </label>
                <p class="why" dir="auto">{c.why}</p>
                <span class="chips">{#each c.nodes as id (id)}<span class="chip">{names.get(id) ?? id}</span>{/each}</span>
              </li>
            {/each}
          </ul>
        {/each}
        {#if first.not_offered.length}
          <div data-testid="wf-opt-not-offered">
            <button type="button" class="fold" aria-expanded={notOfferedOpen} onclick={() => (notOfferedOpen = !notOfferedOpen)}>
              Not offered<span class="count">{first.not_offered.length}</span><Icon name={notOfferedOpen ? 'chevronDown' : 'chevronRight'} size={12} />
            </button>
            {#if notOfferedOpen}
              <ul class="changes">{#each first.not_offered as n, i (i)}<li><span class="title" dir="auto">{n.nodes.map((id) => names.get(id) ?? id).join(', ')}</span><p class="why" dir="auto">{n.reason}</p></li>{/each}</ul>
            {/if}
          </div>
        {/if}
      </div>

      <div class="main">
        {#if phone}
          <h3>After</h3>
          <ol class="outline" data-testid="wf-opt-canvas" data-view="after">
            {#each draft.draft_graph.nodes.filter((x) => !x.kind.endsWith('_trigger')) as n (n.id)}
              <li><span class="ellipsis" dir="auto">{n.name || n.id}</span>{#if marks[n.id]}<span class="mark">{marks[n.id]}</span>{/if}</li>
            {/each}
          </ol>
        {:else}
          <Tabs label="Before or after" tabs={[{ id: 'before', label: 'Before' }, { id: 'after', label: 'After' }]} value={view} onchange={(v) => (view = v)} idBase="wf-opt" />
          <div class="canvas" role="tabpanel" id="wf-opt-panel-{view}" aria-labelledby="wf-opt-tab-{view}" data-testid="wf-opt-canvas" data-view={view}>
            {#key view}
              <EditorCanvas graph={view === 'after' ? draft.draft_graph : wf.graph} readOnly marks={view === 'after' ? marks : {}} selectedId={null} selectedEdgeId={null}
                defaultProvider={defaultAgentProvider()} onselect={() => {}} onedgeselect={() => {}} onconnect={() => {}} ondeletenode={() => {}} ondeleteedge={() => {}} />
            {/key}
          </div>
        {/if}
        <h3>What each step reads and writes</h3>
        <div class="io-wrap">
          <table class="io" data-testid="wf-opt-io">
            <thead><tr><th scope="col">Step</th><th scope="col">Reads</th><th scope="col">Writes</th></tr></thead>
            <tbody>
              {#each draft.checks.files as f (f.node_id)}
                {@const rn = renamed.get(f.node_id)}
                <tr>
                  <th scope="row" dir="auto">{names.get(f.node_id) ?? f.node_id}{#if rn}<span class="dim"> · {rn.from} → {rn.to}</span>{/if}</th>
                  <td>{#each f.reads as r (r)}<span class="file" class:added={f.reads_added.includes(r)}>{r}</span>{/each}
                    {#each f.listing_removed as r (r)}<span class="dim">No longer listed: {r}</span>{/each}</td>
                  <td>{#each f.writes as w (w)}<span class="file">{w}</span>{/each}</td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  {/if}
</PageBody>

<style>
  .opt {
    display: flex;
    flex: 1;
    min-block-size: 0;
  }
  .side,
  .main {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    min-block-size: 0;
    overflow-y: auto;
    padding: var(--sp-6);
  }
  .side {
    flex: none;
    inline-size: 440px;
    border-inline-end: 1px solid var(--border);
  }
  .main {
    flex: 1;
    min-inline-size: 0;
  }
  .canvas {
    display: flex;
    flex: none;
    block-size: 420px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .opt-card {
    padding: var(--sp-5);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .big {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .dim,
  .why {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .checks {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .checks :global(svg) {
    vertical-align: -2px;
    margin-inline-end: var(--sp-2);
  }
  .warn {
    display: block;
    margin-block-start: var(--sp-2);
    color: var(--warning);
  }
  .banner {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--sp-4);
    margin: 0;
    padding: var(--sp-4) var(--sp-5);
    border-radius: var(--radius-m);
    background: var(--warning-soft);
    font-size: var(--fs-m);
  }
  h3 {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    margin: var(--sp-4) 0 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .count {
    font-weight: 400;
    color: var(--text-dim);
  }
  .changes,
  .outline {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .changes li,
  .outline li {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    padding-block: var(--sp-4);
    border-block-start: 1px solid var(--border);
  }
  .outline li {
    flex-direction: row;
    align-items: center;
    gap: var(--sp-4);
    min-block-size: 44px;
  }
  .opt-row {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    min-block-size: 32px;
    cursor: pointer;
  }
  .opt-row input {
    flex: none;
    inline-size: 16px;
    block-size: 16px;
  }
  .title {
    flex: 1;
    min-inline-size: 0;
    font-size: var(--fs-m);
  }
  .saving {
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
    color: var(--success);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
  }
  .mark {
    font-size: var(--fs-xs);
    color: var(--warning);
  }
  .fold {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-block-size: 32px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    font-weight: 600;
    cursor: pointer;
  }
  .io-wrap {
    overflow-x: auto;
  }
  .io {
    inline-size: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  .io th,
  .io td {
    padding: var(--sp-3) var(--sp-4);
    border-block-end: 1px solid var(--border);
    text-align: start;
    vertical-align: top;
  }
  .file {
    display: inline-block;
    margin-inline-end: var(--sp-3);
    font-family: var(--font-mono);
  }
  .file.added {
    background: var(--warning-soft);
  }
  @media (max-width: 1024px) {
    .side {
      inline-size: 360px;
    }
  }
  @media (max-width: 640px) {
    .opt {
      flex-direction: column;
      overflow-y: auto;
    }
    .side,
    .main {
      overflow: visible;
      padding: var(--sp-5);
    }
    .side {
      inline-size: auto;
      border-inline-end: 0;
    }
    .opt-row {
      min-block-size: 44px;
    }
    .opt-row input {
      inline-size: 20px;
      block-size: 20px;
    }
  }
</style>
