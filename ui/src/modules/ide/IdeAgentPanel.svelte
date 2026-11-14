<script lang="ts">
  // Side-by-side AI Coding Agent dock (`⌘L`) running Otto's real Threads UI
  // (`EmbeddedRuntimeThread` / `RuntimeThread` for active runtime threads,
  // `IdeAgentComposer` with 100% `NewThreadStage` parity for new drafts, and
  // `PtyConversationView` for legacy PTY threads) — layered seamlessly with
  // smart IDE context pills (`IdeContextBar`) and 1-click `IdeProposalCard`
  // Diff Preview + Apply/Undo.
  import { onMount, setContext } from 'svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import type { TurnItem } from '../../lib/api/types';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { harnessLabel } from '../../lib/providers';
  import { CONV_CTX, type PreviewReq } from '../agents/conversation/context'; // ui-guards: allow — route Markdown file/code links directly to IDE editor
  import EmbeddedRuntimeThread from '../threads/runtime/EmbeddedRuntimeThread.svelte'; // ui-guards: allow — IDE agent dock embeds real Otto Threads runtime view
  import IdeAgentComposer from './agent/IdeAgentComposer.svelte';
  import IdeContextBar, { showIdeContextMenu, showIdeSessionMenu } from './agent/IdeContextBar.svelte';
  import IdeProposalCard from './agent/IdeProposalCard.svelte';
  import {
    buildIdeAgentPrompt,
    ideAgentEngine as ideAgent,
    stripIdeContextBlocks,
    type IdeCodeProposal,
  } from './agent/ideAgentEngine.svelte';
  import { ide } from './ideStore.svelte';

  const PtyConversationView = lazyComponent(() => import('../agents/conversation/ConversationView.svelte')); // ui-guards: allow — lazy PTY thread conversation view

  onMount(() => ideAgent.ensureMounted());

  $effect(() => {
    const r = (ide.target.root ?? '').trim().replace(/\/+$/, '');
    ideAgent.syncTargetRoot(r ? `${ide.target.kind}:${r}` : '');
  });

  // Route every Markdown file link & code-block Open action inside EmbeddedRuntimeThread to the IDE editor.
  setContext(CONV_CTX, {
    get sessionId() {
      return ideAgent.sessionId ?? '';
    },
    get cwd() {
      return ideAgent.activeRuntimeSession?.cwd ?? ideAgent.resolveProjectCwd();
    },
    readonly: false,
    openPreview(req: PreviewReq) {
      if (req.kind === 'file') {
        void ide.openFile(ide.resolvePath(req.path), { line: req.line ?? 1, preview: false });
      } else if (req.kind === 'code' && req.file) {
        void ide.openFile(ide.resolvePath(req.file), { preview: false });
      }
    },
  });

  const activeThread = $derived(ideAgent.activeThread);
  const sessionTitle = $derived(activeThread?.title?.trim() || 'New thread');
  const activeProvider = $derived(activeThread?.provider ?? ideAgent.provider);

  // Focus the active composer whenever a fresh IDE context chip (`⌘L`, Problems, Debug, Test) is attached.
  const contextSig = $derived(ideAgent.contexts.map((c) => `${c.id}:${c.snippet.length}`).join('|'));
  let prevContextSig: string | null = null;
  let prevContextCount = 0;
  $effect(() => {
    const sig = contextSig;
    const count = ideAgent.contexts.length;
    if (sig && ideAgent.sessionId && (prevContextSig === null || (sig !== prevContextSig && count >= prevContextCount))) {
      requestAnimationFrame(() => {
        document.querySelector<HTMLElement>('[data-testid="ide-agent-composer"]')?.focus({ preventScroll: true });
      });
    }
    prevContextSig = sig;
    prevContextCount = count;
  });

  function proposalsFor(item: Extract<TurnItem, { type: 'assistant_message' }>): IdeCodeProposal[] {
    const map = ideAgent.proposalsByMessageId;
    return (map[item.id]?.length ? map[item.id] : map[item.message_id]) ?? [];
  }

  function formatAssistantMarkdown(item: Extract<TurnItem, { type: 'assistant_message' }>): string {
    const proposals = proposalsFor(item);
    const cleaned = stripIdeContextBlocks(item.text.replace(/^echo:\s*(?=<ide_context\b)/i, ''));
    if (!cleaned || proposals.length === 0) return cleaned;
    const codeSet = new Set(proposals.map((p) => p.code.trim()));
    return cleaned
      .replace(/^(?:[ \t]*|echo:[ \t]*)```([^\r\n`]*)\r?\n([\s\S]*?)^[ \t]*```[ \t]*$/gm, (full, _info, body: string) =>
        codeSet.has(body.replace(/\r\n/g, '\n').replace(/\n$/, '').trim()) ? '' : full,
      )
      .replace(/\n{3,}/g, '\n\n')
      .trim();
  }

  function wrapOutgoingWithIdeContext(assembled: string): string {
    return buildIdeAgentPrompt(assembled, ideAgent.contexts, {
      workspaceRoot: ideAgent.resolveProjectCwd() ?? undefined,
      activePath: ide.activePath,
      planMode: ideAgent.planMode,
      omitWhenEmpty: true,
    });
  }

  function closeAgentPanel(): void {
    ideAgent.togglePanel(false);
    requestAnimationFrame(() => {
      (
        document.querySelector<HTMLElement>('[data-testid="ide-toggle-agent"]') ??
        document.querySelector<HTMLElement>('.ide-editor-area .cm-content')
      )?.focus();
    });
  }
</script>

<aside class="ia-panel" aria-label="AI coding agent" data-testid="ide-agent-panel">
  <header class="ia-head">
    <button
      type="button"
      class="ia-session-btn"
      aria-haspopup="menu"
      title="Switch project thread or start a new one"
      onpointerenter={() => void ideAgent.refreshWorkspaceSessions()}
      onclick={(e) => showIdeSessionMenu(e.currentTarget)}
      data-testid="ide-agent-sessions"
    >
      {#if hasProviderIcon(activeProvider)}
        <ProviderIcon provider={activeProvider} size={13} />
      {:else}
        <Icon name="threads" size={13} />
      {/if}
      <span class="ellipsis" dir="auto">{sessionTitle}</span>
      <Icon name="chevronDown" size={12} />
    </button>

    <button
      type="button"
      class="icon-btn"
      aria-label="New thread"
      title="New thread"
      onclick={() => ideAgent.startNewChat()}
      data-testid="ide-agent-new"
    >
      <Icon name="plus" size={13} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="Open in Threads"
      title="Open in Threads"
      onclick={() => ideAgent.openActiveInThreads()}
      data-testid="ide-agent-open-in-threads"
    >
      <Icon name="external" size={13} />
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="Close agent panel"
      title="Close agent (⌘L)"
      onclick={closeAgentPanel}
      data-testid="ide-agent-close"
    >
      <Icon name="x" size={13} />
    </button>
  </header>

  {#if ideAgent.error}
    <div class="ia-error" use:announceShown={{ text: ideAgent.error, assertive: true }}>
      <Icon name="warning" size={13} />
      <span class="ia-error-text" dir="auto">{ideAgent.error}</span>
      <button
        type="button"
        class="icon-btn"
        aria-label="Dismiss agent error"
        title="Dismiss"
        onclick={() => {
          ideAgent.error = null;
        }}
      >
        <Icon name="x" size={12} />
      </button>
    </div>
  {/if}

  {#if ideAgent.activeThreadKind === 'pty' && ideAgent.sessionId}
    <div class="ia-pty-host" data-testid="ide-agent-messages">
      {#key ideAgent.sessionId}
        <LazyMount
          lazy={PtyConversationView}
          what="the thread"
          variant="compact"
          props={{
            sessionId: ideAgent.sessionId,
            workspaceId: ideAgent.activeWorkspaceId,
            surface: 'threads',
            resumeOnOpen: true,
            ontoggleterminal: () => {
              ide.openBottomTab('terminal');
            },
            onnewthread: ({ prompt }: { beside: boolean; prompt?: string }) => {
              ideAgent.startNewChat();
              if (prompt) ideAgent.composerText = prompt;
            },
          }}
        />
      {/key}
    </div>
    <footer class="ia-pty-foot">
      <IdeContextBar />
      {#if ideAgent.contexts.length > 0}
        <div class="ia-pty-send-row">
          <span class="ia-pty-send-hint">
            {ideAgent.contexts.length} {ideAgent.contexts.length === 1 ? 'context item' : 'context items'} attached
          </span>
          <button
            type="button"
            class="btn small ia-pty-cta"
            disabled={ideAgent.sending}
            onclick={() => void ideAgent.sendPrompt()}
            data-testid="ide-agent-pty-send-context"
          >
            <Icon name="send" size={12} /> Send context to thread
          </button>
        </div>
      {/if}
    </footer>
  {:else if ideAgent.activeThreadKind === 'runtime' && ideAgent.sessionId}
    {@const activeSid = ideAgent.sessionId}
    <div class="ia-runtime-host" data-testid="ide-agent-messages">
      {#key activeSid}
        <EmbeddedRuntimeThread
          sessionId={activeSid}
          workspaceId={ideAgent.activeWorkspaceId}
          compact
          focusOnMount
          hideEmbedBarWhenUnlinked
          fallbackCwd={ideAgent.resolveProjectCwd()}
          hasExternalContext={ideAgent.contexts.length > 0}
          transformOutgoingText={wrapOutgoingWithIdeContext}
          onExternalContextConsumed={() => ideAgent.clearContexts()}
          onopenpath={(path, line) => {
            void ide.openFile(ide.resolvePath(path), { line: line ?? 1, preview: false });
          }}
          formatAssistantText={formatAssistantMarkdown}
          hostCommands={{
            newThread: () => ideAgent.startNewChat(),
            openTerminal: () => ide.openBottomTab('terminal'),
            openFiles: () => {
              ide.searchInitialTab = 'files';
              ide.searchEverywhereOpen = true;
            },
          }}
          testIds={{
            composer: 'ide-agent-composer',
            send: 'ide-agent-send',
            stop: 'ide-agent-stop',
            model: 'ide-agent-provider',
            attach: 'ide-agent-attach-image',
          }}
        >
          {#snippet assistantFooter(item)}
            {#each proposalsFor(item) as proposal (proposal.id)}
              <IdeProposalCard {proposal} />
            {/each}
          {/snippet}
          {#snippet composerTopSlot({ focusComposer })}
            <div class="ia-embed-ctx">
              <IdeContextBar onfocuscomposer={focusComposer} />
            </div>
          {/snippet}
          {#snippet composerFootLeadSlot({ focusComposer })}
            <button
              type="button"
              class="icon-btn"
              aria-haspopup="menu"
              aria-label="Attach IDE context"
              title="Attach selection, file, diagnostic, or debug frame"
              onclick={(e) => showIdeContextMenu(e.currentTarget, focusComposer)}
              data-testid="ide-agent-add-context"
            >
              <Icon name="plus" size={13} />
            </button>
            <button
              type="button"
              class="btn small ghost"
              class:ia-mode-plan={ideAgent.planMode}
              aria-pressed={ideAgent.planMode}
              title={ideAgent.planMode
                ? 'Plan mode: propose code blocks without modifying files directly'
                : 'Toggle Plan mode'}
              onclick={() => {
                ideAgent.planMode = !ideAgent.planMode;
                focusComposer();
              }}
              data-testid="ide-agent-mode"
            >
              Plan
            </button>
          {/snippet}
        </EmbeddedRuntimeThread>
      {/key}
    </div>
  {:else}
    <div class="ia-stream scroll-thin" data-testid="ide-agent-messages">
      {#if ideAgent.sending}
        <div class="ia-streaming">
          <span class="spinner" style:--spinner-size="12px" aria-hidden="true"></span>
          <span>Starting thread…</span>
        </div>
      {:else}
        <div class="ia-empty">
          <EmptyState
            variant="panel"
            icon="threads"
            title="Threads in IDE"
            body="Same Threads composer, @file & @agent mentions, slash commands, and workspace sessions."
          />
          <div class="ia-quick-actions" role="group" aria-label="Quick context actions">
            <button
              type="button"
              class="btn small"
              disabled={!ide.activeTab}
              onclick={() => ideAgent.markSelectionFromEditor({ openPanel: true })}
            >
              <Icon name="cursor" size={12} /> Mark selection (⌘L)
            </button>
            <button
              type="button"
              class="btn small"
              disabled={!ide.activePath}
              onclick={() => ide.activePath && void ideAgent.attachFile(ide.activePath)}
            >
              <Icon name="file" size={12} /> Attach active file
            </button>
            {#if ide.problems.length > 0}
              <button
                type="button"
                class="btn small"
                onclick={() => ideAgent.attachDiagnostic(ide.problems[0], { openPanel: true })}
              >
                <Icon name="warning" size={12} /> Attach problem ({ide.problems.length})
              </button>
            {/if}
          </div>

          {#if ideAgent.threads.length > 0}
            <div class="ia-recent" aria-label="Workspace threads">
              <span class="ia-recent-title">Workspace threads</span>
              {#each ideAgent.threads.slice(0, 6) as t (t.id)}
                <button
                  type="button"
                  class="ia-recent-row"
                  title="{t.title} — {harnessLabel(t.provider)}"
                  onclick={() => void ideAgent.selectSession(t.id)}
                >
                  {#if hasProviderIcon(t.provider)}
                    <ProviderIcon provider={t.provider} size={12} />
                  {:else}
                    <Icon name="threads" size={12} />
                  {/if}
                  <span class="ellipsis" dir="auto">{t.title}</span>
                  <span class="ia-recent-meta">{harnessLabel(t.provider)}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <IdeAgentComposer />
  {/if}
</aside>

<style>
  .ia-panel {
    display: flex;
    flex-direction: column;
    inline-size: 100%;
    block-size: 100%;
    min-inline-size: 0;
    min-block-size: 0;
    background: var(--surface);
    border-inline-start: 1px solid var(--border);
  }
  .ia-head {
    display: flex;
    align-items: center;
    gap: 4px;
    block-size: 36px;
    padding-inline: 8px 6px;
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .ia-session-btn {
    flex: 1;
    min-inline-size: 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    block-size: 26px;
    padding-inline: 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 500;
    cursor: pointer;
    text-align: start;
  }
  .ia-session-btn:hover {
    background: var(--hover);
  }
  .ia-error {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-block: 6px;
    padding-inline: 10px 6px;
    background: var(--danger-soft);
    border-block-end: 1px solid var(--border);
    color: var(--danger);
    font-size: var(--fs-xs);
    flex-shrink: 0;
  }
  .ia-error-text {
    flex: 1;
    min-inline-size: 0;
    overflow-wrap: anywhere;
  }
  .ia-pty-host,
  .ia-runtime-host {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    display: flex;
    flex-direction: column;
  }
  .ia-embed-ctx {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-block-end: 2px;
  }
  .ia-mode-plan {
    color: var(--warning);
    background: var(--warning-soft);
  }
  .ia-pty-foot {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border-block-start: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }
  .ia-pty-send-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ia-pty-send-hint {
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ia-pty-cta {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .ia-pty-cta:hover:not(:disabled) {
    background: var(--accent-soft-strong);
  }
  .ia-stream {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px;
  }
  .ia-streaming {
    margin-block: auto;
    align-self: center;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ia-empty {
    margin-block: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding-block: 12px;
    inline-size: 100%;
  }
  .ia-quick-actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
    padding-inline: 8px;
  }
  .ia-recent {
    display: flex;
    flex-direction: column;
    gap: 2px;
    inline-size: 100%;
    max-inline-size: 320px;
    padding-block-start: 6px;
    border-block-start: 1px solid var(--border);
  }
  .ia-recent-title {
    padding-inline: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-weight: 500;
  }
  .ia-recent-row {
    display: flex;
    align-items: center;
    gap: 6px;
    inline-size: 100%;
    min-inline-size: 0;
    block-size: 28px;
    padding-inline: 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .ia-recent-row:hover {
    background: var(--hover);
  }
  .ia-recent-meta {
    margin-inline-start: auto;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  @media (max-width: 640px) {
    .ia-panel {
      border-inline-start: none;
    }
  }
  @media (pointer: coarse) {
    .ia-session-btn,
    .ia-recent-row {
      min-block-size: var(--hit-min);
    }
  }
</style>
