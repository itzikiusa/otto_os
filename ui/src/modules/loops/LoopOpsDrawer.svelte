<script lang="ts">
  // Goal Loop operator controls (E-01):
  //   1. Mid-loop steering ("Steer next iteration") via POST /goal-loops/{id}/steer
  //   2. Collapsible "Changes so far" worktree diff via GET /goal-loops/{id}/diff
  import { api } from '../../lib/api/client';
  import { copyWithToast } from '../../lib/clipboard';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import type { GoalLoopStatus } from '../../lib/api/types';

  interface SteerResp {
    loop_id: string;
    note: string;
    queued_notes: string[];
  }
  interface GoalLoopDiffResp {
    loop_id: string;
    available: boolean;
    branch: string | null;
    stat: string;
    patch: string;
    truncated: boolean;
    untracked: string[];
  }

  interface Props {
    loopId: string;
    status: GoalLoopStatus;
    hasWorktree: boolean;
    queuedNotes?: readonly string[];
  }
  let { loopId, status, hasWorktree, queuedNotes = [] }: Props = $props();

  const terminal = $derived(status === 'succeeded' || status === 'failed' || status === 'stopped');

  // ---- Mid-loop steering ----
  let note = $state('');
  let sending = $state(false);
  let localQueued = $state<string[] | null>(null);
  const shownNotes = $derived(localQueued ?? [...queuedNotes]);

  $effect(() => {
    void loopId;
    void queuedNotes;
    localQueued = null;
  });

  async function queueSteering(): Promise<void> {
    const trimmed = note.trim();
    if (!trimmed || sending || terminal) return;
    const id = loopId;
    sending = true;
    try {
      const resp = await api.post<SteerResp>(`/goal-loops/${encodeURIComponent(id)}/steer`, { note: trimmed });
      if (loopId !== id) return;
      note = '';
      localQueued = resp.queued_notes ?? [...shownNotes, trimmed];
      toasts.success('Steering note queued', 'Planner and executors will read it on the next iteration.');
    } catch (e) {
      if (loopId === id) toastError('Couldn’t queue steering note', e);
    } finally {
      if (loopId === id) sending = false;
    }
  }

  // ---- Worktree diff ("Changes so far") ----
  let diffOpen = $state(false);
  let diffLoading = $state(false);
  let diffError = $state<string | null>(null);
  let diffData = $state<GoalLoopDiffResp | null>(null);
  let diffSeq = 0;

  async function loadDiff(): Promise<void> {
    const seq = ++diffSeq;
    const id = loopId;
    diffLoading = true;
    diffError = null;
    try {
      const resp = await api.get<GoalLoopDiffResp>(`/goal-loops/${encodeURIComponent(id)}/diff`);
      if (seq !== diffSeq || loopId !== id) return;
      diffData = resp;
    } catch (e) {
      if (seq !== diffSeq || loopId !== id) return;
      diffError = loadErrorText(e);
    } finally {
      if (seq === diffSeq && loopId === id) diffLoading = false;
    }
  }

  function toggleDiff(): void {
    diffOpen = !diffOpen;
    if (diffOpen) void loadDiff();
  }

  function lineKind(line: string): 'add' | 'del' | 'hunk' | 'head' | 'ctx' {
    if (line.startsWith('diff --git') || line.startsWith('+++ ') || line.startsWith('--- ')) return 'head';
    if (line.startsWith('@@')) return 'hunk';
    if (line.startsWith('+')) return 'add';
    if (line.startsWith('-')) return 'del';
    return 'ctx';
  }
</script>

{#if !terminal || hasWorktree}
  <section class="ops card-box" aria-label="Loop steering and worktree changes" data-testid="loop-ops-drawer">
    {#if !terminal}
      <div class="steer">
        <div class="ops-head">
          <Icon name="send" size={12} />
          <h3>Steer next iteration</h3>
          <span class="dim">Consumed automatically by the next iteration without pausing.</span>
        </div>
        <form class="steer-row" onsubmit={(e) => { e.preventDefault(); void queueSteering(); }}>
          <input
            class="input steer-in"
            dir="auto"
            maxlength="4000"
            bind:value={note}
            disabled={sending}
            aria-label="Steering note for next iteration"
            placeholder="e.g. Focus on edge cases in parser.rs and skip docs for now"
            data-testid="loop-steer-input"
          />
          <button type="submit" class="btn small" disabled={sending || !note.trim()} data-testid="loop-steer-submit">
            {sending ? 'Queuing…' : 'Queue note'}
          </button>
        </form>
        {#if shownNotes.length > 0}
          <ul class="steer-list" aria-label="Queued steering notes" data-testid="loop-steer-queued">
            {#each shownNotes as item, i (`${i}:${item}`)}
              <li><span class="steer-badge">Queued</span> <span dir="auto">{item}</span></li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}

    {#if hasWorktree}
      <div class="diff-wrap">
        <div class="diff-bar">
          <button type="button" class="diff-toggle" aria-expanded={diffOpen} onclick={toggleDiff} data-testid="loop-diff-toggle">
            <Icon name={diffOpen ? 'chevronDown' : 'chevronRight'} size={12} />
            <span>Changes so far</span>
            {#if diffData?.branch}<span class="mono dim">({diffData.branch})</span>{/if}
          </button>
          {#if diffOpen}
            <span class="grow"></span>
            {#if diffData?.patch}
              <button type="button" class="btn small ghost" onclick={() => void copyWithToast(diffData?.patch ?? '', 'worktree diff')}>
                <Icon name="copy" size={12} /> Copy diff
              </button>
            {/if}
            <button type="button" class="btn small" disabled={diffLoading} onclick={() => void loadDiff()}>
              <Icon name="refresh" size={12} /> {diffLoading ? 'Refreshing…' : 'Refresh'}
            </button>
          {/if}
        </div>
        {#if diffOpen}
          <LoadState
            what="worktree diff"
            variant="compact"
            loading={diffLoading && !diffData}
            error={diffError}
            empty={!diffData || (!diffData.available && !diffData.patch && diffData.untracked.length === 0)}
            rows={4}
            onretry={() => void loadDiff()}
          >
            {#snippet emptyView()}
              <p class="dim empty-diff">No worktree changes recorded yet.</p>
            {/snippet}
            {#if diffData}
              {#if diffData.stat.trim()}
                <pre class="diff-stat" dir="ltr">{diffData.stat.trim()}</pre>
              {/if}
              {#if diffData.untracked.length > 0}
                <p class="dim small-note">Untracked files: <span class="mono">{diffData.untracked.join(', ')}</span></p>
              {/if}
              {#if diffData.truncated}
                <p class="warn-note"><Icon name="warning" size={12} /> Diff output was truncated to keep the view responsive.</p>
              {/if}
              {#if diffData.patch.trim()}
                <pre class="diff-patch" dir="ltr" data-testid="loop-diff-patch">{#each diffData.patch.split('\n') as line, i (i)}<span class="dl dl-{lineKind(line)}">{line}
</span>{/each}</pre>
              {:else if diffData.untracked.length === 0}
                <p class="dim empty-diff">Working tree matches its base commit.</p>
              {/if}
            {/if}
          </LoadState>
        {/if}
      </div>
    {/if}
  </section>
{/if}

<style>
  .ops {
    display: flex;
    flex-direction: column;
    gap: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 12px 14px;
  }
  .ops-head {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    margin-block-end: 6px;
    color: var(--text-dim);
  }
  .ops-head h3 {
    margin: 0;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .dim { color: var(--text-dim); font-size: var(--fs-xs); }
  .steer-row { display: flex; gap: 8px; align-items: center; }
  .steer-in { flex: 1; min-width: 0; }
  .steer-list {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-s);
  }
  .steer-badge {
    display: inline-block;
    padding: 0 6px;
    border-radius: var(--radius-s);
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .diff-wrap {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .diff-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .diff-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
  }
  .grow { flex: 1; }
  .mono { font-family: var(--font-mono); }
  .empty-diff, .small-note { margin: 2px 0; }
  .warn-note {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--warning);
  }
  .diff-stat {
    margin: 0;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    overflow-x: auto;
  }
  .diff-patch {
    margin: 0;
    padding: 8px;
    max-height: 360px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.45;
  }
  .dl { display: block; white-space: pre; }
  .dl-add { color: var(--success); background: var(--success-soft); }
  .dl-del { color: var(--danger); background: var(--danger-soft); }
  .dl-hunk { color: var(--info); }
  .dl-head { font-weight: 600; color: var(--text); }
</style>
