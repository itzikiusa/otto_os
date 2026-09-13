<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { GitRecoveryEntry, GitInteractivePlan, GitBisectState, RepoStatusResp, MergeResult } from '../../lib/api/types';
  import { git } from '../../lib/stores/git.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { gitBridge, type RecoveryMode } from './gitBridge.svelte';

  let { repoId, initialMode = 'history', initialOnto, onclose, onresolve }: {
    repoId: string; initialMode?: RecoveryMode; initialOnto?: string;
    onclose: () => void; onresolve: () => void;
  } = $props();
  // This component is mounted per repository. Requests capture that identity;
  // closing while one runs cannot apply its result to another repository.
  // svelte-ignore state_referenced_locally
  const id = repoId;
  let alive = true;
  onDestroy(() => { alive = false; });
  // svelte-ignore state_referenced_locally
  let mode = $state<RecoveryMode>(initialMode);
  let busy = $state(false);
  let error = $state('');
  let entries = $state<GitRecoveryEntry[]>([]);
  let more = $state(true);
  // svelte-ignore state_referenced_locally
  let onto = $state(initialOnto ?? '');
  let plan = $state<GitInteractivePlan | null>(null);
  let bisect = $state<GitBisectState | null>(null);
  let good = $state(gitBridge.bisectTargets[id]?.good ?? '');
  let bad = $state(gitBridge.bisectTargets[id]?.bad ?? 'HEAD');
  const op = $derived(git.statusById[id]?.op_in_progress);

  async function run(action: () => Promise<void>): Promise<void> {
    if (busy) return;
    busy = true; error = '';
    try { await action(); }
    catch (e) { if (alive) error = e instanceof Error ? e.message : String(e); }
    finally { if (alive) busy = false; }
  }
  async function loadHistory(reset = false): Promise<void> {
    const offset = reset ? 0 : entries.length;
    const rows = await api.get<GitRecoveryEntry[]>(`/repos/${id}/reflog?limit=50&skip=${offset}`);
    if (alive) { entries = reset ? rows : [...entries, ...rows]; more = rows.length === 50; }
  }
  async function refresh(): Promise<void> {
    await git.refreshStatus(id);
    const state = await api.get<GitBisectState>(`/repos/${id}/bisect`);
    if (alive) bisect = state;
  }
  onMount(() => { void run(async () => { await refresh(); await loadHistory(true); }); });

  async function recover(entry: GitRecoveryEntry): Promise<void> {
    const name = await confirmer.promptText(`Create a branch at ${entry.sha.slice(0, 10)}. Your current branch and files stay in place.`, {
      title: 'Recover a commit', placeholder: `recovery/${entry.sha.slice(0, 8)}`, confirmLabel: 'Create branch',
    });
    if (!name || !alive) return;
    await run(async () => {
      const status = await api.post<RepoStatusResp>(`/repos/${id}/branch`, { name, start_point: entry.sha, checkout: false });
      git.setStatus(id, status); toasts.success('Recovery branch created', name);
    });
  }
  function inspect(sha: string): void { gitBridge.focusCommit(id, sha); onclose(); }
  async function preview(): Promise<void> {
    await run(async () => {
      const result = await api.get<GitInteractivePlan>(`/repos/${id}/rebase/plan?onto=${encodeURIComponent(onto)}`);
      if (alive) plan = result;
    });
  }
  function move(index: number, delta: number): void {
    if (!plan || busy) return;
    const commits = [...plan.commits];
    const other = index + delta;
    if (other < 0 || other >= commits.length) return;
    [commits[index], commits[other]] = [commits[other], commits[index]];
    plan = { ...plan, commits };
  }
  async function startRebase(): Promise<void> {
    if (!plan) return;
    const snapshot = $state.snapshot(plan);
    const yes = await confirmer.ask(`Replay ${snapshot.commits.length} commits in the displayed order onto ${snapshot.onto_sha.slice(0, 10)}? This rewrites local history.`, {
      title: 'Start interactive rebase', confirmLabel: 'Start rebase', danger: true,
    });
    if (!yes || !alive) return;
    await run(async () => {
      const status = await api.post<RepoStatusResp>(`/repos/${id}/rebase/plan`, snapshot);
      git.setStatus(id, status); if (alive) plan = null;
      await refresh();
      if (alive && status.changes.some(c => c.kind === 'conflicted')) onresolve();
    });
  }
  async function rebaseAction(action: 'continue' | 'skip' | 'abort'): Promise<void> {
    if (action !== 'continue' && !await confirmer.ask(action === 'skip' ? 'Skip the current commit and discard all uncommitted changes in this worktree?' : 'Abort the rebase and restore the original branch?', {
      title: action === 'skip' ? 'Skip commit' : 'Abort rebase', confirmLabel: action === 'skip' ? 'Skip' : 'Abort', danger: true,
    })) return;
    await run(async () => {
      let status: RepoStatusResp;
      if (action === 'continue') {
        const result = await api.post<MergeResult>(`/repos/${id}/merge/commit`, {}); status = result.repo_status;
      } else { status = await api.post<RepoStatusResp>(`/repos/${id}/${action === 'skip' ? 'rebase/skip' : 'merge/abort'}`, {}); }
      git.setStatus(id, status); await refresh();
      if (alive && status.changes.some(c => c.kind === 'conflicted')) onresolve();
    });
  }
  async function bisectAction(action: 'start' | 'good' | 'bad' | 'skip' | 'reset'): Promise<void> {
    if (action === 'start' && !await confirmer.ask('Start bisect? Git will check out candidate commits for you to test, and End bisect returns to your current branch.', { title: 'Start bisect', confirmLabel: 'Start' })) return;
    await run(async () => {
      gitBridge.bisectTargets[id] = { good, bad };
      const state = await api.post<GitBisectState>(`/repos/${id}/bisect`, { op: action, good, bad, expected_head: bisect?.current_sha });
      if (alive) bisect = state;
      await git.refreshStatus(id);
    });
  }
</script>

<Modal title="Git recovery tools" width={820} {onclose}>
  <div class="tools">
    <nav aria-label="Recovery tools">
      {#each [{ id: 'history', label: 'Recovery history' }, { id: 'rebase', label: 'Interactive rebase' }, { id: 'bisect', label: 'Bisect' }] as tab}
        <button class="btn" class:primary={mode === tab.id} onclick={() => { mode = tab.id as RecoveryMode; }}>{tab.label}</button>
      {/each}
      <button class="btn" disabled={busy} onclick={() => run(refresh)}>Refresh</button>
    </nav>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    {#if busy}<p role="status">Working…</p>{/if}
    {#if mode === 'history'}
      <p>Browse previous HEAD positions. Recover creates a new branch without moving your current branch.</p>
      <div class="scroll">
        {#each entries as entry (entry.selector)}
          <div class="entry">
            <div><code>{entry.selector} · {entry.sha.slice(0, 10)}</code><p>{entry.subject}</p></div>
            <button class="btn small" onclick={() => inspect(entry.sha)}>Inspect</button>
            <button class="btn small" disabled={busy} onclick={() => recover(entry)}>Recover…</button>
          </div>
        {/each}
        {#if entries.length === 0 && !busy}<p>No recovery history is available.</p>{/if}
      </div>
      <button class="btn" disabled={busy || !more} onclick={() => run(() => loadHistory())}>Load more</button>
    {:else if mode === 'rebase'}
      {#if op === 'rebase'}
        <p>A rebase is in progress. At an Edit stop, use Changes to amend the current commit, then continue.</p>
        <div class="actions">
          <button class="btn primary" disabled={busy} onclick={() => rebaseAction('continue')}>Continue rebase</button>
          <button class="btn" disabled={busy} onclick={onresolve}>Open conflict resolver</button>
          <button class="btn" disabled={busy} onclick={() => rebaseAction('skip')}>Skip commit…</button>
          <button class="btn danger" disabled={busy} onclick={() => rebaseAction('abort')}>Abort…</button>
        </div>
      {:else}
        <p>Preview linear commits, reorder them, squash into the previous commit, or stop to edit. Commit or stash changes before starting.</p>
        <div class="actions"><label>Onto revision <input bind:value={onto} oninput={() => { plan = null; }} placeholder="main or a commit SHA" disabled={busy} /></label>
          <button class="btn" disabled={busy || !onto.trim()} onclick={preview}>Preview plan</button></div>
        {#if plan}
          <p>{plan.commits.length} commits · current {plan.head_sha.slice(0, 10)} → onto {plan.onto_sha.slice(0, 10)}</p>
          <div class="scroll">
            {#each plan.commits as commit, index (commit.sha)}
              <div class="entry">
                <span>{index + 1}</span>
                <select aria-label="Action for {commit.subject}" bind:value={commit.action} disabled={busy}>
                  <option value="pick">Pick</option><option value="edit">Edit</option><option value="squash" disabled={index === 0}>Squash</option>
                </select>
                <div><code>{commit.sha.slice(0, 10)}</code><p>{commit.subject}</p></div>
                <button class="btn small" aria-label="Move {commit.subject} up" disabled={busy || index === 0} onclick={() => move(index, -1)}>↑</button>
                <button class="btn small" aria-label="Move {commit.subject} down" disabled={busy || index === plan!.commits.length - 1} onclick={() => move(index, 1)}>↓</button>
              </div>
            {/each}
          </div>
          {#if plan.commits[0]?.action === 'squash'}<p class="error">The first commit must be Pick or Edit.</p>{/if}
          <button class="btn primary" disabled={busy || plan.commits.length === 0 || plan.commits[0]?.action === 'squash'} onclick={startRebase}>Start rebase…</button>
        {/if}
      {/if}
    {:else}
      {#if bisect?.active}
        <p>{bisect.finished ? 'First bad commit found' : 'Test the current candidate, then mark the result.'}</p>
        <div class="entry"><div><code>{bisect.first_bad ?? bisect.current_sha}</code><p>{bisect.current_subject}</p></div>
          <button class="btn" onclick={() => inspect(bisect!.first_bad ?? bisect!.current_sha)}>Inspect in graph</button></div>
        {#if bisect.remaining !== null && !bisect.finished}<p>{bisect.remaining} commits remain in the range.</p>{/if}
        <div class="actions">
          {#if !bisect.finished}
            <button class="btn primary" disabled={busy} onclick={() => bisectAction('good')}>Works (good)</button>
            <button class="btn danger" disabled={busy} onclick={() => bisectAction('bad')}>Broken (bad)</button>
            <button class="btn" disabled={busy} onclick={() => bisectAction('skip')}>Cannot test (skip)</button>
          {/if}
          <button class="btn" disabled={busy} onclick={() => bisectAction('reset')}>End bisect / return to branch</button>
        </div>
        {#if bisect.output}<pre>{bisect.output}</pre>{/if}
        <details><summary>Bisect history</summary><pre>{bisect.log}</pre></details>
      {:else}
        <p>Choose known good and bad revisions, or select them from a commit's graph menu. Progress survives closing Otto.</p>
        <label>Known good <input bind:value={good} placeholder="Commit SHA or revision" disabled={busy} /></label>
        <label>Known bad <input bind:value={bad} placeholder="HEAD" disabled={busy} /></label>
        <button class="btn primary" disabled={busy || !good.trim() || !bad.trim()} onclick={() => bisectAction('start')}>Start bisect…</button>
      {/if}
    {/if}
  </div>
</Modal>
<style>
  .tools { padding: 16px; display: flex; flex-direction: column; gap: 12px; min-width: 0; }
  nav, .actions { display: flex; flex-wrap: wrap; align-items: end; gap: 8px; }
  p { margin: 4px 0; overflow-wrap: anywhere; }
  label { display: flex; flex-direction: column; gap: 5px; flex: 1; }
  input, select { padding: 7px; color: var(--text); background: var(--surface); border: 1px solid var(--border); border-radius: 5px; min-width: 0; }
  .scroll { max-height: 45vh; overflow: auto; }
  .entry { display: flex; align-items: center; gap: 8px; padding: 10px 0; border-bottom: 1px solid var(--border); }
  .entry > div { flex: 1; min-width: 0; }
  code { font-size: 11px; overflow-wrap: anywhere; }
  pre { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 25vh; overflow: auto; }
  .error { color: var(--status-error, #d44); }
  @media (max-width: 600px) { .entry { flex-wrap: wrap; } .entry > div { flex-basis: 55%; } }
</style>
