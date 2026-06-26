<script lang="ts">
  import { ws } from '../../lib/stores/workspace.svelte';
  import { scheduledTasks } from '../../lib/stores/scheduledTasks.svelte';
  import { authedText } from '../../lib/api/client';
  import { scheduledTasksApi, type ScheduledTaskInput } from '../../lib/api/scheduledTasks';
  import type { ScheduledTask, ScheduledTaskRun } from '../../lib/api/types';

  let creating = $state(false);
  let editId = $state<string | null>(null);
  let expandedId = $state<string | null>(null);
  let busy = $state(false);
  let error = $state('');

  // Report viewer modal
  let reportOpen = $state(false);
  let reportText = $state('');
  let reportLoading = $state(false);

  // --- form model ---
  let fName = $state('');
  let fPrompt = $state('');
  let fSkill = $state('');
  let fCadence = $state<'interval' | 'daily' | 'weekly'>('interval');
  let fEveryMin = $state(60);
  let fAt = $state('03:00');
  let fWeekday = $state(0);
  let fDestType = $state<'none' | 'slack' | 'telegram' | 'email' | 'webhook'>('none');
  let fChatId = $state('');
  let fEmailTo = $state('');
  let fUrl = $state('');
  let fEnabled = $state(true);
  let fCwd = $state('');

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
    fCadence = 'interval';
    fEveryMin = 60;
    fAt = '03:00';
    fWeekday = 0;
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
    const cad = (p.schedule.cadence as string) ?? 'interval';
    fCadence = cad === 'daily' || cad === 'weekly' ? cad : 'interval';
    fEveryMin = (p.schedule.every_min as number) ?? 60;
    fAt = (p.schedule.at as string) ?? '03:00';
    fWeekday = (p.schedule.weekday as number) ?? 0;
    const dt = (p.suggested_destination?.type as string) ?? 'none';
    fDestType = ['slack', 'telegram', 'email', 'webhook'].includes(dt) ? (dt as typeof fDestType) : 'none';
  }

  function startEdit(t: ScheduledTask): void {
    resetForm();
    editId = t.id;
    creating = false;
    fName = t.name;
    fPrompt = t.prompt;
    fSkill = t.skill ?? '';
    const cad = (t.schedule.cadence as string) ?? 'interval';
    fCadence = cad === 'daily' || cad === 'weekly' ? cad : 'interval';
    fEveryMin = (t.schedule.every_min as number) ?? 60;
    fAt = (t.schedule.at as string) ?? '03:00';
    fWeekday = (t.schedule.weekday as number) ?? 0;
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

  async function save(): Promise<void> {
    error = '';
    if (!fName.trim()) {
      error = 'Name is required.';
      return;
    }
    const body: ScheduledTaskInput = {
      name: fName.trim(),
      prompt: fPrompt,
      skill: fSkill.trim() || null,
      cwd: fCwd.trim(),
      schedule: buildSchedule(),
      destination: buildDestination(),
      enabled: fEnabled,
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

  async function remove(t: ScheduledTask): Promise<void> {
    if (!confirm(`Delete scheduled task "${t.name}"?`)) return;
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
    if (c === 'interval') return `every ${(s.every_min as number) ?? 60} min`;
    if (c === 'daily') return `daily at ${(s.at as string) ?? '09:00'} UTC`;
    const wd = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'][(s.weekday as number) ?? 0];
    return `weekly ${wd} at ${(s.at as string) ?? '09:00'} UTC`;
  }

  function destLabel(t: ScheduledTask): string {
    return ((t.destination?.type as string) ?? 'none') as string;
  }

  function statusClass(status: string | null | undefined): string {
    switch (status) {
      case 'ok':
        return 'pill ok';
      case 'error':
        return 'pill bad';
      case 'running':
        return 'pill working';
      default:
        return 'pill';
    }
  }
</script>

<div class="sched">
  {#if creating || editId}
    <header class="head">
      <h1>{editId ? 'Edit scheduled task' : 'New scheduled task'}</h1>
    </header>
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

      <label class="fld">
        <span>Prompt (the agent's instructions)</span>
        <textarea bind:value={fPrompt} rows="6" placeholder="Go over every ticket updated in the last 24h…"></textarea>
      </label>

      <div class="row">
        <label class="fld">
          <span>Cadence</span>
          <select bind:value={fCadence}>
            <option value="interval">Interval</option>
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
          </select>
        </label>
        {#if fCadence === 'interval'}
          <label class="fld">
            <span>Every (minutes, min 5)</span>
            <input type="number" min="5" bind:value={fEveryMin} />
          </label>
        {:else}
          <label class="fld">
            <span>At (HH:MM UTC)</span>
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

      <div class="row">
        <label class="fld">
          <span>Skill (optional, inlined)</span>
          <input bind:value={fSkill} placeholder="e.g. db-mysql" />
        </label>
        <label class="fld">
          <span>Working dir (optional)</span>
          <input bind:value={fCwd} placeholder="repo path — not a sandbox" />
        </label>
      </div>

      <label class="chk"><input type="checkbox" bind:checked={fEnabled} /> Enabled</label>

      <div class="actions">
        <button class="btn primary" disabled={busy} onclick={save}>{busy ? 'Saving…' : 'Save'}</button>
        <button class="btn" disabled={busy} onclick={() => { creating = false; editId = null; }}>Cancel</button>
      </div>
    </div>
  {:else}
    <header class="head">
      <div>
        <h1>Scheduled Tasks</h1>
        <p class="sub">
          Recurring agent jobs — run a prompt on a cadence, produce a report, and deliver it to
          Slack, email, or a webhook. Also driveable over MCP.
        </p>
      </div>
      <button class="btn primary" onclick={startCreate}>New task</button>
    </header>

    {#if error}<div class="err" role="alert">{error}</div>{/if}

    {#if list.length === 0}
      <div class="empty">No scheduled tasks yet. Create one to run an agent on a cadence.</div>
    {:else}
      <ul class="tasks">
        {#each list as t (t.id)}
          <li class="task">
            <div class="task-main">
              <div class="task-info">
                <strong class="name">{t.name}</strong>
                <span class="meta">{cadenceLabel(t)} · → {destLabel(t)}</span>
                {#if t.last_status}<span class={statusClass(t.last_status)}>{t.last_status}</span>{/if}
                {#if !t.enabled}<span class="pill">paused</span>{/if}
              </div>
              <div class="task-actions">
                <button class="btn sm" onclick={() => runNow(t)} disabled={busy}>Run now</button>
                <button class="btn sm" onclick={() => toggleRuns(t)}>
                  {expandedId === t.id ? 'Hide runs' : 'Runs'}
                </button>
                <button class="btn sm" onclick={() => toggle(t)}>{t.enabled ? 'Pause' : 'Enable'}</button>
                <button class="btn sm" onclick={() => startEdit(t)}>Edit</button>
                <button class="btn sm danger" onclick={() => remove(t)}>Delete</button>
              </div>
            </div>
            {#if expandedId === t.id}
              <div class="runs">
                {#each scheduledTasks.runsByTask[t.id] ?? [] as r (r.id)}
                  <div class="run">
                    <span class={statusClass(r.status)}>{r.status}</span>
                    <span class="run-when">{r.started_at}</span>
                    <span class="run-sum">{r.summary || '(no summary)'}</span>
                    {#if r.report_rel}
                      <button class="btn sm" onclick={() => viewReport(r)}>View report</button>
                    {/if}
                    {#if r.delivered}<span class="pill ok">delivered</span>{/if}
                    {#if r.delivery_error}<span class="pill warn" title={r.delivery_error}>delivery failed</span>{/if}
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
    <div
      class="modal-bg"
      onclick={(e) => { if (e.target === e.currentTarget) reportOpen = false; }}
      onkeydown={(e) => { if (e.key === 'Escape') reportOpen = false; }}
      role="presentation"
    >
      <div class="modal" role="dialog" aria-label="Report" aria-modal="true" tabindex="-1">
        <header class="modal-head">
          <strong>Report</strong>
          <button class="btn sm" onclick={() => (reportOpen = false)}>Close</button>
        </header>
        {#if reportLoading}
          <div class="muted">Loading…</div>
        {:else}
          <pre class="report">{reportText}</pre>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .sched { padding: 1rem 1.25rem; max-width: 980px; margin: 0 auto; }
  .head { display: flex; justify-content: space-between; align-items: flex-start; gap: 1rem; margin-bottom: 0.75rem; }
  .head h1 { margin: 0; font-size: 1.25rem; }
  .sub { margin: 0.25rem 0 0; color: var(--muted, #8b949e); font-size: 0.85rem; max-width: 60ch; }
  .empty, .muted { color: var(--muted, #8b949e); padding: 0.75rem 0; font-size: 0.9rem; }
  .err { background: rgba(248,81,73,0.12); color: #f85149; padding: 0.5rem 0.75rem; border-radius: 6px; margin-bottom: 0.75rem; font-size: 0.85rem; }
  .tasks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 0.5rem; }
  .task { border: 1px solid var(--border, #30363d); border-radius: 8px; padding: 0.6rem 0.75rem; }
  .task-main { display: flex; justify-content: space-between; align-items: center; gap: 0.75rem; flex-wrap: wrap; }
  .task-info { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
  .name { font-size: 0.95rem; }
  .meta { color: var(--muted, #8b949e); font-size: 0.8rem; }
  .task-actions { display: flex; gap: 0.35rem; flex-wrap: wrap; }
  .runs { margin-top: 0.6rem; border-top: 1px solid var(--border, #30363d); padding-top: 0.5rem; display: flex; flex-direction: column; gap: 0.35rem; }
  .run { display: flex; align-items: center; gap: 0.5rem; font-size: 0.82rem; flex-wrap: wrap; }
  .run-when { color: var(--muted, #8b949e); font-variant-numeric: tabular-nums; }
  .run-sum { flex: 1; min-width: 12ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pill { font-size: 0.7rem; padding: 0.05rem 0.4rem; border-radius: 999px; border: 1px solid var(--border, #30363d); }
  .pill.ok { background: rgba(126,231,135,0.16); color: #7ee787; border-color: transparent; }
  .pill.bad { background: rgba(248,81,73,0.16); color: #f85149; border-color: transparent; }
  .pill.warn { background: rgba(210,153,34,0.16); color: #d29922; border-color: transparent; }
  .pill.working { background: rgba(88,166,255,0.16); color: #58a6ff; border-color: transparent; }
  .form { display: flex; flex-direction: column; gap: 0.75rem; max-width: 720px; }
  .row { display: flex; gap: 0.75rem; flex-wrap: wrap; }
  .row .fld { flex: 1; min-width: 180px; }
  .fld { display: flex; flex-direction: column; gap: 0.25rem; font-size: 0.85rem; }
  .fld span { color: var(--muted, #8b949e); }
  .fld input, .fld select, .fld textarea {
    background: var(--input-bg, #0d1117); color: inherit; border: 1px solid var(--border, #30363d);
    border-radius: 6px; padding: 0.4rem 0.5rem; font: inherit;
  }
  .chk { display: flex; align-items: center; gap: 0.4rem; font-size: 0.85rem; }
  .actions { display: flex; gap: 0.5rem; margin-top: 0.5rem; }
  .btn { background: var(--btn-bg, #21262d); color: inherit; border: 1px solid var(--border, #30363d); border-radius: 6px; padding: 0.35rem 0.7rem; font: inherit; cursor: pointer; }
  .btn.sm { padding: 0.2rem 0.5rem; font-size: 0.78rem; }
  .btn.primary { background: var(--accent, #238636); border-color: transparent; color: #fff; }
  .btn.danger, .btn.sm.danger { color: #f85149; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .modal-bg { position: fixed; inset: 0; background: rgba(0,0,0,0.55); display: flex; align-items: center; justify-content: center; z-index: 50; }
  .modal { background: var(--bg, #161b22); border: 1px solid var(--border, #30363d); border-radius: 10px; width: min(760px, 92vw); max-height: 82vh; overflow: auto; padding: 0.85rem 1rem; }
  .modal-head { display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.5rem; }
  .report { white-space: pre-wrap; word-break: break-word; font-family: var(--mono, ui-monospace, monospace); font-size: 0.8rem; line-height: 1.45; }
</style>
