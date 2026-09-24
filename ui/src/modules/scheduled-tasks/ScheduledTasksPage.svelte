<script lang="ts">
  import PathField from '../../lib/components/PathField.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus } from '../../lib/status';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { scheduledTasks } from '../../lib/stores/scheduledTasks.svelte';
  import { authedText } from '../../lib/api/client';
  import { scheduledTasksApi, type ScheduledTaskInput } from '../../lib/api/scheduledTasks';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { ScheduledTask, ScheduledTaskRun } from '../../lib/api/types';
  import { allProviders, defaultAgentProvider } from '../../lib/providers';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import Modal from '../../lib/components/Modal.svelte';

  let creating = $state(false);
  let editId = $state<string | null>(null);
  let expandedId = $state<string | null>(null);
  let busy = $state(false);
  let error = $state('');

  // Report viewer modal
  let reportOpen = $state(false);
  let reportText = $state('');
  let reportLoading = $state(false);

  // Set after a successful "Convert to workflow" — surfaces a link to Workflows.
  let convertedWfId = $state<string | null>(null);

  /** The browser's IANA timezone, e.g. "Europe/London" (default for new tasks). */
  const browserTz = (() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
    } catch {
      return 'UTC';
    }
  })();

  // --- form model ---
  let fName = $state('');
  let fPrompt = $state('');
  let fSkill = $state('');
  let fKind = $state<'agent_prompt' | 'workflow'>('agent_prompt');
  let fProvider = $state(defaultAgentProvider());
  let fModel = $state(''); // '' = provider default
  let fWorkflowId = $state('');
  let fCadence = $state<'interval' | 'daily' | 'weekly' | 'cron'>('interval');
  let fEveryMin = $state(60);
  let fAt = $state('03:00');
  let fWeekday = $state(0);
  let fCronExpr = $state('0 9 * * 1');
  let fTimezone = $state(browserTz);
  let fSandbox = $state<'none' | 'worktree'>('none');
  let fMaxRetries = $state(0);
  let fNotifyOnChange = $state(false);
  let fAttachProof = $state(false);
  let fDestType = $state<'none' | 'slack' | 'telegram' | 'email' | 'webhook'>('none');
  let fChatId = $state('');
  let fEmailTo = $state('');
  let fUrl = $state('');
  let fEnabled = $state(true);
  let fCwd = $state('');

  // Live registry (built-ins + custom); shell is valid for scheduled tasks.
  const PROVIDERS = $derived(allProviders());

  $effect(() => {
    const id = ws.currentId;
    if (id) {
      void scheduledTasks.loadList(id);
      void scheduledTasks.loadPresets();
    }
  });

  const list = $derived(scheduledTasks.list);
  const presets = $derived(scheduledTasks.presets);

  function resetForm(): void {
    fName = '';
    fPrompt = '';
    fSkill = '';
    fKind = 'agent_prompt';
    fProvider = defaultAgentProvider();
    fModel = '';
    fWorkflowId = '';
    fCadence = 'interval';
    fEveryMin = 60;
    fAt = '03:00';
    fWeekday = 0;
    fCronExpr = '0 9 * * 1';
    fTimezone = browserTz;
    fSandbox = 'none';
    fMaxRetries = 0;
    fNotifyOnChange = false;
    fAttachProof = false;
    fDestType = 'none';
    fChatId = '';
    fEmailTo = '';
    fUrl = '';
    fEnabled = true;
    fCwd = '';
    error = '';
  }

  function startCreate(): void {
    resetForm();
    creating = true;
    editId = null;
  }

  function applyPreset(id: string): void {
    const p = presets.find((x) => x.id === id);
    if (!p) return;
    fName = p.name;
    fPrompt = p.prompt;
    fSkill = p.skill ?? '';
    loadSchedule(p.schedule);
    // Review/security/dependency presets benefit from an isolated worktree + change-only notify.
    if (p.id.startsWith('weekly-')) {
      fSandbox = 'worktree';
      fNotifyOnChange = p.id !== 'weekly-code-review';
      fAttachProof = p.id === 'weekly-code-review';
    }
    const dt = (p.suggested_destination?.type as string) ?? 'none';
    fDestType = ['slack', 'telegram', 'email', 'webhook'].includes(dt) ? (dt as typeof fDestType) : 'none';
  }

  /** Populate the cadence form vars from a schedule object (preset or task). */
  function loadSchedule(s: Record<string, unknown>): void {
    const cad = (s.cadence as string) ?? 'interval';
    fCadence = ['daily', 'weekly', 'cron'].includes(cad) ? (cad as typeof fCadence) : 'interval';
    fEveryMin = (s.every_min as number) ?? 60;
    fAt = (s.at as string) ?? '03:00';
    fWeekday = (s.weekday as number) ?? 0;
    fCronExpr = (s.expr as string) ?? '0 9 * * 1';
  }

  function startEdit(t: ScheduledTask): void {
    resetForm();
    editId = t.id;
    creating = false;
    fName = t.name;
    fPrompt = t.prompt;
    fSkill = t.skill ?? '';
    fKind = t.kind === 'workflow' ? 'workflow' : 'agent_prompt';
    fProvider = t.provider || defaultAgentProvider();
    fModel = t.model ?? '';
    fWorkflowId = t.workflow_id ?? '';
    loadSchedule(t.schedule ?? {});
    fTimezone = t.timezone || browserTz;
    fSandbox = t.sandbox === 'worktree' ? 'worktree' : 'none';
    fMaxRetries = t.max_retries ?? 0;
    fNotifyOnChange = !!t.notify_on_change;
    fAttachProof = !!t.attach_proof;
    const d = t.destination ?? {};
    const dt = (d.type as string) ?? 'none';
    fDestType = ['slack', 'telegram', 'email', 'webhook'].includes(dt) ? (dt as typeof fDestType) : 'none';
    fChatId = (d.chat_id as string) ?? '';
    fEmailTo = (d.to as string) ?? '';
    fUrl = (d.url as string) ?? '';
    fEnabled = t.enabled;
    fCwd = t.cwd ?? '';
  }

  function buildSchedule(): Record<string, unknown> {
    if (fCadence === 'interval') return { cadence: 'interval', every_min: Math.max(5, fEveryMin) };
    if (fCadence === 'daily') return { cadence: 'daily', at: fAt };
    if (fCadence === 'cron') return { cadence: 'cron', expr: fCronExpr.trim() };
    return { cadence: 'weekly', at: fAt, weekday: fWeekday };
  }

  function buildDestination(): Record<string, unknown> {
    switch (fDestType) {
      case 'slack':
      case 'telegram':
        return fChatId ? { type: fDestType, chat_id: fChatId } : { type: fDestType };
      case 'email':
        return { type: 'email', to: fEmailTo };
      case 'webhook':
        return { type: 'webhook', url: fUrl };
      default:
        return { type: 'none' };
    }
  }

  /** Provider <select> change: a known provider sets the slug directly; "Custom…"
   * clears it (only when currently a known provider) so the slug text field shows. */
  function onProviderSelect(v: string): void {
    if (v === 'custom') {
      if (PROVIDERS.includes(fProvider)) fProvider = '';
    } else {
      fProvider = v;
    }
  }

  async function save(): Promise<void> {
    error = '';
    if (!fName.trim()) {
      error = 'Name is required.';
      return;
    }
    const body: ScheduledTaskInput = {
      name: fName.trim(),
      prompt: fPrompt,
      kind: fKind,
      provider: fProvider,
      model: fModel.trim(),
      skill: fSkill.trim() || null,
      cwd: fCwd.trim(),
      schedule: buildSchedule(),
      destination: buildDestination(),
      enabled: fEnabled,
      timezone: fTimezone.trim() || 'UTC',
      sandbox: fSandbox,
      max_retries: fMaxRetries,
      notify_on_change: fNotifyOnChange,
      attach_proof: fAttachProof,
      ...(fKind === 'workflow' ? { workflow_id: fWorkflowId.trim() || null } : {}),
    };
    busy = true;
    try {
      const wsId = ws.currentId;
      if (!wsId) return;
      if (editId) await scheduledTasks.update(editId, body);
      else await scheduledTasks.create(wsId, body);
      creating = false;
      editId = null;
    } catch (e) {
      error = e instanceof Error ? e.message : 'Save failed';
    } finally {
      busy = false;
    }
  }

  async function toggle(t: ScheduledTask): Promise<void> {
    try {
      await scheduledTasks.setEnabled(t.id, !t.enabled);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Toggle failed';
    }
  }

  async function runNow(t: ScheduledTask): Promise<void> {
    busy = true;
    try {
      await scheduledTasks.runNow(t.id);
      expandedId = t.id;
    } catch (e) {
      error = e instanceof Error ? e.message : 'Run failed';
    } finally {
      busy = false;
    }
  }

  async function convertToWorkflow(t: ScheduledTask): Promise<void> {
    busy = true;
    error = '';
    try {
      const res = await scheduledTasksApi.convertToWorkflow(t.id);
      convertedWfId = res.workflow_id;
      toasts.success('Converted to workflow', `Created a workflow from “${t.name}”.`);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Convert failed';
    } finally {
      busy = false;
    }
  }

  async function remove(t: ScheduledTask): Promise<void> {
    if (!(await confirmer.ask(`Delete scheduled task "${t.name}"?`, { title: 'Delete scheduled task' }))) return;
    try {
      await scheduledTasks.remove(t.id);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Delete failed';
    }
  }

  async function toggleRuns(t: ScheduledTask): Promise<void> {
    if (expandedId === t.id) {
      expandedId = null;
      return;
    }
    expandedId = t.id;
    await scheduledTasks.loadRuns(t.id);
  }

  /** Navigate to the agent session a run drove (visible session row). */
  function openSession(sessionId: string | null | undefined): void {
    if (sessionId) ws.navigateToSession(sessionId);
  }

  async function viewReport(run: ScheduledTaskRun): Promise<void> {
    reportOpen = true;
    reportLoading = true;
    reportText = '';
    try {
      reportText = await authedText(scheduledTasksApi.reportPath(run.id));
    } catch (e) {
      reportText = e instanceof Error ? `Failed to load report: ${e.message}` : 'Failed to load report';
    } finally {
      reportLoading = false;
    }
  }

  function cadenceLabel(t: ScheduledTask): string {
    const s = t.schedule ?? {};
    const c = (s.cadence as string) ?? 'interval';
    const tz = t.timezone || 'UTC';
    if (c === 'interval') return `every ${(s.every_min as number) ?? 60} min`;
    if (c === 'cron') return `cron \`${(s.expr as string) ?? ''}\` ${tz}`;
    if (c === 'daily') return `daily at ${(s.at as string) ?? '09:00'} ${tz}`;
    const wd = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'][(s.weekday as number) ?? 0];
    return `weekly ${wd} at ${(s.at as string) ?? '09:00'} ${tz}`;
  }

  function destLabel(t: ScheduledTask): string {
    return ((t.destination?.type as string) ?? 'none') as string;
  }

</script>

<div class="sched-page">
<PageHeader
  title={creating || editId ? (editId ? 'Edit scheduled task' : 'New scheduled task') : 'Scheduled Tasks'}
  subtitle={creating || editId ? undefined : 'Recurring agent jobs — run a prompt on a cadence, produce a report, and deliver it to Slack, email, or a webhook. Also driveable over MCP.'}
>
  {#snippet leading()}
    {#if creating || editId}
      <button class="icon-btn" title="Back to Scheduled Tasks" aria-label="Back to Scheduled Tasks" disabled={busy} onclick={() => { creating = false; editId = null; }}>
        <Icon name="chevronLeft" size={15} />
      </button>
    {/if}
  {/snippet}
  {#snippet actions()}
    {#if !(creating || editId) && list.length > 0}
      <button class="btn primary" onclick={startCreate}><Icon name="plus" size={12} /> New task</button>
    {/if}
  {/snippet}
</PageHeader>
<PageBody width="readable">
<div class="sched">
  {#if creating || editId}
    <div class="form">
      {#if error}<div class="err" role="alert">{error}</div>{/if}

      {#if !editId && presets.length}
        <label class="fld">
          <span>Start from a preset</span>
          <select onchange={(e) => applyPreset((e.currentTarget as HTMLSelectElement).value)}>
            <option value="">— blank —</option>
            {#each presets as p}<option value={p.id}>{p.name}</option>{/each}
          </select>
        </label>
      {/if}

      <label class="fld">
        <span>Name</span>
        <input bind:value={fName} placeholder="Nightly ticket review" />
      </label>

      <div class="row">
        <label class="fld">
          <span>Type</span>
          <select bind:value={fKind}>
            <option value="agent_prompt">Run an agent</option>
            <option value="workflow">Hand off to a workflow</option>
          </select>
        </label>
        {#if fKind === 'agent_prompt'}
          <label class="fld">
            <span>Provider</span>
            <select
              value={PROVIDERS.includes(fProvider) ? fProvider : 'custom'}
              onchange={(e) => onProviderSelect((e.currentTarget as HTMLSelectElement).value)}
            >
              {#each PROVIDERS as p}<option value={p}>{p}</option>{/each}
              <option value="custom">Custom…</option>
            </select>
          </label>
        {:else}
          <label class="fld">
            <span>Workflow id</span>
            <input bind:value={fWorkflowId} placeholder="workflow to launch" />
          </label>
        {/if}
      </div>

      {#if fKind === 'agent_prompt' && !PROVIDERS.includes(fProvider)}
        <label class="fld">
          <span>Custom provider slug</span>
          <input bind:value={fProvider} placeholder="my-custom-agent (register it in Settings first)" />
        </label>
      {/if}

      {#if fKind === 'agent_prompt'}
        <!-- Hides itself when the provider has no model-flag template (e.g. shell). -->
        <ModelPicker provider={fProvider} value={fModel} onchange={(m) => (fModel = m)} />
      {/if}

      {#if fKind === 'workflow'}
        <p class="hint">The task launches this workflow on its cadence and reports the run outcome.</p>
      {:else if fProvider === 'shell'}
        <label class="fld">
          <span>Shell command</span>
          <textarea bind:value={fPrompt} rows="4" placeholder="e.g. df -h && uptime"></textarea>
        </label>
      {:else}
        <label class="fld">
          <span>Prompt (the agent's instructions)</span>
          <textarea bind:value={fPrompt} rows="6" placeholder="Go over every ticket updated in the last 24h…"></textarea>
        </label>
      {/if}

      <div class="row">
        <label class="fld">
          <span>Cadence</span>
          <select bind:value={fCadence}>
            <option value="interval">Interval</option>
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
            <option value="cron">Cron</option>
          </select>
        </label>
        {#if fCadence === 'interval'}
          <label class="fld">
            <span>Every (minutes, min 5)</span>
            <input type="number" min="5" bind:value={fEveryMin} />
          </label>
        {:else if fCadence === 'cron'}
          <label class="fld">
            <span>Cron expression (5 fields)</span>
            <input bind:value={fCronExpr} placeholder="0 9 * * 1" />
          </label>
        {:else}
          <label class="fld">
            <span>At (HH:MM)</span>
            <input bind:value={fAt} placeholder="03:00" />
          </label>
          {#if fCadence === 'weekly'}
            <label class="fld">
              <span>Weekday</span>
              <select bind:value={fWeekday}>
                {#each ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'] as d, i}
                  <option value={i}>{d}</option>
                {/each}
              </select>
            </label>
          {/if}
        {/if}
        {#if fCadence !== 'interval'}
          <label class="fld">
            <span>Timezone</span>
            <input bind:value={fTimezone} placeholder="e.g. Europe/London" />
          </label>
        {/if}
      </div>

      <div class="row">
        <label class="fld">
          <span>Destination</span>
          <select bind:value={fDestType}>
            <option value="none">None (store only)</option>
            <option value="slack">Slack</option>
            <option value="telegram">Telegram</option>
            <option value="email">Email</option>
            <option value="webhook">HTTP webhook</option>
          </select>
        </label>
        {#if fDestType === 'slack' || fDestType === 'telegram'}
          <label class="fld">
            <span>Chat / channel id (optional)</span>
            <input bind:value={fChatId} placeholder="defaults to the integration channel" />
          </label>
        {:else if fDestType === 'email'}
          <label class="fld">
            <span>Send to (email)</span>
            <input bind:value={fEmailTo} placeholder="you@example.com" />
          </label>
        {:else if fDestType === 'webhook'}
          <label class="fld">
            <span>Webhook URL</span>
            <input bind:value={fUrl} placeholder="https://…" />
          </label>
        {/if}
      </div>

      {#if fKind === 'agent_prompt' && fProvider !== 'shell'}
        <div class="row">
          <label class="fld">
            <span>Skill (optional, inlined)</span>
            <input bind:value={fSkill} placeholder="e.g. db-mysql" />
          </label>
          <label class="fld">
            <span>Working dir (optional)</span>
            <PathField bind:value={fCwd}><input bind:value={fCwd} placeholder="repo path — not a sandbox" /></PathField>
          </label>
        </div>

        <div class="row">
          <label class="fld">
            <span>Sandbox</span>
            <select bind:value={fSandbox}>
              <option value="none">Run in working dir</option>
              <option value="worktree">Isolated git worktree</option>
            </select>
          </label>
          <label class="fld">
            <span>Retries on failure (0–5)</span>
            <input type="number" min="0" max="5" bind:value={fMaxRetries} />
          </label>
        </div>
      {:else if fProvider === 'shell'}
        <label class="fld">
          <span>Working dir (optional)</span>
          <PathField bind:value={fCwd}><input bind:value={fCwd} placeholder="dir to run the command in" /></PathField>
        </label>
      {/if}

      <div class="toggles">
        <label class="chk"><input type="checkbox" bind:checked={fNotifyOnChange} /> Only notify on meaningful change</label>
        <label class="chk"><input type="checkbox" bind:checked={fAttachProof} /> Attach a proof pack to each run</label>
        <label class="chk"><input type="checkbox" bind:checked={fEnabled} /> Enabled</label>
      </div>

      <div class="actions">
        <button class="btn primary" disabled={busy} onclick={save}>{busy ? 'Saving…' : 'Save'}</button>
        <button class="btn" disabled={busy} onclick={() => { creating = false; editId = null; }}>Cancel</button>
      </div>
    </div>
  {:else}
    {#if error}<div class="err" role="alert">{error}</div>{/if}

    {#if convertedWfId}
      <div class="notice" role="status">
        <span>Created a workflow from this task.</span>
        <span class="grow"></span>
        <button class="btn small" onclick={() => { convertedWfId = null; router.go('workflows'); }}>Open Workflows</button>
        <button class="btn small" onclick={() => (convertedWfId = null)}>Dismiss</button>
      </div>
    {/if}

    {#if list.length === 0}
      <EmptyState
        variant="page"
        icon="clock"
        title="No scheduled tasks yet"
        body="Create one to run an agent on a cadence and deliver its report."
        actionLabel="New task"
        actionIcon="plus"
        onaction={startCreate}
      />
    {:else}
      <ul class="tasks">
        {#each list as t (t.id)}
          <li class="task">
            <div class="task-main">
              <div class="task-info">
                <strong class="name">{t.name}</strong>
                <span class="meta">{cadenceLabel(t)} · → {destLabel(t)}</span>
                <!-- Shared run vocabulary (lib/status.ts): ok → Succeeded, error → Failed. -->
                {#if t.last_status}<StatusBadge status={runStatus(t.last_status)} />{/if}
                {#if !t.enabled}<span class="pill">Paused</span>{/if}
              </div>
              <div class="task-actions">
                <button class="btn small" onclick={() => runNow(t)} disabled={busy}>Run now</button>
                <button class="btn small ghost" onclick={() => toggleRuns(t)}>
                  {expandedId === t.id ? 'Hide runs' : 'Runs'}
                </button>
                <button class="btn small ghost" onclick={() => toggle(t)}>{t.enabled ? 'Pause' : 'Enable'}</button>
                <button class="btn small ghost" title="Create a multi-step workflow (+ schedule trigger) from this task" onclick={() => convertToWorkflow(t)} disabled={busy}>To workflow</button>
                <button class="btn small ghost" onclick={() => startEdit(t)}>Edit</button>
                <!-- Destructive: quiet icon at the end of the row, confirmed by confirmer.ask(). -->
                <button class="icon-btn del" onclick={() => remove(t)} aria-label="Delete task" title="Delete task"><Icon name="trash" size={14} /></button>
              </div>
            </div>
            {#if expandedId === t.id}
              <div class="runs">
                {#each scheduledTasks.runsByTask[t.id] ?? [] as r (r.id)}
                  <div class="run">
                    <StatusBadge status={runStatus(r.status)} />
                    <span class="run-when">{r.started_at}</span>
                    <span class="run-sum">{r.summary || '(no summary)'}</span>
                    {#if r.report_rel}
                      <button class="btn small" onclick={() => viewReport(r)}>View report</button>
                    {/if}
                    {#if r.session_id}
                      <button class="btn small" title="Open the agent session this run drove" onclick={() => openSession(r.session_id)}>Open session</button>
                    {/if}
                    {#if (r.attempts ?? 1) > 1}<span class="pill warn">{r.attempts} attempts</span>{/if}
                    {#if r.delivered}<span class="pill ok">delivered</span>{/if}
                    {#if r.skipped_delivery}<span class="pill" title="report unchanged since last run">no change</span>{/if}
                    {#if r.delivery_error}<span class="pill warn" title={r.delivery_error}>delivery failed</span>{/if}
                    {#if r.proof_pack_id}<span class="pill ok" title="proof pack attached">proof</span>{/if}
                    {#if r.workflow_run_id}<span class="pill" title={r.workflow_run_id}>workflow</span>{/if}
                  </div>
                {:else}
                  <div class="muted">No runs yet.</div>
                {/each}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  {/if}

  {#if reportOpen}
    <Modal title="Report" width={760} onclose={() => (reportOpen = false)}>
      {#if reportLoading}
        <div class="muted">Loading…</div>
      {:else}
        <pre class="report">{reportText}</pre>
      {/if}
    </Modal>
  {/if}
</div>
</PageBody>
</div>

<style>
  /* Colors come from the app theme tokens (tokens.css) so the page adapts to
     light + dark. Buttons reuse the global `.btn`/`.btn.small/.primary/.danger`. */
  .sched-page { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .muted { color: var(--text-dim); padding: 0.75rem 0; font-size: 0.9rem; }
  .err {
    background: var(--danger-soft);
    color: var(--danger); padding: 0.5rem 0.75rem;
    border-radius: var(--radius-s); margin-bottom: 0.75rem; font-size: 0.85rem;
  }
  .notice {
    display: flex; align-items: center; gap: 0.5rem;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--text); padding: 0.5rem 0.75rem;
    border-radius: var(--radius-s); margin-bottom: 0.75rem; font-size: 0.85rem;
  }
  .grow { flex: 1; }
  .tasks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.5rem; }
  .task { border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 0.6rem 0.75rem; color: var(--text); }
  .task-main { display: flex; justify-content: space-between; align-items: center; gap: 0.75rem; flex-wrap: wrap; }
  .task-info { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
  .name { font-size: var(--fs-m); font-weight: 600; color: var(--text); }
  .meta { color: var(--text-dim); font-size: var(--fs-s); }
  .task-actions { display: flex; align-items: center; gap: 2px; flex-wrap: wrap; }
  .task-actions .del:hover { color: var(--danger); background: var(--danger-soft); }
  .runs { margin-top: 0.6rem; border-top: 1px solid var(--border); padding-top: 0.5rem; display: flex; flex-direction: column; gap: 0.35rem; }
  .run { display: flex; align-items: center; gap: 0.5rem; font-size: 0.82rem; flex-wrap: wrap; color: var(--text); }
  .run-when { color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .run-sum { flex: 1; min-width: 12ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pill { font-size: var(--fs-xs); padding: 0.05rem 0.45rem; border-radius: 999px; border: 1px solid var(--border); color: var(--text-dim); }
  .pill.ok { background: var(--success-soft); color: var(--success); border-color: transparent; }
  .pill.warn { background: var(--warning-soft); color: var(--warning); border-color: transparent; }
  .form { display: flex; flex-direction: column; gap: 0.75rem; max-width: 720px; }
  .row { display: flex; gap: 0.75rem; flex-wrap: wrap; }
  .row .fld { flex: 1; min-width: 180px; }
  .fld { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.85rem; color: var(--text); }
  .fld span { color: var(--text-dim); }
  .fld input, .fld select, .fld textarea {
    background: var(--bg); color: var(--text); border: 1px solid var(--border);
    border-radius: var(--radius-s); padding: 0.45rem 0.55rem; font: inherit;
  }
  .fld input::placeholder, .fld textarea::placeholder { color: var(--text-dim); }
  .fld input:focus-visible, .fld select:focus-visible, .fld textarea:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent); outline-offset: 1px;
  }
  .chk { display: flex; align-items: center; gap: 0.4rem; font-size: 0.85rem; color: var(--text); }
  .toggles { display: flex; flex-direction: column; gap: 0.4rem; margin: 0.25rem 0; }
  .hint { font-size: 0.82rem; color: var(--text-dim); margin: 0 0 0.25rem; }
  .actions { display: flex; gap: 0.5rem; margin-top: 0.5rem; }
  .report { margin: 0; white-space: pre-wrap; word-break: break-word; font-family: var(--font-mono); font-size: 0.8rem; line-height: 1.45; color: var(--text); }
</style>
