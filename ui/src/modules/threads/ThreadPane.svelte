<script lang="ts">
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import LazyTerminal from '../../lib/components/LazyTerminal.svelte'; // xterm stays out of page:threads until a terminal shows
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { events } from '../../lib/events.svelte';
  import { focusOnMount } from '../../lib/focusOnMount';
  import { resolveSessionThreadConfig, type InteractionMode } from '../../lib/providerModels';
  import { sessionState } from '../../lib/status';
  import { git } from '../../lib/stores/git.svelte';
  import { transcript } from '../../lib/stores/transcript.svelte';
  import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import ConversationView from '../agents/conversation/ConversationView.svelte'; // ui-guards: allow — experimental threads embeds shared ConversationView
  import { buildThreadMenu } from './threadMenu';
  import ThreadTerminalDrawer from './ThreadTerminalDrawer.svelte';
  import { harnessLabel, isPlainShellSession, isThreadSession, sessionBranch } from './threadsHelpers';
  import { threadRepos } from './threadRepos.svelte';
  import { threads } from './threadsStore.svelte';

  interface Props {
    paneId: string;
    focused: boolean;
    multiPane: boolean;
    onstartroom: (sessionId: string) => void;
  }
  let { paneId, focused, multiPane, onstartroom }: Props = $props();

  const session = $derived(ws.getSession(paneId) ?? null);
  const rawStatus = $derived(ws.statusMap[paneId] ?? session?.status ?? 'idle');
  const needsYou = $derived(ws.needsYou[paneId] === true);
  const paneState = $derived(sessionState(session, rawStatus, needsYou, { stale: events.state !== 'connected' }));
  // Branch for the composer meta row too: metadata / registered repo, else the
  // repo detected behind the thread's folder.
  const detectedRepo = $derived(session ? threadRepos.repoFor(session.cwd, session.workspace_id) : null);
  $effect(() => {
    if (detectedRepo) git.ensureStatus(detectedRepo.id);
  });
  const detectedBranch = $derived.by((): string | null => {
    if (!detectedRepo) return null;
    return git.statusById[detectedRepo.id]?.branch || null;
  });
  const branch = $derived(detectedBranch ?? sessionBranch(session, git.repos, git.statusById));

  let convRef = $state<ReturnType<typeof ConversationView> | null>(null);
  let renaming = $state(false);
  let draftTitle = $state('');
  // Terminal drawer + peer bar live in the store so the page's top bar can
  // drive the focused pane (single view has no pane header of its own).
  const showTerminal = $derived(threads.isTerminalOpen(paneId));
  const showPeerBar = $derived(threads.peerBarFor === paneId);
  const toggleTerminal = (): void => threads.toggleTerminal(paneId);
  let peerTarget = $state('broadcast');
  let peerText = $state('');
  let peerSending = $state(false);

  const peerTargets = $derived(
    ws.sessions.filter((s) => !s.archived && s.id !== paneId && isThreadSession(s)),
  );

  // The top bar's "Search conversation" asks the focused pane.
  let lastSearchN = 0;
  $effect(() => {
    const req = threads.searchRequest;
    if (req && req.id === paneId && req.n !== lastSearchN) {
      lastSearchN = req.n;
      convRef?.triggerSearch();
    }
  });

  $effect(() => {
    if (showPeerBar && peerTargets.length > 0 && peerTarget === 'broadcast') {
      peerTarget = peerTargets[0].id;
    }
  });

  $effect(() => {
    if (showPeerBar && peerTarget !== 'broadcast' && !peerTargets.some((t) => t.id === peerTarget)) {
      peerTarget = peerTargets[0]?.id ?? 'broadcast';
    }
  });

  function beginRename(): void {
    if (!session) return;
    draftTitle = session.title;
    renaming = true;
  }

  async function commitRename(): Promise<void> {
    if (!renaming) return;
    renaming = false;
    const next = draftTitle.trim();
    if (!session || !next || next === session.title) return;
    try {
      await ws.renameSession(paneId, next);
    } catch (e) {
      toastError('Couldn’t rename thread', e);
    }
  }

  function onRenameKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      void commitRename();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      renaming = false;
    }
  }

  async function submitInlinePeer(): Promise<void> {
    if (peerSending || !peerText.trim()) return;
    peerSending = true;
    try {
      const ok = await threads.sendPeerMessage({
        fromSessionId: paneId,
        target: peerTarget,
        text: peerText,
        workspaceId: session?.workspace_id ?? ws.currentId,
      });
      if (ok) {
        peerText = '';
        if (threads.peerBarFor === paneId) threads.peerBarFor = null;
      }
    } finally {
      peerSending = false;
    }
  }

  async function spawnFollowupThread(opts: {
    beside: boolean;
    prompt?: string;
    mode?: InteractionMode;
  }): Promise<void> {
    if (!session) return;
    if (!opts.prompt && !opts.beside) {
      threads.startNewThread({ workspaceId: session.workspace_id, cwd: session.cwd });
      return;
    }
    try {
      const cfg = resolveSessionThreadConfig(session, null);
      const meta: Record<string, unknown> = {
        surface: 'threads',
        interaction_mode: opts.mode ?? cfg.interactionMode,
        ...(cfg.pinnedModel ? { model: cfg.pinnedModel } : {}),
        ...(cfg.effort ? { effort: cfg.effort } : {}),
      };
      const scratchOpts = {
        scratch: session.workspace_id === SCRATCH_WORKSPACE_ID && ws.currentId !== SCRATCH_WORKSPACE_ID,
      };
      const firstMsg = opts.prompt?.trim() ?? '';
      const created = firstMsg
        ? await ws.openSessionWithPrompt(
            {
              provider: session.provider,
              title: firstMsg.replace(/\s+/g, ' ').slice(0, 56),
              cwd: session.cwd,
              prompt: firstMsg,
              meta,
            },
            scratchOpts,
          )
        : await ws.createSessionQuiet(
            {
              kind: 'agent',
              provider: session.provider,
              title: `${harnessLabel(session.provider)} thread`,
              cwd: session.cwd,
              meta,
            },
            scratchOpts,
          );
      if (firstMsg) {
        transcript.recordOptimisticPrompt(created.id, firstMsg);
      }
      if (opts.beside) threads.openBeside(created.id);
      else threads.selectThread(created.id);
    } catch (e) {
      toastError('Couldn’t start thread', e);
    }
  }

  function openPaneMenu(e: MouseEvent): void {
    if (session) ctxMenu.show(e, buildThreadMenu(session, { onstartroom }));
  }
  function paneFocus(node: HTMLElement): { destroy(): void } {
    const mark = (): void => {
      if (!focused) threads.focusPane(paneId);
    };
    node.addEventListener('pointerdown', mark);
    node.addEventListener('focusin', mark);
    return {
      destroy() {
        node.removeEventListener('pointerdown', mark);
        node.removeEventListener('focusin', mark);
      },
    };
  }
</script>

<article
  class="tp-pane"
  class:focused={focused && multiPane}
  aria-label={session ? `Thread ${session.title}` : 'Thread pane'}
  data-testid="thread-pane-{paneId}"
  use:paneFocus
>
  {#if multiPane}
    <header class="tp-head">
      <div class="tp-lead">
        <StatusDot state={paneState} />
        {#if session && hasProviderIcon(session.provider)}
          <ProviderIcon provider={session.provider} size={14} />
        {/if}
        {#if renaming}
          <input
            class="tp-rename-input"
            dir="auto"
            aria-label="Thread title"
            bind:value={draftTitle}
            use:focusOnMount
            onblur={() => void commitRename()}
            onkeydown={onRenameKey}
          />
        {:else}
          <button
            type="button"
            class="tp-title ellipsis"
            dir="auto"
            title="{session?.title ?? 'Thread'}{branch ? ` · ${branch}` : ''} — double-click to rename"
            ondblclick={beginRename}
          >
            {session?.title ?? 'Loading thread…'}
          </button>
        {/if}
      </div>
      <div class="tp-actions">
        {#if !isPlainShellSession(session)}
          <button
            type="button"
            class="icon-btn"
            class:active={showTerminal}
            aria-pressed={showTerminal}
            aria-label={showTerminal ? 'Hide terminal drawer' : 'Show terminal drawer'}
            title="Terminal drawer"
            data-testid="pane-terminal-btn-{paneId}"
            onclick={toggleTerminal}
          >
            <Icon name="terminal" size={13} />
          </button>
        {/if}
        <button
          type="button"
          class="icon-btn"
          aria-label="More thread actions"
          title="More actions"
          aria-haspopup="menu"
          onclick={openPaneMenu}
        >
          <Icon name="more" size={13} />
        </button>
        <button
          type="button"
          class="icon-btn"
          aria-label="Close pane from split"
          title="Close from split (keeps thread running)"
          data-testid="pane-close-{paneId}"
          onclick={() => threads.closePane(paneId)}
        >
          <Icon name="x" size={13} />
        </button>
      </div>
    </header>
  {/if}

  {#if showPeerBar}
    <div class="tp-peer-bar" data-testid="pane-peer-bar-{paneId}">
      <span class="tp-peer-label">To:</span>
      <select
        class="input tp-peer-select"
        aria-label="Target thread for peer message"
        bind:value={peerTarget}
      >
        {#each peerTargets as t (t.id)}
          <option value={t.id}>{t.title}</option>
        {/each}
        <option value="broadcast">Broadcast to all active threads</option>
      </select>
      <input
        type="text"
        dir="auto"
        class="input tp-peer-input"
        aria-label="Message to send to peer thread"
        placeholder="Relay message from this thread…"
        bind:value={peerText}
        use:focusOnMount
        onkeydown={(e) => {
          if (e.key === 'Enter' && !e.isComposing) {
            e.preventDefault();
            void submitInlinePeer();
          } else if (e.key === 'Escape') {
            threads.peerBarFor = null;
          }
        }}
      />
      <button
        type="button"
        class="btn small primary"
        disabled={peerSending || !peerText.trim()}
        onclick={() => void submitInlinePeer()}
      >
        Send
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Close peer relay bar"
        title="Close"
        onclick={() => (threads.peerBarFor = null)}
      >
        <Icon name="x" size={12} />
      </button>
    </div>
  {/if}

  <div class="tp-body">
    {#if !session}
      <EmptyState
        variant="panel"
        icon="threads"
        title="Thread unavailable"
        body="This session is no longer loaded in the workspace."
      >
        <button type="button" class="btn small" onclick={() => threads.closePane(paneId)}>
          Close pane
        </button>
      </EmptyState>
    {:else if isPlainShellSession(session)}
      <div class="tp-shell-wrap otto-force-dark">
        <LazyTerminal sessionId={paneId} forceDark autoFocus={focused} resumeOnOpen={focused} />
      </div>
    {:else}
      <ConversationView
        bind:this={convRef}
        sessionId={paneId}
        workspaceId={session.workspace_id}
        resumeOnOpen={focused}
        surface="threads"
        ontoggleterminal={toggleTerminal}
        onopeninspector={(tab) => threads.setInspectorTab(tab)}
        onnewthread={(opts) => void spawnFollowupThread(opts)}
        onsidechat={(q) => threads.openSideChat(paneId, q)}
        branchHint={branch}
      />
    {/if}
  </div>

  {#if showTerminal && session && !isPlainShellSession(session)}
    <ThreadTerminalDrawer
      sessionId={paneId}
      title={session.title}
      onclose={toggleTerminal}
    />
  {/if}
</article>

<style>
  .tp-pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .tp-pane.focused {
    border-color: var(--accent-line-strong);
  }
  .tp-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    block-size: 36px;
    padding-inline: 10px;
    background: var(--surface);
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .tp-lead {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    flex: 1;
  }
  .tp-title {
    border: 0;
    padding: 2px 4px;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 600;
    text-align: start;
    cursor: pointer;
    min-width: 0;
    max-width: 320px;
  }
  .tp-title:hover {
    background: var(--hover);
  }
  .tp-rename-input {
    min-width: 120px;
    max-width: 240px;
    padding: 2px 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--accent-line-strong);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    outline: none;
  }
  .tp-rename-input:focus {
    border-color: var(--accent-line-strong);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }
  .tp-actions {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }
  .tp-actions :global(.icon-btn.active) {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .tp-peer-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: var(--surface-2);
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .tp-peer-label {
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .tp-peer-select {
    max-width: 180px;
    font-size: var(--fs-xs);
    padding-block: 4px;
  }
  .tp-peer-input {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-s);
    padding-block: 4px;
  }
  .tp-body {
    flex: 1;
    min-height: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .tp-shell-wrap {
    flex: 1;
    min-height: 0;
    min-width: 0;
    display: flex;
  }
</style>
