<script lang="ts">
  // Analysis tab — multi-provider per-lens config, summarizer select, live polling.
  import { product } from '../../lib/stores/product.svelte';
  import { api } from '../../lib/api/client';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { ProductAnalysis, ProductAnalysisDetail, ProductAnalysisAgent } from './types';

  // ── Lens definitions ─────────────────────────────────────────────────────────
  const LENS_DEFS = [
    { skill: 'po-story-overview',           label: 'PO Overview' },
    { skill: 'story-architecture-overview', label: 'Architecture' },
    { skill: 'story-clarifying-questions',  label: 'Clarifying Questions' },
  ] as const;

  type LensSkill = (typeof LENS_DEFS)[number]['skill'];

  // ── Provider state (fetched from /meta) ──────────────────────────────────────
  let availableProviders = $state<string[]>(['claude']);
  let metaLoaded = $state(false);

  async function fetchMeta(): Promise<void> {
    if (metaLoaded) return;
    try {
      const meta = await api.get<{ providers: string[] }>('/meta');
      availableProviders = (meta.providers ?? []).filter((p) => p !== 'shell');
      if (availableProviders.length === 0) availableProviders = ['claude'];
    } catch {
      availableProviders = ['claude'];
    }
    metaLoaded = true;
    // Default all lens selected-providers to first available if 'claude' not in list.
    for (const skill of Object.keys(lensProviders) as LensSkill[]) {
      lensProviders[skill] = lensProviders[skill].filter((p) => availableProviders.includes(p));
      if (lensProviders[skill].length === 0 && availableProviders.length > 0) {
        lensProviders[skill] = [availableProviders[0]];
      }
    }
    if (!availableProviders.includes(summarizerProvider)) {
      summarizerProvider = availableProviders[0] ?? 'claude';
    }
  }

  // ── Per-lens UI state ─────────────────────────────────────────────────────────
  // Whether each lens is enabled.
  let lensEnabled = $state<Record<LensSkill, boolean>>({
    'po-story-overview': true,
    'story-architecture-overview': true,
    'story-clarifying-questions': true,
  });

  // Selected providers per lens (multi-select chips).
  let lensProviders = $state<Record<LensSkill, string[]>>({
    'po-story-overview': ['claude'],
    'story-architecture-overview': ['claude'],
    'story-clarifying-questions': ['claude'],
  });

  let summarizerProvider = $state<string>('claude');

  // ── Focus input ───────────────────────────────────────────────────────────────
  let focusText = $state<string>('');

  // ── Other UI state ────────────────────────────────────────────────────────────
  let running = $state(false);
  let loadingHistory = $state(false);
  let historyLoaded = $state(false);
  let activeDetail = $state<ProductAnalysisDetail | null>(null);
  let activeId = $state<string | null>(null);
  let collapsed = $state<Record<string, boolean>>({});

  // ── Load meta on mount ────────────────────────────────────────────────────────
  $effect(() => {
    void fetchMeta();
    return () => {};
  });

  // ── Polling ────────────────────────────────────────────────────────────────────
  let pollTimer: ReturnType<typeof setInterval> | null = null;

  function clearPoll(): void {
    if (pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  }

  function isTerminal(status: string): boolean {
    return status === 'done' || status === 'error' || status === 'partial';
  }

  async function pollOnce(): Promise<void> {
    if (!activeId) return;
    try {
      const detail = await product.getAnalysis(activeId);
      activeDetail = detail;
      if (isTerminal(detail.analysis.status)) clearPoll();
    } catch (e) {
      console.error('[AnalysisTab] poll error', e);
    }
  }

  function startPolling(id: string): void {
    clearPoll();
    activeId = id;
    void pollOnce();
    pollTimer = setInterval(() => { void pollOnce(); }, 3000);
  }

  // Stop polling when story changes or tab unmounts.
  $effect(() => {
    product.selectedId;
    return () => { clearPoll(); };
  });

  $effect(() => {
    if (activeDetail && isTerminal(activeDetail.analysis.status)) clearPoll();
  });

  // ── Story / derived ────────────────────────────────────────────────────────────
  const story = $derived(product.detail?.story ?? null);

  $effect(() => {
    product.selectedId;
    activeDetail = null;
    activeId = null;
    historyLoaded = false;
    collapsed = {};
    clearPoll();
  });

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function toggleLens(skill: LensSkill): void {
    lensEnabled[skill] = !lensEnabled[skill];
  }

  function toggleLensProvider(skill: LensSkill, provider: string): void {
    const cur = lensProviders[skill];
    if (cur.includes(provider)) {
      lensProviders[skill] = cur.filter((p) => p !== provider);
    } else {
      lensProviders[skill] = [...cur, provider];
    }
  }

  // Derived: can we run?
  const canRun = $derived(
    LENS_DEFS.some(
      (l) => lensEnabled[l.skill] && lensProviders[l.skill].length > 0
    )
  );

  // ── Actions ────────────────────────────────────────────────────────────────────

  async function runAnalysis(): Promise<void> {
    if (running || !canRun) return;
    const agents = LENS_DEFS.filter(
      (l) => lensEnabled[l.skill] && lensProviders[l.skill].length > 0
    ).map((l) => ({
      skill: l.skill,
      name: l.label,
      providers: lensProviders[l.skill],
    }));
    running = true;
    try {
      const trimmedFocus = focusText.trim() || undefined;
      const analysis = await product.analyze({
        agents,
        summarizer_provider: summarizerProvider || availableProviders[0],
        focus: trimmedFocus,
      });
      startPolling(analysis.id);
    } catch (e) {
      toasts.error('Analysis failed to start', product.errMsg(e));
    } finally {
      running = false;
    }
  }

  async function loadHistory(): Promise<void> {
    if (historyLoaded) return;
    loadingHistory = true;
    try {
      await product.loadAnalyses();
      historyLoaded = true;
    } catch (e) {
      toasts.error('Could not load history', product.errMsg(e));
    } finally {
      loadingHistory = false;
    }
  }

  async function selectHistory(a: ProductAnalysis): Promise<void> {
    if (a.id === activeId) return;
    clearPoll();
    activeId = a.id;
    try {
      activeDetail = await product.getAnalysis(a.id);
      if (!isTerminal(activeDetail.analysis.status)) startPolling(a.id);
    } catch (e) {
      toasts.error('Could not load analysis', product.errMsg(e));
    }
  }

  // Inline terminal state — multiple may be open at once, keyed by session id.
  // NOTE: No ws.openSession() here — the inline <Terminal sessionId={...} />
  // connects directly by id. Calling openSession would push it into the Agents
  // grid sidebar which we don't want.
  let openTerminals = $state<Set<string>>(new Set());
  function toggleTerminal(sid: string): void {
    const next = new Set(openTerminals);
    if (next.has(sid)) next.delete(sid);
    else next.add(sid);
    openTerminals = next;
  }

  // ── Per-agent retry ───────────────────────────────────────────────────────
  // Tracks which agent IDs currently have a retry in-flight.
  let retryingAgents = $state<Set<string>>(new Set());

  async function retryAgent(analysisId: string, agentId: string, agentName: string): Promise<void> {
    if (retryingAgents.has(agentId)) return;
    const next = new Set(retryingAgents);
    next.add(agentId);
    retryingAgents = next;
    try {
      await product.retryAgent(analysisId, agentId);
      toasts.info(`Re-running ${agentName}…`);
      // Resume polling so results refresh automatically.
      if (analysisId) startPolling(analysisId);
    } catch (e) {
      toasts.error('Retry failed', product.errMsg(e));
    } finally {
      const done = new Set(retryingAgents);
      done.delete(agentId);
      retryingAgents = done;
    }
  }

  // ── Findings parsing ──────────────────────────────────────────────────────────

  interface OpenQuestion { text: string; rationale: string; category: string; }
  interface SuggestedLearning { kind: string; title: string; body: string; }
  interface Findings {
    summary?: string;
    related_repos?: string[];
    functionalities?: string[];
    integration_points?: string[];
    risks?: string[];
    open_questions?: OpenQuestion[];
    suggested_learnings?: SuggestedLearning[];
  }

  function parseFindings(json: string | null): Findings | null {
    if (!json) return null;
    try { return JSON.parse(json) as Findings; } catch { return null; }
  }

  function statusClass(status: string): string {
    switch (status) {
      case 'done':    return 'pill-done';
      case 'error':   return 'pill-error';
      case 'running': return 'pill-running';
      case 'partial': return 'pill-partial';
      case 'waiting': return 'pill-waiting';
      default:        return 'pill-pending';
    }
  }

  function toggleCollapse(key: string): void {
    collapsed = { ...collapsed, [key]: !collapsed[key] };
  }

  function fmtDate(s: string | null): string {
    if (!s) return '';
    try { return new Date(s).toLocaleString(); } catch { return s; }
  }

  function shortId(id: string): string {
    return id.slice(0, 8);
  }

  // Separate summarizer agent from regular lens agents
  const currentAgents = $derived(activeDetail?.agents ?? []);
  const summarizerAgent = $derived(
    currentAgents.find((a) => a.skill === 'summarizer' || a.name?.toLowerCase().startsWith('summarizer'))
  );
  const lensAgents = $derived(
    currentAgents.filter((a) => a !== summarizerAgent)
  );
  const currentAnalysis = $derived(activeDetail?.analysis ?? null);
  const analysisStatus = $derived(currentAnalysis?.status ?? '');
</script>

{#if !story}
  <div class="muted">No story selected.</div>
{:else}
  <div class="analysis-tab">

    <!-- ── Run panel ───────────────────────────────────────────────────────── -->
    <section class="run-panel card">
      <div class="run-header">
        <span class="section-head">Configure</span>
      </div>

      <div class="lens-grid">
        {#each LENS_DEFS as lens (lens.skill)}
          <div class="lens-row" class:lens-disabled={!lensEnabled[lens.skill]}>
            <!-- Enable toggle -->
            <label class="lens-toggle">
              <input
                type="checkbox"
                checked={lensEnabled[lens.skill]}
                onchange={() => toggleLens(lens.skill)}
                disabled={running}
              />
              <span class="lens-name">{lens.label}</span>
            </label>

            <!-- Provider chips -->
            <div class="chip-group" class:chips-muted={!lensEnabled[lens.skill]}>
              {#each availableProviders as p (p)}
                <button
                  class="chip"
                  class:chip-on={lensProviders[lens.skill].includes(p)}
                  disabled={running || !lensEnabled[lens.skill]}
                  onclick={() => toggleLensProvider(lens.skill, p)}
                  title="{p}"
                >
                  {p}
                </button>
              {/each}
            </div>
          </div>
        {/each}
      </div>

      <!-- Focus input -->
      <div class="focus-wrap">
        <label class="field-label" for="focus-input">Focus <span class="focus-optional">(optional)</span></label>
        <textarea
          id="focus-input"
          class="focus-input"
          rows={2}
          placeholder="What to focus on — e.g. 'security implications' or 'edge cases around payments'"
          bind:value={focusText}
          disabled={running}
        ></textarea>
      </div>

      <!-- Bottom row: summarizer + run button -->
      <div class="bottom-row">
        <div class="summarizer-wrap">
          <label class="field-label" for="summarizer-sel">Summarizer</label>
          <select
            id="summarizer-sel"
            class="small-select"
            bind:value={summarizerProvider}
            disabled={running}
          >
            {#each availableProviders as p (p)}
              <option value={p}>{p}</option>
            {/each}
          </select>
        </div>

        <button
          class="run-btn"
          onclick={runAnalysis}
          disabled={running || !canRun}
        >
          {running ? 'Starting…' : 'Run analysis'}
        </button>
      </div>
    </section>

    <!-- ── History selector ─────────────────────────────────────────────────── -->
    <section class="history-row">
      <span class="field-label">History</span>
      <select
        class="hist-select"
        onfocus={loadHistory}
        onchange={(e) => {
          const id = (e.target as HTMLSelectElement).value;
          const found = product.analyses.find((a) => a.id === id);
          if (found) void selectHistory(found);
        }}
        disabled={loadingHistory}
        value={activeId ?? ''}
      >
        <option value="">— select a past run —</option>
        {#each product.analyses as a (a.id)}
          <option value={a.id}>
            {fmtDate(a.created_at)} · {a.status}
          </option>
        {/each}
      </select>
      {#if loadingHistory}
        <span class="dim-sm">Loading…</span>
      {/if}
    </section>

    <!-- ── Active run area ──────────────────────────────────────────────────── -->
    {#if activeDetail}

      <!-- Synthesized summary (top, once done/partial) -->
      {#if (analysisStatus === 'done' || analysisStatus === 'partial') && currentAnalysis?.summary}
        <section class="synthesis-card card">
          <div class="section-head">Synthesized Summary</div>
          {#if summarizerAgent}
            <div class="summarizer-badge">
              <span class="pill {statusClass(summarizerAgent.status)}">{summarizerAgent.status}</span>
              {#if summarizerAgent.session_id}
                <button
                  class="open-link"
                  onclick={() => toggleTerminal(summarizerAgent!.session_id!)}
                  title="Open summarizer session"
                >
                  {openTerminals.has(summarizerAgent.session_id) ? 'Hide' : 'Open'} session: {shortId(summarizerAgent.session_id)}
                </button>
              {/if}
            </div>
          {/if}
          {#if summarizerAgent?.session_id && openTerminals.has(summarizerAgent.session_id)}
            <div class="inline-term">
              {#key summarizerAgent.session_id}
                <Terminal sessionId={summarizerAgent.session_id} forceDark />
              {/key}
            </div>
          {/if}
          <p class="synthesis-body">{currentAnalysis.summary}</p>
        </section>
      {/if}

      <!-- Agent progress rows -->
      <section class="agents-section card">
        <div class="agents-header">
          <span class="section-head">Agents</span>
          <span class="pill {statusClass(analysisStatus)}">
            {#if analysisStatus === 'running' || analysisStatus === 'waiting'}
              <span class="spinner-xs"></span>
            {/if}
            {analysisStatus}
          </span>
        </div>
        {#each currentAgents as agent (agent.id)}
          {@const isLens = agent !== summarizerAgent}
          {@const isRetrying = retryingAgents.has(agent.id)}
          <div class="agent-row">
            <div class="agent-info">
              <span class="agent-name">{agent.name || agent.skill}</span>
              <span class="agent-meta">{agent.provider}{agent.model ? ` / ${agent.model}` : ''}</span>
            </div>
            <div class="agent-right">
              {#if agent.session_id}
                <button
                  class="open-link"
                  onclick={() => toggleTerminal(agent.session_id!)}
                  title="Open agent session"
                >
                  {openTerminals.has(agent.session_id) ? 'Hide' : 'Open'} {shortId(agent.session_id)}
                </button>
              {/if}
              {#if isLens && (agent.status === 'error' || agent.status === 'done')}
                <button
                  class="btn-retry"
                  disabled={isRetrying}
                  onclick={() => retryAgent(currentAnalysis!.id, agent.id, agent.name || agent.skill)}
                  title="Re-run this agent"
                >
                  {isRetrying ? 'Retrying…' : 'Retry'}
                </button>
              {/if}
              <span class="pill {statusClass(agent.status)}">
                {#if agent.status === 'running' || agent.status === 'waiting'}
                  <span class="spinner-xs"></span>
                {/if}
                {agent.status}
              </span>
            </div>
          </div>
          {#if agent.error && agent.status === 'error' && !isLens}
            <p class="agent-note error-note">{agent.error}</p>
          {/if}
          {#if agent.status === 'waiting'}
            <p class="agent-waiting">
              This agent looks blocked on input. Click <strong>Open</strong> to view its session and respond.
            </p>
          {/if}
          {#if agent.session_id && openTerminals.has(agent.session_id)}
            <div class="inline-term">
              {#key agent.session_id}
                <Terminal sessionId={agent.session_id} forceDark />
              {/key}
            </div>
          {/if}
        {/each}
      </section>

      <!-- Per-lens findings -->
      {#each lensAgents as agent (agent.id)}
        {#if agent.status === 'done' && agent.findings_json}
          {@const findings = parseFindings(agent.findings_json)}
          {#if findings}
            <section class="findings-card card">
              <div class="findings-header">
                <span class="findings-agent-name">{agent.name || agent.skill}</span>
                <div class="findings-right">
                  {#if agent.session_id}
                    <button
                      class="open-link"
                      onclick={() => toggleTerminal(agent.session_id!)}
                      title="Open session"
                    >
                      {openTerminals.has(agent.session_id) ? 'Hide' : 'Open'} {shortId(agent.session_id)}
                    </button>
                  {/if}
                  {#if currentAnalysis}
                    <button
                      class="btn-retry"
                      disabled={retryingAgents.has(agent.id)}
                      onclick={() => retryAgent(currentAnalysis!.id, agent.id, agent.name || agent.skill)}
                      title="Re-run this agent"
                    >
                      {retryingAgents.has(agent.id) ? 'Retrying…' : 'Retry'}
                    </button>
                  {/if}
                  <span class="pill pill-done">done</span>
                </div>
              </div>

              {#if agent.session_id && openTerminals.has(agent.session_id)}
                <div class="inline-term">
                  {#key agent.session_id}
                    <Terminal sessionId={agent.session_id} forceDark />
                  {/key}
                </div>
              {/if}

              {#if findings.summary}
                <p class="findings-summary">{findings.summary}</p>
              {/if}

              <!-- Related repos -->
              {#if findings.related_repos && findings.related_repos.length > 0}
                {@const key = agent.id + ':repos'}
                <div class="collapsible">
                  <button class="coll-trigger" onclick={() => toggleCollapse(key)}>
                    <span class="coll-arrow">{collapsed[key] ? '▶' : '▼'}</span>
                    Related Repos
                    <span class="coll-count">({findings.related_repos.length})</span>
                  </button>
                  {#if !collapsed[key]}
                    <ul class="findings-list">
                      {#each findings.related_repos as repo}
                        <li class="mono-sm">{repo}</li>
                      {/each}
                    </ul>
                  {/if}
                </div>
              {/if}

              <!-- Functionalities -->
              {#if findings.functionalities && findings.functionalities.length > 0}
                {@const key = agent.id + ':func'}
                <div class="collapsible">
                  <button class="coll-trigger" onclick={() => toggleCollapse(key)}>
                    <span class="coll-arrow">{collapsed[key] ? '▶' : '▼'}</span>
                    Functionalities
                    <span class="coll-count">({findings.functionalities.length})</span>
                  </button>
                  {#if !collapsed[key]}
                    <ul class="findings-list">
                      {#each findings.functionalities as f}
                        <li>{f}</li>
                      {/each}
                    </ul>
                  {/if}
                </div>
              {/if}

              <!-- Integration points -->
              {#if findings.integration_points && findings.integration_points.length > 0}
                {@const key = agent.id + ':int'}
                <div class="collapsible">
                  <button class="coll-trigger" onclick={() => toggleCollapse(key)}>
                    <span class="coll-arrow">{collapsed[key] ? '▶' : '▼'}</span>
                    Integration Points
                    <span class="coll-count">({findings.integration_points.length})</span>
                  </button>
                  {#if !collapsed[key]}
                    <ul class="findings-list">
                      {#each findings.integration_points as ip}
                        <li>{ip}</li>
                      {/each}
                    </ul>
                  {/if}
                </div>
              {/if}

              <!-- Risks -->
              {#if findings.risks && findings.risks.length > 0}
                {@const key = agent.id + ':risks'}
                <div class="collapsible">
                  <button class="coll-trigger" onclick={() => toggleCollapse(key)}>
                    <span class="coll-arrow">{collapsed[key] ? '▶' : '▼'}</span>
                    Risks
                    <span class="coll-count">({findings.risks.length})</span>
                  </button>
                  {#if !collapsed[key]}
                    <ul class="findings-list risk-list">
                      {#each findings.risks as risk}
                        <li>{risk}</li>
                      {/each}
                    </ul>
                  {/if}
                </div>
              {/if}

              <!-- Open questions -->
              {#if findings.open_questions && findings.open_questions.length > 0}
                {@const key = agent.id + ':oq'}
                <div class="collapsible">
                  <button class="coll-trigger" onclick={() => toggleCollapse(key)}>
                    <span class="coll-arrow">{collapsed[key] ? '▶' : '▼'}</span>
                    Open Questions
                    <span class="coll-count">({findings.open_questions.length})</span>
                  </button>
                  {#if !collapsed[key]}
                    <ul class="findings-list question-list">
                      {#each findings.open_questions as q}
                        <li>
                          <span class="q-text">{q.text}</span>
                          {#if q.category}
                            <span class="q-cat">{q.category}</span>
                          {/if}
                        </li>
                      {/each}
                    </ul>
                  {/if}
                </div>
              {/if}

              <!-- Suggested learnings -->
              {#if findings.suggested_learnings && findings.suggested_learnings.length > 0}
                {@const key = agent.id + ':sl'}
                <div class="collapsible">
                  <button class="coll-trigger" onclick={() => toggleCollapse(key)}>
                    <span class="coll-arrow">{collapsed[key] ? '▶' : '▼'}</span>
                    Suggested Learnings
                    <span class="coll-count">({findings.suggested_learnings.length})</span>
                  </button>
                  {#if !collapsed[key]}
                    <div class="sl-hint">
                      <span class="sl-hint-text">Saved to your Learnings knowledge base as suggestions — open the Learnings tab to review and Accept them.</span>
                      <button class="sl-hint-btn" onclick={() => { product.view = 'learnings'; }}>
                        Open Learnings
                      </button>
                    </div>
                    <div class="sl-list">
                      {#each findings.suggested_learnings as sl}
                        <div class="sl-item">
                          <div class="sl-header">
                            <span class="sl-kind">{sl.kind}</span>
                            <span class="sl-title">{sl.title}</span>
                          </div>
                          {#if sl.body}
                            <p class="sl-body">{sl.body}</p>
                          {/if}
                        </div>
                      {/each}
                    </div>
                  {/if}
                </div>
              {/if}
            </section>
          {/if}
        {:else if agent.status === 'error'}
          <section class="findings-card card error-card">
            <div class="findings-header">
              <span class="findings-agent-name">{agent.name || agent.skill}</span>
              <div class="findings-right">
                {#if agent.session_id}
                  <button
                    class="open-link"
                    onclick={() => toggleTerminal(agent.session_id!)}
                    title="Open session"
                  >
                    {openTerminals.has(agent.session_id) ? 'Hide' : 'Open'} {shortId(agent.session_id)}
                  </button>
                {/if}
                {#if currentAnalysis}
                  <button
                    class="btn-retry"
                    disabled={retryingAgents.has(agent.id)}
                    onclick={() => retryAgent(currentAnalysis!.id, agent.id, agent.name || agent.skill)}
                    title="Re-run this agent"
                  >
                    {retryingAgents.has(agent.id) ? 'Retrying…' : 'Retry'}
                  </button>
                {/if}
                <span class="pill pill-error">error</span>
              </div>
            </div>
            {#if agent.session_id && openTerminals.has(agent.session_id)}
              <div class="inline-term">
                {#key agent.session_id}
                  <Terminal sessionId={agent.session_id} forceDark />
                {/key}
              </div>
            {/if}
            <p class="error-msg">{agent.error ?? 'Unknown error'}</p>
          </section>
        {/if}
      {/each}

    {:else if !running}
      <div class="muted">Configure lenses above and click Run analysis.</div>
    {:else}
      <div class="muted">Starting analysis…</div>
    {/if}

  </div>
{/if}

<style>
  .muted {
    padding: 24px 0;
    font-size: 13px;
    color: var(--text-dim);
    font-style: italic;
  }
  .analysis-tab {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 860px;
    width: 100%;
  }

  /* ── Card ──────────────────────────────────────────────────────── */
  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 12px 14px;
    background: var(--surface-raised, var(--surface));
  }

  /* ── Run panel ─────────────────────────────────────────────────── */
  .run-panel {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .run-header {
    margin-bottom: 2px;
  }
  .lens-grid {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .lens-row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 28px;
  }
  .lens-disabled {
    opacity: 0.5;
  }
  .lens-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    user-select: none;
    min-width: 170px;
  }
  .lens-toggle input {
    accent-color: var(--accent);
    cursor: pointer;
    flex-shrink: 0;
  }
  .lens-name {
    font-size: 12.5px;
    color: var(--text);
    white-space: nowrap;
  }

  /* ── Provider chips ─────────────────────────────────────────────── */
  .chip-group {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .chips-muted {
    pointer-events: none;
  }
  .chip {
    height: 22px;
    padding: 0 9px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    transition: background 100ms, color 100ms, border-color 100ms;
    white-space: nowrap;
  }
  .chip:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--text);
  }
  .chip-on {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    border-color: var(--accent);
    color: var(--accent);
  }
  .chip:disabled {
    cursor: not-allowed;
  }

  /* ── Focus input ────────────────────────────────────────────────── */
  .focus-wrap {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
    margin-top: 2px;
  }
  .focus-optional {
    font-weight: 400;
    font-size: 10px;
    text-transform: none;
    letter-spacing: 0;
    color: var(--text-dim);
  }
  .focus-input {
    resize: vertical;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12px;
    padding: 6px 8px;
    line-height: 1.5;
    font-family: inherit;
    transition: border-color 100ms;
    min-height: 48px;
  }
  .focus-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .focus-input:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .focus-input::placeholder {
    color: var(--text-dim);
    font-style: italic;
  }

  /* ── Bottom row ─────────────────────────────────────────────────── */
  .bottom-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 10px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
    margin-top: 2px;
  }
  .summarizer-wrap {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .field-label {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .small-select {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12px;
    padding: 3px 8px;
    height: 26px;
  }
  .run-btn {
    height: 30px;
    padding: 0 16px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: background 110ms, opacity 110ms;
  }
  .run-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .run-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  /* ── History row ──────────────────────────────────────────────── */
  .history-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .hist-select {
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12px;
    padding: 4px 8px;
    max-width: 340px;
  }
  .dim-sm {
    font-size: 11px;
    color: var(--text-dim);
  }

  /* ── Synthesis card ───────────────────────────────────────────── */
  .synthesis-card {
    border-color: color-mix(in srgb, var(--accent) 30%, var(--border));
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .summarizer-badge {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 2px;
  }
  .synthesis-body {
    margin: 0;
    font-size: 13.5px;
    line-height: 1.6;
    color: var(--text);
  }

  /* ── Agents section ───────────────────────────────────────────── */
  .agents-section {
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  .agents-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .section-head {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
  }
  .agent-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 7px 0;
    border-top: 1px solid var(--border);
    gap: 12px;
  }
  .agent-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .agent-name {
    font-size: 13px;
    font-weight: 500;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .agent-meta {
    font-size: 11px;
    color: var(--text-dim);
  }
  .agent-right {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  /* ── Open-session link ────────────────────────────────────────── */
  .open-link {
    background: none;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text-dim);
    font-size: 10.5px;
    font-family: var(--font-mono, monospace);
    padding: 1px 6px;
    cursor: pointer;
    transition: color 100ms, border-color 100ms;
    white-space: nowrap;
  }
  .open-link:hover {
    color: var(--accent);
    border-color: var(--accent);
  }

  /* ── Inline terminal ──────────────────────────────────────────── */
  .inline-term {
    height: 320px;
    margin: 8px 0 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 6px);
    overflow: hidden;
    background: #1b1b1b;
  }

  /* ── Status pills ─────────────────────────────────────────────── */
  .pill {
    flex-shrink: 0;
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 2px 8px;
    border-radius: 999px;
  }
  .pill-done {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .pill-running {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .pill-error {
    background: color-mix(in srgb, #ef4444 18%, transparent);
    color: #b91c1c;
  }
  .pill-partial {
    background: color-mix(in srgb, #f59e0b 18%, transparent);
    color: #b45309;
  }
  .pill-pending {
    background: color-mix(in srgb, var(--text-dim) 15%, transparent);
    color: var(--text-dim);
  }
  .pill-waiting {
    background: color-mix(in srgb, #e0a000 20%, transparent);
    color: #b07d00;
  }

  /* ── Findings card ────────────────────────────────────────────── */
  .findings-card {
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  .error-card {
    border-color: color-mix(in srgb, #ef4444 35%, var(--border));
  }
  .findings-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .findings-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .findings-agent-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .findings-summary {
    font-size: 13px;
    line-height: 1.6;
    color: var(--text);
    margin: 0 0 8px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--border);
  }

  /* ── Collapsible panels ───────────────────────────────────────── */
  .collapsible {
    border-top: 1px solid var(--border);
    padding: 6px 0 2px;
  }
  .coll-trigger {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: none;
    border: none;
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    cursor: pointer;
    padding: 2px 0;
    transition: color 100ms;
  }
  .coll-trigger:hover {
    color: var(--text);
  }
  .coll-arrow {
    font-size: 9px;
    color: var(--text-dim);
  }
  .coll-count {
    font-weight: 400;
    font-size: 11px;
    color: var(--text-dim);
  }

  /* ── Lists ────────────────────────────────────────────────────── */
  .findings-list {
    list-style: disc;
    padding-left: 20px;
    margin: 6px 0 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .findings-list li {
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--text);
  }
  .risk-list li {
    color: #b45309;
  }
  .mono-sm {
    font-family: var(--font-mono, monospace);
    font-size: 11.5px;
  }

  /* ── Open questions ───────────────────────────────────────────── */
  .question-list li {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .q-text {
    font-size: 12.5px;
    color: var(--text);
    line-height: 1.4;
  }
  .q-cat {
    font-size: 10.5px;
    color: var(--text-dim);
    font-style: italic;
  }

  /* ── Suggested learnings ──────────────────────────────────────── */
  .sl-list {
    margin-top: 6px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .sl-item {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
  }
  .sl-header {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }
  .sl-kind {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
  }
  .sl-title {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text);
  }
  .sl-body {
    font-size: 12px;
    line-height: 1.55;
    color: var(--text-dim);
    margin: 0;
  }

  /* ── Error message ────────────────────────────────────────────── */
  .error-msg {
    font-size: 12.5px;
    color: #b91c1c;
    line-height: 1.5;
    margin: 4px 0 0;
    font-family: var(--font-mono, monospace);
  }

  /* ── Retry button (mirrors ReviewAgents' btn small ghost) ─────── */
  .btn-retry {
    height: 22px;
    padding: 0 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: color 100ms, border-color 100ms;
    flex-shrink: 0;
  }
  .btn-retry:hover:not(:disabled) {
    color: var(--accent);
    border-color: var(--accent);
  }
  .btn-retry:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  /* ── Spinner (mirrors ReviewAgents) ───────────────────────────── */
  .spinner-xs {
    display: inline-block;
    width: 9px;
    height: 9px;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    vertical-align: middle;
    margin-right: 2px;
  }
  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  /* ── Agent note / waiting ─────────────────────────────────────── */
  .agent-note {
    margin: 2px 0 4px;
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.4;
    padding-left: 2px;
  }
  .agent-note.error-note {
    color: #b91c1c;
    font-family: var(--font-mono, monospace);
  }
  .agent-waiting {
    margin: 4px 0 4px;
    font-size: 11.5px;
    line-height: 1.45;
    color: #b07d00;
    padding-left: 2px;
  }

  /* ── Suggested learnings discoverability hint ────────────────── */
  .sl-hint {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    margin: 6px 0 4px;
    padding: 7px 10px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
    border: 1px solid var(--border);
  }
  .sl-hint-text {
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.4;
    flex: 1;
    min-width: 0;
  }
  .sl-hint-btn {
    height: 22px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: background 100ms, border-color 100ms;
    flex-shrink: 0;
  }
  .sl-hint-btn:hover {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-color: var(--accent);
  }
</style>
