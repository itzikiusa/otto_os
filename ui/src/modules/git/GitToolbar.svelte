<script lang="ts">
  import { plural } from '../../lib/plural';
  import { needResolution } from '../../lib/api/git';
  import { toastError } from '../../lib/toastError';
  // The everyday git verbs — Fetch · Pull · Push · Stash · Pop — always one
  // visible click away (the owner's rule: frequent actions never hide in ⋯).
  //   header: the Git page's header row — the repo's state in words, the view
  //           switch (children), then the five verbs. A tight row turns labels
  //           into icons (Stash, Pop, Fetch first; Pull and Push last); it
  //           never wraps or moves a verb into ⋯. ⋯ (menuItems) keeps the rare
  //           ones. On a phone the verbs are the 'row' under the header.
  //   row:    the phone's second row of finger-sized verbs.
  //   sync:   Pull + Push beside a commit card (Agents' Git tab, Threads).
  // The verbs are also ⌘K commands (group "Git", header copy only).
  import { flushSync, untrack } from 'svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { api } from '../../lib/api/client';
  import type { PullMode, PullModeResp, RemoteInfo, RepoStatusResp, StashInfo } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { runPull } from './pullFlow';
  import { runPush } from './pushFlow';
  import { gitBridge } from '../../lib/diff/gitBridge.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
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
     *  stash) so the parent can refresh the graph. */
    onrefresh?: () => void;
    variant?: 'header' | 'row' | 'sync';
    /** Header: Push may be the accent (false while a commit card's own
     *  primary is on screen — one blue button per screen). */
    accent?: boolean;
    /** Opens the stash browser OUTSIDE the toolbar (a dialog mounted in
     *  PageHeader's actions row would be measured and hidden with it). */
    onbrowsestashes?: () => void;
    /** Header: the repo tabs' floor (PageHeader's tabsMin) — what they can still give up. */
    tabsMin?: number;
    /** …and never below this many px (the compact repo switcher's room). */
    tabsFloor?: number;
    /** sync: false while the card beside it already offers Push (its primary). */
    push?: boolean;
    /** Header: rendered between the state words and the verbs (the view switch). */
    children?: import('svelte').Snippet<[views: 'words' | 'icons' | 'menu']>;
  }
  let { repoId, status, onstatus, onrefresh, variant = 'row', accent = false, tabsMin = 0.5, tabsFloor = 0, push = true, onbrowsestashes, children }: Props = $props();

  let busy = $state('');
  const header = $derived(variant === 'header');
  // The header's verbs move to the 'row' under it on a phone.
  const showVerbs = $derived(!header || !viewport.isPhone);

  // Nothing to push: the branch tracks an upstream and has no local commits on
  // top of it. (No upstream → the button is Publish, always available.)
  const nothingToPush = $derived(status.upstream != null && status.ahead === 0);
  // `git status` reports a detached HEAD as the branch "(detached)": there is
  // no branch to push or pull, so both would only fail with git's
  // "You are not currently on a branch".
  const detached = $derived(status.branch === '(detached)');
  const clean = $derived(status.changes.length === 0);
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
    // A repo with an origin has a remote: no request (most repos). Only one
    // without it asks which remotes exist (an `upstream`-only clone).
    if ((git.allRepos.find((r) => r.id === id) ?? git.repos.find((r) => r.id === id))?.remote_url) {
      remoteCount = 1;
      return;
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
  const syncWhy = $derived(detached ? 'HEAD is detached — check out a branch first' : noRemote ? NO_REMOTE : undefined);
  // Push is the accent only with something to push and nothing to commit.
  const pushPrimary = $derived(header && accent && clean && status.upstream != null && status.ahead > 0 && !detached);

  // The stash list: the graph reports its count (gitBridge); where the graph
  // isn't loaded (Changes) the toolbar reads it once, and after its own ops.
  const stashCount = $derived(gitBridge.stashCount[repoId]);
  const nothingToPop = $derived(stashCount === 0);
  let stashes: StashInfo[] = [];
  async function loadStashes(id = repoId): Promise<StashInfo[]> {
    const list = await api.get<StashInfo[]>(`/repos/${id}/stashes`);
    if (id === repoId) {
      stashes = list;
      gitBridge.stashCount[id] = list.length;
    }
    return list;
  }
  $effect(() => {
    const id = repoId;
    if (variant !== 'sync' && showVerbs && gitBridge.stashCount[id] === undefined) untrack(() => void loadStashes(id).catch(() => {}));
  });
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

  // The repo's EFFECTIVE pull mode (its `pull.rebase`/`pull.ff` config), read once
  // per repo for the caret's "default" mark.
  let pullMode = $state<PullMode | null>(null);
  let pullModeFor = '';
  function loadPullMode(id: string): Promise<void> {
    if (pullModeFor === id) return Promise.resolve();
    pullModeFor = id;
    pullMode = null;
    return api
      .get<PullModeResp>(`/repos/${id}/pull-mode`)
      .then((r) => {
        if (pullModeFor === id) pullMode = r.mode;
      })
      .catch(() => {
        // Not sticky: the next time this repo is shown, read it again.
        if (pullModeFor === id) pullModeFor = '';
      });
  }

  async function doPull(mode?: PullMode): Promise<void> {
    busy = 'pull';
    try {
      await runPull(repoId, onstatus, { mode });
      onrefresh?.();
    } finally {
      busy = '';
    }
  }

  function pullModeItems(): MenuItem[] {
    return (['merge', 'rebase', 'ff_only'] as const).map((m) => ({
      label: `Pull (${m === 'ff_only' ? 'fast-forward only' : m})`,
      icon: 'arrowDown',
      hint: pullMode === m ? 'default' : undefined,
      disabled: busy !== '' || !!syncWhy,
      title: syncWhy,
      action: () => void doPull(m),
    }));
  }
  function pullMenu(e: MouseEvent): void {
    if (noRemote) return;
    ctxMenu.show(e, pullModeItems());
  }
  // The caret marks the repo's default mode: read it with the repo.
  $effect(() => {
    if (header && showVerbs) void loadPullMode(repoId);
  });

  // ⌘K: the open repo's everyday verbs (the header copy only).
  $effect(() => {
    if (!header) return;
    const branch = status.branch;
    const upstream = status.upstream;
    const isDetached = detached;
    const hasNoRemote = noRemote;
    const canPush = !nothingToPush && !isDetached;
    return registry.register('git-repo', [
      { id: 'git.fetch', title: 'Fetch', group: 'Git', keywords: 'remote refresh update', disabled: hasNoRemote, detail: hasNoRemote ? NO_REMOTE : undefined, run: () => { if (!hasNoRemote) void doFetch(); } },
      ...(isDetached
        ? []
        : [{ id: 'git.pull', title: 'Pull', group: 'Git', keywords: 'upstream merge rebase', disabled: hasNoRemote, detail: hasNoRemote ? NO_REMOTE : undefined, run: () => { if (!hasNoRemote) void doPull(); } }]),
      ...(canPush
        ? [{ id: 'git.push', title: upstream ? 'Push' : 'Publish branch', group: 'Git', keywords: `upload ${branch}`, disabled: hasNoRemote, detail: hasNoRemote ? NO_REMOTE : undefined, run: () => { if (!hasNoRemote) void doPush(); } }]
        : []),
      { id: 'git.branch', title: 'New branch…', group: 'Git', keywords: `create checkout from ${branch}`, run: () => void doCreateBranch() },
    ]);
  });

  async function doPush(): Promise<void> {
    // No confirm: push is the most routine git action. A REJECTED push asks
    // Pull vs Force-with-lease (runPush) — force is never sent without that pick.
    busy = 'push';
    try {
      await runPush(repoId, { branch: status.branch, upstream: status.upstream }, onstatus);
      onrefresh?.();
    } finally {
      busy = '';
    }
  }

  // Same sheet the graph's "Create branch here…" uses.
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

  const DONE = { save: 'Stashed', pop: 'Stash popped', apply: 'Stash applied', drop: 'Stash dropped' } as const;
  /** save | pop (the newest) | pop/apply/drop one stash by its SHA. */
  async function stashOp(op: 'save' | 'pop' | 'apply' | 'drop', s?: StashInfo): Promise<void> {
    busy = op === 'save' ? 'stash' : 'pop';
    try {
      const st = await api.post<RepoStatusResp>(`/repos/${repoId}/stash`, s ? { op, sha: s.sha } : { op });
      onstatus(st);
      // The graph's STASHES section + stash node reload with it.
      onrefresh?.();
      void loadStashes().catch(() => {});
      // A conflicting pop/apply is a 200 with unmerged paths (git keeps the
      // stash entry) — guide the user into the resolver instead of claiming success.
      const conflicts = st.changes.filter((c) => c.kind === 'conflicted').length;
      if (conflicts > 0 && op !== 'drop') {
        toasts.warn(`${DONE[op]} with conflicts`, `${needResolution(conflicts)} — open "Resolve conflicts". The stash entry was kept.`);
      } else {
        toasts.success(DONE[op], s?.ref);
      }
    } catch (e) {
      toastError(op === 'save' ? 'Couldn’t stash' : `Couldn’t ${op} the stash`, e);
    } finally {
      busy = '';
    }
  }

  /** Pop's caret: every stash (pop / apply / drop), then the browser. */
  async function stashMenu(e: MouseEvent): Promise<void> {
    const el = e.currentTarget as HTMLElement;
    const [list, m] = await Promise.all([loadStashes().catch(() => stashes), import('./StashBrowser.svelte')]);
    ctxMenu.showAt(el, m.stashMenuItems(list, (op, s) => void stashOp(op, s), browseStashes));
  }

  /** The header's ⋯ rows: the rare ones only (the verbs are visible). On a
   *  phone the verbs row has no carets, so the pull modes and the stash
   *  browser ride ⋯ there. */
  export function menuItems(): MenuItem[] {
    if (viewport.isPhone) void loadPullMode(repoId);
    return [
      { label: 'New branch…', icon: 'plus', title: `Create a new branch from ${status.branch} and switch to it`, action: () => void doCreateBranch() },
      ...(viewport.isPhone
        ? [
            { separator: true },
            ...pullModeItems(),
            { label: stashCount ? `Browse stashes (${stashCount})…` : 'Browse stashes…', icon: 'eye', disabled: nothingToPop, action: browseStashes },
          ]
        : []),
      { separator: true },
      {
        label: 'Auto-fetch', checked: git.autoFetchEnabled, action: () => git.setAutoFetch(!git.autoFetchEnabled),
        title: git.autoFetchEnabled
          ? `On — the selected repo every 60s, other recently used repos every ${Math.max(60, git.autoFetchIntervalSec)}s while Git is open and the window is focused`
          : 'Paused — a repo is still fetched once when you open it',
      },
    ];
  }
  /** The repo's state in words for the header ("↑2 not pushed · ↓1 to pull"). */
  // The file count rides the Changes badge (and the commit card), not this line.
  const words = $derived.by(() => {
    const w = stateWords({ changes: 0, ahead: status.ahead, behind: status.behind, upstream: status.upstream, branch: detached ? null : status.branch });
    return status.changes.length > 0 && w === 'Clean' ? '' : w;
  });

  // ── Header fit: once the repo tabs are at their floor, the row gives way in
  // this order before anything is clipped: the state words (they repeat the
  // ↑/↓ counts) and "Auto-fetch paused" (⋯ has the toggle), then each verb's
  // label (the icon stays, the word is its name and title), then the view
  // switch turns to icons, then to one menu button. Measured against the header's actions wrap on
  // every resize and whenever a label or count changes.
  const SQUEEZE = ['state', 'stash', 'pop', 'fetch', 'pull', 'push', 'views', 'viewmenu'];
  let squeeze = $state(0);
  const iconOnly = (v: string): boolean => SQUEEZE.indexOf(v) !== -1 && SQUEEZE.indexOf(v) < squeeze;
  let fetchEl = $state<HTMLButtonElement>();
  function fit(): void {
    const wrap = fetchEl?.closest<HTMLElement>('.ph-actions-wrap');
    const row = wrap?.parentElement;
    if (!wrap || !row) return;
    // What the repo tabs can still give up (PageHeader shrinks them to their
    // floor before anything scrolls).
    const tabs = row.querySelector<HTMLElement>(':scope > .ph-tabs-inline');
    // Below the floor and cut (the compact repo switcher's name ellipsized),
    // the tabs are owed room: give goes negative and the verbs give way.
    const cut = tabs?.querySelector<HTMLElement>('.git-repo-switch-name');
    const want = tabs ? tabs.offsetWidth + (cut ? cut.scrollWidth - cut.clientWidth : 0) : 0;
    const give = tabs ? tabs.offsetWidth - Math.max(row.clientWidth * tabsMin, Math.min(tabsFloor, want)) : 0;
    const fits = (): boolean => wrap.scrollWidth - wrap.clientWidth <= give + 1;
    const set = (n: number): void => {
      squeeze = n;
      flushSync();
    };
    // From the level it is at (each step is a forced layout): up while the row
    // overflows, else down while the wider labels still fit.
    if (!fits()) {
      while (squeeze < SQUEEZE.length && !fits()) set(squeeze + 1);
      return;
    }
    while (squeeze > 0) {
      set(squeeze - 1);
      if (!fits()) return set(squeeze + 1);
    }
  }
  $effect(() => {
    if (!header || !fetchEl) return;
    const wrap = fetchEl.closest<HTMLElement>('.ph-actions-wrap');
    if (!wrap) return;
    let raf = 0;
    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(fit);
    });
    ro.observe(wrap);
    return () => {
      ro.disconnect();
      cancelAnimationFrame(raf);
    };
  });
  // Content that changes the row's width (counts, "Pushing…", the state words).
  $effect(() => {
    if (!header) return;
    void [busy, words, status.ahead, status.behind, stashCount, git.autoFetchEnabled];
    const raf = requestAnimationFrame(fit);
    return () => cancelAnimationFrame(raf);
  });
</script>

{#snippet label(text: string, count?: string | number | null, hide = false)}
  <!-- Icon-only: the word stays the accessible name (visually hidden). -->
  <span class="vlbl" class:sr-only={hide}>{text}</span>{#if count}<span class="vcount">{count}</span>{/if}
{/snippet}
{#snippet glyph(v: string, icon: 'fetch' | 'arrowDown' | 'arrowUp' | 'stash' | 'archive')}
  {#if busy === v}<span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span>{:else}<Icon name={icon} size={variant === 'row' ? 16 : 12} />{/if}
{/snippet}

{#snippet verbs()}
  {#if variant !== 'sync'}
    <button
      bind:this={fetchEl}
      class="btn small ghost tbtn"
      class:icon-only={iconOnly('fetch')}
      data-keep
      disabled={busy !== '' || noRemote}
      onclick={doFetch}
      title={noRemote ? NO_REMOTE : 'Fetch from remote'}
    >{@render glyph('fetch', 'fetch')}{@render label(busy === 'fetch' ? 'Fetching…' : 'Fetch', null, iconOnly('fetch'))}</button>
  {/if}

  <!-- Pull: the repo's configured mode; the caret picks another one once. -->
  <span class="split" data-keep>
    <button
      class="btn small ghost tbtn"
      class:icon-only={iconOnly('pull')}
      disabled={busy !== '' || !!syncWhy}
      onclick={() => void doPull()}
      title={syncWhy ?? (status.behind > 0 ? `Pull ${plural(status.behind, 'commit')} from ${status.upstream}` : 'Pull from upstream using the repo’s configured mode')}
    >{@render glyph('pull', 'arrowDown')}{@render label(busy === 'pull' ? 'Pulling…' : 'Pull', status.behind > 0 ? `↓${status.behind}` : null, iconOnly('pull'))}</button>
    {#if header}
      <button
        class="btn small ghost tbtn caret"
        disabled={busy !== '' || !!syncWhy}
        aria-haspopup="menu"
        onclick={pullMenu}
        title={syncWhy ?? 'Pull with a different mode'}
        aria-label="Pull options"
      ><Icon name="chevronDown" size={12} /></button>
    {/if}
  </span>

  {#if push}
  <button
    class="btn small tbtn"
    class:ghost={!pushPrimary}
    class:primary={pushPrimary}
    class:icon-only={iconOnly('push')}
    data-keep
    disabled={busy !== '' || nothingToPush || !!syncWhy}
    onclick={() => void doPush()}
    title={syncWhy ??
      (status.upstream
        ? status.ahead > 0
          ? `Push ${plural(status.ahead, 'commit')} to ${status.upstream}`
          : `Nothing to push — ${status.branch} has no commits that ${status.upstream} doesn’t`
        : `Publish ${status.branch} to origin`)}
  >{@render glyph('push', 'arrowUp')}{@render label(busy === 'push' ? (status.upstream ? 'Pushing…' : 'Publishing…') : status.upstream ? 'Push' : 'Publish', status.ahead > 0 && status.upstream ? `↑${status.ahead}` : null, iconOnly('push'))}</button>
  {/if}

  {#if variant !== 'sync'}
    <button
      class="btn small ghost tbtn"
      class:icon-only={iconOnly('stash')}
      data-keep
      disabled={busy !== '' || clean}
      onclick={() => void stashOp('save')}
      title={clean ? 'Nothing to stash — the working tree is clean' : 'Stash working changes (including untracked files)'}
    >{@render glyph('stash', 'stash')}{@render label(busy === 'stash' ? 'Stashing…' : 'Stash', null, iconOnly('stash'))}</button>

    <!-- Pop: the newest stash; the caret lists every stash (pop / apply / drop). -->
    <span class="split" data-keep>
      <button
        class="btn small ghost tbtn"
        class:icon-only={iconOnly('pop')}
        disabled={busy !== '' || nothingToPop}
        onclick={() => void stashOp('pop')}
        title={nothingToPop ? 'There are no stashes' : 'Apply the newest stash and drop it'}
      >{@render glyph('pop', 'archive')}{@render label(busy === 'pop' ? 'Popping…' : 'Pop', stashCount && stashCount > 1 ? stashCount : null, iconOnly('pop'))}</button>
      {#if header}
        <button
          class="btn small ghost tbtn caret"
          disabled={busy !== '' || nothingToPop}
          aria-haspopup="menu"
          onclick={stashMenu}
          title={nothingToPop ? 'There are no stashes' : 'Pop, apply or drop a stash'}
          aria-label="Stashes"
        ><Icon name="chevronDown" size={12} /></button>
      {/if}
    </span>
  {/if}
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
        void loadStashes().catch(() => {});
      },
    }}
  />
{/if}

{#if header}
  <!-- The repo's state in words (its tab names the repo and branch), the view
       switch, then the verbs; the rare ones are in the header's ⋯ (menuItems). -->
  <span class="git-state" class:squeezed={iconOnly('state')} data-keep title="{status.branch}{status.upstream ? ` · tracks ${status.upstream}` : ''}" data-testid="git-state">
    <span class="git-state-words">{words}</span>
  </span>
  <!-- Auto-fetch lives in ⋯; paused is a lasting choice, so it says so here. -->
  {#if !git.autoFetchEnabled}
    <button class="btn small ghost git-autofetch-paused" class:squeezed={iconOnly('state')} data-keep data-testid="git-autofetch-paused" onclick={() => git.setAutoFetch(true)}
      title="Auto-fetch is paused — a repo is still fetched once when you open it. Click to resume.">Auto-fetch paused</button>
  {/if}
  {@render children?.(iconOnly('viewmenu') ? 'menu' : iconOnly('views') ? 'icons' : 'words')}
  {#if showVerbs}{@render verbs()}{/if}
{:else}
  <div class="verbs" class:phone-row={variant === 'row'} class:sync={variant === 'sync'} role="group" aria-label="Git" data-testid={variant === 'row' ? 'git-verbs-row' : 'git-verbs-sync'}>
    {@render verbs()}
  </div>
{/if}

<style>
  /* Header: "↑2 not pushed · ↓1 to pull". */
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
  .squeezed {
    display: none;
  }
  .git-autofetch-paused {
    flex: none;
    color: var(--warning);
    white-space: nowrap;
  }
  @media (max-width: 640px) {
    .git-state {
      display: none;
    }
  }
  /* Quiet verbs: the global .btn.ghost with a dimmer label until hovered,
     on the tighter toolbar padding (the header row holds five of them). */
  .tbtn {
    color: var(--text-dim);
  }
  :global(.ph-actions) .tbtn {
    padding-inline: 8px;
    gap: 4px;
  }
  /* The last fit step (the views one menu button): the verbs pack tighter. */
  :global(.ph-actions:has(.git-views-menu)) .tbtn.icon-only {
    padding-inline: 4px;
  }
  :global(.ph-actions:has(.git-views-menu)) .tbtn.caret {
    padding-inline: 2px;
  }
  .tbtn:hover:not(:disabled) {
    color: var(--text);
  }
  .tbtn.primary,
  .tbtn.primary:hover:not(:disabled) {
    color: var(--accent-contrast);
  }
  .vcount {
    font-size: var(--fs-s);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  /* Pull / Pop split buttons: one unit — the halves share a square seam. */
  .split {
    display: inline-flex;
    align-items: center;
    flex: none;
  }
  .split .tbtn:first-child:not(:only-child) {
    padding-inline-end: 8px;
    border-start-end-radius: 0;
    border-end-end-radius: 0;
  }
  .split .tbtn.caret {
    padding: 0 4px;
    border-start-start-radius: 0;
    border-end-start-radius: 0;
  }
  .split:hover .tbtn:not(:disabled) {
    border-color: var(--border);
  }
  .verbs {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  /* Phone: a second row of finger-sized verbs under the header — icon over a
     short label, each an equal share of the width. */
  .verbs.phone-row {
    flex-shrink: 0;
    gap: 6px;
    padding-block: 4px 6px;
    padding-inline: 10px;
    border-block-end: 1px solid var(--separator);
  }
  .verbs.phone-row > .split {
    flex: 1 1 0;
    min-width: 0;
  }
  .verbs.phone-row .tbtn {
    position: relative;
    flex: 1 1 0;
    min-width: 44px;
    min-height: 44px;
    height: auto;
    padding: 2px 4px;
    flex-direction: column;
    gap: 2px;
    font-size: var(--fs-s);
  }
  .verbs.phone-row .vcount {
    position: absolute;
    inset-block-start: 2px;
    inset-inline-start: calc(50% + 8px);
    padding: 0 4px;
    border-radius: 999px;
    background: var(--accent-soft-strong);
    color: var(--accent-text);
    font-size: var(--fs-xs);
  }
  @media (pointer: coarse) {
    .verbs.sync .tbtn {
      min-block-size: 44px;
    }
  }
</style>
