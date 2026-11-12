<script lang="ts">
  // The pull request's agent thread, inside the PR workspace: under the PR
  // list and Files on a desktop (folded until first used, resizable,
  // maximisable over the workspace), a full-height sheet on a phone. One
  // Threads thread per PR (pr-agent-thread.ts): opening a PR looks its thread
  // up — a closed one shows as history that a new message picks up again —
  // and nothing is created until the person asks something. Asks arrive from
  // the ⋯ menu, the Files panel and a diff selection (pr-agent-bridge) or the
  // ask box here; each carries the PR's context (pr-agent-context.ts).
  // Questions run read-only; Fix edits the clone, and says so first.
  import { tick, untrack } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { FileDiff, PrDetail, RuntimeSession } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { agentProviders, defaultAgentProvider, harnessLabel, providerReadiness } from '../../lib/providers';
  import { router } from '../../lib/router.svelte';
  import { loadRuntimeThreadView, openInThreads } from '../../lib/runtimeThreadPort';
  import { git } from '../../lib/stores/git.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { prDiffFileLoader } from './diff-load';
  import { prDetailCache, prKey } from './pr-cache';
  import { prAgentBridge, type PrAgentStatus } from './pr-agent-bridge.svelte';
  import { INLINE_CAP, buildAskMessage, repoSlugOf, type PrAgentAsk, type PrAgentSend } from './pr-agent-context';
  import { PR_AGENT_SOURCE, createPrThread, findPrThread, pickRuntimeProvider, prAgentOriginId, prAgentOriginLabel, sendToPrThread } from './pr-agent-thread';
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
  const wsId = $derived(repo?.workspace_id ?? ws.currentId ?? null);
  const provider = $derived(pickRuntimeProvider(defaultAgentProvider(), agentProviders(), (p) => providerReadiness(p).available));
  const detachedId = $derived(prAgentBridge.detached[originId] ?? null);

  type Found = { state: 'loading' } | { state: 'error'; message: string } | { state: 'ready'; thread: RuntimeSession | null };
  let found = $state.raw<Found>({ state: 'loading' });
  let attempt = $state(0);
  const thread = $derived(found.state === 'ready' ? found.thread : null);
  let sending = $state(false);
  let sendError = $state('');
  let lastAsk: PrAgentSend | null = null;
  let draft = $state('');
  let root = $state<HTMLElement | null>(null);

  // The PR's thread, looked up once per PR (and on Retry).
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
    findPrThread(wid, key).then(
      (t) => { if (live) found = { state: 'ready', thread: t }; },
      (e: unknown) => { if (live) found = { state: 'error', message: loadErrorText(e) }; },
    );
    return () => { live = false; };
  });

  // An ask for this PR runs once the lookup has answered — never before, so
  // it can't start a second thread beside an existing one (and after a send
  // in flight, which may be creating it).
  $effect(() => {
    const p = prAgentBridge.pending;
    if (statusOnly || !p || p.key !== originId || found.state !== 'ready' || sending) return;
    untrack(() => {
      const ask = prAgentBridge.take(originId);
      if (ask) void run(ask);
    });
  });

  async function focusAsk(): Promise<void> {
    await tick();
    root?.querySelector<HTMLTextAreaElement>('.pra-body textarea')?.focus();
  }

  async function prDetail(): Promise<PrDetail> {
    return prDetailCache.get(prKey(repoId, number))?.value ?? api.get<PrDetail>(`/repos/${repoId}/prs/${number}`);
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

  async function run(ask: PrAgentAsk): Promise<void> {
    // The panel takes the ask itself (it may beat the host's own look at the
    // bridge): unfold it here, or the answer lands in a folded region.
    onasked?.();
    if (ask.kind === 'ask') return focusAsk();
    prAgentBridge.reattach(originId);
    const r = repo;
    const target0 = thread;
    // The designed states below say what to set up; nothing is sent.
    if (!r?.path || (!target0 && !provider) || sending) return;
    lastAsk = ask;
    sendError = '';
    sending = true;
    try {
      const pr = await prDetail();
      if (ask.kind === 'fix' && !(await confirmFix(pr, r.path, r.name))) return;
      const diffs = ask.kind === 'files' ? await fileDiffs(ask.paths) : [];
      const msg = buildAskMessage(ask, pr, { name: r.name, slug: repoSlugOf(r), path: r.path }, diffs);
      let target = target0;
      if (!target) {
        if (!wsId || !provider) return;
        target = await createPrThread(wsId, {
          provider,
          cwd: r.path,
          title: msg.title,
          access_mode: msg.access,
          source: PR_AGENT_SOURCE,
          origin: { kind: 'pr', id: originId, label: prAgentOriginLabel(r.name, pr.number, pr.title) },
        });
        if (disposed) return;
        found = { state: 'ready', thread: target };
      }
      await sendToPrThread(target, msg.text, msg.access);
      if (!disposed && target.status === 'closed') found = { state: 'ready', thread: { ...target, status: 'idle' } };
      lastAsk = null;
      draft = '';
    } catch (e) {
      if (!disposed) sendError = loadErrorText(e);
    } finally {
      if (!disposed) sending = false;
    }
  }

  function askBox(): void {
    const text = draft.trim();
    if (text) void run({ kind: 'question', text });
  }

  // The thread's live state, reported by the embed (`onstatus`): a dot on
  // the folded header and a hint on the PR's ⋯ entry. The embed stays mounted
  // (hidden) while folded so it keeps reporting.
  let live = $state<PrAgentStatus | null>(null);
  function onstatus(st: PrAgentStatus): void {
    live = st;
    prAgentBridge.setStatus(originId, st);
  }
  $effect(() => {
    const key = originId;
    return () => prAgentBridge.setStatus(key, null);
  });
  const dot = $derived(live === 'working' ? 'The agent is working' : live === 'needs-you' ? 'The agent needs you' : '');

  function detach(id: string): void {
    prAgentBridge.detach(originId, id);
    openInThreads(id);
  }

  const SUGGESTIONS: { label: string; ask: PrAgentSend }[] = [
    { label: 'Explain this PR', ask: { kind: 'explain' } },
    { label: 'What could this break?', ask: { kind: 'question', text: 'What could this change break? Look for risky logic, edge cases and the callers it affects.' } },
    { label: 'Which tests are missing?', ask: { kind: 'question', text: 'Which behavior does this change leave untested? Name the tests you would add, and where.' } },
  ];

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
      {#if thread?.status === 'closed'}<span class="pra-tag">History</span>{/if}
      {#if sending}<span class="spinner" aria-hidden="true"></span>{/if}
      {#if dot && (hidden || layout !== 'dock')}<span class="pra-dot" class:needs={live === 'needs-you'} role="img" aria-label={dot} title={dot}></span>{/if}
    </div>
  {/if}

  <!-- Folded: the body stays mounted but hidden when a thread is open, so its
       embed keeps reporting the agent's state; otherwise it is not rendered. -->
  {#if !hidden || (thread && !detachedId)}
    <div class="pra-body" hidden={hidden}>
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
      {:else if thread}
        {#if thread.status === 'closed'}
          <p class="pra-note">Closed after a while without activity. Send a message to pick it up again.</p>
        {/if}
        <div class="pra-thread">
          {#if ThreadView.error}
            <LoadState what="the thread" error={ThreadView.error} onretry={() => ThreadView.retry()} />
          {:else if ThreadView.component}
            <ThreadView.component {onstatus} sessionId={thread.id} workspaceId={thread.workspace_id} compact
              onmaximize={layout === 'dock' ? onmaximize : undefined} ondetach={detach} />
          {:else}
            <LoadState what="the thread" loading empty rows={3} />
          {/if}
        </div>
      {:else if !repo?.path}
        <EmptyState icon="folder" title="No local copy of this repository"
          body="The agent reads the code from your clone. Add the repository’s folder in Git, then ask again." />
      {:else if !provider}
        <EmptyState icon="sparkle" title="No agent can run here yet"
          body="A pull request’s thread runs on Claude Code, Codex, Gemini, Grok, opencode or pi. Install one, or set it up in Settings › Providers."
          actionLabel="Open Providers" actionKind="secondary" onaction={() => router.go('settings/providers')} />
      {:else}
        <div class="pra-intro">
          <p class="pra-lead">Ask about this pull request. The agent reads the code in <span class="mono" dir="ltr">{repo.path}</span> and answers here, beside the diff.</p>
          <div class="pra-suggest">
            {#each SUGGESTIONS as s (s.label)}
              <button class="btn small pra-btn" disabled={sending} onclick={() => void run(s.ask)}>{s.label}</button>
            {/each}
          </div>
          <textarea dir="auto" class="input pra-ask" rows="3" bind:value={draft} disabled={sending}
            aria-label="Ask about this pull request" placeholder="Ask about this pull request…"
            onkeydown={(e) => { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); askBox(); } }}></textarea>
          <div class="pra-foot">
            <span class="pra-hint">{harnessLabel(provider)} · read-only: it can’t change files or post to the PR</span>
            <button class="btn small primary pra-btn" disabled={sending || draft.trim() === ''} onclick={askBox}>
              {sending ? 'Starting…' : 'Ask'}
            </button>
          </div>
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
  .pra-body[hidden] { display: none; }
  .pra-dot {
    flex: none;
    inline-size: 8px;
    block-size: 8px;
    border-radius: 50%;
    background: var(--accent-solid);
    animation: otto-pulse 1.6s ease-in-out infinite;
  }
  .pra-dot.needs { background: var(--warning); animation: none; }
  @media (prefers-reduced-motion: reduce) { .pra-dot { animation: none; } }
  .pra-body { flex: 1; min-height: 0; display: flex; flex-direction: column; overflow-y: auto; }
  .pra-thread { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .pra-note { flex: none; margin: 0; padding: 6px 12px; font-size: var(--fs-xs); color: var(--text-dim); }
  .pra-intro { display: flex; flex-direction: column; gap: 10px; padding: 8px 12px 12px; }
  .pra-lead { margin: 0; font-size: var(--fs-s); color: var(--text-dim); overflow-wrap: anywhere; }
  .pra-suggest { display: flex; flex-wrap: wrap; gap: 6px; }
  .pra-ask { width: 100%; resize: vertical; min-height: 64px; }
  .pra-foot { display: flex; align-items: center; justify-content: flex-end; flex-wrap: wrap; gap: 8px; }
  .pra-hint { flex: 1 1 160px; min-width: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  @media (max-width: 1024px) {
    .pra-btn { min-height: var(--hit-min); }
    .pra-ask { font-size: 16px; } /* ui-guards: allow — 16px stops iOS zoom-on-focus */
  }
</style>
