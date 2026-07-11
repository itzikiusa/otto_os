<script lang="ts">
  // Docs agents — fan 1-4 writer agents out over a prompt to author notes into
  // the vault (a summarizer consolidates drafts when >1 writer). Center-stage
  // view: a compact form, then per-agent rows with live status + inline
  // terminals, in the spirit of git/ReviewAgents.svelte. The run itself lives
  // on the vault store (survives switching to a note and back); this view owns
  // the 1.5s poll timer and stops it on unmount or a terminal state.
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { cancelDocsRun, docsRun as getDocsRun, runDocsAgents } from '../../lib/api/vault';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import { toasts } from '../../lib/toast.svelte';
  import { vault } from './vault.svelte';

  interface AgentRow {
    provider: string;
    model: string;
  }

  // -- form ---------------------------------------------------------------------
  let prompt = $state('');
  let targetDir = $state('');
  let agents = $state<AgentRow[]>([{ provider: defaultAgentProvider(), model: '' }]);
  let sumProvider = $state(defaultAgentProvider());
  let starting = $state(false);

  const providers = $derived(agentProviders());
  const run = $derived(vault.docsRun);
  const active = $derived(run != null && (run.state === 'running' || run.state === 'summarizing'));

  // Prefill (and re-prefill on "Docs agent here" from a folder's context menu).
  $effect(() => {
    targetDir = vault.docsAgentsDir;
  });

  function addAgent(): void {
    if (agents.length < 4) agents = [...agents, { provider: defaultAgentProvider(), model: '' }];
  }

  function removeAgent(i: number): void {
    if (agents.length > 1) agents = agents.filter((_, x) => x !== i);
  }

  // -- run + polling ---------------------------------------------------------------
  let pollTimer: ReturnType<typeof setInterval> | null = null;

  function stopPoll(): void {
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = null;
  }

  function startPoll(): void {
    stopPoll();
    pollTimer = setInterval(() => void poll(), 1500);
  }

  async function poll(): Promise<void> {
    const r = vault.docsRun;
    if (!r) {
      stopPoll();
      return;
    }
    try {
      const next = await getDocsRun(r.id);
      vault.docsRun = next;
      if (next.state !== 'running' && next.state !== 'summarizing') {
        stopPoll();
        // The run wrote (or trashed drafts of) notes — reflect it everywhere.
        void vault.refreshTree();
        void vault.refreshStatus();
      }
    } catch {
      /* transient — next tick retries */
    }
  }

  async function start(): Promise<void> {
    if (!prompt.trim() || starting || !vault.current) return;
    starting = true;
    try {
      vault.docsRun = await runDocsAgents(vault.wsId, vault.current.id, {
        prompt: prompt.trim(),
        target_dir: targetDir.trim(),
        agents: agents.map((a) => ({ provider: a.provider, model: a.model.trim() || undefined })),
        summarizer: agents.length > 1 ? { provider: sumProvider } : undefined,
      });
      openTerminals = new Set();
      startPoll();
    } catch (e) {
      toasts.error('Docs agent', e instanceof Error ? e.message : String(e));
    } finally {
      starting = false;
    }
  }

  let cancelling = $state(false);
  async function cancel(): Promise<void> {
    const r = vault.docsRun;
    if (!r || cancelling) return;
    cancelling = true;
    try {
      await cancelDocsRun(r.id);
      await poll();
    } catch (e) {
      toasts.error('Cancel', e instanceof Error ? e.message : String(e));
    } finally {
      cancelling = false;
    }
  }

  /** Back to the form (keeps the prompt so a tweak-and-rerun is one edit). */
  function newRun(): void {
    stopPoll();
    vault.docsRun = null;
    openTerminals = new Set();
  }

  // Inline live terminals — multiple may be open at once, keyed by session id.
  let openTerminals = $state<Set<string>>(new Set());
  function toggleTerminal(sessionId: string | null): void {
    if (!sessionId) return;
    const next = new Set(openTerminals);
    if (next.has(sessionId)) next.delete(sessionId);
    else next.add(sessionId);
    openTerminals = next;
  }

  onMount(() => {
    // Coming back to the view mid-run → resume polling.
    const r = vault.docsRun;
    if (r && (r.state === 'running' || r.state === 'summarizing')) startPoll();
    return () => stopPoll();
  });
</script>

<div class="docs-agents">
  <div class="inner">
    <h2><Icon name="zap" size={15} /> Docs agent</h2>

    {#if !run}
      <!-- ── form ─────────────────────────────────────────────────────────── -->
      <label class="fld">
        <span>What should be documented?</span>
        <textarea
          bind:value={prompt}
          rows="4"
          placeholder="e.g. Document the deploy pipeline: triggers, stages, rollback, and the runbook for a failed release."
        ></textarea>
      </label>
      <label class="fld">
        <span>Target folder (vault-relative, blank = root)</span>
        <input bind:value={targetDir} placeholder="runbooks/deploys" />
      </label>

      <div class="fld">
        <span>Writer agents ({agents.length}/4)</span>
        {#each agents as agent, i (i)}
          <div class="agent-row">
            <select bind:value={agent.provider}>
              {#each providers as p (p)}
                <option value={p}>{p}</option>
              {/each}
            </select>
            <input class="model" bind:value={agent.model} placeholder="model (optional)" />
            <button
              class="icon-btn"
              title="Remove agent"
              disabled={agents.length <= 1}
              onclick={() => removeAgent(i)}
            >
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
        {#if agents.length < 4}
          <button class="add-agent" onclick={addAgent}>+ add agent</button>
        {/if}
      </div>

      {#if agents.length > 1}
        <label class="fld">
          <span>Summarizer (consolidates the {agents.length} drafts into final notes)</span>
          <select class="sum-select" bind:value={sumProvider}>
            {#each providers as p (p)}
              <option value={p}>{p}</option>
            {/each}
          </select>
        </label>
      {/if}

      <div class="form-actions">
        <button class="primary" disabled={!prompt.trim() || starting} onclick={() => void start()}>
          {starting ? 'Starting…' : 'Run'}
        </button>
      </div>
    {:else}
      <!-- ── run view ──────────────────────────────────────────────────────── -->
      <div class="run-head">
        <span class="pill st-{run.state}">
          {#if active}<span class="spinner-xs"></span>{/if}
          {run.state}
        </span>
        <span class="run-meta" title={run.prompt}>
          {run.prompt}{run.target_dir ? ` → ${run.target_dir}/` : ''}
        </span>
        <span class="grow"></span>
        {#if active}
          <button class="ghost" disabled={cancelling} onclick={() => void cancel()}>
            {cancelling ? 'Cancelling…' : 'Cancel'}
          </button>
        {:else}
          <button class="ghost" onclick={newRun}>New run</button>
        {/if}
      </div>

      {#if run.error}
        <div class="err" role="alert">{run.error}</div>
      {/if}

      <div class="rows">
        {#each run.agents as agent (agent.index)}
          <div class="agent-card">
            <div class="agent-top">
              <span class="agent-name">{agent.name}</span>
              <span class="chip">{agent.provider}{agent.model ? ' · ' + agent.model : ''}</span>
              <span class="grow"></span>
              {#if agent.session_id}
                <button class="ghost small" onclick={() => toggleTerminal(agent.session_id)}>
                  {openTerminals.has(agent.session_id) ? 'Hide' : 'Open'}
                </button>
              {/if}
              <span class="pill st-{agent.state}">
                {#if agent.state === 'running'}<span class="spinner-xs"></span>{/if}
                {agent.state}
              </span>
            </div>
            {#if agent.error}
              <p class="agent-err">{agent.error}</p>
            {/if}
            {#if agent.drafts.length > 0 && active}
              <p class="drafts">
                {agent.drafts.length} draft{agent.drafts.length === 1 ? '' : 's'}:
                <span class="mono">{agent.drafts.join(' · ')}</span>
              </p>
            {/if}
            {#if agent.session_id && openTerminals.has(agent.session_id)}
              <div class="term">
                {#key agent.session_id}
                  <Terminal sessionId={agent.session_id} />
                {/key}
              </div>
            {/if}
          </div>
        {/each}

        <!-- Summarizer row — same treatment; "skipped" when there is 1 writer. -->
        <div class="agent-card">
          <div class="agent-top">
            <span class="agent-name">summarizer</span>
            <span class="chip">
              {run.summarizer.provider}{run.summarizer.model ? ' · ' + run.summarizer.model : ''}
            </span>
            <span class="grow"></span>
            {#if run.summarizer.session_id}
              <button class="ghost small" onclick={() => toggleTerminal(run.summarizer.session_id)}>
                {openTerminals.has(run.summarizer.session_id) ? 'Hide' : 'Open'}
              </button>
            {/if}
            <span class="pill st-{run.summarizer.state}">
              {#if run.summarizer.state === 'running'}<span class="spinner-xs"></span>{/if}
              {run.summarizer.state}
            </span>
          </div>
          {#if run.summarizer.error}
            <p class="agent-err">{run.summarizer.error}</p>
          {/if}
          {#if run.summarizer.session_id && openTerminals.has(run.summarizer.session_id)}
            <div class="term">
              {#key run.summarizer.session_id}
                <Terminal sessionId={run.summarizer.session_id} />
              {/key}
            </div>
          {/if}
        </div>
      </div>

      {#if run.written.length > 0}
        <div class="written">
          <span class="written-title">
            {run.written.length} note{run.written.length === 1 ? '' : 's'} written
          </span>
          {#each run.written as p (p)}
            <button class="written-link" onclick={() => void vault.open(p)}>{p}</button>
          {/each}
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .docs-agents {
    overflow-y: auto;
    min-height: 0;
  }
  .inner {
    max-width: 760px;
    width: 100%;
    margin: 0 auto;
    padding: 18px 26px 60px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    font-size: 15px;
    color: var(--text);
  }

  /* ── form ─────────────────────────────────────────────────────────────── */
  .fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .fld textarea,
  .fld input,
  .fld select,
  .agent-row select,
  .agent-row input {
    background: var(--panel-2, #222);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    font-size: 13px;
    padding: 8px 10px;
    font-family: inherit;
  }
  .fld textarea {
    resize: vertical;
    min-height: 72px;
    line-height: 1.5;
  }
  .agent-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .agent-row select {
    flex: 0 0 140px;
  }
  .agent-row .model {
    flex: 1;
    min-width: 0;
  }
  .sum-select {
    max-width: 200px;
  }
  .icon-btn {
    display: inline-flex;
    background: none;
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text-dim);
    padding: 7px 8px;
    cursor: pointer;
  }
  .icon-btn:hover:not(:disabled) {
    color: #e88;
    border-color: #a33;
  }
  .icon-btn:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .add-agent {
    align-self: flex-start;
    background: none;
    border: 1px dashed var(--border);
    border-radius: 7px;
    color: var(--text-dim);
    font-size: 12px;
    padding: 5px 12px;
    cursor: pointer;
  }
  .add-agent:hover {
    color: var(--accent, #9ab4ff);
    border-color: var(--accent, #7a9cff);
  }
  .form-actions {
    display: flex;
    justify-content: flex-end;
  }
  .primary {
    background: var(--accent, #4c6fff);
    border: none;
    color: #fff;
    border-radius: 8px;
    padding: 8px 18px;
    font-size: 13px;
    cursor: pointer;
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }

  /* ── run view ─────────────────────────────────────────────────────────── */
  .run-head {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .run-meta {
    font-size: 12px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .grow {
    flex: 1;
  }
  .ghost {
    border: 1px solid var(--border);
    background: var(--panel-2, #222);
    color: var(--text);
    border-radius: 7px;
    padding: 5px 12px;
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
  }
  .ghost.small {
    padding: 3px 10px;
    font-size: 11.5px;
  }
  .ghost:hover:not(:disabled) {
    border-color: var(--accent, #7a9cff);
  }
  .ghost:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .agent-card {
    background: var(--panel, #1c1c1e);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 8px 12px;
  }
  .agent-top {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .agent-name {
    font-size: 12.5px;
    font-weight: 600;
  }
  .chip {
    font-size: 10.5px;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 1px 8px;
  }
  .agent-err {
    margin: 6px 0 0;
    font-size: 11.5px;
    color: #e88;
    line-height: 1.4;
    word-break: break-word;
  }
  .drafts {
    margin: 5px 0 0;
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.4;
    word-break: break-word;
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }
  .term {
    height: min(360px, 60vh);
    margin: 8px 0 2px;
    border: 1px solid var(--border);
    border-radius: 8px;
    overflow: hidden;
    overscroll-behavior: contain;
  }

  /* status pills */
  .pill {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 2px 7px;
    border-radius: 5px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .st-pending,
  .st-skipped,
  .st-cancelled {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text-dim);
  }
  .st-running,
  .st-summarizing {
    background: var(--accent-dim, rgba(90, 120, 255, 0.14));
    color: var(--accent, #9ab4ff);
  }
  .st-done {
    background: rgba(127, 201, 127, 0.14);
    color: #7fc97f;
  }
  .st-error {
    background: rgba(214, 86, 72, 0.12);
    color: #e88;
  }
  .spinner-xs {
    display: inline-block;
    width: 9px;
    height: 9px;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .err {
    color: #e88;
    font-size: 12px;
    border: 1px solid rgba(214, 86, 72, 0.4);
    background: rgba(214, 86, 72, 0.08);
    border-radius: 7px;
    padding: 6px 10px;
    word-break: break-word;
  }

  .written {
    display: flex;
    flex-direction: column;
    gap: 4px;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 10px 12px;
  }
  .written-title {
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .written-link {
    align-self: flex-start;
    background: none;
    border: none;
    padding: 1px 0;
    font-size: 12.5px;
    color: var(--accent, #7a9cff);
    cursor: pointer;
    text-align: start;
    word-break: break-all;
  }
  .written-link:hover {
    text-decoration: underline;
  }
</style>
