<script lang="ts">
  // Side-by-side AI Coding Agent dock for the Otto IDE (`⌘L`).
  // Connects editor selections, files, diagnostics, failing tests, and paused
  // debugger frames to structured runtime sessions with inline diff + Apply/Undo.
  import { announceShown } from '../../lib/announce.svelte';
  import type { ApprovalDecision, ApprovalOption, RuntimeProvider, TurnItem } from '../../lib/api/types';
  import { copyWithToast } from '../../lib/clipboard';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { toastError } from '../../lib/toastError';
  import { runtimeThreads } from '../threads/runtime/runtimeThreads.svelte'; // ui-guards: allow — shared structured runtime approval responder
  import { ide, ideBasename } from './ideStore.svelte';
  import {
    ideAgent, parseIdeContextChipsFromPrompt, stripIdeContextBlocks,
    type IdeCodeProposal, type IdeContextKind, type ParsedPromptContextSummary,
  } from './store/ideAgentController.svelte';

  const PROVIDERS: { id: RuntimeProvider; label: string }[] = [
    { id: 'claude', label: 'Claude' }, { id: 'codex', label: 'Codex' }, { id: 'gemini', label: 'Gemini' },
  ];
  const CHIP_ICONS: Record<IdeContextKind, IconName> = {
    selection: 'cursor', file: 'file', symbol: 'zap', diagnostic: 'warning', test_failure: 'flask', debug_frame: 'bug',
  };
  const DEFAULT_APPROVAL_OPTIONS: ApprovalOption[] = [{ decision: 'decline', label: 'Decline' }, { decision: 'accept', label: 'Approve' }];
  type InlineToken = { kind: 'text' | 'code' | 'bold'; value: string };
  type ProseBlock = { kind: 'p' | 'h'; tokens: InlineToken[] } | { kind: 'ul'; items: InlineToken[][] } | { kind: 'code'; code: string };

  function parseUserMessage(raw: string): { badges: ParsedPromptContextSummary[]; prose: string } {
    const badges = parseIdeContextChipsFromPrompt(raw);
    if (badges.length === 0 && /<ide_context\b/i.test(raw)) {
      badges.push({ kind: 'file', label: 'Editor context' });
    }
    return { badges, prose: stripIdeContextBlocks(raw) };
  }

  function tokenizeInline(line: string): InlineToken[] {
    const out: InlineToken[] = [];
    let last = 0;
    for (const m of line.matchAll(/`([^`]+)`|\*\*([^*]+)\*\*/g)) {
      const idx = m.index ?? 0;
      if (idx > last) out.push({ kind: 'text', value: line.slice(last, idx) });
      out.push(m[1] !== undefined ? { kind: 'code', value: m[1] } : { kind: 'bold', value: m[2] });
      last = idx + m[0].length;
    }
    if (last < line.length) out.push({ kind: 'text', value: line.slice(last) });
    return out;
  }

  function parseProseBlocks(raw: string, proposalCodes: Set<string>): ProseBlock[] {
    const cleaned = stripIdeContextBlocks(raw.replace(/^echo:\s*(?=<ide_context\b)/i, ''));
    if (!cleaned) return [];
    const out: ProseBlock[] = [];
    let cursor = 0;
    const pushChunk = (chunk: string) => {
      let listBuf: InlineToken[][] = [];
      const flush = () => {
        if (listBuf.length > 0) { out.push({ kind: 'ul', items: listBuf }); listBuf = []; }
      };
      for (const rawLine of chunk.split('\n')) {
        const line = rawLine.trim();
        if (!line) { flush(); continue; }
        const bullet = /^[-*]\s+(.+)$/.exec(line);
        if (bullet) { listBuf.push(tokenizeInline(bullet[1])); continue; }
        flush();
        const h = /^#{1,4}\s+(.+)$/.exec(line);
        out.push(h ? { kind: 'h', tokens: tokenizeInline(h[1]) } : { kind: 'p', tokens: tokenizeInline(line) });
      }
      flush();
    };
    for (const m of cleaned.matchAll(/```([^\n`]*)\n([\s\S]*?)```/g)) {
      const idx = m.index ?? 0;
      if (idx > cursor) pushChunk(cleaned.slice(cursor, idx));
      const code = m[2].replace(/\n$/, '');
      if (!proposalCodes.has(code.trim())) out.push({ kind: 'code', code });
      cursor = idx + m[0].length;
    }
    if (cursor < cleaned.length) pushChunk(cleaned.slice(cursor));
    return out;
  }

  function workSummary(item: TurnItem, liveOut: Record<string, string>): {
    icon: IconName; label: string; detail: string; running: boolean; failed: boolean;
  } {
    const running = item.status === 'running' || item.status === 'pending' || item.status === 'waiting';
    const failed = item.status === 'failed';
    switch (item.type) {
      case 'reasoning':
        return { icon: 'sparkle', label: item.title || 'Thinking', detail: item.text.trim(), running: running || item.streaming, failed };
      case 'command_execution':
        return { icon: 'terminal', label: item.input || item.title || 'Command', detail: (liveOut[item.id] ?? item.output ?? item.error ?? '').trim(), running, failed };
      case 'dynamic_tool':
        return { icon: 'zap', label: item.tool_name || item.title || 'Tool call', detail: (item.output ?? item.error ?? (item.input ? JSON.stringify(item.input, null, 2) : '')).trim(), running, failed };
      case 'file_change':
        return { icon: 'edit', label: item.file_name || item.title || 'File edit', detail: (item.diff_str ?? item.new_str ?? item.error ?? '').trim(), running, failed };
      case 'web_search':
        return { icon: 'globe', label: item.patterns?.join(', ') || 'Web search', detail: (item.error ?? '').trim(), running, failed };
      case 'subagent':
        return { icon: 'layers', label: item.description || item.tool_name || 'Subagent', detail: (item.output ?? item.error ?? item.prompt ?? '').trim(), running, failed };
      case 'todo_list':
        return {
          icon: 'check', label: item.explanation || 'Plan', running, failed,
          detail: item.steps.map((s) => `${s.status === 'completed' ? '✓' : s.status === 'running' ? '▸' : '○'} ${s.text}`).join('\n'),
        };
      default:
        return { icon: 'info', label: item.title || item.type, detail: '', running, failed };
    }
  }

  let messagesEl = $state<HTMLDivElement | null>(null);
  let addContextBtnEl = $state<HTMLButtonElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let stickToBottom = $state(true);
  let expandedItems = $state<Record<string, boolean>>({});
  let diffOverrides = $state<Record<string, boolean>>({});
  let respondingReqs = $state<Record<string, boolean>>({});

  const activeSession = $derived(ideAgent.sessions.find((s) => s.id === ideAgent.sessionId) ?? null);
  const sessionTitle = $derived(activeSession?.title?.trim() || 'Agent');
  const providerLabel = $derived(PROVIDERS.find((p) => p.id === ideAgent.provider)?.label ?? 'Claude');
  const items = $derived(ideAgent.projection?.items ?? []);
  const liveOutput = $derived(ideAgent.projection?.output ?? {});
  const pendingReqIds = $derived(new Set((ideAgent.projection?.requests ?? []).filter((r) => r.status === 'pending').map((r) => r.id)));
  const failedTest = $derived(ide.activeRun?.tests.find((t) => t.status === 'failed') ?? null);
  const canSend = $derived((Boolean(ideAgent.composerText.trim()) || ideAgent.contexts.length > 0) && !ideAgent.sending);
  const sessionLoadPending = $derived(Boolean(ideAgent.sessionId && !ideAgent.projection && !ideAgent.error));
  const sessionLoadError = $derived(ideAgent.sessionId && !ideAgent.projection ? ideAgent.error : null);
  const contextSig = $derived(ideAgent.contexts.map((c) => `${c.id}:${c.snippet.length}`).join('|'));
  let prevContextSig: string | null = null;
  let prevContextCount = 0;

  $effect(() => {
    const sig = contextSig;
    const count = ideAgent.contexts.length;
    if (sig && (prevContextSig === null || (sig !== prevContextSig && count >= prevContextCount))) {
      requestAnimationFrame(() => composerEl?.focus());
    }
    prevContextSig = sig;
    prevContextCount = count;
  });

  const streamSignature = $derived.by(() => {
    const last = items[items.length - 1];
    return `${ideAgent.sessionId ?? ''}:${items.length}:${last?.id ?? ''}:${last?.updated_at ?? ''}`;
  });

  $effect(() => {
    void streamSignature;
    if (stickToBottom && messagesEl) messagesEl.scrollTop = messagesEl.scrollHeight;
  });

  function closeAgentPanel(): void {
    ideAgent.togglePanel(false);
    requestAnimationFrame(() => {
      (document.querySelector<HTMLElement>('[data-testid="ide-toggle-agent"]') ??
        document.querySelector<HTMLElement>('.ide-editor-area .cm-content'))?.focus();
    });
  }

  function removeContextChip(id: string): void { ideAgent.removeContext(id); composerEl?.focus(); }
  function clearContextChips(): void { ideAgent.clearContexts(); composerEl?.focus(); }

  function onStreamScroll(): void {
    if (!messagesEl) return;
    stickToBottom = messagesEl.scrollHeight - messagesEl.scrollTop - messagesEl.clientHeight < 48;
  }

  function proposalsFor(item: Extract<TurnItem, { type: 'assistant_message' }>): IdeCodeProposal[] {
    const map = ideAgent.proposalsByMessageId;
    return (map[item.id]?.length ? map[item.id] : map[item.message_id]) ?? [];
  }

  function isDiffOpen(p: IdeCodeProposal): boolean {
    return diffOverrides[p.id] ?? p.diffLines.length <= 40;
  }

  function isProposalReadOnly(p: IdeCodeProposal): boolean {
    if (!ide.canEditCollab) return true;
    return Boolean(ide.tabs.find((t) => t.path === p.path)?.readOnly);
  }

  function jumpTo(path?: string, line?: number): void {
    if (path) void ide.openFile(path, { line: line ?? 1, preview: false });
  }

  function openSessionMenu(e: MouseEvent): void {
    const rows: MenuItem[] = [{ label: 'New chat', icon: 'plus', pinned: true, action: () => ideAgent.startNewChat() }];
    if (ideAgent.sessions.length > 0) {
      rows.push({ separator: true });
      for (const s of ideAgent.sessions) {
        rows.push({ label: s.title?.trim() || 'Untitled chat', checked: s.id === ideAgent.sessionId, hint: s.provider, action: () => void ideAgent.selectSession(s.id) });
      }
    }
    ctxMenu.showAt(e.currentTarget as HTMLElement, rows, { filter: ideAgent.sessions.length > 8, filterPlaceholder: 'Filter chats…' });
  }

  function openProviderMenu(e: MouseEvent): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, PROVIDERS.map((p) => ({ label: p.label, checked: ideAgent.provider === p.id, action: () => { ideAgent.provider = p.id; } })));
  }

  function openContextMenu(anchor?: HTMLElement | null, atIndex: number | null = null): void {
    const wrap = (fn: () => void) => () => {
      if (atIndex !== null && ideAgent.composerText[atIndex] === '@') {
        ideAgent.composerText = ideAgent.composerText.slice(0, atIndex) + ideAgent.composerText.slice(atIndex + 1);
      }
      fn();
      composerEl?.focus();
    };
    const menu: MenuItem[] = [
      { label: 'Mark current editor selection', icon: 'cursor', hint: '⌘L', pinned: true, disabled: !ide.activeTab, title: ide.activeTab ? undefined : 'Open a file in the editor first', action: wrap(() => { ideAgent.markSelectionFromEditor({ openPanel: true }); }) },
      { label: ide.activeTab ? `Attach active file (${ide.activeTab.name})` : 'Attach active file', icon: 'file', pinned: true, disabled: !ide.activePath, action: wrap(() => { if (ide.activePath) void ideAgent.attachFile(ide.activePath, { openPanel: true }); }) },
    ];
    if (ide.problems.length > 0) {
      const top = ide.problems[0];
      menu.push({ label: `Top problem (${ideBasename(top.path)}:${top.line})`, icon: 'warning', pinned: true, action: wrap(() => ideAgent.attachDiagnostic(top, { openPanel: true })) });
    }
    if (failedTest) {
      const tf = failedTest;
      menu.push({ label: `Latest failed test (${tf.name})`, icon: 'flask', pinned: true, action: wrap(() => ideAgent.attachTestFailure(tf, ide.activeRun?.output ?? [], { openPanel: true })) });
    }
    if (ide.activeDebug?.state === 'paused') {
      const dbg = ide.activeDebug;
      menu.push({ label: 'Paused debug frame & locals', icon: 'bug', pinned: true, action: wrap(() => ideAgent.attachDebugFrame(dbg, ide.expandedVariables, { openPanel: true })) });
    }
    const otherTabs = ide.tabs.filter((t) => t.path !== ide.activePath);
    if (otherTabs.length > 0) {
      menu.push({ separator: true });
      for (const tab of otherTabs) menu.push({ label: tab.name, icon: 'file', hint: 'Open tab', action: wrap(() => void ideAgent.attachFile(tab.path, { openPanel: true })) });
    }
    ctxMenu.showAt(anchor ?? addContextBtnEl, menu, { filter: otherTabs.length > 6, filterPlaceholder: 'Filter files & context…' });
  }

  function onComposerKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      if (canSend) { stickToBottom = true; void ideAgent.sendPrompt(); }
    }
  }

  function onComposerInput(e: Event): void {
    const el = e.currentTarget as HTMLTextAreaElement;
    const pos = el.selectionStart ?? 0;
    if (pos > 0 && el.value[pos - 1] === '@' && /\s/.test(pos === 1 ? ' ' : el.value[pos - 2])) {
      openContextMenu(addContextBtnEl ?? el, pos - 1);
    }
  }

  async function respondToApproval(requestId: string, decision: ApprovalDecision): Promise<void> {
    if (!ideAgent.sessionId || respondingReqs[requestId]) return;
    respondingReqs = { ...respondingReqs, [requestId]: true };
    try {
      await runtimeThreads.respond(ideAgent.sessionId, requestId, decision);
    } catch (e) {
      toastError('Couldn’t send approval decision', e);
    } finally {
      respondingReqs = { ...respondingReqs, [requestId]: false };
    }
  }
</script>

{#snippet inlineRuns(tokens: InlineToken[])}
  {#each tokens as tk, ti (ti)}
    {#if tk.kind === 'code'}<code class="ia-inline-code" dir="ltr">{tk.value}</code>
    {:else if tk.kind === 'bold'}<strong>{tk.value}</strong>
    {:else}{tk.value}{/if}
  {/each}
{/snippet}

<aside class="ia-panel" aria-label="AI coding agent" data-testid="ide-agent-panel">
  <header class="ia-head">
    <button type="button" class="ia-session-btn" aria-haspopup="menu" title="Switch or start an Agent chat" onclick={openSessionMenu} data-testid="ide-agent-sessions">
      <Icon name="assistant" size={13} />
      <span class="ellipsis" dir="auto">{sessionTitle}</span>
      <Icon name="chevronDown" size={12} />
    </button>
    <button type="button" class="btn small ghost ia-provider-btn" aria-haspopup="menu" aria-label="AI provider: {providerLabel}" title="Switch AI provider" onclick={openProviderMenu} data-testid="ide-agent-provider">
      <ProviderIcon provider={ideAgent.provider} size={12} />
      <span>{providerLabel}</span>
    </button>
    <button type="button" class="icon-btn" aria-label="New agent chat" title="New chat" onclick={() => ideAgent.startNewChat()} data-testid="ide-agent-new"><Icon name="plus" size={13} /></button>
    <button type="button" class="icon-btn" aria-label="Close agent panel" title="Close agent (⌘L)" onclick={closeAgentPanel} data-testid="ide-agent-close"><Icon name="x" size={13} /></button>
  </header>

  {#if ideAgent.error && !sessionLoadError}
    <div class="ia-error" use:announceShown={{ text: ideAgent.error, assertive: true }}>
      <Icon name="warning" size={13} />
      <span class="ia-error-text" dir="auto">{ideAgent.error}</span>
      <button type="button" class="icon-btn" aria-label="Dismiss agent error" title="Dismiss" onclick={() => { ideAgent.error = null; }}><Icon name="x" size={12} /></button>
    </div>
  {/if}

  <div class="ia-stream scroll-thin" bind:this={messagesEl} onscroll={onStreamScroll} data-testid="ide-agent-messages">
    <LoadState
      what="conversation" variant="compact" loading={sessionLoadPending} error={sessionLoadError}
      empty={!ideAgent.projection || items.length === 0}
      onretry={ideAgent.sessionId ? () => { if (ideAgent.sessionId) void ideAgent.selectSession(ideAgent.sessionId); } : undefined}
    >
      {#snippet errorActions()}
        <button type="button" class="icon-btn" aria-label="Dismiss agent error" title="Dismiss" onclick={() => { ideAgent.error = null; }}>
          <Icon name="x" size={12} />
        </button>
      {/snippet}
      {#snippet emptyView()}
        {#if ideAgent.sending}
          <div class="ia-streaming">
            <span class="spinner" style:--spinner-size="12px" aria-hidden="true"></span>
            <span>Starting agent…</span>
          </div>
        {:else}
          <div class="ia-empty">
            <EmptyState
              variant="panel" icon="assistant" title="Code with Agent"
              body="Mark lines with ⌘L, edit inline with ⌘I, or attach files, problems and failing tests."
            />
            <div class="ia-quick-actions" role="group" aria-label="Quick context actions">
              <button type="button" class="btn small" disabled={!ide.activeTab} onclick={() => ideAgent.markSelectionFromEditor({ openPanel: true })}>
                <Icon name="cursor" size={12} /> Mark editor selection (⌘L)
              </button>
              <button type="button" class="btn small" disabled={!ide.activePath} onclick={() => ide.activePath && void ideAgent.attachFile(ide.activePath)}>
                <Icon name="file" size={12} /> Attach active file
              </button>
              {#if ide.problems.length > 0}
                <button type="button" class="btn small" onclick={() => ideAgent.attachDiagnostic(ide.problems[0], { openPanel: true })}>
                  <Icon name="warning" size={12} /> Attach top problem ({ide.problems.length})
                </button>
              {/if}
            </div>
          </div>
        {/if}
      {/snippet}

      {#each items as item (item.id)}
        {#if item.type === 'user_message'}
          {@const parsed = parseUserMessage(item.text)}
          <article class="ia-user-msg">
            {#if parsed.badges.length > 0}
              <div class="ia-msg-badges" dir="ltr">
                {#each parsed.badges as b, bi (`${bi}:${b.label}`)}
                  {#if b.path}
                    <button
                      type="button" class="chip accent" title="{b.path}{b.startLine ? `:${b.startLine}` : ''}"
                      onclick={() => jumpTo(b.path, b.startLine)}
                    >{b.label}</button>
                  {:else}
                    <Badge tone="accent" variant="outline" label={b.label} />
                  {/if}
                {/each}
              </div>
            {/if}
            {#if parsed.prose}<p class="ia-user-text" dir="auto">{parsed.prose}</p>{/if}
          </article>
        {:else if item.type === 'assistant_message'}
          {@const proposals = proposalsFor(item)}
          {@const codeSet = new Set(proposals.map((p) => p.code.trim()))}
          {@const blocks = parseProseBlocks(item.text, codeSet)}
          <article class="ia-assistant-msg">
            {#if blocks.length > 0}
              <div class="ia-prose" dir="auto">
                {#each blocks as blk, bi (bi)}
                  {#if blk.kind === 'h'}
                    <p class="ia-prose-h">{@render inlineRuns(blk.tokens)}</p>
                  {:else if blk.kind === 'p'}
                    <p class="ia-prose-p">{@render inlineRuns(blk.tokens)}</p>
                  {:else if blk.kind === 'ul'}
                    <ul class="ia-prose-ul">
                      {#each blk.items as li, liIdx (liIdx)}<li>{@render inlineRuns(li)}</li>{/each}
                    </ul>
                  {:else if blk.kind === 'code'}
                    <pre class="ia-code-fence scroll-thin" dir="ltr"><code>{blk.code}</code></pre>
                  {/if}
                {/each}
              </div>
            {:else if item.streaming && proposals.length === 0}
              <div class="ia-streaming">
                <span class="spinner" style:--spinner-size="12px" aria-hidden="true"></span>
                <span>Writing response…</span>
              </div>
            {/if}

            {#each proposals as proposal (proposal.id)}
              {@const diffOpen = isDiffOpen(proposal)}
              {@const hasUndo = ideAgent.appliedUndoByProposalId[proposal.id] !== undefined}
              {@const propReadOnly = isProposalReadOnly(proposal)}
              <section class="ia-proposal" data-testid="ide-agent-proposal">
                <div class="ia-prop-head">
                  <button
                    type="button" class="ia-prop-file" title="Open {proposal.relPath} in editor"
                    onclick={() => jumpTo(proposal.path, proposal.range?.startLine ?? 1)}
                  >
                    <Icon name="file" size={12} />
                    <span class="mono ellipsis" dir="ltr">{proposal.relPath}</span>
                  </button>
                  <Badge tone="ok" label="+{proposal.addedCount}" />
                  <Badge tone="bad" label="-{proposal.removedCount}" />

                  <div class="ia-prop-actions">
                    <button
                      type="button" class="btn small ghost" aria-expanded={diffOpen}
                      onclick={() => { diffOverrides = { ...diffOverrides, [proposal.id]: !diffOpen }; }}
                      data-testid="ide-agent-toggle-diff"
                    >Diff</button>
                    <button
                      type="button" class="icon-btn"
                      aria-label="Copy proposed code for {proposal.relPath}" title="Copy proposed code"
                      onclick={() => void copyWithToast(proposal.code, proposal.relPath)}
                    ><Icon name="copy" size={12} /></button>
                    {#if hasUndo}
                      <button
                        type="button" class="btn small" title="Revert applied edit in {proposal.relPath}"
                        onclick={() => void ideAgent.undoProposal(proposal.id)} data-testid="ide-agent-undo-proposal"
                      ><Icon name="undo" size={12} /> Undo</button>
                    {:else}
                      <button
                        type="button" class="btn small ia-cta" disabled={propReadOnly}
                        title={propReadOnly ? 'File is read-only' : `Apply changes to ${proposal.relPath}`}
                        onclick={() => void ideAgent.applyProposal(proposal)} data-testid="ide-agent-apply-proposal"
                      ><Icon name="check" size={12} /> Apply</button>
                    {/if}
                  </div>
                </div>

                {#if diffOpen}
                  <div class="ia-diff scroll-thin" dir="ltr" data-testid="ide-agent-diff-preview">
                    {#each proposal.diffLines as dl, idx (`${idx}:${dl.kind}:${dl.oldLine ?? ''}:${dl.newLine ?? ''}`)}
                      <div class="ia-diff-row" class:is-add={dl.kind === 'add'} class:is-del={dl.kind === 'del'}>
                        <span class="ia-ln">{dl.newLine ?? dl.oldLine ?? ''}</span>
                        <span class="ia-sign">{dl.kind === 'add' ? '+' : dl.kind === 'del' ? '-' : ' '}</span>
                        <span class="ia-code">{dl.text}</span>
                      </div>
                    {/each}
                  </div>
                {/if}
              </section>
            {/each}
          </article>
        {:else if item.type === 'approval_request'}
          {@const opts = item.options.length > 0 ? item.options : DEFAULT_APPROVAL_OPTIONS}
          {@const canRespond = pendingReqIds.has(item.request_id) || item.status === 'waiting' || item.status === 'pending'}
          {@const busyReq = Boolean(respondingReqs[item.request_id])}
          <div class="ia-approval" role="group" aria-label="Tool approval request">
            <div class="ia-approval-head">
              <Icon name="shield" size={13} />
              <span class="ia-approval-title" dir="auto">{item.prompt || item.title || 'Approval required'}</span>
            </div>
            {#if item.reason}<p class="ia-approval-reason" dir="auto">{item.reason}</p>{/if}
            {#if canRespond}
              <div class="ia-approval-actions">
                {#each opts.filter((o) => o.decision === 'decline' || o.decision === 'accept') as opt (opt.decision)}
                  <button
                    type="button" class="btn small" class:ia-cta={opt.decision === 'accept'} disabled={busyReq}
                    onclick={() => void respondToApproval(item.request_id, opt.decision)}
                  >
                    {opt.label}
                  </button>
                {/each}
              </div>
            {:else}
              <span class="ia-approval-done">{item.status === 'completed' ? 'Handled' : 'Closed'}</span>
            {/if}
          </div>
        {:else if item.type === 'error'}
          <div class="ia-turn-err" dir="auto">
            <Icon name="warning" size={12} />
            <span>{item.failure.message}</span>
          </div>
        {:else if item.type === 'system_notice'}
          <p class="ia-notice" dir="auto">{item.message}</p>
        {:else if item.type !== 'session_boundary'}
          {@const meta = workSummary(item, liveOutput)}
          {@const openWork = Boolean(expandedItems[item.id])}
          <div class="ia-work" class:is-failed={meta.failed}>
            <button
              type="button" class="ia-work-row" aria-expanded={meta.detail ? openWork : undefined}
              disabled={!meta.detail} title={meta.label}
              onclick={() => { if (meta.detail) expandedItems = { ...expandedItems, [item.id]: !openWork }; }}
            >
              {#if meta.running}
                <span class="spinner" style:--spinner-size="12px" aria-hidden="true"></span>
              {:else}
                <Icon name={meta.icon} size={12} />
              {/if}
              <span class="ia-work-label ellipsis" dir="auto">{meta.label}</span>
              {#if meta.detail}<Icon name={openWork ? 'chevronDown' : 'chevronRight'} size={12} />{/if}
            </button>
            {#if openWork && meta.detail}
              <pre class="ia-work-detail scroll-thin" dir="ltr">{meta.detail}</pre>
            {/if}
          </div>
        {/if}
      {/each}
    </LoadState>
  </div>

  <footer class="ia-foot">
    {#if ideAgent.contexts.length > 0}
      <div class="ia-tray" data-testid="ide-agent-context-tray">
        {#each ideAgent.contexts as chip (chip.id)}
          <div class="ia-ctx-item" data-testid="ide-context-chip">
            <button
              type="button" class="ia-ctx-jump" dir="ltr" disabled={!chip.path}
              title={chip.sublabel ? `${chip.label} — ${chip.sublabel}` : chip.label}
              onclick={() => jumpTo(chip.path, chip.startLine)}
            >
              <Icon name={CHIP_ICONS[chip.kind] ?? 'file'} size={12} />
              <span class="ellipsis">{chip.label}</span>
            </button>
            <button
              type="button" class="ia-ctx-rm" aria-label="Remove {chip.label} from context" title="Remove from context"
              onclick={() => removeContextChip(chip.id)}
            >
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
        {#if ideAgent.contexts.length > 1}
          <button type="button" class="ia-clear-btn" onclick={clearContextChips}>Clear</button>
        {/if}
      </div>
    {/if}

    <div class="ia-composer-box">
      <textarea
        bind:this={composerEl} bind:value={ideAgent.composerText}
        class="ia-textarea" rows={2} dir="auto"
        aria-label="Ask Agent or describe a code change"
        placeholder="Ask Agent, or type @ to attach code…"
        oninput={onComposerInput} onkeydown={onComposerKeydown}
        data-testid="ide-agent-composer"
      ></textarea>

      <div class="ia-composer-bar">
        <button
          bind:this={addContextBtnEl} type="button" class="btn small ghost" aria-haspopup="menu"
          title="Attach selection, file, diagnostic, or debug frame (@)"
          onclick={() => openContextMenu(addContextBtnEl)} data-testid="ide-agent-add-context"
        >
          <Icon name="plus" size={12} /> Context
        </button>

        <button
          type="button" class="btn small ghost" class:ia-mode-plan={ideAgent.accessMode === 'plan'}
          aria-pressed={ideAgent.accessMode === 'plan'}
          title={ideAgent.accessMode === 'plan'
            ? 'Plan mode: Agent proposes a plan without running write tools'
            : 'Auto mode: Agent can inspect workspace and propose edits'}
          onclick={() => { ideAgent.accessMode = ideAgent.accessMode === 'plan' ? 'default' : 'plan'; }}
          data-testid="ide-agent-mode"
        >
          {ideAgent.accessMode === 'plan' ? 'Plan' : 'Auto'}
        </button>

        <span class="grow"></span>

        {#if ideAgent.isWorking}
          <button
            type="button" class="btn small ia-stop" aria-label="Stop current agent turn" title="Stop turn"
            onclick={() => void ideAgent.stopTurn()} data-testid="ide-agent-stop"
          >
            <Icon name="stop" size={12} /> Stop
          </button>
        {:else}
          <button
            type="button" class="btn small ia-cta" disabled={!canSend} title="Send message (Enter)"
            onclick={() => { stickToBottom = true; void ideAgent.sendPrompt(); }}
            data-testid="ide-agent-send"
          >
            <Icon name="send" size={12} /> Send
          </button>
        {/if}
      </div>
    </div>
  </footer>
</aside>

<style>
  .ia-panel { display: flex; flex-direction: column; inline-size: 100%; block-size: 100%; min-inline-size: 0; min-block-size: 0; background: var(--surface); border-inline-start: 1px solid var(--border); }
  .ia-head { display: flex; align-items: center; gap: 4px; block-size: 36px; padding-inline: 8px 6px; border-block-end: 1px solid var(--border); flex-shrink: 0; }
  .ia-session-btn { flex: 1; min-inline-size: 0; display: inline-flex; align-items: center; gap: 6px; block-size: 26px; padding-inline: 6px; border: none; border-radius: var(--radius-s); background: transparent; color: var(--text); font: inherit; font-size: var(--fs-s); font-weight: 500; cursor: pointer; text-align: start; }
  .ia-session-btn:hover { background: var(--hover); }
  .ia-provider-btn { flex-shrink: 0; }
  .ia-error { display: flex; align-items: center; gap: 6px; padding-block: 6px; padding-inline: 10px 6px; background: var(--danger-soft); border-block-end: 1px solid var(--border); color: var(--danger); font-size: var(--fs-xs); flex-shrink: 0; }
  .ia-error-text { flex: 1; min-inline-size: 0; overflow-wrap: anywhere; }
  .ia-stream { flex: 1; min-block-size: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 10px; padding: 10px; }
  .ia-empty { margin-block: auto; display: flex; flex-direction: column; align-items: center; gap: 8px; padding-block: 12px; }
  .ia-quick-actions { display: flex; flex-wrap: wrap; justify-content: center; gap: 6px; padding-inline: 8px; }
  .ia-user-msg { align-self: flex-end; max-inline-size: 92%; display: flex; flex-direction: column; gap: 6px; padding-block: 8px; padding-inline: 10px; border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border); }
  .ia-msg-badges { display: flex; flex-wrap: wrap; gap: 4px; }
  .ia-user-text { margin: 0; font-size: var(--fs-s); line-height: 1.45; color: var(--text); white-space: pre-wrap; overflow-wrap: anywhere; }
  .ia-assistant-msg { display: flex; flex-direction: column; gap: 8px; min-inline-size: 0; }
  .ia-prose { display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-s); line-height: 1.5; color: var(--text); overflow-wrap: anywhere; }
  .ia-prose-h { margin: 0; font-weight: 600; font-size: var(--fs-m); }
  .ia-prose-p { margin: 0; }
  .ia-prose-ul { margin: 0; padding-inline-start: 18px; display: flex; flex-direction: column; gap: 2px; }
  .ia-inline-code { padding-inline: 4px; border-radius: var(--radius-s); background: var(--surface-2); font-family: var(--font-mono); font-size: var(--fs-xs); }
  .ia-code-fence { margin: 0; padding: 8px; border-radius: var(--radius-s); border: 1px solid var(--border); background: var(--bg); font-family: var(--font-mono); font-size: var(--fs-xs); overflow-x: auto; }
  .ia-streaming { display: inline-flex; align-items: center; gap: 6px; font-size: var(--fs-xs); color: var(--text-dim); }
  .ia-proposal { display: flex; flex-direction: column; border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--bg); overflow: hidden; }
  .ia-prop-head { display: flex; align-items: center; flex-wrap: wrap; gap: 4px; padding-block: 6px; padding-inline: 8px; background: var(--surface-2); border-block-end: 1px solid var(--border); }
  .ia-prop-file { display: inline-flex; align-items: center; gap: 4px; min-inline-size: 0; max-inline-size: 100%; padding-block: 2px; padding-inline: 4px; border: none; border-radius: var(--radius-s); background: transparent; color: var(--text); font-size: var(--fs-xs); cursor: pointer; }
  .ia-prop-file:hover { background: var(--hover); color: var(--accent-text); }
  .ia-prop-actions { display: inline-flex; align-items: center; gap: 4px; margin-inline-start: auto; flex-shrink: 0; }
  .ia-diff { max-block-size: 240px; overflow: auto; font-family: var(--font-mono); font-size: var(--fs-xs); line-height: 1.45; }
  .ia-diff-row { display: flex; min-inline-size: max-content; padding-inline: 8px; }
  .ia-diff-row.is-add { background: var(--success-soft); }
  .ia-diff-row.is-del { background: var(--danger-soft); }
  .ia-ln { inline-size: 32px; padding-inline-end: 8px; text-align: end; color: var(--text-dim); font-variant-numeric: tabular-nums; user-select: none; flex-shrink: 0; }
  .ia-sign { inline-size: 14px; flex-shrink: 0; user-select: none; color: var(--text-dim); }
  .ia-diff-row.is-add .ia-sign { color: var(--success); }
  .ia-diff-row.is-del .ia-sign { color: var(--danger); }
  .ia-code { white-space: pre; color: var(--text); }
  .ia-approval { display: flex; flex-direction: column; gap: 6px; padding: 8px 10px; border-radius: var(--radius-m); border: 1px solid var(--border-strong); background: var(--warning-soft); font-size: var(--fs-xs); }
  .ia-approval-head { display: flex; align-items: center; gap: 6px; color: var(--warning); font-weight: 500; }
  .ia-approval-title { color: var(--text); overflow-wrap: anywhere; }
  .ia-approval-reason { margin: 0; color: var(--text-dim); }
  .ia-approval-actions { display: flex; gap: 6px; }
  .ia-approval-done { color: var(--text-dim); }
  .ia-turn-err { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-radius: var(--radius-s); background: var(--danger-soft); color: var(--danger); font-size: var(--fs-xs); overflow-wrap: anywhere; }
  .ia-notice { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); text-align: center; }
  .ia-work { display: flex; flex-direction: column; min-inline-size: 0; }
  .ia-work-row { display: inline-flex; align-items: center; gap: 6px; inline-size: 100%; min-inline-size: 0; block-size: 24px; padding-inline: 6px; border: none; border-radius: var(--radius-s); background: transparent; color: var(--text-dim); font-family: var(--font-mono); font-size: var(--fs-xs); cursor: pointer; text-align: start; }
  .ia-work-row:hover:not(:disabled) { background: var(--hover); color: var(--text); }
  .ia-work-row:disabled { cursor: default; }
  .ia-work.is-failed .ia-work-row { color: var(--danger); }
  .ia-work-label { flex: 1; min-inline-size: 0; }
  .ia-work-detail { margin-block: 2px 0; margin-inline: 0; max-block-size: 160px; padding: 6px 8px; border-radius: var(--radius-s); border: 1px solid var(--border); background: var(--bg); color: var(--text-dim); font-family: var(--font-mono); font-size: var(--fs-xs); overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; }
  .ia-foot { display: flex; flex-direction: column; gap: 6px; padding: 8px; border-block-start: 1px solid var(--border); background: var(--surface); flex-shrink: 0; }
  .ia-tray { display: flex; align-items: center; flex-wrap: wrap; gap: 4px; }
  .ia-ctx-item { display: inline-flex; align-items: center; max-inline-size: 100%; block-size: 22px; border-radius: var(--radius-s); border: 1px solid var(--accent-line); background: var(--accent-soft); color: var(--accent-text); font-size: var(--fs-xs); overflow: hidden; }
  .ia-ctx-jump { display: inline-flex; align-items: center; gap: 4px; min-inline-size: 0; max-inline-size: 200px; block-size: 100%; padding-inline: 6px 4px; border: none; background: transparent; color: inherit; font-family: var(--font-mono); font-size: var(--fs-xs); cursor: pointer; }
  .ia-ctx-jump:hover:not(:disabled) { background: var(--accent-soft-strong); }
  .ia-ctx-jump:disabled { cursor: default; }
  .ia-ctx-rm { display: inline-flex; align-items: center; justify-content: center; inline-size: 20px; block-size: 100%; border: none; background: transparent; color: inherit; cursor: pointer; flex-shrink: 0; }
  .ia-ctx-rm:hover { background: var(--accent-soft-strong); }
  .ia-clear-btn { padding-block: 2px; padding-inline: 6px; border: none; border-radius: var(--radius-s); background: transparent; color: var(--text-dim); font: inherit; font-size: var(--fs-xs); cursor: pointer; }
  .ia-clear-btn:hover { color: var(--text); background: var(--hover); }
  .ia-composer-box { display: flex; flex-direction: column; border: 1px solid var(--control-border); border-radius: var(--radius-m); background: var(--surface-2); transition: border-color var(--dur-fast) var(--ease-out), box-shadow var(--dur-fast) var(--ease-out); }
  .ia-composer-box:focus-within { border-color: var(--accent-text); box-shadow: 0 0 0 2px var(--accent-soft-strong); }
  .ia-textarea { inline-size: 100%; min-block-size: 52px; max-block-size: 160px; padding-block: 8px 4px; padding-inline: 10px; border: none; background: transparent; color: var(--text); font: inherit; font-size: var(--fs-s); line-height: 1.45; resize: vertical; }
  .ia-textarea:focus { outline: none; }
  .ia-textarea::placeholder { color: var(--text-dim); }
  .ia-composer-bar { display: flex; align-items: center; gap: 4px; padding-block: 4px 6px; padding-inline: 6px; }
  .ia-mode-plan { color: var(--warning); background: var(--warning-soft); }
  .ia-cta { background: var(--accent-soft); border-color: var(--accent-line); color: var(--accent-text); }
  .ia-cta:hover:not(:disabled) { background: var(--accent-soft-strong); }
  .ia-stop { background: var(--danger-soft); border-color: var(--danger); color: var(--danger); }
  @media (max-width: 640px) { .ia-panel { border-inline-start: none; } }
  @media (pointer: coarse) {
    .ia-session-btn, .ia-work-row, .ia-ctx-item, .ia-clear-btn, .ia-prop-file { min-block-size: var(--hit-min); }
    .ia-ctx-rm { min-inline-size: var(--hit-min); }
  }
  @media (prefers-reduced-motion: reduce) { .ia-composer-box { transition: none; } }
</style>
