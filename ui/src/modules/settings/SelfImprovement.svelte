<script lang="ts">
  // Self-Improvement settings page: per-workspace scheduled self-reflection.
  // Periodically reviews recent sessions and improves the workspace's memory
  // and handling skills — safe edits apply automatically, risky ones queue for
  // approval here, with a recent-runs log.
  import { api } from '../../lib/api/client';
  import { improveApi } from '../../lib/api/improve';
  import type {
    Autonomy,
    ImprovementEdit,
    ImprovementRun,
    SelfImprovementConfig,
  } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { improvementBus } from '../../lib/events.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import DiffView from '../../lib/components/DiffView.svelte';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let cfg: SelfImprovementConfig | null = $state(null);
  let allowlistText = $state('');
  let runs: ImprovementRun[] = $state([]);
  let pending: ImprovementEdit[] = $state([]);
  let loading = $state(false);
  let saving = $state(false);
  let running = $state(false);
  let busyEdit: string | null = $state(null);

  // Evolve-now state: the most recent per-session evolve result.
  let evolving = $state(false);
  // null = never run; [] = ran, no skill changes; [...] = changed skill refs
  let evolveResult: string[] | null = $state(null);

  const wsId = $derived(ws.currentId);
  // The currently-active / focused session in the workspace — used by "Evolve now".
  const activeSession = $derived(ws.activeSession);

  // Agent CLIs the analysis can run on: installed tools (minus shell), plus
  // anything already selected and claude, so the list is stable even before
  // /meta loads. Each selected provider yields its own set of suggestions.
  const providerChoices = $derived.by(() => {
    const found = (auth.meta?.tools ?? [])
      .filter((t) => t.found && t.name !== 'shell')
      .map((t) => t.name);
    const set = new Set<string>(found.length ? found : ['claude', 'codex', 'agy']);
    for (const p of cfg?.providers ?? []) set.add(p);
    set.add('claude');
    return [...set];
  });

  function toggleProvider(name: string): void {
    if (!cfg) return;
    const set = new Set(cfg.providers);
    if (set.has(name)) set.delete(name);
    else set.add(name);
    // Always keep at least one provider selected.
    cfg.providers = set.size > 0 ? [...set] : ['claude'];
  }

  // ---------------------------------------------------------------------------
  // Load on workspace change
  // ---------------------------------------------------------------------------

  $effect(() => {
    if (wsId) void load(wsId);
  });

  // ---------------------------------------------------------------------------
  // Live-refresh driven by improvement_updated WS event (T1).
  // Falls back to a capped 30s poll when the page is visible — the poll is
  // intentionally slower so the WS path is the primary update mechanism.
  // ---------------------------------------------------------------------------

  const POLL_INTERVAL_MS = 30_000;
  const POLL_MAX_TICKS = 20; // stop polling after 10 minutes of inactivity

  let pollTimer: ReturnType<typeof setTimeout> | undefined;
  let pollTicks = 0;

  function schedulePoll(id: string): void {
    if (pollTicks >= POLL_MAX_TICKS) return;
    clearTimeout(pollTimer);
    pollTimer = setTimeout(() => {
      pollTicks += 1;
      void load(id);
      schedulePoll(id);
    }, POLL_INTERVAL_MS);
  }

  $effect(() => {
    // Subscribe to improvementBus: re-load whenever the bus fires.
    const _tick = improvementBus.tick;
    if (wsId) void load(wsId);
  });

  $effect(() => {
    if (wsId) {
      pollTicks = 0;
      schedulePoll(wsId);
    }
    return () => clearTimeout(pollTimer);
  });

  async function load(id: string): Promise<void> {
    loading = true;
    try {
      cfg = await improveApi.getConfig(id);
      allowlistText = cfg.skill_allowlist.join(', ');
      runs = await improveApi.listRuns(id);
      pending = await improveApi.listEdits(id, 'pending');
    } catch (e) {
      toasts.error('Could not load self-improvement', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Save config
  // ---------------------------------------------------------------------------

  async function save(): Promise<void> {
    if (!wsId || !cfg) return;
    saving = true;
    try {
      const body = {
        enabled: cfg.enabled,
        cadence_minutes: cfg.cadence_minutes,
        lookback_hours: cfg.lookback_hours,
        skill_allowlist: allowlistText
          .split(',')
          .map((s) => s.trim())
          .filter(Boolean),
        autonomy: cfg.autonomy as Autonomy,
        providers: cfg.providers,
        live_evolve: cfg.live_evolve,
      };
      cfg = await improveApi.putConfig(wsId, body);
      allowlistText = cfg.skill_allowlist.join(', ');
      toasts.success('Self-improvement settings saved', cfg.enabled ? 'Enabled' : 'Disabled');
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Run now
  // ---------------------------------------------------------------------------

  async function runNow(): Promise<void> {
    if (!wsId) return;
    running = true;
    try {
      await improveApi.runNow(wsId);
      toasts.info('Self-reflection run started', 'Reviewing recent sessions…');
      await load(wsId);
    } catch (e) {
      toasts.error('Run failed', e instanceof Error ? e.message : String(e));
    } finally {
      running = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Approve / reject a pending edit
  // ---------------------------------------------------------------------------

  async function act(edit: ImprovementEdit, action: 'approve' | 'reject'): Promise<void> {
    if (!wsId) return;
    busyEdit = edit.id;
    try {
      await improveApi[action](edit.id);
      toasts.info(
        action === 'approve' ? 'Edit approved & applied' : 'Edit rejected',
        edit.target_ref,
      );
      await load(wsId);
    } catch (e) {
      toasts.error(`${action === 'approve' ? 'Approve' : 'Reject'} failed`, e instanceof Error ? e.message : String(e));
    } finally {
      busyEdit = null;
    }
  }

  // ---------------------------------------------------------------------------
  // Evolve now — triggers a per-session evolve pass and polls until done,
  // then summarises which skills changed (target === 'skill', status === 'applied').
  // The endpoint is fire-and-forget (returns run_id immediately); we poll
  // GET /improvement/runs/{run_id} at 2s intervals until the run settles.
  // ---------------------------------------------------------------------------

  const EVOLVE_POLL_MS = 2_000;
  const EVOLVE_POLL_MAX = 60; // give up after 2 min

  async function evolveNow(): Promise<void> {
    if (!activeSession) {
      toasts.error('No active session', 'Open and focus a session first, then click Evolve.');
      return;
    }
    evolving = true;
    evolveResult = null;
    try {
      const { run_id } = await api.post<{ run_id: string }>(`/sessions/${activeSession.id}/evolve`);
      // Poll until the run settles.
      let ticks = 0;
      let settled = false;
      while (!settled && ticks < EVOLVE_POLL_MAX) {
        await new Promise((r) => setTimeout(r, EVOLVE_POLL_MS));
        ticks++;
        const { run, edits } = await improveApi.getRun(run_id);
        if (run.status === 'done' || run.status === 'failed' || run.status === 'skipped') {
          settled = true;
          if (run.status === 'done') {
            // Collect the skill target_refs that were applied during this run.
            const changed = edits
              .filter((e) => e.target === 'skill' && e.status === 'applied')
              .map((e) => e.target_ref);
            evolveResult = changed;
            if (changed.length > 0) {
              toasts.success(
                `${changed.length} skill${changed.length === 1 ? '' : 's'} updated`,
                changed.slice(0, 3).join(', ') + (changed.length > 3 ? '…' : ''),
              );
            } else {
              toasts.info('Evolve complete', 'No skill changes this session.');
            }
            // Refresh runs + pending so the page is up-to-date.
            if (wsId) await load(wsId);
          } else if (run.status === 'failed') {
            toasts.error('Evolve failed', run.error ?? 'Unknown error');
          } else {
            // skipped
            evolveResult = [];
            toasts.info('Evolve skipped', 'Not enough session content to produce improvements.');
          }
        }
      }
      if (!settled) {
        toasts.info('Evolve running', 'The pass is taking longer than expected — check Recent runs.');
      }
    } catch (e) {
      toasts.error('Evolve failed', e instanceof Error ? e.message : String(e));
    } finally {
      evolving = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Display helpers
  // ---------------------------------------------------------------------------

  function fmtDate(s: string | null): string {
    if (!s) return '';
    const d = new Date(s);
    return Number.isNaN(d.getTime()) ? s : d.toLocaleString();
  }
</script>

<div class="page">
  <!-- Header -->
  <div class="page-header">
    <div>
      <h1>Self-Improvement</h1>
      <div class="sub">
        Periodically review this workspace's recent agent sessions and improve its memory and
        handling skills. Safe edits apply automatically; risky ones wait for your approval below.
      </div>
    </div>
  </div>

  {#if !wsId}
    <!-- No workspace selected -->
    <EmptyState
      icon="gear"
      title="Select a workspace first"
      body="Self-improvement is per-workspace. Choose a workspace from the sidebar to configure it."
    />
  {:else if loading && !cfg}
    <Skeleton rows={2} height={88} />
  {:else if cfg}
    <!-- Config form -->
    <div class="card form">
      <div class="field field-row">
        <label for="si-enabled">Enabled</label>
        <input id="si-enabled" type="checkbox" bind:checked={cfg.enabled} />
      </div>

      <div class="field field-row">
        <label for="si-live-evolve">Live evolve (improve skills right after each interaction)</label>
        <input id="si-live-evolve" type="checkbox" bind:checked={cfg.live_evolve} />
      </div>

      <div class="field">
        <label for="si-cadence">Run every (minutes)</label>
        <input id="si-cadence" class="input" type="number" min="1" bind:value={cfg.cadence_minutes} />
      </div>

      <div class="field">
        <label for="si-lookback">Look back (hours)</label>
        <input id="si-lookback" class="input" type="number" min="1" bind:value={cfg.lookback_hours} />
      </div>

      <div class="field">
        <label for="si-autonomy">Autonomy</label>
        <select id="si-autonomy" class="input" bind:value={cfg.autonomy}>
          <option value="tiered">Tiered — safe edits auto-apply, risky ones need approval</option>
          <option value="propose">Propose — every edit needs approval</option>
          <option value="auto">Auto — apply every allow-listed edit</option>
        </select>
      </div>

      <div class="field">
        <span class="field-label">Providers</span>
        <div class="provider-grid">
          {#each providerChoices as p (p)}
            <label class="provider-chip" class:on={cfg.providers.includes(p)}>
              <input
                type="checkbox"
                checked={cfg.providers.includes(p)}
                onchange={() => toggleProvider(p)}
              />
              <span class="mono">{p}</span>
            </label>
          {/each}
        </div>
        <span class="hint">
          Each selected agent CLI runs the analysis independently with its own default model, so you
          get a separate set of suggestions per provider (labeled in the results). At least one is
          required.
        </span>
      </div>

      <div class="field">
        <label for="si-allow">Skill allow-list</label>
        <input
          id="si-allow"
          class="input"
          bind:value={allowlistText}
          spellcheck="false"
          autocomplete="off"
          placeholder="support-triage-router, code-review"
        />
        <span class="hint">
          Comma-separated skill names. Only these skills may be auto-edited; skill edits outside the
          list always queue for approval. Memory edits follow the autonomy policy directly.
        </span>
      </div>

      <div class="actions">
        <button class="btn primary" disabled={saving} onclick={save}>
          {saving ? 'Saving…' : 'Save'}
        </button>
        <button class="btn" disabled={running} onclick={runNow}>
          {running ? 'Starting…' : 'Run now'}
        </button>
        <!-- Evolve now: triggers a per-session evolve pass on the active session. -->
        <button
          class="btn"
          disabled={evolving || !activeSession}
          title={activeSession ? `Evolve session: ${activeSession.title}` : 'No active session — open one first'}
          onclick={evolveNow}
        >
          {evolving ? 'Evolving…' : 'Evolve now'}
        </button>
        {#if cfg.next_run_at}
          <span class="dim next-run">Next run: {fmtDate(cfg.next_run_at)}</span>
        {/if}
      </div>

      <!-- Evolve result badge — shown after an Evolve now completes. -->
      {#if evolveResult !== null}
        <div class="evolve-badge" class:no-change={evolveResult.length === 0}>
          {#if evolveResult.length === 0}
            <span class="evolve-icon">—</span>
            <span>No skill changes this session.</span>
          {:else}
            <span class="evolve-icon">✓</span>
            <span>
              {evolveResult.length} skill{evolveResult.length === 1 ? '' : 's'} updated:
              <span class="evolve-skills mono">{evolveResult.join(', ')}</span>
            </span>
          {/if}
          <button
            class="evolve-dismiss"
            aria-label="Dismiss"
            onclick={() => (evolveResult = null)}
          >×</button>
        </div>
      {/if}
    </div>

    <!-- Pending approvals -->
    {#if pending.length > 0}
      <h2 class="section-title">Pending approvals <span class="chip">{pending.length}</span></h2>
      <div class="edit-list">
        {#each pending as e (e.id)}
          <div class="edit-card card">
            <div class="edit-head">
              <span class="edit-ref mono">{e.target_ref}</span>
              <span class="chip">{e.target}</span>
              <span class="chip">{e.kind}</span>
              <span class="chip" class:structural={e.risk === 'structural'}>{e.risk}</span>
            </div>
            {#if e.rationale}
              <div class="rationale dim">{e.rationale}</div>
            {/if}
            {#if e.evidence.length > 0}
              <div class="evidence dim">Evidence: {e.evidence.join(', ')}</div>
            {/if}
            <details class="diff">
              <summary>Before / after diff</summary>
              <DiffView
                before={e.before_content ?? ''}
                after={e.after_content}
                mode="word"
                contextLines={4}
              />
            </details>
            <div class="actions">
              <button class="btn primary sm" disabled={busyEdit === e.id} onclick={() => act(e, 'approve')}>
                Approve
              </button>
              <button class="btn sm" disabled={busyEdit === e.id} onclick={() => act(e, 'reject')}>
                Reject
              </button>
            </div>
          </div>
        {/each}
      </div>
    {/if}

    <!-- Recent runs -->
    <h2 class="section-title">Recent runs</h2>
    {#if runs.length === 0}
      <div class="card-info dim">No runs yet. Enable and save, or click “Run now”.</div>
    {:else}
      <div class="run-list">
        {#each runs as r (r.id)}
          <div class="run-card card">
            <div class="run-head">
              <span class="chip" class:done={r.status === 'done'} class:failed={r.status === 'failed'}>
                {r.status}
              </span>
              <span class="run-meta dim">
                {r.trigger} · {r.sessions_reviewed} sessions · {r.applied} applied · {r.pending} pending
              </span>
              <span class="grow"></span>
              <span class="run-time dim">{fmtDate(r.started_at)}</span>
            </div>
            {#if r.summary}<div class="run-summary dim">{r.summary}</div>{/if}
            {#if r.error}<div class="run-error">{r.error}</div>{/if}
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px 18px;
    max-width: 560px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }

  .field-row {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
  }
  .field-row label {
    margin-bottom: 0;
  }

  .field label {
    font-size: 12.5px;
    font-weight: 600;
  }
  .field-label {
    font-size: 12.5px;
    font-weight: 600;
  }

  .provider-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .provider-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    cursor: pointer;
    font-size: 12.5px;
    font-weight: 500;
    user-select: none;
  }
  .provider-chip.on {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .provider-chip input {
    margin: 0;
  }

  .hint {
    font-size: 11.5px;
    color: var(--text-dim);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .next-run {
    font-size: 11.5px;
  }

  .btn.sm {
    font-size: 11.5px;
    height: 24px;
    padding: 0 10px;
  }

  .section-title {
    font-size: 14px;
    font-weight: 600;
    margin: 22px 0 10px;
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .chip {
    font-size: 10.5px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 1px 6px;
    color: var(--text-dim);
    text-transform: lowercase;
  }
  .chip.structural {
    color: var(--status-exited, #c0392b);
    border-color: currentColor;
  }
  .chip.done {
    color: var(--status-working, #2d8);
    border-color: currentColor;
  }
  .chip.failed {
    color: var(--status-exited, #c0392b);
    border-color: currentColor;
  }

  .edit-list,
  .run-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: min(760px, 92vw);
  }

  .edit-card,
  .run-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 16px;
  }

  .edit-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .edit-ref {
    font-size: 13px;
    font-weight: 600;
  }

  .rationale,
  .evidence {
    font-size: 12px;
  }

  .diff summary {
    font-size: 12px;
    cursor: pointer;
    color: var(--accent);
    width: fit-content;
    margin-bottom: 6px;
  }
  .diff :global(.diff-view) {
    font-size: 11px;
  }

  .run-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .run-meta,
  .run-time {
    font-size: 11.5px;
  }
  .run-summary {
    font-size: 12px;
  }
  .run-error {
    font-size: 12px;
    color: var(--status-exited, #c0392b);
  }

  /* Evolve-now result badge */
  .evolve-badge {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 8px 12px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--status-working, #2d8) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--status-working, #2d8) 35%, transparent);
    font-size: 12px;
    max-width: 520px;
  }
  .evolve-badge.no-change {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    border-color: var(--border);
  }
  .evolve-icon {
    flex-shrink: 0;
    font-weight: 700;
    color: var(--status-working, #2d8);
  }
  .evolve-badge.no-change .evolve-icon {
    color: var(--text-dim);
  }
  .evolve-skills {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .evolve-dismiss {
    margin-left: auto;
    flex-shrink: 0;
    background: transparent;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1;
    padding: 0 2px;
  }
  .evolve-dismiss:hover {
    color: var(--text);
  }

</style>
