<script lang="ts">
  import PathField from '../../lib/components/PathField.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus } from '../../lib/status';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { scheduledTasks } from '../../lib/stores/scheduledTasks.svelte';
  import { scheduledTasksPort } from '../../lib/uiCommands/scheduled';
  import { authedText } from '../../lib/api/client';
  import { scheduledTasksApi, type ScheduledTaskInput } from '../../lib/api/scheduledTasks';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { ScheduledTask, ScheduledTaskRun } from '../../lib/api/types';
  import { allProviders, defaultAgentProvider } from '../../lib/providers';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { api } from '../../lib/api/client';
  import { loadErrorText } from '../../lib/loadError';
  import type { Workflow } from '../../lib/api/types';
  import { renderMarkdownGfm } from '../../lib/md';
  import { copyText } from '../../lib/clipboard';

  let creating = $state(false);
  let editId = $state<string | null>(null);
  let expandedId = $state<string | null>(null);
  // Agent UI control (lib/uiCommands/scheduled.ts) expands a task's runs.
  $effect(() => scheduledTasksPort.bind({ expand: (id) => (expandedId = id) }));
  let busy = $state(false);
  /** Inline form error (validation / failed save). Row actions report via toasts. */
  let error = $state('');
  /** Task ids with a "Run now" in flight — per row, so one run never freezes the list. */
  let runningIds = $state<Record<string, boolean>>({});

  // Report viewer modal
  let reportOpen = $state(false);
  let reportText = $state('');
  let reportLoading = $state(false);
  let reportError = $state('');
  let reportRun = $state<ScheduledTaskRun | null>(null);
  let reportTaskName = $state('');
  /** Reports are markdown (an agent's write-up, or the daemon's Command /
   *  stdout / stderr wrapper for a shell task) — rendered by default, with a
   *  Plain text toggle for the raw file. */
  let reportRaw = $state(false);
  const reportHtml = $derived(reportRaw || !reportText ? '' : renderMarkdownGfm(reportText));

  /** Form snapshot taken when the form opens — Back/Cancel only asks before
   *  discarding when something actually changed. */
  let formSnapshot = $state('');
  /** Destination the task had when the edit began ('' for a new task) — a
   *  save only re-confirms delivery when the destination is new or changed. */
  let originalDest = $state('');

  // Workflows in this workspace, for the "Hand off to a workflow" picker.
  let wfOptions = $state<Workflow[]>([]);
  let wfLoading = $state(false);
  let wfError = $state('');
  let wfLoaded = $state(false);
  /** The rarely-changed fields sit behind "Advanced" (a creation form shows the
   *  few fields most people need); editing a task that uses them opens it. */
  let showAdvanced = $state(false);

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

  /** Known IANA zones for the timezone field's suggestions (empty on engines
   *  without `Intl.supportedValuesOf`, where the field is plain free text). */
  const tzNames: string[] = (() => {
    try {
      return (Intl as unknown as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf?.('timeZone') ?? [];
    } catch {
      return [];
    }
  })();
  /** Is `tz` a zone this engine knows? The daemon rejects unknown names on
   *  save; say so while typing instead of after the round-trip. */
  function tzValid(tz: string): boolean {
    const t = tz.trim();
    if (!t) return true; // empty → UTC
    try {
      new Intl.DateTimeFormat('en-US', { timeZone: t });
      return true;
    } catch {
      return false;
    }
  }
  const tzOk = $derived(tzValid(fTimezone));
  /** Standard 5-field cron (minute hour day-of-month month day-of-week). */
  const cronFieldCount = $derived(fCronExpr.trim() ? fCronExpr.trim().split(/\s+/).length : 0);

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
    // Fresh workflow list each time the form opens (one may have been added since).
    wfLoaded = false;
    wfError = '';
    showAdvanced = false;
  }

  function formState(): string {
    return JSON.stringify([fName, fPrompt, fSkill, fKind, fProvider, fModel, fWorkflowId, fCadence, fEveryMin, fAt, fWeekday,
      fCronExpr, fTimezone, fSandbox, fMaxRetries, fNotifyOnChange, fAttachProof, fDestType, fChatId, fEmailTo, fUrl, fEnabled, fCwd]);
  }

  function startCreate(): void {
    resetForm();
    creating = true;
    editId = null;
    originalDest = '';
    formSnapshot = formState();
  }

  /** Leave the form (Back / Cancel), asking first only when edits would be lost. */
  async function closeForm(): Promise<void> {
    if (formState() !== formSnapshot) {
      const ok = await confirmer.ask(editId ? 'Discard your changes to this task?' : 'Discard this new task?', {
        title: 'Discard changes',
        confirmLabel: 'Discard',
      });
      if (!ok) return;
    }
    creating = false;
    editId = null;
    error = '';
  }

  async function loadWorkflowOptions(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    wfLoading = true;
    wfError = '';
    try {
      wfOptions = await api.get<Workflow[]>(`/workspaces/${wsId}/workflows`);
      wfLoaded = true;
    } catch (e) {
      wfError = loadErrorText(e);
    } finally {
      wfLoading = false;
    }
  }
  // Load the workflow picker the first time the form needs it.
  $effect(() => {
    if ((creating || editId) && fKind === 'workflow' && !wfLoaded && !wfLoading && !wfError) void loadWorkflowOptions();
  });

  function applyPreset(id: string): void {
    const p = presets.find((x) => x.id === id);
    if (!p) return;
    fName = p.name;
    fPrompt = p.prompt;
    fSkill = p.skill ?? '';
    loadSchedule(p.schedule);
    // Review/security/dependency presets benefit from an isolated worktree + change-only notify.
    if (p.id.startsWith('weekly-')) {
      showAdvanced = true;
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
    originalDest = JSON.stringify(buildDestination());
    showAdvanced = !!(fSkill || (fCwd && fProvider !== 'shell') || fSandbox !== 'none' || fMaxRetries || fNotifyOnChange || fAttachProof);
    formSnapshot = formState();
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

  /** Where a destination sends reports, in words ("the Slack channel C123"). */
  function destWhere(): string {
    switch (fDestType) {
      case 'slack':
        return fChatId.trim() ? `the Slack channel ${fChatId.trim()}` : 'the Slack integration’s default channel';
      case 'telegram':
        return fChatId.trim() ? `the Telegram chat ${fChatId.trim()}` : 'the Telegram integration’s default chat';
      case 'email':
        return fEmailTo.trim() || 'an email address';
      case 'webhook':
        return fUrl.trim() || 'a webhook';
      default:
        return '';
    }
  }

  async function save(): Promise<void> {
    error = '';
    if (!fName.trim()) {
      error = 'Give the task a name.';
      return;
    }
    if (fKind === 'workflow' && !fWorkflowId.trim()) {
      error = 'Pick the workflow this task launches.';
      return;
    }
    if (fDestType === 'email' && !fEmailTo.trim()) {
      error = 'Add the email address reports go to.';
      return;
    }
    if (fDestType === 'webhook' && !/^https?:\/\//i.test(fUrl.trim())) {
      error = 'Add the webhook URL (starting with https://).';
      return;
    }
    if (fCadence !== 'interval' && !tzOk) {
      error = 'Fix the timezone — use an IANA name like Europe/London.';
      return;
    }
    if (fCadence === 'cron' && cronFieldCount !== 5) {
      error = 'The cron expression needs 5 fields.';
      return;
    }
    // Outward-facing: every run posts its report off the Mac. Say where
    // before the first save that turns delivery on (or points it elsewhere).
    if (fDestType !== 'none' && JSON.stringify(buildDestination()) !== originalDest) {
      const ok = await confirmer.ask(
        `Each run's report will be sent to ${destWhere()}. Anyone who can read it there will see the report.`,
        { title: 'Deliver reports outside Otto', confirmLabel: editId ? 'Save and deliver' : 'Create and deliver', danger: false },
      );
      if (!ok) return;
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
    const wsId = ws.currentId;
    if (!wsId) {
      error = 'Pick a workspace first — scheduled tasks belong to a workspace.';
      return;
    }
    busy = true;
    try {
      if (editId) await scheduledTasks.update(editId, body);
      else await scheduledTasks.create(wsId, body);
      toasts.success(editId ? 'Scheduled task saved' : 'Scheduled task created', body.name);
      creating = false;
      editId = null;
    } catch (e) {
      error = `Couldn't save the task. ${errText(e)}`;
    } finally {
      busy = false;
    }
  }

  function errText(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  async function toggle(t: ScheduledTask): Promise<void> {
    try {
      await scheduledTasks.setEnabled(t.id, !t.enabled);
    } catch (e) {
      toasts.error(t.enabled ? `Couldn't pause “${t.name}”` : `Couldn't resume “${t.name}”`, errText(e));
    }
  }

  async function runNow(t: ScheduledTask): Promise<void> {
    runningIds = { ...runningIds, [t.id]: true };
    try {
      await scheduledTasks.runNow(t.id);
      expandedId = t.id;
    } catch (e) {
      toasts.error(`Couldn't start “${t.name}”`, errText(e));
    } finally {
      const { [t.id]: _done, ...rest } = runningIds;
      runningIds = rest;
    }
  }

  async function convertToWorkflow(t: ScheduledTask): Promise<void> {
    busy = true;
    try {
      const res = await scheduledTasksApi.convertToWorkflow(t.id);
      convertedWfId = res.workflow_id;
      wfLoaded = false; // the picker re-fetches and sees the new workflow
      toasts.success('Converted to workflow', `Created a workflow from “${t.name}”.`);
    } catch (e) {
      toasts.error(`Couldn't convert “${t.name}” to a workflow`, errText(e));
    } finally {
      busy = false;
    }
  }

  async function remove(t: ScheduledTask): Promise<void> {
    const ok = await confirmer.ask(`Delete “${t.name}”? It stops running and can't be restored.`, {
      title: 'Delete scheduled task',
    });
    if (!ok) return;
    try {
      await scheduledTasks.remove(t.id);
      if (expandedId === t.id) expandedId = null;
      toasts.success('Scheduled task deleted', t.name);
    } catch (e) {
      toasts.error(`Couldn't delete “${t.name}”`, errText(e));
    }
  }

  /** The row's ⋯ menu: everything past Run now / Runs (guidelines: >2 row actions → a menu). */
  function rowMenu(e: MouseEvent, t: ScheduledTask): void {
    const items: MenuItem[] = [
      { label: 'Edit…', icon: 'edit', action: () => startEdit(t) },
      { label: t.enabled ? 'Pause' : 'Resume', icon: t.enabled ? 'pause' : 'play', action: () => void toggle(t) },
      { label: 'Convert to workflow', icon: 'split', disabled: busy, action: () => void convertToWorkflow(t) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void remove(t) },
    ];
    ctxMenu.show(e, items);
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

  async function viewReport(run: ScheduledTaskRun, taskName: string, plain = false): Promise<void> {
    reportRun = run;
    reportTaskName = taskName;
    reportRaw = plain;
    reportOpen = true;
    reportLoading = true;
    reportText = '';
    reportError = '';
    try {
      reportText = await authedText(scheduledTasksApi.reportPath(run.id));
    } catch (e) {
      reportError = loadErrorText(e);
    } finally {
      reportLoading = false;
    }
  }

  function everyLabel(min: number): string {
    if (min % 1440 === 0) return min === 1440 ? 'Every day' : `Every ${min / 1440} days`;
    if (min % 60 === 0) return min === 60 ? 'Every hour' : `Every ${min / 60} hours`;
    return `Every ${min} min`;
  }

  /** Human cadence for a row ("Daily at 03:00 · Europe/London"). Cron keeps
   *  its expression, which the row renders in mono. */
  function cadenceLabel(t: ScheduledTask): string {
    const s = t.schedule ?? {};
    const c = (s.cadence as string) ?? 'interval';
    const tz = t.timezone || 'UTC';
    if (c === 'interval') return everyLabel((s.every_min as number) ?? 60);
    if (c === 'cron') return 'Cron';
    if (c === 'daily') return `Daily at ${(s.at as string) ?? '09:00'} · ${tz}`;
    const wd = ['Mondays', 'Tuesdays', 'Wednesdays', 'Thursdays', 'Fridays', 'Saturdays', 'Sundays'][(s.weekday as number) ?? 0];
    return `${wd} at ${(s.at as string) ?? '09:00'} · ${tz}`;
  }

  /** Where the report goes, in words. */
  function destLabel(t: ScheduledTask): string {
    const d = t.destination ?? {};
    switch (d.type as string) {
      case 'slack':
        return 'Sends to Slack';
      case 'telegram':
        return 'Sends to Telegram';
      case 'email':
        return d.to ? `Emails ${d.to as string}` : 'Sends by email';
      case 'webhook':
        return 'Posts to a webhook';
      default:
        return 'Report kept in Otto';
    }
  }

  function destTitle(t: ScheduledTask): string {
    const d = t.destination ?? {};
    if (d.type === 'webhook' && d.url) return String(d.url);
    if ((d.type === 'slack' || d.type === 'telegram') && d.chat_id) return `Chat / channel ${String(d.chat_id)}`;
    return destLabel(t);
  }

</script>

<div class="sched-page">
<PageHeader
  title={creating || editId ? (editId ? `Edit “${list.find((t) => t.id === editId)?.name ?? 'task'}”` : 'New scheduled task') : 'Scheduled Tasks'}
  subtitle={creating || editId ? undefined : 'Run an agent on a cadence and deliver its report'}
>
  {#snippet leading()}
    {#if creating || editId}
      <button class="icon-btn" title="Back to Scheduled Tasks" aria-label="Back to Scheduled Tasks" disabled={busy} onclick={closeForm}>
        <Icon name="chevronLeft" size={15} />
      </button>
    {/if}
  {/snippet}
  {#snippet actions()}
    {#if !(creating || editId) && list.length > 0}
      <button class="btn small primary" onclick={startCreate}><Icon name="plus" size={12} /> New task</button>
    {/if}
  {/snippet}
</PageHeader>
<PageBody width="readable">
<div class="sched">
  {#if creating || editId}
    <form class="form" onsubmit={(e) => { e.preventDefault(); void save(); }}>
      {#if error}<div class="err" role="alert"><Icon name="warning" size={12} /> {error}</div>{/if}

      {#if !editId && presets.length}
        <label class="fld">
          <span>Start from a preset</span>
          <select class="input" onchange={(e) => applyPreset((e.currentTarget as HTMLSelectElement).value)}>
            <option value="">Blank task</option>
            {#each presets as p}<option value={p.id}>{p.name}</option>{/each}
          </select>
        </label>
      {/if}

      <label class="fld">
        <span>Name</span>
        <input class="input" bind:value={fName} placeholder="Nightly ticket review" required />
      </label>

      <div class="frow">
        <label class="fld">
          <span>Type</span>
          <select class="input" bind:value={fKind}>
            <option value="agent_prompt">Run an agent</option>
            <option value="workflow">Hand off to a workflow</option>
          </select>
        </label>
        {#if fKind === 'agent_prompt'}
          <label class="fld">
            <span>Provider</span>
            <select
              class="input"
              value={PROVIDERS.includes(fProvider) ? fProvider : 'custom'}
              onchange={(e) => onProviderSelect((e.currentTarget as HTMLSelectElement).value)}
            >
              {#each PROVIDERS as p}<option value={p}>{p}</option>{/each}
              <option value="custom">Custom…</option>
            </select>
          </label>
        {:else}
          <label class="fld">
            <span>Workflow</span>
            <select class="input" bind:value={fWorkflowId} disabled={wfLoading}>
              <option value="">{wfLoading ? 'Loading workflows…' : wfOptions.length ? 'Choose a workflow…' : 'No workflows in this workspace'}</option>
              {#each wfOptions as w (w.id)}<option value={w.id}>{w.name || 'Untitled workflow'}</option>{/each}
              {#if fWorkflowId && !wfLoading && !wfOptions.some((w) => w.id === fWorkflowId)}
                <option value={fWorkflowId}>Unknown workflow ({fWorkflowId.slice(0, 8)}…)</option>
              {/if}
            </select>
            {#if wfError}
              <small class="fld-hint bad">Couldn't load workflows. {wfError}
                <button type="button" class="btn small ghost" onclick={loadWorkflowOptions}>Retry</button></small>
            {/if}
          </label>
        {/if}
      </div>

      {#if fKind === 'agent_prompt' && !PROVIDERS.includes(fProvider)}
        <label class="fld">
          <span>Custom provider slug</span>
          <input class="input" bind:value={fProvider} placeholder="my-custom-agent (register it in Settings first)" />
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
          <textarea class="input mono" bind:value={fPrompt} rows="4" placeholder="e.g. df -h && uptime"></textarea>
        </label>
      {:else}
        <label class="fld">
          <span>Prompt (the agent's instructions)</span>
          <textarea class="input" bind:value={fPrompt} rows="6" placeholder="Go over every ticket updated in the last 24h…"></textarea>
        </label>
      {/if}

      <div class="frow">
        <label class="fld">
          <span>Cadence</span>
          <select class="input" bind:value={fCadence}>
            <option value="interval">Interval</option>
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
            <option value="cron">Cron</option>
          </select>
        </label>
        {#if fCadence === 'interval'}
          <label class="fld">
            <span>Every (minutes, min 5)</span>
            <input class="input" type="number" min="5" bind:value={fEveryMin} />
          </label>
        {:else if fCadence === 'cron'}
          <label class="fld">
            <span>Cron expression (5 fields)</span>
            <input class="input mono" bind:value={fCronExpr} placeholder="0 9 * * 1" aria-invalid={cronFieldCount !== 5} />
            <small class="fld-hint" class:bad={cronFieldCount !== 5}>
              {cronFieldCount === 5
                ? 'minute · hour · day of month · month · day of week (0 or 7 = Sun)'
                : `Needs 5 fields (minute hour day month weekday) — has ${cronFieldCount}`}
            </small>
          </label>
        {:else}
          <label class="fld">
            <span>At (24h, in the timezone)</span>
            <input class="input" type="time" bind:value={fAt} />
          </label>
          {#if fCadence === 'weekly'}
            <label class="fld">
              <span>Weekday</span>
              <select class="input" bind:value={fWeekday}>
                {#each ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'] as d, i}
                  <option value={i}>{d}</option>
                {/each}
              </select>
            </label>
          {/if}
        {/if}
        {#if fCadence !== 'interval'}
          <label class="fld">
            <span>Timezone</span>
            <input class="input" bind:value={fTimezone} placeholder="e.g. Europe/London" list="sched-tz-list" aria-invalid={!tzOk} />
            {#if !tzOk}<small class="fld-hint bad">Unknown timezone — use an IANA name like Europe/London</small>{/if}
            {#if tzNames.length}
              <datalist id="sched-tz-list">{#each tzNames as z (z)}<option value={z}></option>{/each}</datalist>
            {/if}
          </label>
        {/if}
      </div>

      <div class="frow">
        <label class="fld">
          <span>Destination</span>
          <select class="input" bind:value={fDestType}>
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
            <input class="input" bind:value={fChatId} placeholder="defaults to the integration channel" />
          </label>
        {:else if fDestType === 'email'}
          <label class="fld">
            <span>Send to (email)</span>
            <input class="input" type="email" bind:value={fEmailTo} placeholder="you@example.com" />
          </label>
        {:else if fDestType === 'webhook'}
          <label class="fld">
            <span>Webhook URL</span>
            <input class="input" type="url" bind:value={fUrl} placeholder="https://…" />
          </label>
        {/if}
      </div>

      {#if fProvider === 'shell' && fKind === 'agent_prompt'}
        <label class="fld">
          <span>Working dir (optional)</span>
          <PathField bind:value={fCwd}><input class="input" bind:value={fCwd} placeholder="dir to run the command in" /></PathField>
        </label>
      {/if}

      <label class="checkbox-row"><input type="checkbox" bind:checked={fEnabled} /> Run on this schedule (off = paused)</label>

      <div class="adv">
        <button type="button" class="adv-toggle" aria-expanded={showAdvanced} aria-controls="sched-advanced" onclick={() => (showAdvanced = !showAdvanced)}>
          <Icon name={showAdvanced ? 'chevronDown' : 'chevronRight'} size={12} /> Advanced
          <span class="adv-hint">{fKind === 'agent_prompt' && fProvider !== 'shell' ? 'skill, working dir, sandbox, retries, delivery rules' : 'delivery rules'}</span>
        </button>
      {#if showAdvanced}
      <div class="adv-body" id="sched-advanced">
      {#if fKind === 'agent_prompt' && fProvider !== 'shell'}
        <div class="frow">
          <label class="fld">
            <span>Skill (optional, inlined)</span>
            <input class="input" bind:value={fSkill} placeholder="e.g. db-mysql" />
          </label>
          <label class="fld">
            <span>Working dir (optional)</span>
            <PathField bind:value={fCwd}><input class="input" bind:value={fCwd} placeholder="repo path — not a sandbox" /></PathField>
          </label>
        </div>

        <div class="frow">
          <label class="fld">
            <span>Sandbox</span>
            <select class="input" bind:value={fSandbox}>
              <option value="none">Run in working dir</option>
              <option value="worktree">Isolated git worktree</option>
            </select>
          </label>
          <label class="fld">
            <span>Retries on failure (0–5)</span>
            <input class="input" type="number" min="0" max="5" bind:value={fMaxRetries} />
          </label>
        </div>
      {/if}

      <div class="toggles">
        <label class="checkbox-row"><input type="checkbox" bind:checked={fNotifyOnChange} /> Only deliver when the report meaningfully changes</label>
        <label class="checkbox-row"><input type="checkbox" bind:checked={fAttachProof} /> Attach a proof pack to each run</label>
      </div>
      </div>
      {/if}
      </div>

      <div class="actions">
        <button type="button" class="btn" disabled={busy} onclick={closeForm}>Cancel</button>
        <button type="submit" class="btn primary" disabled={busy}>
          {busy ? 'Saving…' : editId ? 'Save changes' : 'Create task'}
        </button>
      </div>
    </form>
  {:else}
    {#if convertedWfId}
      <div class="notice" role="status">
        <span>Created a workflow from this task.</span>
        <span class="grow"></span>
        <button class="btn small" onclick={() => { convertedWfId = null; router.go('workflows'); }}>Open Workflows</button>
        <button class="btn small" onclick={() => (convertedWfId = null)}>Dismiss</button>
      </div>
    {/if}

    <LoadState
      what="scheduled tasks"
      variant="page"
      loading={scheduledTasks.loadingList}
      error={scheduledTasks.listError}
      empty={list.length === 0}
      onretry={() => ws.currentId && void scheduledTasks.loadList(ws.currentId)}
    >
      {#snippet emptyView()}
        <EmptyState
          variant="page"
          icon="calendar"
          title="No scheduled tasks yet"
          body="Create one to run an agent on a cadence and deliver its report."
          actionLabel="New task"
          actionIcon="plus"
          onaction={startCreate}
        />
      {/snippet}
      <ul class="tasks">
        {#each list as t (t.id)}
          <li class="task" data-task-id={t.id}>
            <div class="task-main">
              <div class="task-info">
                <div class="task-title">
                  <strong class="name" title={t.name}>{t.name}</strong>
                  {#if !t.enabled}
                    <StatusBadge tone="neutral" label="Paused" />
                  {:else if t.last_status}
                    <!-- Shared run vocabulary (lib/status.ts): ok → Succeeded, error → Failed. -->
                    <StatusBadge status={runStatus(t.last_status)} />
                  {/if}
                </div>
                <div class="meta">
                  <span>{cadenceLabel(t)}</span>
                  {#if (t.schedule?.cadence as string) === 'cron'}<code class="cron">{t.schedule?.expr as string}</code><span>{t.timezone || 'UTC'}</span>{/if}
                  <span aria-hidden="true">·</span>
                  <span class="dest" title={destTitle(t)}>{destLabel(t)}</span>
                  <!-- When it fires next (ticks via RelTime; exact time on hover). -->
                  {#if t.enabled && t.next_run_at}<span aria-hidden="true">·</span><span>Next <RelTime iso={t.next_run_at} /></span>{/if}
                </div>
              </div>
              <div class="task-actions">
                <button class="btn small" onclick={() => runNow(t)} disabled={!!runningIds[t.id]}
                  title={destLabel(t) === 'Report kept in Otto' ? 'Run once now' : `Run once now — ${destLabel(t).toLowerCase()}`}>
                  <Icon name="play" size={12} /> {runningIds[t.id] ? 'Starting…' : 'Run now'}
                </button>
                <button class="btn small ghost" onclick={() => toggleRuns(t)} aria-expanded={expandedId === t.id} aria-controls="runs-{t.id}">
                  <Icon name={expandedId === t.id ? 'chevronUp' : 'chevronDown'} size={12} /> Runs
                </button>
                <button class="icon-btn" onclick={(e) => rowMenu(e, t)} aria-label="More actions for {t.name}" title="More actions" aria-haspopup="menu">
                  <Icon name="more" size={14} />
                </button>
              </div>
            </div>
            {#if expandedId === t.id}
              <div class="runs" id="runs-{t.id}">
                <LoadState
                  what="runs"
                  variant="compact"
                  loading={!scheduledTasks.runsByTask[t.id] && !scheduledTasks.runsError[t.id]}
                  error={scheduledTasks.runsError[t.id]}
                  empty={(scheduledTasks.runsByTask[t.id] ?? []).length === 0}
                  onretry={() => void scheduledTasks.loadRuns(t.id)}
                >
                  {#snippet emptyView()}<div class="muted">No runs yet — Run now starts one.</div>{/snippet}
                {#each scheduledTasks.runsByTask[t.id] ?? [] as r (r.id)}
                  <div class="run">
                    <StatusBadge status={runStatus(r.status)} />
                    <span class="run-when"><RelTime iso={r.started_at} /></span>
                    <span class="run-sum" class:none={!r.summary} title={r.summary || undefined}>{r.summary || 'No summary'}</span>
                    {#if r.report_rel}
                      <button class="btn small" onclick={() => viewReport(r, t.name)}>View report</button>
                    {/if}
                    {#if r.session_id}
                      <button class="btn small" title="Open the agent session this run drove" onclick={() => openSession(r.session_id)}>Open session</button>
                    {/if}
                    {#if (r.attempts ?? 1) > 1}<span class="pill warn">{r.attempts} attempts</span>{/if}
                    {#if r.delivered}<span class="pill ok">Delivered</span>{/if}
                    {#if r.skipped_delivery}<span class="pill" title="Not delivered: the report is unchanged since the last run">No change</span>{/if}
                    {#if r.delivery_error}<span class="pill warn" title={r.delivery_error}>Delivery failed</span>{/if}
                    {#if r.proof_pack_id}<span class="pill ok" title="A proof pack is attached to this run">Proof</span>{/if}
                    {#if r.workflow_run_id}<span class="pill" title="Workflow run {r.workflow_run_id}">Workflow run</span>{/if}
                  </div>
                {/each}
                </LoadState>
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    </LoadState>
  {/if}

  {#if reportOpen}
    <Modal title={reportTaskName ? `Report — ${reportTaskName}` : 'Report'} width={760} onclose={() => (reportOpen = false)}>
      {#if reportLoading}
        <div class="muted" role="status">Loading the report…</div>
      {:else if reportError}
        <div class="err" role="alert">
          <Icon name="warning" size={12} /> Couldn't load the report. {reportError}
          <button class="btn small" onclick={() => reportRun && viewReport(reportRun, reportTaskName, reportRaw)}>Retry</button>
        </div>
      {:else if !reportText.trim()}
        <div class="muted">This run wrote an empty report.</div>
      {:else if reportRaw}
        <pre class="report">{reportText}</pre>
      {:else}
        <!-- renderMarkdownGfm output goes through the allowlist sanitizer. -->
        <div class="md-body report-md">{@html reportHtml}</div>
      {/if}
      {#snippet footer()}
        {#if reportText.trim() && !reportLoading && !reportError}
          <div class="segmented" role="group" aria-label="Report view">
            <button type="button" class:active={!reportRaw} aria-pressed={!reportRaw} onclick={() => (reportRaw = false)}>Formatted</button>
            <button type="button" class:active={reportRaw} aria-pressed={reportRaw} onclick={() => (reportRaw = true)}>Plain text</button>
          </div>
          <button class="btn" onclick={async () => (await copyText(reportText)) ? toasts.success('Report copied') : toasts.error("Couldn't copy the report")}>
            <Icon name="copy" size={12} /> Copy
          </button>
          <span class="grow"></span>
        {/if}
        <button class="btn" onclick={() => (reportOpen = false)}>Close</button>
      {/snippet}
    </Modal>
  {/if}
</div>
</PageBody>
</div>

<style>
  /* Colors come from the app theme tokens (tokens.css) so the page adapts to
     light + dark. Buttons reuse the global `.btn`/`.btn.small/.primary/.danger`,
     fields the global `.input` / `.checkbox-row`. */
  .sched-page { display: flex; flex-direction: column; height: 100%; min-height: 0; }
  .muted { color: var(--text-dim); padding: 8px 0; font-size: var(--fs-s); }
  .err {
    display: flex; align-items: center; gap: 6px; flex-wrap: wrap;
    background: var(--danger-soft);
    color: var(--text); padding: 8px 12px;
    border-radius: var(--radius-s); margin-block-end: 12px; font-size: var(--fs-s);
  }
  .err :global(svg) { color: var(--danger); flex-shrink: 0; }
  .notice {
    display: flex; align-items: center; gap: 8px;
    background: var(--accent-soft);
    color: var(--text); padding: 8px 12px;
    border-radius: var(--radius-s); margin-block-end: 12px; font-size: var(--fs-s);
  }
  .grow { flex: 1; }
  .tasks { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .task { border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 10px 12px; color: var(--text); }
  .task-main { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  .task-info { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
  .task-title { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .name { font-size: var(--fs-m); font-weight: 600; color: var(--text); min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .meta { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; color: var(--text-dim); font-size: var(--fs-s); min-width: 0; }
  .dest { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 36ch; }
  .cron { font-family: var(--font-mono); font-size: var(--fs-xs); padding: 0 5px; border-radius: var(--radius-s); background: var(--surface-2); color: var(--text); direction: ltr; }
  .task-actions { display: flex; align-items: center; gap: 4px; flex-shrink: 0; }
  .runs { margin-block-start: 10px; border-block-start: 1px solid var(--border); padding-block-start: 8px; display: flex; flex-direction: column; gap: 6px; }
  .run { display: flex; align-items: center; gap: 8px; font-size: var(--fs-s); flex-wrap: wrap; color: var(--text); min-height: 26px; }
  .run-when { color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .run-sum { flex: 1; min-width: 12ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .run-sum.none { color: var(--text-dim); font-style: italic; }
  .pill { font-size: var(--fs-xs); padding: 1px 7px; border-radius: 999px; border: 1px solid var(--border); color: var(--text-dim); white-space: nowrap; }
  .pill.ok { background: var(--success-soft); color: var(--success); border-color: transparent; }
  .pill.warn { background: var(--warning-soft); color: var(--warning); border-color: transparent; }
  .form { display: flex; flex-direction: column; gap: 12px; max-width: 720px; }
  .frow { display: flex; gap: 12px; flex-wrap: wrap; }
  .frow .fld { flex: 1; min-width: 180px; }
  .fld { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-s); color: var(--text); min-width: 0; }
  .fld > span { color: var(--text-dim); font-weight: 500; }
  .fld :global(.input) { width: 100%; }
  .fld :global(.mono) { font-family: var(--font-mono); }
  .fld .fld-hint { color: var(--text-dim); font-size: var(--fs-xs); display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .fld .fld-hint.bad { color: var(--danger); }
  .toggles { display: flex; flex-direction: column; gap: 6px; margin: 4px 0; }
  .adv { border-block-start: 1px solid var(--border); padding-block-start: 10px; }
  .adv-toggle {
    display: flex; align-items: center; gap: 6px; width: 100%;
    background: none; border: none; padding: 2px 0; font: inherit; font-size: var(--fs-m); font-weight: 600;
    color: var(--text); cursor: pointer; text-align: start;
  }
  .adv-hint { font-weight: 400; font-size: var(--fs-s); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .adv-body { display: flex; flex-direction: column; gap: 12px; margin-block-start: 12px; }
  .hint { font-size: var(--fs-s); color: var(--text-dim); margin: 0 0 4px; }
  .actions { display: flex; gap: 8px; justify-content: flex-end; margin-block-start: 8px; padding-block-start: 12px; border-block-start: 1px solid var(--border); }
  .report-md { color: var(--text); overflow-wrap: anywhere; }
  .report-md :global(h1), .report-md :global(h2), .report-md :global(h3) { font-size: var(--fs-m); font-weight: 600; margin: 16px 0 6px; }
  .report-md :global(h1:first-child), .report-md :global(h2:first-child), .report-md :global(h3:first-child) { margin-block-start: 0; }
  .report-md :global(ul), .report-md :global(ol) { padding-inline: 22px 0; }
  .report-md :global(table) { border-collapse: collapse; display: block; overflow-x: auto; font-size: var(--fs-s); margin: 8px 0; }
  .report-md :global(th), .report-md :global(td) { border: 1px solid var(--border); padding: 4px 8px; text-align: start; }
  .report-md :global(a) { color: var(--accent-text); }
  .report-md :global(hr) { border: none; border-block-start: 1px solid var(--border); margin: 12px 0; }
  .report { margin: 0; white-space: pre-wrap; word-break: break-word; font-family: var(--font-mono); font-size: var(--fs-s); line-height: 1.5; color: var(--text); }
  @media (max-width: 640px) {
    .task-main { flex-direction: column; align-items: stretch; gap: 8px; }
    .task-actions { justify-content: flex-end; }
    .meta { gap: 2px 10px; }
    .meta > [aria-hidden='true'] { display: none; }
    .dest { max-width: 100%; }
  }
</style>
