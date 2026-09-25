<script lang="ts">
  // Toolbar row: Fetch / Pull / Push / Branch / Stash / Pop + current branch chip.
  import { git } from '../../lib/stores/git.svelte';
  import { api } from '../../lib/api/client';
  import type { PullMode, PullModeResp, RepoStatusResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { runPull } from './pullFlow';

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    /** Called after an op that may have MOVED refs/history (fetch/pull/push/
     *  branch) so the parent can re-mount the graph — otherwise the commit graph
     *  shows stale data until the repo tab is reopened. */
    onrefresh?: () => void;
  }
  let { repoId, status, onstatus, onrefresh }: Props = $props();

  let busy = $state('');

  async function doFetch(): Promise<void> {
    busy = 'fetch';
    try {
      const s = await git.fetchRepo(repoId);
      onstatus(s); // Store refsRev quietly refreshes the graph without a remount.
      // Say what the fetch found, not which remote we assume it hit.
      toasts.success(
        'Fetched',
        s.behind > 0
          ? `${s.behind} new commit${s.behind === 1 ? '' : 's'} on ${s.upstream ?? 'the upstream'} — pull to bring them in`
          : s.upstream
            ? `${s.branch} is up to date with ${s.upstream}`
            : 'Remote branches and tags refreshed',
      );
    } catch (e) {
      toasts.error('Fetch failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  // The repo's EFFECTIVE pull mode (its `pull.rebase`/`pull.ff` config), loaded
  // once per repo so the button says what it will actually do. A failure here is
  // cosmetic — fall back to the git default.
  let pullMode = $state<PullMode>('merge');
  let pullModeFor = '';
  $effect(() => {
    const id = repoId;
    if (pullModeFor === id) return;
    pullModeFor = id;
    pullMode = 'merge';
    void api
      .get<PullModeResp>(`/repos/${id}/pull-mode`)
      .then((r) => {
        if (pullModeFor === id) pullMode = r.mode;
      })
      .catch(() => {});
  });

  const MODE_LABEL: Record<PullMode, string> = {
    merge: 'merge',
    rebase: 'rebase',
    ff_only: 'ff-only',
  };

  async function doPull(mode?: PullMode): Promise<void> {
    busy = 'pull';
    try {
      await runPull(repoId, onstatus, { mode });
      onrefresh?.();
    } finally {
      busy = '';
    }
  }

  function pullMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Pull (merge)', icon: 'arrowDown', action: () => void doPull('merge') },
      { label: 'Pull (rebase)', icon: 'arrowDown', action: () => void doPull('rebase') },
      { label: 'Pull (fast-forward only)', icon: 'arrowDown', action: () => void doPull('ff_only') },
    ]);
  }

  async function doPush(): Promise<void> {
    // No confirm: push is the most routine git action (the user asked for
    // fewer nag dialogs, and no git client asks before a plain push). The
    // button label already says Push vs Publish, and the toast reports it.
    busy = 'push';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/push`, {});
      onstatus(s);
      onrefresh?.();
      toasts.success(status.upstream ? 'Pushed' : 'Branch published', s.upstream ?? status.branch);
    } catch (e) {
      toasts.error('Push failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  // Same sheet the graph's "Create branch here…" uses. (This used to be an
  // absolutely-positioned inline popover, which the ≤1024px toolbar's
  // horizontal scroller clipped — the input was unreachable on a tablet.)
  async function doCreateBranch(): Promise<void> {
    const raw = await confirmer.promptText(`New branch from ${status.branch}`, {
      title: 'Create branch',
      confirmLabel: 'Create',
      placeholder: 'feature/my-branch',
    });
    const name = raw?.trim() ?? '';
    if (!name) return;
    busy = 'branch';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/checkout`, { branch: name, create: true });
      onstatus(s);
      onrefresh?.();
      toasts.success('Branch created', name);
    } catch (e) {
      toasts.error('Branch failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function doStash(): Promise<void> {
    busy = 'stash';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/stash`, { op: 'save' });
      onstatus(s);
      // Re-mount the graph: its STASHES section + stash node only reload then.
      onrefresh?.();
      toasts.success('Stashed');
    } catch (e) {
      toasts.error('Stash failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function doPop(): Promise<void> {
    busy = 'pop';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/stash`, { op: 'pop' });
      onstatus(s);
      onrefresh?.();
      // A conflicting pop is a 200 with unmerged paths (git keeps the stash
      // entry) — guide the user into the resolver instead of claiming success.
      const conflicts = s.changes.filter((c) => c.kind === 'conflicted').length;
      if (conflicts > 0) {
        toasts.warn(
          'Stash popped with conflicts',
          `${conflicts} file${conflicts === 1 ? '' : 's'} need resolution — open "Resolve conflicts". The stash entry was kept.`,
        );
      } else {
        toasts.success('Stash popped');
      }
    } catch (e) {
      toasts.error('Pop failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }
</script>

<div class="toolbar">
  <!-- Branch chip -->
  <span
    class="branch-chip"
    title="{status.branch}{status.upstream ? ` · tracks ${status.upstream}` : ''}{status.ahead > 0 ? ` · ${status.ahead} ahead` : ''}{status.behind > 0 ? ` · ${status.behind} behind` : ''}"
  >
    <Icon name="branch" size={12} />
    <span class="mono branch-name">{status.branch}</span>
    {#if status.ahead > 0}<span class="ab up" aria-label="{status.ahead} ahead">↑{status.ahead}</span>{/if}
    {#if status.behind > 0}<span class="ab down" aria-label="{status.behind} behind">↓{status.behind}</span>{/if}
  </span>

  <span class="divider"></span>

  <!-- Fetch -->
  <button class="btn ghost tbtn" disabled={busy !== ''} onclick={doFetch} title="Fetch from remote">
    <Icon name="fetch" size={14} />
    {busy === 'fetch' ? 'Fetching…' : 'Fetch'}
  </button>

  <!-- Pull (split button: the repo's configured mode, ▾ overrides it once) -->
  <span class="split">
    <button
      class="btn ghost tbtn"
      disabled={busy !== ''}
      onclick={() => void doPull()}
      title="Pull from upstream using the repo's configured mode"
    >
      <Icon name="arrowDown" size={14} />
      {busy === 'pull' ? 'Pulling…' : `Pull (${MODE_LABEL[pullMode]})`}
    </button>
    <button
      class="btn ghost tbtn caret"
      disabled={busy !== ''}
      onclick={pullMenu}
      title="Pull with a different mode"
      aria-label="Pull options"
    ><Icon name="chevronDown" size={12} /></button>
  </span>

  <!-- Push -->
  <button
    class="btn ghost tbtn"
    disabled={busy !== ''}
    onclick={() => void doPush()}
    title={status.upstream
      ? status.ahead > 0
        ? `Push ${status.ahead} commit${status.ahead === 1 ? '' : 's'} to ${status.upstream}`
        : `Nothing to push — ${status.branch} matches ${status.upstream}`
      : `Publish ${status.branch} to origin`}
  >
    <Icon name="arrowUp" size={14} />
    {busy === 'push' ? 'Pushing…' : status.upstream ? 'Push' : 'Publish'}
  </button>

  <span class="divider"></span>

  <!-- Branch (create) -->
  <button class="btn ghost tbtn" disabled={busy !== ''} onclick={() => void doCreateBranch()} title="Create a new branch from {status.branch} and switch to it">
    <Icon name="plus" size={14} />
    {busy === 'branch' ? 'Creating…' : 'Branch'}
  </button>

  <span class="divider"></span>

  <!-- Stash -->
  <button
    class="btn ghost tbtn"
    disabled={busy !== '' || status.changes.length === 0}
    onclick={doStash}
    title={status.changes.length === 0 ? 'Nothing to stash — the working tree is clean' : 'Stash working changes (including untracked files)'}
  >
    <Icon name="stash" size={14} />
    {busy === 'stash' ? 'Stashing…' : 'Stash'}
  </button>

  <!-- Pop -->
  <button class="btn ghost tbtn" disabled={busy !== ''} onclick={doPop} title="Apply the latest stash and drop it">
    <Icon name="archive" size={14} />
    {busy === 'pop' ? 'Popping…' : 'Pop'}
  </button>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-wrap: nowrap;
  }
  /* Shrinks (never below its icon + counts) so a long branch name ellipsizes
     instead of shoving Fetch…Pop out of the row; the full name is the title. */
  .branch-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    max-width: 260px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent-text);
    border-radius: var(--radius-s);
    padding: 2px 8px;
    font-size: var(--fs-s);
    font-weight: 500;
    flex-shrink: 1;
  }
  .branch-chip > :global(svg),
  .branch-chip .ab {
    flex-shrink: 0;
  }
  .branch-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ab {
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .ab.up { color: var(--accent-text); }
  .ab.down { color: var(--warning); }
  .divider {
    display: inline-block;
    width: 1px;
    height: 16px;
    background: var(--border);
    margin: 0 4px;
    flex-shrink: 0;
  }
  /* Toolbar buttons are global .btn.ghost; only the quieter label tone and
     the tighter toolbar padding are local. */
  .tbtn {
    padding: 0 9px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .tbtn:hover:not(:disabled) {
    color: var(--text);
  }
  /* Pull split button: one visual unit — the halves share a square seam with a
     hairline between them, and hovering either half outlines both. */
  .split {
    display: inline-flex;
    align-items: center;
  }
  .split .tbtn:first-child {
    padding-inline-end: 7px;
    border-start-end-radius: 0;
    border-end-end-radius: 0;
  }
  .split .caret {
    padding: 0 5px;
    border-start-start-radius: 0;
    border-end-start-radius: 0;
    border-inline-start-color: color-mix(in srgb, var(--border) 70%, transparent);
  }
  .split:hover .tbtn:not(:disabled) {
    border-color: var(--border);
  }

  /* ── Mobile + tablet (≤1024px): the toolbar already scrolls horizontally
     inside RepoView's header, so just give the buttons comfortable touch
     heights. ── */
  @media (max-width: 1024px) {
    .tbtn {
      height: 36px;
      padding: 0 11px;
      font-size: var(--fs-m);
    }
  }
</style>
