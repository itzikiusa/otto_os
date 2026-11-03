<script lang="ts">
  import { untrack } from 'svelte';
  // Bulk branch delete: tick many local branches (optionally with their
  // upstream copies) or remote branches and delete them in one go. The
  // checked-out branch, the default branch (main / master / the repo's base)
  // and branches held by another worktree can't be ticked. Unmerged locals get
  // their own force warning; any remote deletion confirms the exact
  // `remote: branch` list first (outward-facing). Results are reported per
  // branch, inline and in a toast.
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Tabs from '../../lib/components/Tabs.svelte';
  import { api } from '../../lib/api/client';
  import type { BranchDeleteItem, BranchDeleteResp, BranchDeleteResult, RefBranch, RefsResp, RepoStatusResp } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { confirmOutward } from '../../lib/confirmOutward';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { plural } from '../../lib/plural';
  import {
    describeRemoteTargets,
    isUnmerged,
    planBranchDelete,
    protectionReason,
    resultLine,
    summarizeDelete,
    type ProtectContext,
  } from './gitopsPlan';

  interface Props {
    repoId: string;
    refs: RefsResp;
    currentBranch: string | null;
    /** Branches checked out in another worktree. */
    inWorktree: Set<string>;
    /** Pre-ticked branch names (local names, or `remote/branch` for remote refs). */
    preselect?: string[];
    onclose: () => void;
    ondone: (status: RepoStatusResp) => void;
  }
  let { repoId, refs, currentBranch, inWorktree, preselect = [], onclose, ondone }: Props = $props();

  type Tab = 'local' | 'remote';
  let tab = $state<Tab>('local');
  let filter = $state('');
  let alsoRemote = $state(false);
  let picked = $state(untrack(() => new Set<string>(preselect)));
  let busy = $state(false);
  let results = $state<BranchDeleteResult[] | null>(null);
  /** The items last sent, by name — a "force" follow-up re-sends them. */
  let lastItems = new Map<string, BranchDeleteItem>();

  const ctx = $derived<ProtectContext>({ current: currentBranch, base: refs.base_branch ?? null, inWorktree });
  const rows = $derived(tab === 'local' ? refs.local : refs.remote);
  const visible = $derived(
    rows.filter((b) => !filter.trim() || b.name.toLowerCase().includes(filter.trim().toLowerCase())),
  );
  const selectable = $derived(visible.filter((b) => protectionReason(b, ctx) === null));
  const allBranches = $derived([...refs.local, ...refs.remote]);
  const selected = $derived(allBranches.filter((b) => picked.has(b.name) && protectionReason(b, ctx) === null));
  const selLocal = $derived(selected.filter((b) => !b.remote));
  const selRemote = $derived(selected.filter((b) => b.remote));
  const withUpstream = $derived(selLocal.filter((b) => !!b.upstream).length);
  const allVisiblePicked = $derived(selectable.length > 0 && selectable.every((b) => picked.has(b.name)));

  function toggle(name: string): void {
    const next = new Set(picked);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    picked = next;
  }
  function toggleAllVisible(): void {
    const next = new Set(picked);
    for (const b of selectable) {
      if (allVisiblePicked) next.delete(b.name);
      else next.add(b.name);
    }
    picked = next;
  }
  function selectMerged(): void {
    const next = new Set(picked);
    for (const b of selectable) if (b.merged_into_base || (b.merged_into?.length ?? 0) > 0) next.add(b.name);
    picked = next;
  }

  async function send(items: BranchDeleteItem[]): Promise<BranchDeleteResult[] | null> {
    busy = true;
    try {
      const resp = await api.post<BranchDeleteResp>(`/repos/${repoId}/branches/delete`, { items });
      ondone(resp.status);
      for (const i of items) lastItems.set(i.name, i);
      return resp.results;
    } catch (e) {
      toastError('Couldn’t delete the branches', e);
      return null;
    } finally {
      busy = false;
    }
  }

  function report(rs: BranchDeleteResult[]): void {
    const s = summarizeDelete(rs);
    if (s.failed.length === 0) toasts.success(`Deleted ${plural(s.okCount, 'branch', 'branches')}`);
    else if (s.okCount > 0) toasts.warn(`Deleted ${s.okCount} of ${rs.length}`, `${plural(s.failed.length, 'branch', 'branches')} failed — see the list`);
    else toasts.error('No branches deleted', `${plural(s.failed.length, 'branch', 'branches')} failed — see the list`);
  }

  async function deleteSelected(): Promise<void> {
    if (selected.length === 0) return;
    const unmerged = selLocal.filter(isUnmerged).map((b) => b.name);
    let forceUnmerged = false;
    if (unmerged.length > 0) {
      forceUnmerged = await confirmer.ask(
        `${plural(unmerged.length, 'branch', 'branches')} ${unmerged.length === 1 ? 'has' : 'have'} commits that aren’t in ${refs.base_branch ?? 'the default branch'} or any integration branch:\n\n${unmerged.map((n) => `• ${n}`).join('\n')}\n\nForce-deleting drops those commits from this repo. This cannot be undone.`,
        { title: 'Force-delete unmerged branches', confirmLabel: `Force delete ${unmerged.length}`, danger: true },
      );
      if (!forceUnmerged) return;
    }
    const plan = planBranchDelete(selected, { alsoRemote, forceUnmerged });
    const remoteCount = [...plan.remoteTargets.values()].reduce((n, l) => n + l.length, 0);
    if (remoteCount > 0) {
      const ok = await confirmOutward({
        verb: `Delete ${plural(remoteCount, 'remote branch', 'remote branches')}`,
        where: describeRemoteTargets(plan.remoteTargets),
        what: selLocal.length > 0 ? `Also deletes ${plural(selLocal.length, 'local branch', 'local branches')}: ${selLocal.map((b) => b.name).join(', ')}` : undefined,
        who: 'Everyone with access to the remote loses these branches; open pull requests from them may close.',
        danger: true,
      });
      if (!ok) return;
    } else if (unmerged.length === 0) {
      const names = selLocal.map((b) => b.name);
      const ok = await confirmer.ask(
        `Delete ${plural(names.length, 'local branch', 'local branches')}?\n\n${names.map((n) => `• ${n}`).join('\n')}`,
        { title: 'Delete branches', confirmLabel: `Delete ${names.length}`, danger: true },
      );
      if (!ok) return;
    }
    const rs = await send(plan.items);
    if (!rs) return;
    results = rs;
    report(rs);
  }

  async function forceRetry(): Promise<void> {
    if (!results) return;
    const names = summarizeDelete(results).needForce;
    if (names.length === 0) return;
    const ok = await confirmer.ask(
      `Git refused to delete ${names.length === 1 ? 'this branch' : 'these branches'} because ${names.length === 1 ? 'it isn’t' : 'they aren’t'} fully merged:\n\n${names.map((n) => `• ${n}`).join('\n')}\n\nForce-delete and drop the unmerged commits? This cannot be undone.`,
      { title: 'Force-delete unmerged branches', confirmLabel: `Force delete ${names.length}`, danger: true },
    );
    if (!ok) return;
    const items = names.map((n) => ({ ...(lastItems.get(n) ?? { name: n }), force: true }));
    const rs = await send(items);
    if (!rs) return;
    const byName = new Map(rs.map((r) => [r.name, r]));
    results = results.map((r) => byName.get(r.name) ?? r);
    report(rs);
  }

  function badge(b: RefBranch): string {
    if (b.merged_into_base || (b.merged_into?.length ?? 0) > 0) {
      return `in ${(b.merged_into?.length ? b.merged_into : [refs.base_branch ?? 'base']).join(', ')}`;
    }
    return b.remote ? '' : 'unmerged';
  }
</script>

<Modal title="Delete branches" width={620} height={640} {onclose} dismissable={!busy}>
  {#if results}
    <div class="results" role="list" aria-label="Delete results">
      {#each results as r (r.name + (r.remote ?? ''))}
        <div class="res" role="listitem" class:fail={!!r.error}>
          <Icon name={r.error ? 'warning' : 'check'} size={12} />
          <span class="mono name">{r.local_deleted || !r.remote ? r.name : `${r.remote}/${r.name}`}</span>
          <span class="dim line">{resultLine(r)}</span>
        </div>
      {/each}
    </div>
  {:else}
    <div class="body">
      <div class="bar">
        <Tabs
          label="Branch kind"
          size="s"
          tabs={[{ id: 'local', label: 'Local', count: refs.local.length }, { id: 'remote', label: 'Remote', count: refs.remote.length }]}
          value={tab}
          onchange={(id) => (tab = id as Tab)}
        />
        <input class="filter" dir="ltr" type="search" placeholder="Filter branches" aria-label="Filter branches" bind:value={filter} />
      </div>
      <div class="bar small">
        <label class="chk"><input type="checkbox" checked={allVisiblePicked} disabled={selectable.length === 0} onchange={toggleAllVisible} /> Select all shown</label>
        <button class="btn small ghost" onclick={selectMerged} disabled={selectable.length === 0} title="Tick every shown branch already contained in an integration branch">Select merged</button>
      </div>
      <div class="list" role="list">
        {#each visible as b (b.name)}
          {@const why = protectionReason(b, ctx)}
          {@const mark = badge(b)}
          <label class="bc-row" class:locked={!!why} role="listitem" title={why ? `${b.name} — ${why}, can’t be deleted here` : b.name}>
            <input type="checkbox" checked={!why && picked.has(b.name)} disabled={!!why || busy} onchange={() => toggle(b.name)} aria-label={`Select ${b.name}`} />
            <span class="mono name">{b.name}</span>
            {#if why}
              <span class="tag dim"><Icon name="lock" size={12} />{why}</span>
            {:else if mark}
              <span class="tag" class:warn={mark === 'unmerged'} class:ok={mark !== 'unmerged'}>{mark}</span>
            {/if}
            {#if !b.remote && ((b.ahead ?? 0) > 0 || (b.behind ?? 0) > 0)}
              <span class="tag dim">{#if (b.ahead ?? 0) > 0}↑{b.ahead}{/if}{#if (b.behind ?? 0) > 0} ↓{b.behind}{/if}</span>
            {/if}
            {#if !b.remote && b.upstream}<span class="tag dim mono">{b.upstream}</span>{/if}
          </label>
        {:else}
          <EmptyState icon="branch" title={filter ? 'No branches match' : tab === 'local' ? 'No local branches' : 'No remote branches'} variant="panel" headingLevel={4} />
        {/each}
      </div>
      {#if selLocal.length > 0}
        <label class="chk opt">
          <input type="checkbox" bind:checked={alsoRemote} disabled={withUpstream === 0 || busy} />
          Also delete their remote branches ({withUpstream} of {selLocal.length} have an upstream)
        </label>
      {/if}
    </div>
  {/if}

  {#snippet footer()}
    {#if results}
      {@const needForce = summarizeDelete(results).needForce}
      {#if needForce.length > 0}
        <button class="btn danger" disabled={busy} onclick={forceRetry}>Force delete {needForce.length} unmerged…</button>
      {/if}
      <button class="btn primary" disabled={busy} onclick={onclose}>Done</button>
    {:else}
      <span class="dim summary">
        {plural(selLocal.length, 'local')}{selRemote.length ? ` · ${plural(selRemote.length, 'remote')}` : ''}{alsoRemote && withUpstream ? ` · ${withUpstream} upstream` : ''}
      </span>
      <button class="btn ghost" onclick={onclose} disabled={busy}>Cancel</button>
      <button class="btn danger" disabled={busy || selected.length === 0} onclick={deleteSelected}>
        {busy ? 'Deleting…' : `Delete ${plural(selected.length, 'branch', 'branches')}…`}
      </button>
    {/if}
  {/snippet}
</Modal>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    min-height: 0;
    height: 100%;
  }
  .bar {
    display: flex;
    gap: var(--sp-2);
    align-items: center;
    flex-wrap: wrap;
  }
  .bar.small {
    font-size: var(--fs-xs);
    justify-content: space-between;
  }
  .filter {
    flex: 1;
    min-width: 140px;
  }
  .list {
    flex: 1;
    min-height: 160px;
    max-height: 50vh;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .bc-row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    padding: var(--sp-1) var(--sp-2);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .bc-row:hover {
    background: var(--hover);
  }
  .bc-row.locked {
    cursor: default;
    color: var(--text-dim);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
    font-size: var(--fs-xs);
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag.warn {
    color: var(--warning);
  }
  .tag.ok {
    color: var(--success);
  }
  .chk {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    font-size: var(--fs-xs);
  }
  .opt {
    font-size: var(--fs-s);
  }
  .summary {
    margin-inline-end: auto;
    font-size: var(--fs-xs);
  }
  .results {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 60vh;
    overflow-y: auto;
  }
  .res {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--fs-s);
    color: var(--success);
    padding: var(--sp-1) 0;
  }
  .res.fail {
    color: var(--danger);
  }
  .res .name {
    color: var(--text);
    flex: 0 1 auto;
  }
  .res .line {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
</style>
