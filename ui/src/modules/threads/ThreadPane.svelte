<script lang="ts">
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { events } from '../../lib/events.svelte';
  import { focusOnMount } from '../../lib/focusOnMount';
  import { badgeTone, sessionState } from '../../lib/status';
  import { git } from '../../lib/stores/git.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import ConversationView from '../agents/conversation/ConversationView.svelte'; // ui-guards: allow — experimental threads embeds shared ConversationView
  import ThreadTerminalDrawer from './ThreadTerminalDrawer.svelte';
  import { delegatedParentId, isPlainShellSession, isThreadSession, sessionBranch } from './threadsHelpers';
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
  const pinned = $derived(threads.isPinned(paneId));
  const branch = $derived(sessionBranch(session, git.repos, git.statusById));
  const parentId = $derived(delegatedParentId(session));
  const parentSession = $derived(parentId ? (ws.getSession(parentId) ?? null) : null);

  let renaming = $state(false);
  let draftTitle = $state('');
  let showTerminal = $state(false);
  let showPeerBar = $state(false);
  let peerTarget = $state('broadcast');
  let peerText = $state('');
  let peerSending = $state(false);

  const peerTargets = $derived(
    ws.sessions.filter((s) => !s.archived && s.id !== paneId && isThreadSession(s)),
  );

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
        showPeerBar = false;
      }
    } finally {
      peerSending = false;
    }
  }

  function openPaneMenu(e: MouseEvent): void {
    if (!session) return;
    const items: MenuItem[] = [
      { label: 'Rename thread…', icon: 'edit', action: beginRename },
      { label: pinned ? 'Unpin thread' : 'Pin thread', icon: 'star', action: () => threads.togglePin(paneId) },
      { label: 'Send message to peer…', icon: 'send', action: () => (showPeerBar = !showPeerBar) },
      { label: showTerminal ? 'Hide terminal drawer' : 'Open terminal drawer', icon: 'terminal', action: () => (showTerminal = !showTerminal) },
      { label: 'Session room…', icon: 'people', action: () => onstartroom(paneId) },
      ...(parentId
        ? [
            {
              label: 'Open with parent in Split',
              icon: 'split',
              action: () => threads.openPairInSplit(parentId, paneId),
            },
          ]
        : []),
      { separator: true },
      {
        label: threads.isSettled(paneId) ? 'Move back to Active' : 'Settle to History',
        icon: 'check',
        action: () => threads.toggleSettle(paneId),
      },
      {
        label: 'Archive',
        icon: 'archive',
        action: () => {
          void ws.requestArchive(paneId).then((ok) => {
            if (ok) threads.closePane(paneId);
          });
        },
      },
    ];
    ctxMenu.show(e, items);
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
  <header class="tp-head">
    <div class="tp-lead">
      {#if session && hasProviderIcon(session.provider)}
        <ProviderIcon provider={session.provider} size={14} />
      {:else}
        <Icon name="terminal" size={14} />
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
          title="Double-click to rename thread"
          ondblclick={beginRename}
        >
          {session?.title ?? 'Loading thread…'}
        </button>
      {/if}

      <StatusDot state={paneState} />
      {#if !multiPane}
        <Badge tone={badgeTone(paneState.tone)} variant="outline" label={paneState.label} />
      {/if}

      {#if branch && !multiPane}
        <span class="tp-branch mono ellipsis" dir="ltr" title="Git branch: {branch}">
          <Icon name="branch" size={12} />
          {branch}
        </span>
      {/if}

      {#if parentId && !multiPane}
        <button
          type="button"
          class="chip tp-parent-link"
          title="Open parent and worker side-by-side in Split"
          onclick={() => threads.openPairInSplit(parentId, paneId)}
        >
          <Icon name="merge" size={12} />
          <span class="ellipsis">Delegated by {parentSession?.title ?? 'parent'}</span>
        </button>
      {/if}
    </div>

    <div class="tp-actions">
      <button
        type="button"
        class="icon-btn"
        class:tp-pin-on={pinned}
        aria-label={pinned ? 'Unpin thread' : 'Pin thread'}
        title={pinned ? 'Unpin thread' : 'Pin thread'}
        onclick={() => threads.togglePin(paneId)}
      >
        <Icon name="star" size={13} />
      </button>

      {#if !multiPane}
        <button
          type="button"
          class="icon-btn"
          class:active={showPeerBar}
          aria-pressed={showPeerBar}
          aria-label="Send message to peer thread"
          title="Send to peer…"
          data-testid="pane-peer-btn-{paneId}"
          onclick={() => {
            showPeerBar = !showPeerBar;
            if (showPeerBar && peerTargets.length > 0 && peerTarget === 'broadcast') {
              peerTarget = peerTargets[0].id;
            }
          }}
        >
          <Icon name="send" size={13} />
        </button>
      {/if}

      {#if !isPlainShellSession(session)}
        <button
          type="button"
          class="icon-btn"
          class:active={showTerminal}
          aria-pressed={showTerminal}
          aria-label={showTerminal ? 'Hide raw terminal drawer' : 'Show raw terminal drawer'}
          title="Terminal drawer"
          data-testid="pane-terminal-btn-{paneId}"
          onclick={() => (showTerminal = !showTerminal)}
        >
          <Icon name="terminal" size={13} />
        </button>
      {/if}

      {#if !multiPane}
        <button
          type="button"
          class="icon-btn"
          aria-label="Session room"
          title="Jump to or start a Session Room"
          onclick={() => onstartroom(paneId)}
        >
          <Icon name="people" size={13} />
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

      {#if multiPane}
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
      {/if}
    </div>
  </header>

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
            showPeerBar = false;
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
        onclick={() => (showPeerBar = false)}
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
        <Terminal sessionId={paneId} forceDark autoFocus={focused} resumeOnOpen={focused} />
      </div>
    {:else}
      <ConversationView
        sessionId={paneId}
        workspaceId={session.workspace_id}
        resumeOnOpen={focused}
      />
    {/if}
  </div>

  {#if showTerminal && session && !isPlainShellSession(session)}
    <ThreadTerminalDrawer
      sessionId={paneId}
      title={session.title}
      onclose={() => (showTerminal = false)}
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
  .tp-branch {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 140px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .tp-parent-link {
    cursor: pointer;
    max-width: 160px;
    font-size: var(--fs-xs);
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
  .tp-pin-on {
    color: var(--warning);
  }
  .tp-pin-on :global(svg path) {
    fill: currentColor;
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
