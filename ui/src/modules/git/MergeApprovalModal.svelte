<script lang="ts">
  // Explicit merge approval. Opened by dropping one local branch onto another in
  // the graph's refs panel — NOTHING merges until the user clicks Merge here.
  // Shows source → target, a strategy picker, and a dirty-tree warning that
  // DISABLES merging when the working tree has uncommitted changes.
  import type {
    LocalMergeStrategy,
    MergeResult,
    RepoStatusResp,
  } from '../../lib/api/types';
  import { api, ApiError } from '../../lib/api/client';
  import { git } from '../../lib/stores/git.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    repoId: string;
    source: string;
    target: string;
    onclose: () => void;
    /** status 'merged' / 'up_to_date' — refresh + close handled by parent. */
    onmerged: (result: MergeResult) => void;
    /** status 'conflicts' — parent should open the conflict resolver. */
    onconflicts: (result: MergeResult) => void;
  }
  let { repoId, source, target, onclose, onmerged, onconflicts }: Props = $props();

  const STRATEGIES: { id: LocalMergeStrategy; label: string; hint: string }[] = [
    { id: 'merge_commit', label: 'Merge commit', hint: 'Always create a merge commit' },
    { id: 'ff', label: 'Fast-forward', hint: 'FF when possible, else a merge commit' },
    { id: 'ff_only', label: 'Fast-forward only', hint: 'Refuse if a merge commit is needed' },
    { id: 'squash', label: 'Squash', hint: 'Collapse source into one commit' },
  ];

  let strategy = $state<LocalMergeStrategy>('merge_commit');
  let merging = $state(false);
  let error = $state<string | null>(null);

  // Read the current repo status for the dirty-tree check. Seed from the store
  // (fast, avoids a flash) and always refresh from the daemon so we never merge
  // on a stale read.
  let status = $state<RepoStatusResp | null>(seedStatus());
  let statusLoading = $state(true);

  function seedStatus(): RepoStatusResp | null {
    return git.primary?.id === repoId ? git.primaryStatus : null;
  }

  $effect(() => {
    const id = repoId;
    statusLoading = true;
    void api
      .get<RepoStatusResp>(`/repos/${id}/status`)
      .then((s) => (status = s))
      .catch(() => {
        /* keep seeded status */
      })
      .finally(() => (statusLoading = false));
  });

  const dirty = $derived((status?.changes.length ?? 0) > 0);
  const canMerge = $derived(!merging && !dirty && !statusLoading);

  async function doMerge(): Promise<void> {
    if (!canMerge) return;
    merging = true;
    error = null;
    try {
      const result = await git.mergeBranch(repoId, { source, target, strategy });
      if (result.status === 'conflicts') {
        onconflicts(result);
        return;
      }
      // A squash merge reports status 'merged' but commit === null: the changes
      // are STAGED, not committed. Don't imply it's done — tell the user to
      // review & commit in Changes.
      if (result.status === 'merged' && result.commit === null) {
        toasts.success('Squash merge staged', 'Review & commit in Changes');
      } else {
        const verb = result.status === 'up_to_date' ? 'Already up to date' : 'Merged';
        toasts.success(verb, `${source} → ${target}`);
      }
      onmerged(result);
    } catch (e) {
      error = e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
    } finally {
      merging = false;
    }
  }
</script>

<Modal title="Merge branch" width={480} {onclose}>
  <div class="body">
    <!-- source → target -->
    <div class="flow">
      <span class="bchip src" title={source}>
        <Icon name="branch" size={12} />
        <span class="mono">{source}</span>
      </span>
      <span class="arrow"><Icon name="arrowDown" size={14} /></span>
      <span class="bchip tgt" title={target}>
        <Icon name="branch" size={12} />
        <span class="mono">{target}</span>
      </span>
    </div>

    <p class="note">
      Otto will switch to <span class="mono em">{target}</span>, then merge
      <span class="mono em">{source}</span>.
    </p>

    <!-- strategy -->
    <div class="section-label">Strategy</div>
    <div class="strats">
      {#each STRATEGIES as s (s.id)}
        <label class="strat" class:active={strategy === s.id}>
          <input type="radio" name="strategy" value={s.id} bind:group={strategy} />
          <span class="strat-main">
            <span class="strat-label">{s.label}</span>
            <span class="strat-hint dim">{s.hint}</span>
          </span>
        </label>
      {/each}
    </div>

    <!-- dirty-tree warning -->
    {#if dirty}
      <div class="warn">
        <Icon name="info" size={14} />
        <span>
          Working tree has {status?.changes.length} uncommitted change{status && status.changes.length === 1 ? '' : 's'}.
          Commit or stash them before merging.
        </span>
      </div>
    {/if}

    {#if error}
      <div class="err">
        <Icon name="x" size={13} />
        <span>{error}</span>
      </div>
    {/if}
  </div>

  {#snippet footer()}
    <button class="btn ghost" onclick={onclose} disabled={merging}>Cancel</button>
    <button class="btn primary" onclick={doMerge} disabled={!canMerge}>
      {merging ? 'Merging…' : 'Merge'}
    </button>
  {/snippet}
</Modal>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .flow {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 6px 0 2px;
  }
  .bchip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    padding: 4px 10px;
    border-radius: var(--radius-s);
    font-size: 12.5px;
    font-weight: 500;
  }
  .bchip .mono {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bchip.src {
    background: color-mix(in srgb, var(--status-working) 14%, transparent);
    color: var(--status-working);
  }
  .bchip.tgt {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .arrow {
    color: var(--text-dim);
    display: inline-flex;
  }
  .note {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
    text-align: center;
    line-height: 1.5;
  }
  .em {
    color: var(--text);
    font-weight: 600;
  }
  .section-label {
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    text-transform: uppercase;
  }
  .strats {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .strat {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    cursor: pointer;
    transition: border-color 110ms, background 110ms;
  }
  .strat:hover {
    background: var(--surface-2);
  }
  .strat.active {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .strat input {
    margin-top: 2px;
    accent-color: var(--accent);
    flex-shrink: 0;
  }
  .strat-main {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .strat-label {
    font-size: 12.5px;
    font-weight: 500;
  }
  .strat-hint {
    font-size: 11px;
  }
  .warn {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: color-mix(in srgb, #febc2e 14%, transparent);
    color: #b8860b;
    font-size: 11.5px;
    line-height: 1.45;
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
    color: var(--status-exited);
    font-size: 11.5px;
    line-height: 1.45;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
