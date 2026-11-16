<script lang="ts">
  // The pull request's agent thread, inside the PR workspace: under the PR
  // list and Files on a desktop (folded until first used, resizable,
  // maximisable over the workspace), a full-height sheet on a phone. One
  // Threads thread per PR (pr-agent-thread.ts): opening a PR looks its thread
  // up (an open one first) and shows it; with none, the embed is the Threads
  // start composer — the person picks the agent, model, effort and access,
  // then asks (or taps a suggestion). Asks also arrive from the ⋯ menu, the
  // Files panel and a diff selection (pr-agent-bridge); they go through the
  // embed's own send (`onhandle`), so the Threads store stays the one truth.
  // Each carries the PR's context (pr-agent-context.ts); Fix edits the clone,
  // and says so first. New thread (/new) closes this one and starts over, on
  // any agent.
  import { tick, untrack } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { FileDiff, PrDetail, RuntimeAccessMode, RuntimeSession } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import {
    loadIdeRuntimeSessionPort,
    loadRuntimeThreadView,
    openInThreads,
    type RuntimeEmbedDraft,
    type RuntimeEmbedHandle,
    type RuntimeHostCommands,
    type RuntimeSessionPort,
  } from '../../lib/runtimeThreadPort';
  import { git } from '../../lib/stores/git.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import { prDiffFileLoader } from '../../lib/diff/diff-load';
  import { prDetailCache, prKey } from './pr-cache';
  import { prAgentBridge, type PrAgentStatus } from './pr-agent-bridge.svelte';
  import { INLINE_CAP, buildAskMessage, prContextBlock, prThreadTitle, repoSlugOf, typedAskMessage, type PrAgentSend, type PrRepoCtx } from './pr-agent-context';
  import { PR_AGENT_SOURCE, findPrThread, prAgentOriginId, prAgentOriginLabel, prThreadStatus, rememberPrThread } from './pr-agent-thread';
  import { prFilesBridge, prFilesKey } from './prFilesBridge.svelte';

  interface Props {
    repoId: string;
    number: number;
    /** The PR's title as the tab strip knows it ('' before it loads). */
    title: string;
    /** dock = the left column; max = over the whole workspace; sheet = phone. */
    layout: 'dock' | 'max' | 'sheet';
    folded?: boolean;
    ontogglefolded?: () => void;
    onmaximize?: () => void;
    onrestore?: () => void;
    /** An ask arrived (any entry point): the host brings the panel forward. */
    onasked?: () => void;
    /** Phone, sheet closed: only the thread's embed, hidden, so its state still
     *  reaches the bridge (the opener's dot, the ⋯ hint). Takes no asks: the
     *  sheet's own panel does, once the ask opens it. */
    statusOnly?: boolean;
  }
  let { repoId, number, title, layout, folded = false, ontogglefolded, onmaximize, onrestore, onasked, statusOnly = false }: Props = $props();
  let disposed = false;
  $effect(() => () => { disposed = true; });
  const hidden = $derived((folded && layout === 'dock') || statusOnly);

  const ThreadView = lazyComponent(loadRuntimeThreadView);
  const originId = $derived(prAgentOriginId(repoId, number));
  const repo = $derived(git.allRepos.find((r) => r.id === repoId) ?? git.repos.find((r) => r.id === repoId) ?? null);
  // A repo of another workspace is only known once the full list is in.
  const repoPending = $derived(!repo && !git.allReposLoaded && !git.allReposError);
  $effect(() => {
    if (!repo && !git.allReposLoaded) untrack(() => void git.loadAllRepos().catch(() => {}));
  });
  const wsId = $derived(repo?.workspace_id ?? ws.currentId ?? null);
  const repoCtx = $derived<PrRepoCtx | null>(repo?.path ? { name: repo.name, slug: repoSlugOf(repo), path: repo.path } : null);
  const detachedId = $derived(prAgentBridge.detached[originId] ?? null);

  let port = $state.raw<RuntimeSessionPort | null>(null);
  type Found = { state: 'loading' } | { state: 'error'; message: string } | { state: 'ready'; thread: RuntimeSession | null };
  let found = $state.raw<Found>({ state: 'loading' });
  let attempt = $state(0);
  const thread = $derived(found.state === 'ready' ? found.thread : null);
  // The live row (the store follows reopen / close / status), else the lookup's.
  const row = $derived(thread ? (port?.runtimeThreads.get(thread.id) ?? thread) : null);
  let handle = $state.raw<RuntimeEmbedHandle | null>(null);
  let sending = $state(false);
  let sendError = $state('');
  let lastAsk: PrAgentSend | null = null;
  let root = $state<HTMLElement | null>(null);

  // The PR's thread, looked up once per PR (and on Retry), through the
  // Threads store.
  $effect(() => {
    const key = originId;
    const wid = wsId;
    void attempt;
    found = { state: 'loading' };
    if (!wid) {
      found = { state: 'ready', thread: null };
      return;
    }
    let live = true;
    loadIdeRuntimeSessionPort()
      .then((p) => {
        if (live) port = p;
        return findPrThread(p.runtimeThreads, wid, key);
      })
      .then(
        (t) => { if (live) found = { state: 'ready', thread: t }; },
        (e: unknown) => { if (live) found = { state: 'error', message: loadErrorText(e) }; },
      );
    return () => { live = false; };
  });

  // The PR itself, for the context every message carries.
  let pr = $state.raw<PrDetail | null>(null);
  $effect(() => {
    const key = prKey(repoId, number);
    const n = number;
    const id = repoId;
    const cached = prDetailCache.get(key)?.value ?? null;
    pr = cached;
    if (cached) return;
    let live = true;
    api.get<PrDetail>(`/repos/${id}/prs/${n}`).then((d) => { if (live) pr = d; }, () => {});
    return () => { live = false; };
  });

  // An ask for this PR runs once the lookup has answered, the repository is
  // known and the embed can send — never before (so it can't start a second
  // thread beside an existing one, or be dropped while the repo list loads).
  $effect(() => {
    const p = prAgentBridge.pending;
    if (statusOnly || !p || p.key !== originId || found.state !== 'ready' || sending) return;
    if (p.ask.kind === 'ask') {
      untrack(() => {
        prAgentBridge.take(originId);
        onasked?.();
        void focusAsk();
      });
      return;
    }
    if (detachedId) return untrack(() => prAgentBridge.reattach(originId));
    if (!repo?.path || !handle) return untrack(() => onasked?.());
    untrack(() => {
      const ask = prAgentBridge.take(originId);
      if (ask && ask.kind !== 'ask') void run(ask);
    });
  });

  async function focusAsk(): Promise<void> {
    await tick();
    root?.querySelector<HTMLTextAreaElement>('.pra-body textarea')?.focus();
  }

  async function prDetail(): Promise<PrDetail> {
    return pr ?? prDetailCache.get(prKey(repoId, number))?.value ?? api.get<PrDetail>(`/repos/${repoId}/prs/${number}`);
  }

  /** The picked files' diffs, as the Code tab loads them (up to the inline cap). */
  async function fileDiffs(paths: readonly string[]): Promise<FileDiff[]> {
    const feed = prFilesBridge.feed?.key === prFilesKey(repoId, number) ? prFilesBridge.feed : null;
    if (!feed) return [];
    const byPath = new Map(feed.files.map((f) => [f.path, f]));
    const load = prDiffFileLoader(repoId, number);
    const signal = new AbortController().signal;
    const picked = paths.map((p) => byPath.get(p)).filter((f): f is FileDiff => !!f).slice(0, INLINE_CAP.files);
    const got = await Promise.all(picked.map((f) =>
      f.hunks.length > 0 || f.is_binary ? Promise.resolve(f) : load(f, { full: false, signal }).catch(() => null)));
    return got.filter((f): f is FileDiff => !!f);
  }

  function confirmFix(pr: PrDetail, path: string, name: string): Promise<boolean> {
    return confirmer.ask(
      `The agent checks out ${pr.source_branch} in ${path} and edits files there to address the open review comments, then shows you the diff. Commit or stash your own changes in that folder first. It does not commit, push, or post to the pull request: you review the diff and apply it.`,
      { title: `Let the agent change files in ${name}?`, confirmLabel: 'Start fixing' },
    );
  }

  async function run(ask: PrAgentSend): Promise<void> {
    // The panel takes the ask itself (it may beat the host's own look at the
    // bridge): unfold it here, or the answer lands in a folded region.
    onasked?.();
    const r = repoCtx;
    const h = handle;
    if (!r || !h || sending) return;
    lastAsk = ask;
    sendError = '';
    sending = true;
    try {
      const detail = await prDetail();
      if (ask.kind === 'fix' && !(await confirmFix(detail, r.path, r.name))) return;
      const diffs = ask.kind === 'files' ? await fileDiffs(ask.paths) : [];
      const msg = buildAskMessage(ask, detail, r, diffs);
      await h.send(msg.text, { accessMode: msg.access });
      lastAsk = null;
    } catch (e) {
      if (!disposed) sendError = loadErrorText(e);
    } finally {
      if (!disposed) sending = false;
    }
  }

  // The thread's live state: a dot on the folded header and a hint on the
  // PR's ⋯ entry. Shown, the embed reports it (`onstatus`). Folded (or the
  // phone's closed sheet) nothing is mounted for it: the store's row, kept
  // live by its event listener, says enough (prThreadStatus).
  let live = $state<PrAgentStatus | null>(null);
  function onstatus(st: PrAgentStatus): void {
    live = st;
    prAgentBridge.setStatus(originId, st);
  }
  const quiet = $derived(hidden && !!thread && !detachedId);
  $effect(() => {
    const p = port;
    if (!quiet || !p) return;
    return untrack(() => p.runtimeThreads.mount());
  });
  $effect(() => {
    if (!quiet) return;
    const st = prThreadStatus(row);
    if (st) untrack(() => onstatus(st));
  });
  $effect(() => {
    const key = originId;
    return () => prAgentBridge.setStatus(key, null);
  });
  const dot = $derived(live === 'working' ? 'The agent is working' : live === 'needs-you' ? 'The agent needs you' : live === 'background' ? 'Background task running' : '');

  function detach(id: string): void {
    prAgentBridge.detach(originId, id);
    openInThreads(id);
  }

  // New thread (/new, the embed bar's +): this one closes (it stays in
  // Threads' History) and the start composer comes back, where any agent can
  // be picked. The lookup then finds the new one (an open thread wins).
  async function newThread(id: string): Promise<void> {
    const p = port;
    if (!p) return;
    const cur = p.runtimeThreads.get(id);
    const busy = cur && (cur.status === 'working' || cur.status === 'starting' || cur.status === 'awaiting_permission');
    if (busy && !(await confirmer.ask('The agent is still working on this thread. Starting a new one stops it; the thread stays in Threads › History.', { title: 'Start a new thread for this PR?', confirmLabel: 'New thread' }))) return;
    try {
      if (cur?.status !== 'closed') await p.runtimeThreads.close(id);
    } catch (e) {
      toastError('Couldn’t close the thread', e);
      return;
    }
    if (!disposed) found = { state: 'ready', thread: null };
  }
  const hostCommands: RuntimeHostCommands = { newThread: (id) => void newThread(id) };

  function onstarted(session: RuntimeSession): void {
    rememberPrThread(originId, session.id);
    if (!disposed) found = { state: 'ready', thread: session };
  }

  async function messageFor(ask: PrAgentSend): Promise<string> {
    const r = repoCtx;
    if (!r) throw new Error('This repository has no local copy.');
    return buildAskMessage(ask, await prDetail(), r).text;
  }

  function confirmAccess(m: RuntimeAccessMode): Promise<boolean> {
    const path = repoCtx?.path ?? 'the clone';
    return m === 'full-access'
      ? Promise.resolve(true) // the chip asks for full access itself
      : confirmer.ask(`From the first message on, the agent changes files in ${path} without asking first. It still asks before other commands. Commit or stash your own changes there first.`, { title: 'Let the agent change files?', confirmLabel: 'Allow edits' });
  }

  const SUGGESTIONS: { label: string; ask: PrAgentSend }[] = [
    { label: 'Explain this PR', ask: { kind: 'explain' } },
    { label: 'What could this break?', ask: { kind: 'question', text: 'What could this change break? Look for risky logic, edge cases and the callers it affects.' } },
    { label: 'Which tests are missing?', ask: { kind: 'question', text: 'Which behavior does this change leave untested? Name the tests you would add, and where.' } },
  ];

  const draft = $derived.by((): RuntimeEmbedDraft | undefined => {
    const r = repoCtx;
    if (!wsId || !r) return undefined;
    return {
      workspaceId: wsId,
      cwd: r.path,
      title: pr ? prThreadTitle(pr) : `PR #${number}${title ? ` · ${title}` : ''}`,
      source: PR_AGENT_SOURCE,
      origin: { kind: 'pr', id: originId, label: prAgentOriginLabel(r.name, number, pr?.title ?? title) },
      defaultAccess: 'approval-required',
      confirmAccess,
      suggestions: SUGGESTIONS.map((s) => ({ label: s.label, prompt: () => messageFor(s.ask) })),
      placeholder: 'Ask about this pull request…',
      lead,
      transformFirstPrompt: async (text) => typedAskMessage(text, await prDetail(), r),
      onstarted,
    };
  });
  // After /clear the agent starts from nothing: the next message names the PR again.
  const contextOnFresh = (): string | null => (pr && repoCtx ? prContextBlock(pr, repoCtx) : null);

  // Phone sheet: keep the composer above the on-screen keyboard (the sheet is
  // fixed to the layout viewport, which the keyboard does not shrink).
  let kb = $state(0);
  $effect(() => {
    const vv = window.visualViewport;
    if (layout !== 'sheet' || statusOnly || !vv) return;
    const sync = (): void => { kb = Math.max(0, Math.round(window.innerHeight - vv.height - vv.offsetTop)); };
    sync();
    vv.addEventListener('resize', sync);
    vv.addEventListener('scroll', sync);
    return () => {
      vv.removeEventListener('resize', sync);
      vv.removeEventListener('scroll', sync);
    };
  });
</script>

{#snippet lead()}
  Ask about this pull request. The agent reads the code in <span class="mono" dir="ltr">{repoCtx?.path}</span> and answers here, beside the diff.
{/snippet}

<section
  bind:this={root}
  class="pra pra-{layout}"
  class:folded={folded && layout === 'dock'}
  style={kb ? `padding-block-end:${kb}px` : undefined}
  aria-label="Agent thread for pull request #{number}"
  data-testid={statusOnly ? 'pr-agent-status' : 'pr-agent-panel'}
>
  {#if layout !== 'sheet'}
    <div class="pra-head">
      {#if layout === 'max'}
        <button class="btn small" onclick={onrestore} title="Back to the pull request" data-testid="pr-agent-restore">
          <Icon name="chevronLeft" size={12} /> Back to the PR
        </button>
      {:else}
        <button class="pra-title" aria-expanded={!folded} title={folded ? 'Show the agent thread' : 'Hide the agent thread'} onclick={ontogglefolded}>
          <Icon name={folded ? 'chevronRight' : 'chevronDown'} size={12} />
          <span>Agent</span>
        </button>
      {/if}
      <span class="pra-pr" dir="auto" title={title ? `#${number} ${title}` : `#${number}`}>#{number}{title ? ` ${title}` : ''}</span>
      {#if row?.status === 'closed'}<span class="pra-tag">History</span>{/if}
      {#if sending}<span class="spinner" aria-hidden="true"></span>{/if}
      {#if dot && (hidden || layout !== 'dock')}<span class="pra-dot" class:needs={live === 'needs-you'} class:calm={live === 'background'} role="img" aria-label={dot} title={dot}></span>{/if}
    </div>
  {/if}

  <!-- Folded (or the phone's closed sheet): not rendered; the dot comes from
       the thread's row (see `quiet`). -->
  {#if !hidden}
    <div class="pra-body">
      {#if sendError}
        <LoadState what="the ask" errorTitle="Couldn’t send it to the agent" error={sendError} variant="compact"
          onretry={() => { const a = lastAsk; if (a) void run(a); else sendError = ''; }} />
      {/if}
      {#if found.state === 'loading'}
        <LoadState what="this pull request’s agent thread" loading empty variant="compact" rows={3} />
      {:else if found.state === 'error'}
        <LoadState what="this pull request’s agent thread" error={found.message} empty onretry={() => (attempt += 1)} />
      {:else if detachedId}
        <EmptyState icon="external" title="Open in Threads" body="This pull request’s thread is open in Threads. Bring it back to read it beside the diff."
          actionLabel="Open here again" actionKind="secondary" onaction={() => prAgentBridge.reattach(originId)} />
      {:else if !thread && repoPending}
        <LoadState what="the repository" loading empty variant="compact" rows={2} />
      {:else if !thread && !draft}
        <EmptyState icon="folder" title="No local copy of this repository"
          body="The agent reads the code from your clone. Add the repository’s folder in Git, then ask again." />
      {:else}
        {#if row?.status === 'closed'}
          <p class="pra-note">Closed after a while without activity. Send a message to pick it up again.</p>
        {/if}
        <div class="pra-thread">
          {#if ThreadView.error}
            <LoadState what="the thread" error={ThreadView.error} onretry={() => ThreadView.retry()} />
          {:else if ThreadView.component}
            <ThreadView.component {onstatus} sessionId={thread?.id ?? null} workspaceId={thread?.workspace_id ?? wsId ?? ''} compact
              {draft} {hostCommands} {contextOnFresh} onhandle={(h) => (handle = h)}
              onmaximize={thread && layout === 'dock' ? onmaximize : undefined} ondetach={detach} />
          {:else}
            <LoadState what="the thread" loading empty rows={3} />
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</section>

<style>
  .pra {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    height: 100%;
    background: var(--bg);
  }
  .pra-dock { border-block-start: 1px solid var(--separator); }
  /* The phone sheet (DockedDrawer in a Drawer): its body is a plain block, so
     give the panel's wrapper the drawer's height — the start composer then
     sits at the bottom of the sheet, like a conversation's. */
  :global(.drawer-body > .dd-body:has(> .pra-sheet)) { height: 100%; }
  .pra.folded { height: auto; }
  .pra-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding-block: 6px;
    padding-inline: 6px 8px;
    min-width: 0;
  }
  .pra-max .pra-head { padding-inline: 8px; border-block-end: 1px solid var(--separator); }
  .pra-title {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: none;
    background: none;
    border: none;
    color: var(--text);
    font-size: var(--fs-s);
    font-weight: 600;
    padding: 2px 4px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .pra-title:hover { background: var(--hover); }
  .pra-pr {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .pra-tag { flex: none; font-size: var(--fs-xs); color: var(--text-dim); }
  .pra-dot {
    flex: none;
    inline-size: 8px;
    block-size: 8px;
    border-radius: 50%;
    background: var(--accent-solid);
    animation: otto-pulse 1.6s ease-in-out infinite;
  }
  .pra-dot.needs { background: var(--warning); animation: none; }
  .pra-dot.calm { background: var(--text-dim); animation: none; }
  @media (prefers-reduced-motion: reduce) { .pra-dot { animation: none; } }
  .pra-body { flex: 1; min-height: 0; display: flex; flex-direction: column; overflow-y: auto; }
  .pra-thread { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .pra-note { flex: none; margin: 0; padding: 6px 12px; font-size: var(--fs-xs); color: var(--text-dim); }
</style>
