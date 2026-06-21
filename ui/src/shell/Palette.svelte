<script lang="ts">
  // ⌘K palette. Two modes: fuzzy command list (registry) and plain-English
  // orchestrator (⇥ toggles). English mode posts /orchestrate, shows the plan
  // for confirmation, then /orchestrate/execute.
  // Commands mode also fans out to GET /workspaces/{id}/search when the query
  // is ≥ 2 chars and a workspace is active; cross-module hits appear as a
  // second "Results" section below commands.
  import { api, isAbortError } from '../lib/api/client';
  import type {
    Action,
    ExecuteResult,
    OrchestrateResp,
    SearchHit,
  } from '../lib/api/types';
  import { registry, type Command } from '../lib/commands.svelte';
  import { parseCommand, parseClose, type CloseRequest } from '../lib/commandParser';
  import { fuzzyMatch } from '../lib/fuzzy';
  import { ui } from '../lib/stores/ui.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { toasts } from '../lib/toast.svelte';
  import Icon from '../lib/components/Icon.svelte';

  // ---- frecency tracking ----
  // Persist command-usage counts + last-used timestamps in localStorage so
  // frequently- and recently-used commands float to the top of the palette.
  // Shape: { [commandId]: { count: number; lastUsed: number } }
  const FRECENCY_KEY = 'otto_palette_frecency';

  interface FrecencyEntry { count: number; lastUsed: number; }
  type FrecencyMap = Record<string, FrecencyEntry>;

  function loadFrecency(): FrecencyMap {
    try {
      return JSON.parse(localStorage.getItem(FRECENCY_KEY) ?? '{}') as FrecencyMap;
    } catch {
      return {};
    }
  }

  function saveFrecency(m: FrecencyMap): void {
    try { localStorage.setItem(FRECENCY_KEY, JSON.stringify(m)); } catch { /* quota */ }
  }

  function recordUsage(id: string): void {
    const m = loadFrecency();
    const prev = m[id] ?? { count: 0, lastUsed: 0 };
    m[id] = { count: prev.count + 1, lastUsed: Date.now() };
    saveFrecency(m);
  }

  /** Blend frecency into a base fuzzy score. Returns a boost in [0, 20]. */
  function frecencyBoost(id: string, frecency: FrecencyMap): number {
    const e = frecency[id];
    if (!e) return 0;
    const countBoost = Math.min(e.count * 1.5, 12);          // up to 12
    const ageMs = Date.now() - e.lastUsed;
    const recencyBoost = Math.max(0, 8 - ageMs / (1000 * 60 * 60 * 24)); // decay over 8d
    return countBoost + recencyBoost;
  }

  let mode: 'commands' | 'english' = $state('commands');
  let query = $state('');
  let englishText = $state('');
  let optimize = $state(localStorage.getItem('otto_orch_optimize') === '1');
  let aiFallback = $state(localStorage.getItem('otto_orch_fallback') !== '0');
  let selected = $state(0);
  let busy = $state(false);
  let plan: Action[] | null = $state(null);
  let optimizedText: string | null = $state(null);
  let inputEl: HTMLInputElement | null = $state(null);
  let textareaEl: HTMLTextAreaElement | null = $state(null);

  // ---- cross-module search ----
  // Debounced fan-out to GET /workspaces/{id}/search when the query is
  // non-trivial and a workspace is active. Results render as a second section.
  let searchHits: SearchHit[] = $state([]);
  let searchBusy = $state(false);
  let searchAbort: AbortController | null = null;

  const SEARCH_DEBOUNCE_MS = 250;
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  /** Kick off a debounced cross-module search. Cancels any in-flight request. */
  function scheduleSearch(q: string): void {
    if (searchTimer !== null) clearTimeout(searchTimer);
    // Cancel in-flight request immediately so we don't show stale hits.
    searchAbort?.abort();
    searchAbort = null;
    if (q.trim().length < 2 || !ws.currentId) {
      searchHits = [];
      searchBusy = false;
      return;
    }
    searchBusy = true;
    searchTimer = setTimeout(() => {
      void doSearch(q.trim(), ws.currentId!);
    }, SEARCH_DEBOUNCE_MS);
  }

  async function doSearch(q: string, wsId: string): Promise<void> {
    const ctrl = new AbortController();
    searchAbort = ctrl;
    try {
      const hits = await api.get<SearchHit[]>(
        `/workspaces/${wsId}/search?q=${encodeURIComponent(q)}`,
        ctrl.signal,
      );
      searchHits = hits ?? [];
    } catch (e) {
      if (!isAbortError(e)) {
        // Silently swallow search errors — the palette still shows commands.
        searchHits = [];
      }
    } finally {
      if (searchAbort === ctrl) {
        searchAbort = null;
        searchBusy = false;
      }
    }
  }

  /** Icon name for a search hit kind. */
  function hitIcon(kind: string): string {
    switch (kind) {
      case 'story': return 'book-open';
      case 'workflow': return 'git-merge';
      case 'api_request': return 'zap';
      case 'swarm_task': return 'check-square';
      case 'swarm_project': return 'layers';
      case 'memory': return 'database';
      case 'repo': return 'git-branch';
      case 'broker_cluster': return 'server';
      default: return 'file';
    }
  }

  /** Emit an event/action for a search hit action button. */
  function hitAction(hit: SearchHit, action: string): void {
    // "open" navigates to the appropriate module view. Other actions dispatch
    // a custom event that module-level listeners can pick up.
    close();
    if (action === 'open') {
      // Navigation: emit a global custom event that App.svelte routes on.
      window.dispatchEvent(new CustomEvent('otto:open-hit', { detail: hit }));
    } else {
      // Contextual actions (send-to-agent, copy-context, rerun, review, …)
      // are emitted as a distinct event so they don't require nav awareness here.
      window.dispatchEvent(new CustomEvent('otto:hit-action', { detail: { hit, action } }));
    }
  }

  const filtered: { cmd: Command; score: number }[] = $derived.by(() => {
    const cmds = registry.all;
    const frecency = loadFrecency();
    if (query.trim() === '') {
      // No query: show top 14 sorted by frecency boost (most-used first).
      return cmds
        .map((cmd) => ({ cmd, score: frecencyBoost(cmd.id, frecency) }))
        .sort((a, b) => b.score - a.score)
        .slice(0, 14);
    }
    return cmds
      .map((cmd) => {
        const m = fuzzyMatch(query, `${cmd.title} ${cmd.keywords ?? ''} ${cmd.group ?? ''}`);
        if (!m) return null;
        const score = m.score + frecencyBoost(cmd.id, frecency);
        return { cmd, score };
      })
      .filter((x): x is { cmd: Command; score: number } => x !== null)
      .sort((a, b) => b.score - a.score)
      .slice(0, 14);
  });

  // Fire a debounced cross-module search whenever the query changes in commands
  // mode. Using $effect ensures this tracks `query` and `mode` reactively.
  $effect(() => {
    if (mode === 'commands') {
      scheduleSearch(query);
    } else {
      // Clear stale results when switching to English mode.
      searchHits = [];
      searchBusy = false;
      if (searchTimer !== null) { clearTimeout(searchTimer); searchTimer = null; }
      searchAbort?.abort();
      searchAbort = null;
    }
  });

  $effect(() => {
    // reset on open + focus, honoring the requested mode + prefill
    if (ui.paletteOpen) {
      mode = ui.paletteMode;
      query = '';
      selected = 0;
      plan = null;
      optimizedText = null;
      searchHits = [];
      searchBusy = false;
      if (mode === 'english') englishText = ui.palettePrefill;
      ui.palettePrefill = '';
      queueMicrotask(() => {
        if (mode === 'commands') inputEl?.focus();
        else {
          textareaEl?.focus();
          // place caret at the end (after any prefill like "broadcast ")
          const len = englishText.length;
          textareaEl?.setSelectionRange(len, len);
        }
      });
    } else {
      // Palette closed — cancel any pending search so we don't waste a round-trip.
      if (searchTimer !== null) { clearTimeout(searchTimer); searchTimer = null; }
      searchAbort?.abort();
      searchAbort = null;
    }
  });

  // Free text in commands mode is always offered as an orchestrator ask, so
  // "open 2 claude sessions" works without knowing about the English mode.
  const askRow = $derived(query.trim() !== '');
  const totalRows = $derived(filtered.length + (askRow ? 1 : 0));

  $effect(() => {
    void totalRows;
    if (selected >= totalRows) selected = Math.max(0, totalRows - 1);
  });

  function askOtto(): void {
    englishText = query;
    mode = 'english';
    plan = null;
    optimizedText = null;
    void submitEnglish();
    queueMicrotask(() => textareaEl?.focus());
  }

  function close(): void {
    ui.paletteOpen = false;
  }

  function toggleMode(): void {
    mode = mode === 'commands' ? 'english' : 'commands';
    plan = null;
    optimizedText = null;
    queueMicrotask(() => (mode === 'commands' ? inputEl?.focus() : textareaEl?.focus()));
  }

  function setOptimize(v: boolean): void {
    optimize = v;
    localStorage.setItem('otto_orch_optimize', v ? '1' : '0');
  }
  function setFallback(v: boolean): void {
    aiFallback = v;
    localStorage.setItem('otto_orch_fallback', v ? '1' : '0');
  }

  async function run(cmd: Command): Promise<void> {
    // Record this invocation before closing so loadFrecency() in the next open
    // already sees the updated entry.
    recordUsage(cmd.id);
    close();
    try {
      await cmd.run();
    } catch (e) {
      toasts.error('Command failed', e instanceof Error ? e.message : String(e));
    }
  }

  function onCommandsKey(e: KeyboardEvent): void {
    if (e.key === 'Tab') {
      e.preventDefault();
      toggleMode();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      selected = Math.min(selected + 1, totalRows - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      selected = Math.max(selected - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const item = filtered[selected];
      if (item) void run(item.cmd);
      else if (askRow && selected === filtered.length) askOtto();
    } else if (e.key === 'Escape') {
      close();
    }
  }

  function onEnglishKey(e: KeyboardEvent): void {
    if (e.key === 'Tab' && englishText === '') {
      e.preventDefault();
      toggleMode();
    } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void submitEnglish();
    } else if (e.key === 'Escape') {
      if (plan) plan = null;
      else close();
    }
  }

  async function submitEnglish(): Promise<void> {
    if (englishText.trim() === '' || busy) return;
    if (!ws.currentId) {
      toasts.error('No workspace selected', 'Pick a workspace in the navigator first.');
      return;
    }

    // Close-by-position is purely client-side (it needs the on-screen layout),
    // so handle it before anything else: "close sessions 1,2" → close the panes
    // in place 1 and place 2.
    const closeReq = parseClose(englishText);
    if (closeReq) {
      await closeByPosition(closeReq);
      return;
    }

    // Deterministic first: common intents ("open 2 claude sessions",
    // "broadcast run the tests") execute instantly with no LLM and no
    // confirmation step. Only fall through to the AI planner when this
    // can't parse the request AND the user enabled AI fallback.
    const literal = parseCommand(englishText);
    if (literal && literal.length > 0) {
      plan = literal;
      await executePlan();
      return;
    }

    if (!aiFallback) {
      toasts.warn(
        "Couldn't parse that",
        'Try e.g. "open 2 claude sessions", or enable AI fallback for free-form requests.',
      );
      return;
    }

    busy = true;
    plan = null;
    try {
      const resp = await api.post<OrchestrateResp>(`/workspaces/${ws.currentId}/orchestrate`, {
        text: englishText,
        optimize,
        ai_fallback: aiFallback,
        focused_session_id: ws.activeSessionId,
      });
      plan = resp.plan;
      optimizedText = resp.optimized_text;
    } catch (e) {
      toasts.error('Orchestrate failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  /** Close sessions by their on-screen position (place 1, 2, …). The visible
   *  layout order is how the user counts: the tiled grid when tiled, else the
   *  side-by-side panes. "close" archives (recoverable); "kill/delete" removes. */
  async function closeByPosition(req: CloseRequest): Promise<void> {
    const order: string[] =
      ws.viewMode === 'tiled' && !ws.maximizedId
        ? ws.mainSessions.map((s) => s.id)
        : ws.panes.filter((id) => ws.sessions.some((s) => s.id === id));

    const ids = req.all
      ? order.slice()
      : req.positions.map((p) => order[p - 1]).filter((id): id is string => !!id);

    if (ids.length === 0) {
      toasts.warn(
        'Nothing to close',
        req.all ? 'No open sessions.' : `No session at position ${req.positions.join(', ')}.`,
      );
      return;
    }

    busy = true;
    try {
      for (const id of ids) {
        if (req.permanent) await ws.killSession(id);
        else await ws.archiveSession(id);
      }
      if (req.permanent) {
        toasts.success('Sessions killed', `${ids.length} session${ids.length === 1 ? '' : 's'} removed`);
      }
      close();
      englishText = '';
    } catch (e) {
      toasts.error('Close failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  async function executePlan(): Promise<void> {
    if (!ws.currentId || !plan || busy) return;
    busy = true;
    // Remember which sessions exist now, so we can OPEN whatever gets spawned.
    const before = new Set(ws.sessions.map((s) => s.id));
    try {
      const resp = await api.post<{ results: ExecuteResult[] }>(
        `/workspaces/${ws.currentId}/orchestrate/execute`,
        { plan },
      );
      const ok = resp.results.filter((r) => r.ok).length;
      const fail = resp.results.length - ok;
      if (fail === 0) toasts.success('Plan executed', `${ok} action${ok === 1 ? '' : 's'} completed`);
      else toasts.warn('Plan partially executed', `${ok} ok, ${fail} failed`);
      await ws.refreshSessions();
      // Foreground the freshly-spawned sessions so they're immediately workable
      // (not left running in the background). Multiple → tile them all.
      const created = ws.sessions.filter((s) => !before.has(s.id) && !s.archived);
      if (created.length > 1) ws.setViewMode('tiled');
      // Open all sessions in the store; navigate the route to the last one so
      // Back/Forward can return to it. Store-only openSession for all but last.
      for (const s of created.slice(0, -1)) ws.openSession(s.id);
      if (created.length > 0) ws.navigateToSession(created[created.length - 1].id);
      close();
      englishText = '';
      plan = null;
    } catch (e) {
      toasts.error('Execution failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  function describeAction(a: Action): string {
    switch (a.action) {
      case 'spawn_sessions':
        return `Spawn ${a.count} ${a.provider} session${a.count === 1 ? '' : 's'}`;
      case 'broadcast':
        return `Broadcast to all sessions: "${a.text}"`;
      case 'open_connection':
        return `Open connection ${a.connection_id}`;
      case 'run_command':
        return `Send to session: "${a.text}"`;
    }
  }
</script>

{#if ui.paletteOpen}
  <div
    class="pal-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) close();
    }}
  >
    <div class="palette" role="dialog" aria-modal="true" aria-label="Command palette">
      <div class="pal-mode-row">
        <div class="segmented">
          <button class:active={mode === 'commands'} onclick={() => mode !== 'commands' && toggleMode()}>
            Commands
          </button>
          <button class:active={mode === 'english'} onclick={() => mode !== 'english' && toggleMode()}>
            Plain English
          </button>
        </div>
        <span class="pal-hint">⇥ to switch</span>
      </div>

      {#if mode === 'commands'}
        <div class="pal-input-row">
          <Icon name="search" size={14} />
          <input
            bind:this={inputEl}
            bind:value={query}
            placeholder="Type a command…"
            onkeydown={onCommandsKey}
            spellcheck="false"
          />
        </div>
        <div class="pal-list" role="listbox">
          {#each filtered as item, i (item.cmd.id)}
            <button
              class="pal-item"
              class:selected={i === selected}
              role="option"
              aria-selected={i === selected}
              onmouseenter={() => (selected = i)}
              onclick={() => run(item.cmd)}
            >
              <span class="pal-item-title">{item.cmd.title}</span>
              <span class="grow"></span>
              {#if item.cmd.group}<span class="pal-group">{item.cmd.group}</span>{/if}
              {#if item.cmd.shortcut}<kbd>{item.cmd.shortcut}</kbd>{/if}
            </button>
          {:else}
            {#if !askRow}
              <div class="pal-empty">No matching commands</div>
            {/if}
          {/each}
          {#if askRow}
            <button
              class="pal-item pal-ask"
              class:selected={selected === filtered.length}
              role="option"
              aria-selected={selected === filtered.length}
              onmouseenter={() => (selected = filtered.length)}
              onclick={askOtto}
            >
              <Icon name="zap" size={13} />
              <span class="pal-item-title">Ask Otto: "{query}"</span>
              <span class="grow"></span>
              <span class="pal-group">plain english</span>
            </button>
          {/if}

          {#if searchHits.length > 0 || searchBusy}
            <div class="pal-section-header">
              {searchBusy ? 'Searching…' : 'Results'}
            </div>
          {/if}
          {#each searchHits as hit (hit.kind + ':' + hit.id)}
            <div class="pal-hit">
              <div class="pal-hit-main">
                <Icon name={hitIcon(hit.kind)} size={12} />
                <span class="pal-hit-title">{hit.title}</span>
                {#if hit.subtitle}
                  <span class="pal-hit-sub">{hit.subtitle}</span>
                {/if}
                <span class="grow"></span>
                <span class="pal-group pal-hit-kind">{hit.kind.replace('_', ' ')}</span>
              </div>
              <div class="pal-hit-actions">
                {#each hit.actions as action}
                  <button class="pal-hit-btn" onclick={() => hitAction(hit, action)}>
                    {action}
                  </button>
                {/each}
              </div>
            </div>
          {/each}
        </div>
      {:else}
        <div class="pal-english">
          <textarea
            bind:this={textareaEl}
            bind:value={englishText}
            placeholder="Describe what you want… e.g. 'spawn 3 claude agents and tell them to fix the failing tests'"
            onkeydown={onEnglishKey}
            rows="3"
            spellcheck="false"
          ></textarea>
          <div class="pal-english-row">
            <button class="pill-toggle" class:on={optimize} onclick={() => setOptimize(!optimize)}>
              <Icon name="zap" size={11} /> optimize
            </button>
            <button class="pill-toggle" class:on={aiFallback} onclick={() => setFallback(!aiFallback)}>
              AI fallback
            </button>
            <span class="grow"></span>
            <button class="btn primary" disabled={busy || englishText.trim() === ''} onclick={submitEnglish}>
              {busy && !plan ? 'Planning…' : 'Plan it  ⌘↵'}
            </button>
          </div>

          {#if optimizedText}
            <div class="pal-optimized">
              <span class="dim">optimized:</span>
              {optimizedText}
            </div>
          {/if}

          {#if plan}
            <div class="pal-plan">
              <div class="pal-plan-title">Proposed plan — confirm to execute</div>
              {#each plan as a, i (i)}
                <div class="pal-plan-item">
                  <span class="pal-plan-num">{i + 1}</span>
                  {describeAction(a)}
                </div>
              {:else}
                <div class="pal-empty">Planner returned no actions</div>
              {/each}
              <div class="pal-plan-actions">
                <button class="btn" onclick={() => (plan = null)}>Cancel</button>
                <button class="btn primary" disabled={busy || plan.length === 0} onclick={executePlan}>
                  {busy ? 'Executing…' : 'Execute plan'}
                </button>
              </div>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .pal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 150;
    background: rgba(0, 0, 0, 0.25);
    display: flex;
    justify-content: center;
    padding-top: 12vh;
    animation: fade-in 120ms ease-out;
  }
  .palette {
    width: 560px;
    max-width: calc(100vw - 48px);
    max-height: 60vh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow);
    overflow: hidden;
    align-self: flex-start;
    animation: pal-in 150ms ease-out;
  }
  .pal-mode-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px 0;
  }
  .pal-hint {
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .pal-input-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    color: var(--text-dim);
  }
  .pal-input-row input {
    flex: 1;
    border: none;
    background: transparent;
    font-size: 14px;
    color: var(--text);
    outline: none;
  }
  .pal-list {
    overflow-y: auto;
    padding: 6px;
  }
  .pal-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 30px;
    padding: 0 10px;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    font-size: 12.5px;
    color: var(--text);
    cursor: pointer;
    text-align: left;
  }
  .pal-item.selected {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .pal-group {
    font-size: 10.5px;
    color: var(--text-dim);
  }
  kbd {
    font-family: var(--font-ui);
    font-size: 10.5px;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 1px 5px;
  }
  .pal-ask {
    border-top: 1px solid var(--border);
    border-radius: 0 0 var(--radius-s) var(--radius-s);
    color: var(--accent);
  }
  .pal-empty {
    padding: 16px;
    text-align: center;
    font-size: 12px;
    color: var(--text-dim);
  }
  .pal-english {
    padding: 10px 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    overflow-y: auto;
  }
  .pal-english textarea {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    padding: 9px 11px;
    font-size: 13px;
    line-height: 1.5;
    resize: vertical;
    color: var(--text);
  }
  .pal-english textarea:focus {
    outline: none;
    border-color: var(--accent);
  }
  .pal-english-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .pal-optimized {
    font-size: 12px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent);
  }
  .pal-plan {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .pal-plan-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .pal-plan-item {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
    padding: 4px 2px;
  }
  .pal-plan-num {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: var(--surface-2);
    border: 1px solid var(--border);
    font-size: 10.5px;
    display: grid;
    place-items: center;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .pal-plan-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
  .pal-section-header {
    padding: 6px 10px 2px;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    border-top: 1px solid var(--border);
    margin-top: 4px;
  }
  .pal-hit {
    padding: 4px 8px 4px 10px;
    border-radius: var(--radius-s);
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .pal-hit:hover {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .pal-hit-main {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--text);
    min-width: 0;
  }
  .pal-hit-title {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 220px;
  }
  .pal-hit-sub {
    font-size: 10.5px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 140px;
  }
  .pal-hit-kind {
    flex-shrink: 0;
  }
  .pal-hit-actions {
    display: flex;
    gap: 4px;
    padding-left: 18px;
  }
  .pal-hit-btn {
    padding: 1px 6px;
    font-size: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
    line-height: 1.6;
  }
  .pal-hit-btn:hover {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    border-color: var(--accent);
    color: var(--accent);
  }
  @keyframes fade-in {
    from {
      opacity: 0;
    }
  }
  @keyframes pal-in {
    from {
      opacity: 0;
      transform: translateY(-6px) scale(0.99);
    }
  }
</style>
