<script lang="ts">
  // The Browser page's agent — the Agent view of the page's Detail pane
  // (beside Marks): the agent's live SHELL (the same Terminal as Agents,
  // reused — exactly like Canvas's ConversationPanel, the DB Assistant, and
  // the inline PR-review / workflow terminals) plus an ask bar that submits
  // page + marks + question turns into it. readOnly is FALSE — the user types directly to the agent here (a real
  // two-way session), and can also ask through the bar so the agent gets the
  // page and marks as context. The bound session is an ordinary agent session
  // (it also appears in Agents), remembered per workspace.
  import Icon from '../../lib/components/Icon.svelte';
  import { toastError } from '../../lib/toastError';
  import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { sessionState } from '../../lib/status';
  import { events } from '../../lib/events.svelte';
  import AskBar from './AskBar.svelte';
  import { browser } from '../../lib/stores/browser.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';

  const sessionId = $derived(browser.agentSessionId);
  const session = $derived(sessionId ? (ws.getSession(sessionId) ?? null) : null);
  const status = $derived(sessionId ? (ws.statusMap[sessionId] ?? session?.status ?? null) : null);
  const readOnly = $derived(ws.myRole === 'viewer');
  /** The one shared session state (lib/status.ts) — same dot and words as Agents. */
  const sState = $derived(
    session
      ? sessionState(session, status, ws.needsYou[session.id] === true, { stale: events.state !== 'connected' })
      : null,
  );

  // Drop a stale binding once the session list is loaded: the session was
  // archived/deleted, or isn't an agent session any more.
  $effect(() => {
    if (!sessionId || ws.sessionsLoading) return;
    const s = ws.getSession(sessionId);
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
      toastError('Couldn’t start agent', e);
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

</script>

<section class="assistant" aria-label="Browser agent">
  <header class="head">
    {#if session}
      {#if hasProviderIcon(session.provider)}<ProviderIcon provider={session.provider ?? ''} size={13} />{/if}
      <span class="name" title={session.title}>{session.title}</span>
      {#if sState}<span class="state"><StatusDot state={sState} /> {sState.label}</span>{/if}
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
        <button class="btn small" onclick={pick} disabled={creating} title="Attach an agent session that’s already running">
          <Icon name="link" size={12} /> Attach…
        </button>
        <button class="btn small" onclick={() => void createAgent()} disabled={creating} title="Start a new agent session for this page">
          <Icon name="plus" size={12} /> {creating ? 'Starting…' : 'New agent'}
        </button>
      {/if}
    {/if}
  </header>

  {#if sessionId}
    <!-- The live, fully-interactive shell of the bound agent (the SAME Terminal
         as Agents). readOnly is FALSE — the user types directly to it. -->
    <div class="agent-shell otto-force-dark">
      {#key sessionId}
        <LazyTerminal {sessionId} readOnly={readOnly} resumable forceDark preferDom />
      {/key}
    </div>
  {/if}

  <AskBar {sessionId} terminalVisible={!!sessionId} unboundHint="Attach or start an agent to ask about this page." />
</section>

<style>
  .assistant {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    color: var(--text);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    padding-block: 6px;
    padding-inline: 16px 12px;
    border-bottom: 1px solid var(--separator);
    flex: none;
    min-height: 40px;
    min-width: 0;
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
  .state {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
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
  .agent-shell {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    position: relative;
    /* The Terminal is forceDark, so the shell is the same dark island
       (.otto-force-dark, tokens.css): the frame before it mounts is already the
       terminal's own background, never a light flash. */
    background: var(--term-bg);
  }
  .agent-shell > :global(*) {
    flex: 1 1 auto;
    min-height: 0;
  }
</style>
