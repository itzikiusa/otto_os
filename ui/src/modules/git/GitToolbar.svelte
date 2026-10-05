<script lang="ts">
  import { plural } from '../../lib/plural';
  import { toastError } from '../../lib/toastError';
  // Toolbar row: Fetch / Pull / Push / Branch / Stash / Pop + current branch chip.
  // In the page header Fetch folds into the Pull split menu (≤5 header
  // controls) and the verbs are also ⌘K commands (group "Git").
  import { git } from '../../lib/stores/git.svelte';
  import { api } from '../../lib/api/client';
  import type { PullMode, PullModeResp, RepoStatusResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { runPull } from './pullFlow';
  import { runPush } from './pushFlow';
  import { gitBridge } from './gitBridge.svelte';
  import { registry } from '../../lib/commands.svelte';

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    /** Called after an op that may have MOVED refs/history (fetch/pull/push/
     *  branch) so the parent can re-mount the graph — otherwise the commit graph
     *  shows stale data until the repo tab is reopened. */
    onrefresh?: () => void;
    /** Rendered as direct children of the PageHeader's actions row (so they
     *  overflow into its ⋯ by `data-overflow`) instead of in a `.toolbar` wrapper. */
    inHeader?: boolean;
  }
  let { repoId, status, onstatus, onrefresh, inHeader = false }: Props = $props();

  let busy = $state('');

  // Nothing to push: the branch tracks an upstream and has no local commits on
  // top of it. (No upstream → the button is Publish, always available.)
  const nothingToPush = $derived(status.upstream != null && status.ahead === 0);
  // `git status` reports a detached HEAD as the branch "(detached)": there is
  // no branch to push or pull, so both would only fail with git's
  // "You are not currently on a branch".
  const detached = $derived(status.branch === '(detached)');
  // Push is the header's primary action only while there is something to push.
  const pushPrimary = $derived(status.upstream != null && status.ahead > 0 && !detached);
  // Known-empty stash list → Pop can only fail, so say so up front. Unknown
  // (the graph hasn't reported yet) keeps it enabled.
  const stashCount = $derived(gitBridge.stashCount[repoId]);
  const nothingToPop = $derived(stashCount === 0);

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
      toastError('Couldn’t fetch', e);
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
      ...(inHeader
        ? [
            { label: busy === 'fetch' ? 'Fetching…' : 'Fetch', icon: 'fetch' as const, title: 'Fetch from remote', action: () => void doFetch() },
            { separator: true as const },
          ]
        : []),
      { label: 'Pull (merge)', icon: 'arrowDown', disabled: detached, action: () => void doPull('merge') },
      { label: 'Pull (rebase)', icon: 'arrowDown', disabled: detached, action: () => void doPull('rebase') },
      { label: 'Pull (fast-forward only)', icon: 'arrowDown', disabled: detached, action: () => void doPull('ff_only') },
    ]);
  }

  // ⌘K: the open repo's everyday verbs (the header copy only — the side-panel
  // toolbar would register a second, identical set).
  $effect(() => {
    if (!inHeader) return;
    const branch = status.branch;
    const upstream = status.upstream;
    const isDetached = detached;
    const canPush = !nothingToPush && !isDetached;
    return registry.register('git-repo', [
      { id: 'git.fetch', title: 'Fetch', group: 'Git', keywords: 'remote refresh update', run: () => void doFetch() },
      ...(isDetached
        ? []
        : [{ id: 'git.pull', title: `Pull (${MODE_LABEL[pullMode]})`, group: 'Git', keywords: 'upstream merge rebase', run: () => void doPull() }]),
      ...(canPush
        ? [{ id: 'git.push', title: upstream ? 'Push' : 'Publish branch', group: 'Git', keywords: `upload ${branch}`, run: () => void doPush() }]
        : []),
      { id: 'git.branch', title: 'New branch…', group: 'Git', keywords: `create checkout from ${branch}`, run: () => void doCreateBranch() },
    ]);
  });

  async function doPush(): Promise<void> {
    // No confirm: push is the most routine git action (the user asked for
    // fewer nag dialogs, and no git client asks before a plain push). The
    // button label already says Push vs Publish, and the toast reports it.
    // A REJECTED push asks Pull vs Force-with-lease (runPush) — force is
    // never sent without that explicit pick.
    busy = 'push';
    try {
      await runPush(repoId, { branch: status.branch, upstream: status.upstream }, onstatus);
      onrefresh?.();
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
      toastError('Couldn’t create the branch', e);
    } finally {
      busy = '';
    }
  }

  function branchMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'New branch…', icon: 'plus', title: `Create a new branch from ${status.branch} and switch to it`, action: () => void doCreateBranch() },
      { separator: true },
      {
        label: 'Stash changes',
        icon: 'stash',
        disabled: status.changes.length === 0,
        title: status.changes.length === 0 ? 'Nothing to stash — the working tree is clean' : 'Stash working changes (including untracked files)',
        action: () => void doStash(),
      },
      {
        label: 'Pop stash',
        icon: 'archive',
        disabled: nothingToPop,
        title: nothingToPop ? 'Nothing to pop — there are no stashes' : 'Apply the latest stash and drop it',
        action: () => void doPop(),
      },
    ]);
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
      toastError('Couldn’t stash', e);
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
          `${plural(conflicts, 'file')} need resolution — open "Resolve conflicts". The stash entry was kept.`,
        );
      } else {
        toasts.success('Stash popped');
      }
    } catch (e) {
      toastError('Couldn’t pop', e);
    } finally {
      busy = '';
    }
  }
</script>

{#snippet items()}
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

{#if !inHeader}<span class="divider"></span>{/if}

<!-- Fetch (the header folds it into the Pull ▾ menu) -->
{#if !inHeader}
<button class="btn small ghost tbtn" data-overflow="3" data-icon="fetch" data-label="Fetch" disabled={busy !== ''} onclick={doFetch} title="Fetch from remote">
  <Icon name="fetch" size={12} />
  {busy === 'fetch' ? 'Fetching…' : 'Fetch'}
</button>
{/if}

<!-- Pull (split button: the repo's configured mode, ▾ overrides it once) -->
<span class="split">
  <button
    class="btn small ghost tbtn"
    data-icon="arrowDown"
    data-label="Pull ({MODE_LABEL[pullMode]})"
    disabled={busy !== '' || detached}
    onclick={() => void doPull()}
    title={detached
      ? 'HEAD is detached — check out a branch to pull'
      : "Pull from upstream using the repo's configured mode"}
  >
    <Icon name="arrowDown" size={12} />
    {busy === 'pull' ? 'Pulling…' : `Pull (${MODE_LABEL[pullMode]})`}
  </button>
  <button
    class="btn small ghost tbtn caret"
    data-icon="chevronDown"
    data-label={inHeader ? 'Fetch and pull options…' : 'Pull options…'}
    disabled={busy !== '' || (detached && !inHeader)}
    aria-haspopup="menu"
    onclick={pullMenu}
    title={inHeader ? 'Fetch, or pull with a different mode' : 'Pull with a different mode'}
    aria-label={inHeader ? 'Fetch and pull options' : 'Pull options'}
  ><Icon name="chevronDown" size={11} /></button>
</span>

<!-- Push -->
<button
  class="btn small tbtn"
  class:ghost={!pushPrimary}
  class:primary={pushPrimary}
  data-overflow="4"
  data-icon="arrowUp"
  data-label={status.upstream ? 'Push' : 'Publish'}
  disabled={busy !== '' || nothingToPush || detached}
  onclick={() => void doPush()}
  title={detached
    ? 'HEAD is detached — check out a branch to push'
    : status.upstream
    ? status.ahead > 0
      ? `Push ${plural(status.ahead, 'commit')} to ${status.upstream}`
      : `Nothing to push — ${status.branch} has no commits that ${status.upstream} doesn’t`
    : `Publish ${status.branch} to origin`}
>
  <Icon name="arrowUp" size={12} />
  {busy === 'push' ? 'Pushing…' : status.upstream ? 'Push' : 'Publish'}
</button>

{#if inHeader}
  <!-- Header: Pull (▾ holds Fetch) / Push stay; Branch / Stash / Pop fold into
       one menu so the repo's header carries three verbs, not six. -->
  <button
    class="btn small ghost tbtn"
    data-keep
    data-icon="branch"
    data-label="Branch & stash"
    disabled={busy !== ''}
    aria-haspopup="menu"
    aria-label="Branch and stash"
    title="New branch, stash, pop stash"
    onclick={branchMenu}
  ><Icon name="stash" size={12} /> <Icon name="chevronDown" size={11} /></button>
{:else}
<span class="divider"></span>

<!-- Branch (create) -->
<button class="btn small ghost tbtn" data-overflow="-2" data-icon="plus" data-label="New branch" disabled={busy !== ''} onclick={() => void doCreateBranch()} title="Create a new branch from {status.branch} and switch to it">
  <Icon name="plus" size={12} />
  {busy === 'branch' ? 'Creating…' : 'Branch'}
</button>

<span class="divider"></span>

<!-- Stash -->
<button
  class="btn small ghost tbtn"
  data-overflow="-3"
  data-icon="stash"
  data-label="Stash"
  disabled={busy !== '' || status.changes.length === 0}
  onclick={doStash}
  title={status.changes.length === 0 ? 'Nothing to stash — the working tree is clean' : 'Stash working changes (including untracked files)'}
>
  <Icon name="stash" size={12} />
  {busy === 'stash' ? 'Stashing…' : 'Stash'}
</button>

<!-- Pop -->
<button
  class="btn small ghost tbtn"
  data-overflow="-4"
  data-icon="archive"
  data-label="Pop stash"
  disabled={busy !== '' || nothingToPop}
  onclick={doPop}
  title={nothingToPop ? 'Nothing to pop — there are no stashes' : 'Apply the latest stash and drop it'}
>
  <Icon name="archive" size={12} />
  {busy === 'pop' ? 'Popping…' : 'Pop'}
</button>
{/if}
{/snippet}

{#if inHeader}
  {@render items()}
{:else}
  <div class="toolbar">
    {@render items()}
  </div>
{/if}

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
    gap: 4px;
    min-width: 0;
    max-width: 200px;
    background: var(--accent-soft);
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
    display: inline-flex;
    align-items: center;
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
    padding: 0 8px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .tbtn:hover:not(:disabled) {
    color: var(--text);
  }
  .tbtn.primary,
  .tbtn.primary:hover:not(:disabled) {
    color: var(--accent-contrast);
  }
  /* Pull split button: one visual unit — the halves share a square seam with a
     hairline between them, and hovering either half outlines both. */
  .split {
    display: inline-flex;
    align-items: center;
  }
  .split .tbtn:first-child {
    padding-inline-end: 6px;
    border-start-end-radius: 0;
    border-end-end-radius: 0;
  }
  .split .caret {
    padding: 0 4px;
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
    /* The toolbar scrolls here, so the chip needn't shrink — shrinking only
       collapsed it to a bare icon on a phone. */
    .branch-chip {
      flex-shrink: 0;
      max-width: 160px;
    }
    .tbtn {
      height: 36px;
      padding: 0 10px;
      font-size: var(--fs-m);
    }
  }
</style>
