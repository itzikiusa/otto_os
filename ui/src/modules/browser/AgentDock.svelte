<script lang="ts">
  // The Browser page's agent: the agent's live SHELL (the same Terminal as
  // Agents, reused — exactly like Canvas's ConversationPanel, the DB
  // Assistant, and the inline PR-review / workflow terminals) docked under
  // the page, plus an ask bar that submits page + marks + question turns into
  // it. readOnly is FALSE — the user types directly to the agent here (a real
  // two-way session), and can also ask through the bar so the agent gets the
  // page and marks as context. The bound session is an ordinary agent session
  // (it also appears in Agents), remembered per workspace.
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import AskBar from './AskBar.svelte';
  import { browser } from '../../lib/stores/browser.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import { viewport } from '../../lib/stores/viewport.svelte';

  const sessionId = $derived(browser.agentSessionId);
  const session = $derived(sessionId ? (ws.sessions.find((s) => s.id === sessionId) ?? null) : null);
  const status = $derived(sessionId ? (ws.statusMap[sessionId] ?? session?.status ?? null) : null);
  const readOnly = $derived(ws.myRole === 'viewer');
  const open = $derived(ui.browserAgentOpen);
  /** Sentence-case status words (the same ones the share view and session rows use). */
  const STATUS_LABEL: Record<string, string> = {
    running: 'Running',
    idle: 'Idle',
    exited: 'Ended',
    reconnectable: 'Disconnected — resume it in Agents',
  };

  // Drop a stale binding once the session list is loaded: the session was
  // archived/deleted, or isn't an agent session any more.
  $effect(() => {
    if (!sessionId || ws.sessionsLoading) return;
    const s = ws.sessions.find((x) => x.id === sessionId);
    if (!s || s.archived || s.kind !== 'agent') browser.setAgentSession(null);
  });

  // Which agent to start (chosen BEFORE a session exists; same source and
  // defaulting as Canvas / the DB Assistant).
  const providers = $derived(agentProviders());
  let provider = $state('');
  $effect(() => {
    if (!provider && providers.length > 0) provider = defaultAgentProvider();
  });

  let creating = $state(false);
  async function createAgent(): Promise<void> {
    if (creating || readOnly) return;
    creating = true;
    try {
      const s = await ws.createSessionQuiet({
        kind: 'agent',
        provider: provider || defaultAgentProvider(),
        title: 'Browser agent',
        cwd: ws.current?.root_path ?? null,
        // `browser:true` also hands the CLI the browser MCP server (same as
        // the New Session dialog's checkbox) so it can drive pages itself.
        meta: { origin: 'browser', browser: true },
      });
      browser.setAgentSession(s.id);
      ui.setBrowserAgentOpen(true);
    } catch (e) {
      toasts.error('Could not start agent', e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  // Attach an existing agent session (the shared clamped ctxMenu, so a long
  // list stays scrollable — same picker SendToSession uses).
  function pick(e: MouseEvent): void {
    const candidates = ws.agentSessions.filter((s) => s.id !== sessionId);
    if (candidates.length === 0) {
      toasts.warn('No other agent sessions', 'Start a new agent instead.');
      return;
    }
    const items: MenuItem[] = candidates.map((s) => ({
      label: `${s.title} (${s.provider})`,
      icon: 'terminal',
      action: () => {
        browser.setAgentSession(s.id);
        ui.setBrowserAgentOpen(true);
      },
    }));
    ctxMenu.show(e, items, { filter: items.length > 8, filterPlaceholder: 'Filter sessions…' });
  }

  // Drag the dock's top edge to resize it (persisted).
  let resizing = $state(false);
  function startResize(e: MouseEvent): void {
    e.preventDefault();
    resizing = true;
    const startY = e.clientY;
    const startH = ui.browserAgentH;
    const onMove = (ev: MouseEvent) => ui.setBrowserAgentH(startH + (startY - ev.clientY));
    const onUp = () => {
      resizing = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'row-resize';
    document.body.style.userSelect = 'none';
  }
</script>

<aside
  class="assistant"
  class:open={open && !!sessionId}
  class:resizing
  style={open && sessionId ? `height:${ui.browserAgentH}px` : undefined}
  aria-label="Browser agent"
>
  {#if open && sessionId && !viewport.isPhone}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="resize-handle" onmousedown={startResize} ondblclick={() => ui.setBrowserAgentH(280)} title="Drag to resize · double-click to reset"></div>
  {/if}

  <header class="head">
    {#if sessionId}
      <button
        class="icon-btn collapse"
        onclick={() => ui.setBrowserAgentOpen(!open)}
        title={open ? 'Collapse agent' : 'Expand agent'}
        aria-label={open ? 'Collapse agent' : 'Expand agent'}
        aria-expanded={open}
      >
        <Icon name={open ? 'chevronDown' : 'chevronUp'} size={12} />
      </button>
    {/if}
    <span class="title"><Icon name="terminal" size={14} /> Agent</span>
    {#if session}
      <span class="name" title={session.title}>{session.title}</span>
      {#if status === 'working'}<span class="working">Working…</span>{:else if status}<span class="dim">{STATUS_LABEL[status] ?? status}</span>{/if}
      <span class="spacer"></span>
      <button class="btn small" onclick={pick} title="Attach a different session">
        <Icon name="swap" size={12} /> Switch
      </button>
      <button class="icon-btn" onclick={() => browser.setAgentSession(null)} aria-label="Detach session" title="Detach — the session keeps running in Agents">
        <Icon name="x" size={14} />
      </button>
    {:else}
      <span class="dim no-session">No session attached</span>
      {#if !readOnly}
        <span class="spacer"></span>
        {#if providers.length > 1}
          <select class="provider" bind:value={provider} title="Which agent to start" aria-label="Agent to start" disabled={creating}>
            {#each providers as p (p)}
              <option value={p}>{p}</option>
            {/each}
          </select>
        {/if}
        <button class="btn small" onclick={pick} disabled={creating} title="Attach an agent session that's already running">
          <Icon name="link" size={12} /> Attach…
        </button>
        <button class="btn small" onclick={() => void createAgent()} disabled={creating} title="Start a new agent session for this page">
          <Icon name="plus" size={12} /> {creating ? 'Starting…' : 'New agent'}
        </button>
      {/if}
    {/if}
  </header>

  {#if open && sessionId}
    <!-- The live, fully-interactive shell of the bound agent (the SAME Terminal
         as Agents). readOnly is FALSE — the user types directly to it. -->
    <div class="agent-shell">
      {#key sessionId}
        <Terminal {sessionId} readOnly={readOnly} resumable forceDark preferDom />
      {/key}
    </div>
  {/if}

  <AskBar {sessionId} terminalVisible={open && !!sessionId} unboundHint="Attach or start an agent to ask about this page." />
</aside>

<style>
  .assistant {
    position: relative;
    display: flex;
    flex-direction: column;
    flex: none;
    min-height: 0;
    border-top: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
  }
  .assistant.open {
    /* height is set inline from ui.browserAgentH (drag-resizable) */
    min-height: 160px;
  }
  .resize-handle {
    position: absolute;
    top: -3px;
    left: 0;
    right: 0;
    height: 6px;
    cursor: row-resize;
    z-index: var(--z-sticky);
  }
  .resize-handle:hover,
  .assistant.resizing .resize-handle {
    background: color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    flex: none;
    min-height: 40px;
    min-width: 0;
  }
  .title {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-m);
    font-weight: 600;
    flex-shrink: 0;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
    font-size: var(--fs-s);
  }
  .dim {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .working {
    font-size: var(--fs-xs);
    color: var(--accent-text);
    font-weight: 600;
    white-space: nowrap;
  }
  .spacer {
    flex: 1;
  }
  .provider {
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
    padding: 2px 6px;
    height: 24px;
    cursor: pointer;
    text-transform: capitalize;
  }
  .collapse {
    width: 24px;
    height: 24px;
  }
  .agent-shell {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    position: relative;
    /* The Terminal is forceDark; this only shows for a frame while it mounts. */
    background: var(--bg);
  }
  .agent-shell > :global(*) {
    flex: 1 1 auto;
    min-height: 0;
  }
  /* Phone: the ask bar's placeholder already says nothing is attached, so the
     label goes and the actions keep one row. */
  @media (max-width: 640px) {
    .no-session {
      display: none;
    }
  }
</style>
