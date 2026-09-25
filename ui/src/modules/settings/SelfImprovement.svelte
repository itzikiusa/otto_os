<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import SectionIntro from './SectionIntro.svelte';
  import SettingToggle from './SettingToggle.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
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
  import { toasts } from '../../lib/toast.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import { improvementBus } from '../../lib/events.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import DiffView from '../../lib/components/DiffView.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus, sentenceCase } from '../../lib/status';
  import { rel } from '../../lib/stores/now.svelte';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let cfg: SelfImprovementConfig | null = $state(null);
  let allowlistText = $state('');
  let runs: ImprovementRun[] = $state([]);
  let pending: ImprovementEdit[] = $state([]);
  let loading = $state(false);
  let loadError = $state('');
  let saving = $state(false);
  // The editable fields as last loaded/saved, and the workspace they belong
  // to. The 30 s poll and every improvement event re-load this page; they
  // used to overwrite `cfg` too, snapping a half-edited form back with no
  // word. Now the form is only refreshed while it has no unsaved edits.
  let savedKey = $state('');
  let cfgWs: string | null = null;
  function formKey(c: SelfImprovementConfig, allow: string): string {
    return JSON.stringify([
      c.enabled,
      c.live_evolve,
      c.cadence_minutes,
      c.lookback_hours,
      c.autonomy,
      [...c.providers].sort(),
      allow.split(',').map((x) => x.trim()).filter(Boolean),
    ]);
  }
  const dirty = $derived(cfg != null && formKey(cfg, allowlistText) !== savedKey);
  // Leaving Settings (or this page) with unsaved edits asks first.
  $effect(() => guardUnsaved(() => dirty, { what: 'the self-improvement settings' }));
  // number inputs bind `null` when cleared; the daemon would answer with a raw
  // deserialize error.
  const formError = $derived.by(() => {
    if (!cfg) return '';
    const whole = (n: unknown) => typeof n === 'number' && Number.isInteger(n) && n >= 1;
    if (!whole(cfg.cadence_minutes)) return '“Run every” must be a whole number of minutes (1 or more).';
    if (!whole(cfg.lookback_hours)) return '“Look back” must be a whole number of hours (1 or more).';
    return '';
  });
  let running = $state(false);
  let busyEdit: string | null = $state(null);

  // Evolve-now state: the most recent per-session evolve result.
  let evolving = $state(false);
  // null = never run; [] = ran, no skill changes; [...] = changed skill refs
  let evolveResult: string[] | null = $state(null);

  const wsId = $derived(ws.currentId);
  // The currently-active / focused session in the workspace — used by "Evolve now".
  const activeSession = $derived(ws.activeSession);

  // Agent CLIs the analysis can run on: the live provider registry (built-ins +
  // custom, e.g. grok), minus the `shell` pseudo-provider — NOT /meta.tools,
  // which also lists non-agent tools like git/clickhouse. Anything already
  // selected is kept so a since-removed provider doesn't silently vanish. Each
  // selected provider yields its own set of suggestions.
  const providerChoices = $derived.by(() => {
    const set = new Set<string>(agentProviders());
    for (const p of cfg?.providers ?? []) set.add(p);
    return [...set];
  });

  function toggleProvider(name: string): void {
    if (!cfg) return;
    const set = new Set(cfg.providers);
    if (set.has(name)) set.delete(name);
    else set.add(name);
    // Always keep at least one provider selected.
    cfg.providers = set.size > 0 ? [...set] : [defaultAgentProvider()];
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

  function adopt(id: string, fresh: SelfImprovementConfig): void {
    cfg = fresh;
    cfgWs = id;
    allowlistText = fresh.skill_allowlist.join(', ');
    savedKey = formKey(fresh, allowlistText);
  }

  async function load(id: string): Promise<void> {
    loading = true;
    try {
      const fresh = await improveApi.getConfig(id);
      if (!cfg || cfgWs !== id || !dirty) adopt(id, fresh);
      // Status fields always refresh (they aren't in the form).
      else cfg = { ...cfg, last_run_at: fresh.last_run_at, next_run_at: fresh.next_run_at };
      runs = await improveApi.listRuns(id);
      pending = await improveApi.listEdits(id, 'pending');
      loadError = '';
    } catch (e) {
      // Inline (first load) or a slim stale bar (background refresh) — never
      // a toast every 30 s while the daemon is unreachable.
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Save config
  // ---------------------------------------------------------------------------

  async function save(): Promise<void> {
    if (!wsId || !cfg || formError) return;
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
      adopt(wsId, await improveApi.putConfig(wsId, body));
      toasts.success('Self-improvement settings saved', cfg.enabled ? 'Enabled' : 'Disabled');
    } catch (e) {
      toasts.error('Couldn’t save self-improvement settings', e instanceof Error ? e.message : String(e));
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
      toasts.error('Couldn’t start a run', e instanceof Error ? e.message : String(e));
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
      toasts.error(`Couldn’t ${action} the edit`, e instanceof Error ? e.message : String(e));
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
      toasts.error('Couldn’t evolve the session', e instanceof Error ? e.message : String(e));
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

<div class="settings-section">
  <PageHeader title={sectionLabel('self-improvement')} subtitle="Improve memory and skills from recent sessions">
    {#snippet actions()}
      {#if wsId && cfg}
        <button class="btn small" data-icon="play" data-overflow="1" disabled={running} title="Review recent sessions now" onclick={runNow}>
          <Icon name="play" size={12} /> {running ? 'Starting…' : 'Run now'}
        </button>
        <!-- Evolve now: triggers a per-session evolve pass on the active session. -->
        <button
          class="btn small"
          data-icon="sparkle"
          disabled={evolving || !activeSession}
          title={activeSession ? `Evolve skills from “${activeSession.title}” now` : 'No active session — open one first'}
          onclick={evolveNow}
        >
          <Icon name="sparkle" size={12} /> {evolving ? 'Evolving…' : 'Evolve now'}
        </button>
        <button
          class="btn small primary"
          disabled={saving || !dirty || !!formError}
          title={formError || (dirty ? 'Save these settings' : 'No unsaved changes')}
          onclick={save}
        >
          {saving ? 'Saving…' : 'Save'}
        </button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">
  <SectionIntro>Otto periodically reviews this workspace's recent agent sessions and improves its memory and handling skills. <strong>Safe edits apply automatically; risky ones wait for your approval below.</strong></SectionIntro>

  {#if !wsId}
    <!-- No workspace selected -->
    <EmptyState
      variant="page"
      icon="folder"
      title="No workspace selected"
      body="Self-improvement is per workspace. Pick one from the workspace menu at the top of the sidebar to configure it."
    />
  {:else if !cfg}
    <LoadState what="self-improvement settings" loading={loading || !loadError} error={loadError || null} empty rows={3} onretry={() => wsId && void load(wsId)} />
  {:else}
    {#if loadError}
      <LoadState what="self-improvement" error={loadError} onretry={() => wsId && void load(wsId)} />
    {/if}
    {#if formError}
      <p class="form-note error" role="alert">{formError}</p>
    {:else if dirty}
      <p class="form-note unsaved" role="status">Unsaved changes — Save to apply them.</p>
    {/if}
    <!-- Evolve result — shown after an Evolve now completes. -->
    {#if evolveResult !== null}
      <div class="evolve-badge" class:no-change={evolveResult.length === 0} role="status">
        <Icon name={evolveResult.length === 0 ? 'info' : 'check'} size={14} />
        <span class="grow">
          {#if evolveResult.length === 0}
            Evolve finished — no skill changes this session.
          {:else}
            {evolveResult.length} skill{evolveResult.length === 1 ? '' : 's'} updated:
            <span class="evolve-skills mono">{evolveResult.join(', ')}</span>
          {/if}
        </span>
        <button
          class="icon-btn"
          aria-label="Dismiss evolve result"
          title="Dismiss evolve result"
          onclick={() => (evolveResult = null)}
        ><Icon name="x" size={12} /></button>
      </div>
    {/if}
    <!-- Pending approvals — first, since they need you. -->
    {#if pending.length > 0}
      <h2 class="section-title">Pending approvals <span class="count">{pending.length}</span></h2>
      <div class="edit-list">
        {#each pending as e (e.id)}
          <div class="edit-card card">
            <div class="edit-head">
              <span class="edit-ref mono" title={e.target_ref}>{e.target_ref}</span>
              <span class="chip">{sentenceCase(e.target)}</span>
              <span class="chip">{sentenceCase(e.kind)}</span>
              <span class="chip" class:structural={e.risk === 'structural'}>{sentenceCase(e.risk)} risk</span>
            </div>
            {#if e.rationale}
              <div class="rationale">{e.rationale}</div>
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
              <button class="btn small" disabled={busyEdit === e.id} onclick={() => act(e, 'reject')}>
                Reject
              </button>
              <button class="btn small primary" disabled={busyEdit === e.id} onclick={() => act(e, 'approve')}>
                Approve
              </button>
            </div>
          </div>
        {/each}
      </div>
    {/if}

    <h2 class="section-title">Schedule</h2>
    <div class="card form tfirst">
      <SettingToggle
        label="Review recent sessions on a schedule"
        checked={cfg.enabled}
        testid="si-enabled"
        onchange={(v) => { if (cfg) cfg.enabled = v; }}
      >
        {#if cfg.enabled && cfg.next_run_at}<span title={fmtDate(cfg.next_run_at)}>Next run {rel(cfg.next_run_at)}.</span>{:else if cfg.enabled}Runs after you save.{:else}Off — Run now still works on demand.{/if}
      </SettingToggle>
      <div class="grid2 indent">
        <div class="field">
          <label for="si-cadence">Run every (minutes)</label>
          <input
            id="si-cadence"
            class="input"
            type="number"
            min="1"
            bind:value={cfg.cadence_minutes}
            disabled={!cfg.enabled}
            title={cfg.enabled ? undefined : 'Turn on the schedule to change how often it runs'}
          />
        </div>
        <div class="field">
          <label for="si-lookback">Look back (hours)</label>
          <input id="si-lookback" class="input" type="number" min="1" bind:value={cfg.lookback_hours} />
          <span class="hint">Also used by Run now.</span>
        </div>
      </div>
    </div>

    <h2 class="section-title">What may change</h2>
    <div class="card form">
      <div class="field">
        <label for="si-autonomy">Autonomy</label>
        <select id="si-autonomy" class="input" bind:value={cfg.autonomy}>
          <option value="tiered">Tiered — safe edits auto-apply, risky ones need approval</option>
          <option value="propose">Propose — every edit needs approval</option>
          <option value="auto">Auto — apply every allow-listed edit</option>
        </select>
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
    </div>

    <h2 class="section-title">Agents</h2>
    <div class="card form">
      <div class="field">
        <span class="field-label" id="si-providers-lbl">Providers</span>
        <div class="provider-grid" role="group" aria-labelledby="si-providers-lbl">
          {#each providerChoices as p (p)}
            {@const on = cfg.providers.includes(p)}
            <button
              type="button"
              class="provider-chip pill-toggle"
              class:on
              aria-pressed={on}
              disabled={on && cfg.providers.length === 1}
              title={on && cfg.providers.length === 1 ? 'At least one provider is required' : undefined}
              onclick={() => toggleProvider(p)}
            >
              {#if on}<Icon name="check" size={12} />{/if}<span class="mono">{p}</span>
            </button>
          {/each}
        </div>
        <span class="hint">
          Each selected agent CLI runs the scheduled / Run-now analysis independently with its own
          default model, so you get a separate set of suggestions per provider (labeled in the
          results). Evolve now and Live evolve use only the first one. At least one is required.
        </span>
      </div>
      <SettingToggle
        label="Live evolve"
        hint="Improve skills right after each interaction, using the first provider above."
        checked={cfg.live_evolve}
        testid="si-live-evolve"
        onchange={(v) => { if (cfg) cfg.live_evolve = v; }}
      />
    </div>

    <!-- Recent runs -->
    <h2 class="section-title">Recent runs</h2>
    {#if runs.length === 0}
      <p class="card-info dim">No runs yet. Turn on the schedule and save, or use Run now.</p>
    {:else}
      <div class="run-list">
        {#each runs as r (r.id)}
          <div class="run-card card">
            <div class="run-head">
              <StatusBadge status={runStatus(r.status)} />
              <span class="run-meta dim">
                {sentenceCase(r.trigger)} · {r.sessions_reviewed} session{r.sessions_reviewed === 1 ? '' : 's'} · {r.applied} applied · {r.pending} pending
              </span>
              <span class="grow"></span>
              <span class="run-time dim" title={fmtDate(r.started_at)}>{rel(r.started_at)}</span>
            </div>
            {#if r.summary}<div class="run-summary dim">{r.summary}</div>{/if}
            {#if r.error}<div class="run-error">{r.error}</div>{/if}
          </div>
        {/each}
      </div>
    {/if}
  {/if}
  </PageBody>
</div>

<style>
  .form-note {
    margin: 0 0 12px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .form-note.unsaved {
    color: var(--warning);
  }
  .form-note.error {
    color: var(--danger);
  }
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  /* The same card as the other Settings pages. */
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 14px 16px;
    max-width: var(--settings-col);
  }
  /* Opens on a SettingToggle, which brings its own 8px. */
  .form.tfirst {
    padding-top: 6px;
    gap: 8px;
  }
  /* Sub-controls line up with the toggle's label text (15px box + 10px gap). */
  .indent {
    margin-inline-start: 25px;
  }
  .form .field {
    margin: 0;
  }
  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .field-label {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .provider-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .provider-chip :global(svg) {
    flex-shrink: 0;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    flex-wrap: wrap;
  }
  .section-title {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .count {
    color: var(--warning);
  }
  .chip.structural {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 35%, transparent);
  }
  .edit-list,
  .run-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: var(--settings-col);
  }
  .edit-card,
  .run-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 16px;
    min-width: 0;
  }
  .edit-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .edit-ref {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .rationale,
  .evidence {
    font-size: var(--fs-s);
  }
  .diff summary {
    font-size: var(--fs-s);
    cursor: pointer;
    color: var(--accent-text);
    width: fit-content;
    margin-bottom: 6px;
  }
  .run-head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .run-meta {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .run-meta,
  .run-time {
    font-size: var(--fs-s);
  }
  .run-time {
    flex-shrink: 0;
  }
  .run-summary {
    font-size: var(--fs-s);
  }
  .run-error {
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .card-info {
    margin: 0;
    font-size: var(--fs-s);
  }
  .dim {
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  /* Evolve-now result */
  .evolve-badge {
    max-width: var(--settings-col);
    box-sizing: border-box;
    margin: 0 0 12px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 6px 12px;
    border-radius: var(--radius-m);
    background: var(--success-soft);
    border: 1px solid color-mix(in srgb, var(--success) 35%, transparent);
    font-size: var(--fs-s);
  }
  .evolve-badge > :global(svg) {
    color: var(--success);
    flex-shrink: 0;
  }
  .evolve-badge.no-change {
    background: var(--surface-2);
    border-color: var(--border);
  }
  .evolve-badge.no-change > :global(svg) {
    color: var(--text-dim);
  }
  .evolve-skills {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  @media (max-width: 640px) {
    .grid2 {
      grid-template-columns: 1fr;
    }
  }
</style>
