<script lang="ts">
  import { plural } from '../../lib/plural';
  import { needResolution } from '../../lib/api/git';
  import { toastError } from '../../lib/toastError';
  // Toolbar row: Fetch / Pull / Push / Branch / Stash / Pop + current branch chip.
  // In the Git page header it is the repo's state in words + a visible Fetch;
  // every other verb rides the header's ONE ⋯ (`menuItems`), grouped, and
  // stays a ⌘K command (group "Git").
  import { git } from '../../lib/stores/git.svelte';
  import { api } from '../../lib/api/client';
  import type { PullMode, PullModeResp, RemoteInfo, RepoStatusResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { runPull } from './pullFlow';
  import { runPush } from './pushFlow';
  import { gitBridge } from '../../lib/diff/gitBridge.svelte';
  import { registry } from '../../lib/commands.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { stateWords } from '../../lib/gitNextStep';
  // Code-split: loaded on the first open, not with the toolbar.
  const StashBrowser = lazyComponent(() => import('./StashBrowser.svelte'));

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
    /** Opens the stash browser OUTSIDE the toolbar. In the header a dialog
     *  mounted here lands in PageHeader's actions row, whose overflow
     *  measurer hides it (and Pull with it). */
    onbrowsestashes?: () => void;
    /** Header: rendered between the state words and Fetch (the view switch). */
    children?: import('svelte').Snippet;
  }
  let { repoId, status, onstatus, onrefresh, inHeader = false, onbrowsestashes, children }: Props = $props();

  let busy = $state('');

  // Nothing to push: the branch tracks an upstream and has no local commits on
  // top of it. (No upstream → the button is Publish, always available.)
  const nothingToPush = $derived(status.upstream != null && status.ahead === 0);
  // `git status` reports a detached HEAD as the branch "(detached)": there is
  // no branch to push or pull, so both would only fail with git's
  // "You are not currently on a branch".
  const detached = $derived(status.branch === '(detached)');
  // A repo with no remote can't pull or publish (git's "No configured push
  // destination" / "no tracking information"). Re-read when refs move (a
  // remote added in Remotes… bumps them). Unknown (null) keeps both enabled.
  let remoteCount = $state<number | null>(null);
  let remoteCountRepo = '';
  $effect(() => {
    const id = repoId;
    void git.refsRev[id];
    if (remoteCountRepo !== id) {
      remoteCountRepo = id;
      remoteCount = null;
    }
    const ctl = new AbortController();
    api
      .get<RemoteInfo[]>(`/repos/${id}/remotes`, ctl.signal)
      .then((r) => {
        if (!ctl.signal.aborted && id === repoId) remoteCount = r.length;
      })
      .catch(() => {});
    return () => ctl.abort();
  });
  const noRemote = $derived(remoteCount === 0);
  const NO_REMOTE = 'Add a remote first (⋯ → Remotes…)';
  // Push is the header's primary action only while there is something to push.
  const pushPrimary = $derived(status.upstream != null && status.ahead > 0 && !detached);
  // Known-empty stash list → Pop can only fail, so say so up front. Unknown
  // (the graph hasn't reported yet) keeps it enabled.
  const stashCount = $derived(gitBridge.stashCount[repoId]);
  const nothingToPop = $derived(stashCount === 0);
  /** The stash browser is open (files + patch, Apply / Pop / Drop). */
  let stashesOpen = $state(false);
  function browseStashes(): void {
    if (onbrowsestashes) onbrowsestashes();
    else stashesOpen = true;
  }

  async function doFetch(): Promise<void> {
    busy = 'fetch';
    try {
      const s = await git.fetchRepo(repoId);
      onstatus(s); // Store refsRev quietly refreshes the graph without a remount.
      // Say what the fetch found, not which remote we assume it hit.
      toasts.success(
        'Fetched',
        s.behind > 0
          ? `${plural(s.behind, 'new commit')} on ${s.upstream ?? 'the upstream'} — pull to bring them in`
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
  // once per repo so the button says what it will actually do. Until it is
  // known (or when the read fails) the button says plain "Pull": guessing
  // "merge" mislabelled a repo configured to rebase.
  let pullMode = $state<PullMode | null>(null);
  let pullModeFor = '';
  $effect(() => {
    const id = repoId;
    if (pullModeFor === id) return;
    pullModeFor = id;
    pullMode = null;
    void api
      .get<PullModeResp>(`/repos/${id}/pull-mode`)
      .then((r) => {
        if (pullModeFor === id) pullMode = r.mode;
      })
      .catch(() => {
        // Not sticky: the next time this repo is shown, read it again.
        if (pullModeFor === id) pullModeFor = '';
      });
  });

  const MODE_LABEL: Record<PullMode, string> = {
    merge: 'merge',
    rebase: 'rebase',
    ff_only: 'ff-only',
  };
  const pullLabel = $derived(pullMode ? `Pull (${MODE_LABEL[pullMode]})` : 'Pull');

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
    if (noRemote) return;
    ctxMenu.show(e, [
      { label: 'Pull (merge)', icon: 'arrowDown', disabled: detached || noRemote, title: noRemote ? NO_REMOTE : undefined, action: () => void doPull('merge') },
      { label: 'Pull (rebase)', icon: 'arrowDown', disabled: detached || noRemote, title: noRemote ? NO_REMOTE : undefined, action: () => void doPull('rebase') },
      { label: 'Pull (fast-forward only)', icon: 'arrowDown', disabled: detached || noRemote, title: noRemote ? NO_REMOTE : undefined, action: () => void doPull('ff_only') },
    ]);
  }

  // ⌘K: the open repo's everyday verbs (the header copy only — the side-panel
  // toolbar would register a second, identical set).
  $effect(() => {
    if (!inHeader) return;
    const branch = status.branch;
    const upstream = status.upstream;
    const isDetached = detached;
    const hasNoRemote = noRemote;
    const canPush = !nothingToPush && !isDetached;
    return registry.register('git-repo', [
      { id: 'git.fetch', title: 'Fetch', group: 'Git', keywords: 'remote refresh update', disabled: hasNoRemote, detail: hasNoRemote ? NO_REMOTE : undefined, run: () => { if (!hasNoRemote) void doFetch(); } },
      ...(isDetached
        ? []
        : [{ id: 'git.pull', title: pullLabel, group: 'Git', keywords: 'upstream merge rebase', disabled: hasNoRemote, detail: hasNoRemote ? NO_REMOTE : undefined, run: () => { if (!hasNoRemote) void doPull(); } }]),
      ...(canPush
        ? [{ id: 'git.push', title: upstream ? 'Push' : 'Publish branch', group: 'Git', keywords: `upload ${branch}`, disabled: hasNoRemote, detail: hasNoRemote ? NO_REMOTE : undefined, run: () => { if (!hasNoRemote) void doPush(); } }]
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
  async function doCreateBranch(initial?: string): Promise<void> {
    const raw = await confirmer.promptText(`New branch from ${status.branch}`, {
      title: 'Create branch',
      confirmLabel: 'Create',
      placeholder: 'feature/my-branch',
      initial,
    });
    const name = raw?.trim() ?? '';
    if (!name) return;
    busy = 'branch';
    let failed = false;
    try {
      const s = await git.checkout(repoId, { branch: name, create: true });
      onstatus(s);
      onrefresh?.();
      toasts.success('Branch created', name);
    } catch (e) {
      toastError('Couldn’t create the branch', e);
      failed = true;
    } finally {
      busy = '';
    }
    // A bad or taken name: re-open with what was typed, not an empty field.
    if (failed) await doCreateBranch(name);
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
          `${needResolution(conflicts)} — open "Resolve conflicts". The stash entry was kept.`,
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

  /** The header's ⋯ rows for this repo, grouped: sync, then branch & stash. */
  export function menuItems(): MenuItem[] {
    const why = detached ? 'HEAD is detached — check out a branch first' : noRemote ? NO_REMOTE : undefined;
    return [
      // The three pull modes; the repo's configured one says "default".
      ...(['merge', 'rebase', 'ff_only'] as const).map((m) => ({
        label: `Pull (${m === 'ff_only' ? 'fast-forward only' : m})`,
        icon: 'arrowDown',
        hint: pullMode === m ? 'default' : undefined,
        disabled: busy !== '' || detached || noRemote,
        title: why,
        action: () => void doPull(m),
      })),
      {
        label: status.upstream ? 'Push' : 'Publish branch',
        icon: 'arrowUp',
        hint: status.ahead > 0 ? `↑${status.ahead}` : undefined,
        disabled: busy !== '' || nothingToPush || detached || noRemote,
        title: why ?? (nothingToPush ? `Nothing to push — ${status.branch} has no commits that ${status.upstream} doesn’t` : undefined),
        action: () => void doPush(),
      },
      { separator: true },
      { label: 'New branch…', icon: 'plus', title: `Create a new branch from ${status.branch} and switch to it`, action: () => void doCreateBranch() },
      { label: 'Stash changes', icon: 'stash', disabled: status.changes.length === 0, title: status.changes.length === 0 ? 'Nothing to stash — the working tree is clean' : undefined, action: () => void doStash() },
      { label: 'Pop stash', icon: 'archive', disabled: nothingToPop, title: nothingToPop ? 'There are no stashes' : undefined, action: () => void doPop() },
      { label: stashCount ? `Browse stashes (${stashCount})…` : 'Browse stashes…', icon: 'eye', disabled: nothingToPop, action: browseStashes },
    ];
  }
  /** The repo's state in words for the header ("3 files changed · ↑2 not pushed"). */
  const words = $derived(
    stateWords({ changes: status.changes.length, ahead: status.ahead, behind: status.behind, upstream: status.upstream, branch: detached ? null : status.branch }),
  );
</script>

{#snippet items()}
<!-- Branch chip -->
<span
  class="branch-chip"
  title="{status.branch}{status.upstream ? ` · tracks ${status.upstream}` : ''}{status.ahead > 0 ? ` · ${status.ahead} ahead` : ''}{status.behind > 0 ? ` · ${status.behind} behind` : ''}"
>
  <Icon name="branch" size={12} />
  <span class="mono branch-name">{status.branch}</span>
  <!-- aria-label on a role-less span is not read: the glyph is hidden and an
       sr-only twin carries the words. -->
  {#if status.ahead > 0}<span class="ab up"><span aria-hidden="true">↑{status.ahead}</span><span class="sr-only">{status.ahead} ahead</span></span>{/if}
  {#if status.behind > 0}<span class="ab down"><span aria-hidden="true">↓{status.behind}</span><span class="sr-only">{status.behind} behind</span></span>{/if}
</span>

<span class="divider"></span>

<!-- Fetch -->
<button class="btn small ghost tbtn" data-overflow="3" data-icon="fetch" data-label="Fetch" disabled={busy !== '' || noRemote} onclick={doFetch} title={noRemote ? NO_REMOTE : 'Fetch from remote'}>
  <Icon name="fetch" size={12} />
  {busy === 'fetch' ? 'Fetching…' : 'Fetch'}
</button>

<!-- Pull (split button: the repo's configured mode, ▾ overrides it once) -->
<span class="split">
  <button
    class="btn small ghost tbtn"
    data-icon="arrowDown"
    data-label={pullLabel}
    disabled={busy !== '' || detached || noRemote}
    onclick={() => void doPull()}
    title={detached
      ? 'HEAD is detached — check out a branch to pull'
      : noRemote
      ? NO_REMOTE
      : "Pull from upstream using the repo’s configured mode"}
  >
    <Icon name="arrowDown" size={12} />
    {busy === 'pull' ? 'Pulling…' : pullLabel}
  </button>
  <button
    class="btn small ghost tbtn caret"
    data-icon="chevronDown"
    data-label="Pull options…"
    disabled={busy !== '' || noRemote || detached}
    aria-haspopup="menu"
    onclick={pullMenu}
    title={noRemote ? NO_REMOTE : 'Pull with a different mode'}
    aria-label="Pull options"
  ><Icon name="chevronDown" size={12} /></button>
</span>

<!-- Push -->
<button
  class="btn small tbtn"
  class:ghost={!pushPrimary}
  class:primary={pushPrimary}
  data-overflow="4"
  data-icon="arrowUp"
  data-label={status.upstream ? 'Push' : 'Publish'}
  disabled={busy !== '' || nothingToPush || detached || noRemote}
  onclick={() => void doPush()}
  title={detached
    ? 'HEAD is detached — check out a branch to push'
    : noRemote
    ? NO_REMOTE
    : status.upstream
    ? status.ahead > 0
      ? `Push ${plural(status.ahead, 'commit')} to ${status.upstream}`
      : `Nothing to push — ${status.branch} has no commits that ${status.upstream} doesn’t`
    : `Publish ${status.branch} to origin`}
>
  <Icon name="arrowUp" size={12} />
  {busy === 'push' ? 'Pushing…' : status.upstream ? 'Push' : 'Publish'}
</button>

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

<!-- Stashes: opens the browser (what each stash holds; Apply / Pop / Drop)
     instead of a blind pop of whatever is on top. -->
<button
  class="btn small ghost tbtn"
  data-overflow="-4"
  data-icon="archive"
  data-label="Stashes"
  disabled={busy !== '' || nothingToPop}
  onclick={browseStashes}
  title={nothingToPop ? 'There are no stashes' : 'See what each stash holds, then apply, pop or drop it'}
>
  <Icon name="archive" size={12} />
  {busy === 'pop' ? 'Popping…' : stashCount ? `Stashes (${stashCount})` : 'Stashes'}
</button>
{/snippet}

{#if stashesOpen}
  <LazyMount
    lazy={StashBrowser}
    what="the stash browser"
    quiet
    onerror={() => (stashesOpen = false)}
    props={{
      repoId,
      onclose: () => (stashesOpen = false),
      onchanged: (st: RepoStatusResp) => {
        onstatus(st);
        onrefresh?.();
      },
    }}
  />
{/if}

{#if inHeader}
  <!-- The repo's state in words (its tab names the repo and branch), then the
       one visible verb (Fetch); the rest is in the header's ⋯ (menuItems). -->
  <span class="git-state" data-keep title="{status.branch}{status.upstream ? ` · tracks ${status.upstream}` : ''}" data-testid="git-state">
    <span class="git-state-words">{words}</span>
  </span>
  {@render children?.()}
  <button class="btn small ghost tbtn" data-keep data-icon="fetch" data-label="Fetch" disabled={busy !== '' || noRemote} onclick={doFetch} title={noRemote ? NO_REMOTE : 'Fetch from remote'}>
    <Icon name="fetch" size={12} />
    {busy === 'fetch' ? 'Fetching…' : 'Fetch'}
  </button>
{:else}
  <div class="toolbar">
    {@render items()}
  </div>
{/if}

<style>
  /* Header: "feat/x · 3 files changed · ↑2 not pushed" — shrinks first (ellipsis). */
  .git-state {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    flex: 0 1 auto;
    overflow: hidden;
    white-space: nowrap;
    font-size: var(--fs-m);
    color: var(--text-dim);
  }
  .git-state-words {
    overflow: hidden;
    text-overflow: ellipsis;
  }
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
