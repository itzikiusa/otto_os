<script lang="ts">
  import { latestOnly } from '../../lib/latest';
  import { onDestroy, tick, untrack } from 'svelte';
  import PathField from '../../lib/components/PathField.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus } from '../../lib/status';
  import { plural } from '../../lib/plural';
  import Badge from '../../lib/components/Badge.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import AutomateGuide from '../../lib/components/AutomateGuide.svelte';
  import AutomateGuideButton from '../../lib/components/AutomateGuideButton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { confirmOutward, confirmRunAndDeliver, destWho } from '../../lib/confirmOutward';
  import { cadenceLabel, runAtLocal } from '../../lib/cadence';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { scheduledTasks } from '../../lib/stores/scheduledTasks.svelte';
  import { scheduledTasksPort } from '../../lib/uiCommands/scheduled';
  import { authedText } from '../../lib/api/client';
  import { scheduledTasksApi, type ScheduledTaskInput } from '../../lib/api/scheduledTasks';
  import { router } from '../../lib/router.svelte';
  import { registry, type Command } from '../../lib/commands.svelte';
  import { rememberSelection } from '../../lib/lastSelection';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { events } from '../../lib/events.svelte';
  import { announce, focusAppContent } from '../../lib/announce.svelte';
  import { fireLabel, previewSpeech } from './firePreview';
  import type { ScheduledTask, ScheduledTaskRun } from '../../lib/api/types';
  import { allProviders, defaultAgentProvider } from '../../lib/providers';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { api } from '../../lib/api/client';
  import { loadErrorText } from '../../lib/loadError';
  import type { EmailSenderResp, Integration, Workflow } from '../../lib/api/types';
  import { copyText } from '../../lib/clipboard';
  import { convertOutcome } from './convertOutcome';

  let creating = $state(false);
  let editId = $state<string | null>(null);
  // Model catalog chunk: only the create/edit form asks for a model.
  const ModelPickerLazy = lazyComponent(() => import('../../lib/components/ModelPicker.svelte'));
  let expandedId = $state<string | null>(null);
  // Agent UI control (lib/uiCommands/scheduled.ts) expands a task's runs.
  $effect(() => scheduledTasksPort.bind({ expand: (id) => (expandedId = id) }));

  // The URL carries the task whose runs are open (`#/scheduled-tasks/<id>`), so
  // a reload / share / notification lands on it. Route → page: a link (or
  // Back/Forward) expands that task's runs and scrolls it into view; page →
  // route: expanding / collapsing rewrites the URL in place.
  $effect(() => {
    const [mod, id] = router.parts;
    if (mod !== 'scheduled-tasks' || !id) return;
    untrack(() => {
      if (id !== expandedId) void expandLinked(id);
    });
  });
  async function expandLinked(id: string): Promise<void> {
    expandedId = id;
    void scheduledTasks.loadRuns(id);
    await tick();
    document.querySelector(`[data-task-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: 'nearest' });
  }
  let routedId: string | null = null;
  $effect(() => {
    const id = expandedId;
    if (id) rememberSelection('scheduled-tasks', id);
    const was = routedId;
    routedId = id;
    if (!id && !was) return;
    untrack(() => {
      if (router.module === 'scheduled-tasks' && (router.parts[1] ?? null) !== id) router.replace(id ? `scheduled-tasks/${id}` : 'scheduled-tasks');
    });
  });

  let busy = $state(false);
  /** Daemon save failure. Field problems use `fieldError` / `intervalError`, not this banner. */
  let error = $state('');
  /** The control a submit rejected, with the message rendered under it. */
  let fieldError = $state<{ id: string; message: string } | null>(null);
  /** Set on blur or submit when the interval is under 5 minutes. Blocks save. */
  let intervalError = $state('');
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
  /** Markdown stays out of the page chunk until a formatted report is open. */
  let reportHtml = $state('');
  let reportRenderError = $state<string | null>(null);
  let reportRenderRetry = $state(0);
  $effect(() => {
    const text = reportText;
    const raw = reportRaw;
    void reportRenderRetry;
    reportHtml = '';
    reportRenderError = null;
    if (raw || !text.trim()) {
      return;
    }
    let canceled = false;
    void import('../../lib/md').then(({ renderMarkdownGfm }) => {
      if (!canceled) reportHtml = renderMarkdownGfm(text);
    }).catch((error: unknown) => {
      if (!canceled) reportRenderError = loadErrorText(error);
    });
    return () => {
      canceled = true;
    };
  });

  /** Form snapshot taken when the form opens — Back/Cancel only asks before
   *  discarding when something actually changed. */
  let formSnapshot = $state('');
  let formGeneration = 0;
  let alive = true;
  onDestroy(() => { alive = false; formGeneration++; });
  $effect(() => router.guard(() => canLeaveForm()));
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

  // Set after a successful "Convert to workflow" — surfaces a link that opens
  // the new workflow (`#/workflows/<id>`).
  let convertedWfId = $state<string | null>(null);

  /** Which delivery destinations are set up (null = unknown — offer them all).
   *  Slack/Telegram need an enabled workspace integration with a bot token
   *  (Settings → Slack, Telegram & webhooks); email needs the owner's verified sender (Settings →
   *  Sharing). Offering an unconfigured one only failed at the first run. */
  let destReady = $state<{ slack: boolean; telegram: boolean; email: boolean } | null>(null);
  async function loadDestReady(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    const [intg, sender] = await Promise.all([
      api.get<Integration[]>(`/workspaces/${wsId}/integrations`).catch(() => null),
      api.get<EmailSenderResp>('/email-sender').catch(() => null),
    ]);
    const chan = (c: string) =>
      intg ? intg.some((i) => i.channel === c && i.enabled && i.has_bot_token) : true;
    destReady = { slack: chan('slack'), telegram: chan('telegram'), email: sender ? sender.verified === true : true };
  }
  /** A destination that is known NOT to be set up (the current pick of an
   *  edited task stays selectable so its form still reads true). */
  function destBlocked(d: 'slack' | 'telegram' | 'email'): boolean {
    return destReady !== null && !destReady[d] && fDestType !== d;
  }
  $effect(() => {
    if (creating || editId) untrack(() => void loadDestReady());
  });

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
  let fCadence = $state<'interval' | 'daily' | 'weekly' | 'cron' | 'once'>('interval');
  /** `once`: the local wall-clock fire time (`YYYY-MM-DDTHH:MM`, in the task's timezone). */
  let fRunAt = $state('');
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

  // Keyed on the workspace ONLY: the loaders read store state synchronously
  // (`loadPresets` checks `presets.length`), which would otherwise subscribe
  // this effect — the presets landing then re-ran it and fired a second list
  // load that silently replaced a failed load's "Couldn’t load" + Retry.
  $effect(() => {
    const id = ws.currentId;
    if (!id) return;
    untrack(() => {
      void scheduledTasks.loadList(id);
      void scheduledTasks.loadPresets();
    });
  });

  const list = $derived(scheduledTasks.list);
  const presets = $derived(scheduledTasks.presets);

  // ⌘K: create, plus Run / Pause / Edit for the expanded task (or the only one)
  // while the form is closed. One register() owns the set, so closing the
  // form or losing the task drops the row verbs.
  $effect(() => {
    const commands: Command[] = [];
    if (ws.currentId) {
      commands.push({
        id: 'scheduled-tasks.new',
        title: 'New scheduled task…',
        group: 'Scheduled Tasks',
        keywords: 'create cron cadence report',
        run: () => { if (!creating && !editId) startCreate(); },
      });
    }
    if (!(creating || editId)) {
      const task = expandedId
        ? (list.find((t) => t.id === expandedId) ?? null)
        : (list.length === 1 ? list[0] : null);
      if (task) {
        commands.push(
          { id: 'scheduled-tasks.run', title: `Run ${task.name} once`, group: 'Scheduled Tasks', keywords: 'run now', run: () => void runNow(task) },
          { id: 'scheduled-tasks.toggle', title: task.enabled ? `Pause ${task.name}` : `Resume ${task.name}`, group: 'Scheduled Tasks', run: () => void toggle(task) },
          { id: 'scheduled-tasks.edit', title: `Edit ${task.name}…`, group: 'Scheduled Tasks', run: () => startEdit(task) },
        );
      }
    }
    return registry.register('scheduled-tasks', commands);
  });

  function resetForm(): void {
    formGeneration++;
    busy = false;
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
    fRunAt = '';
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
    fieldError = null;
    intervalError = '';
    // Fresh workflow list each time the form opens (one may have been added since).
    wfLoaded = false;
    wfError = '';
    showAdvanced = false;
  }

  function formState(): string {
    return JSON.stringify([fName, fPrompt, fSkill, fKind, fProvider, fModel, fWorkflowId, fCadence, fEveryMin, fAt, fWeekday,
      fCronExpr, fRunAt, fTimezone, fSandbox, fMaxRetries, fNotifyOnChange, fAttachProof, fDestType, fChatId, fEmailTo, fUrl, fEnabled, fCwd]);
  }

  function startCreate(): void {
    resetForm();
    creating = true;
    editId = null;
    originalDest = '';
    formSnapshot = formState();
  }

  /** Local Back/Cancel and route/workspace navigation share one decision. */
  async function canLeaveForm(): Promise<boolean> {
    if ((creating || editId) && formState() !== formSnapshot) {
      const ok = await confirmer.ask(editId ? 'Discard your changes to this task?' : 'Discard this new task?', {
        title: 'Discard changes',
        danger: true, confirmLabel: 'Discard',
      });
      if (!ok) return false;
    }
    formGeneration++;
    creating = false;
    editId = null;
    busy = false;
    error = '';
    fieldError = null;
    intervalError = '';
    return true;
  }

  async function closeForm(): Promise<void> {
    await canLeaveForm();
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

  /** A `once` schedule's `run_at` as a `datetime-local` value in `tz`: a local
   *  wall time passes through; an RFC3339 instant is shown in the timezone. */
  /** Populate the cadence form vars from a schedule object (preset or task). */
  function loadSchedule(s: Record<string, unknown>, tz?: string): void {
    const cad = (s.cadence as string) ?? 'interval';
    // `once` (the Assistant and MCP create them) used to load as an interval
    // and silently became "every 60 min" on the next save.
    fCadence = ['daily', 'weekly', 'cron', 'once'].includes(cad) ? (cad as typeof fCadence) : 'interval';
    fRunAt = typeof s.run_at === 'string' ? runAtLocal(s.run_at, tz ?? fTimezone) : '';
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
    loadSchedule(t.schedule ?? {}, t.timezone || browserTz);
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
    // Under 5 is rejected in the form (`intervalError`); never stored as 5.
    if (fCadence === 'interval') return { cadence: 'interval', every_min: Number(fEveryMin) };
    if (fCadence === 'daily') return { cadence: 'daily', at: fAt };
    if (fCadence === 'cron') return { cadence: 'cron', expr: fCronExpr.trim() };
    if (fCadence === 'once') return { cadence: 'once', run_at: fRunAt };
    return { cadence: 'weekly', at: fAt, weekday: fWeekday };
  }

  function intervalTooShort(): boolean {
    return !(Number(fEveryMin) >= 5);
  }

  function onIntervalBlur(): void {
    if (fCadence !== 'interval') {
      intervalError = '';
      return;
    }
    intervalError = intervalTooShort() ? 'The shortest interval is 5 minutes' : '';
  }

  /** A corrected value drops the error immediately, so submit is not stuck on a stale flag. */
  function onIntervalInput(): void {
    if (intervalError && !intervalTooShort()) intervalError = '';
  }

  // ── "Next fires" preview (S20-17) ─────────────────────────────────────────
  // The form asks the daemon's own cadence evaluator (the one that stamps
  // `next_run_at`) for the next fires of the UNSAVED schedule, so `0 9 * * 0`
  // typed for Monday reads "Sun …" before Save — same idea as the workflow
  // trigger preview, but live (debounced) instead of behind a button.
  let firePreview = $state<{ key: string; times: string[]; tz: string; error: string } | null>(null);
  const previewKey = $derived.by(() => {
    if (!(creating || editId)) return '';
    if (fCadence === 'interval') {
      if (intervalTooShort()) return '';
    } else if (!tzOk) return '';
    if (fCadence === 'cron' && cronFieldCount !== 5) return '';
    if (fCadence === 'once' && !fRunAt) return '';
    return JSON.stringify([buildSchedule(), fTimezone.trim() || 'UTC']);
  });
  $effect(() => {
    const key = previewKey;
    if (!key) {
      firePreview = null;
      return;
    }
    const [schedule, timezone] = JSON.parse(key) as [Record<string, unknown>, string];
    const timer = setTimeout(() => {
      scheduledTasksApi
        .preview(schedule, timezone)
        .then((r) => {
          if (alive && previewKey === key) firePreview = { key, times: r.next_fire_times, tz: timezone, error: '' };
        })
        .catch((e: unknown) => {
          if (alive && previewKey === key) {
            firePreview = { key, times: [], tz: timezone, error: loadErrorText(e) };
          }
        });
    }, 350);
    return () => clearTimeout(timer);
  });
  /** Only a preview for the CURRENT form is shown (never a stale one). */
  const shownPreview = $derived(firePreview && firePreview.key === previewKey ? firePreview : null);
  $effect(() => { if (shownPreview) announce(previewSpeech(shownPreview)); }); // the inserted line is silent

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

  async function focusField(id: string): Promise<void> {
    await tick();
    const el = document.getElementById(id);
    if (!el) return;
    el.focus();
    el.scrollIntoView({ block: 'nearest' });
  }

  function failField(id: string, message: string): void {
    fieldError = { id, message };
    void focusField(id);
  }

  async function save(): Promise<void> {
    if (busy) return;
    error = '';
    fieldError = null;
    if (!ws.currentId) return;
    if (!fName.trim()) {
      failField('sched-name', 'Give the task a name.');
      return;
    }
    if (fKind === 'workflow' && !fWorkflowId.trim()) {
      failField('sched-workflow', 'Pick the workflow this task launches.');
      return;
    }
    if (fDestType === 'email' && !fEmailTo.trim()) {
      failField('sched-email', 'Add the email address reports go to.');
      return;
    }
    if (fDestType === 'webhook' && !/^https?:\/\//i.test(fUrl.trim())) {
      failField('sched-webhook', 'Add the webhook URL (starting with http:// or https://).');
      return;
    }
    if (fCadence === 'interval' && (intervalTooShort() || intervalError)) {
      intervalError = 'The shortest interval is 5 minutes';
      void focusField('sched-every');
      return;
    }
    intervalError = '';
    if (fCadence !== 'interval' && !tzOk) {
      void focusField('sched-tz');
      return;
    }
    if (fCadence === 'cron' && cronFieldCount !== 5) {
      void focusField('sched-cron');
      return;
    }
    if (fCadence === 'once' && !fRunAt) {
      failField('sched-run-at', 'Pick the date and time it runs.');
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
    const wsId = ws.currentId;
    if (!wsId) return;
    const generation = formGeneration;
    const submittedId = editId;
    const submitted = formState();
    const destination = JSON.stringify(body.destination);
    const outward = fDestType !== 'none' && destination !== originalDest ? {
      verb: submittedId ? 'Save and deliver' : 'Create and deliver',
      title: 'Deliver reports outside Otto?',
      where: destWhere(),
      what: `The report of every run of “${body.name}” on its schedule, starting with the next run.`,
      who: destWho(fDestType),
    } : null;
    const ownsForm = () => alive && generation === formGeneration && ws.currentId === wsId;
    busy = true;
    try {
      if (outward && !await confirmOutward(outward)) return;
      if (!ownsForm()) return;
      const savedId = submittedId
        ? (await scheduledTasks.update(submittedId, body), submittedId)
        : (await scheduledTasks.create(wsId, body)).id;
      if (!ownsForm()) return;
      toasts.success(submittedId ? 'Scheduled task saved' : 'Scheduled task created', body.name);
      // Acknowledge exactly what was submitted. Keep edits made while saving,
      // and adopt a created id so the next save updates this same task.
      formSnapshot = submitted;
      originalDest = destination;
      creating = false;
      editId = formState() === submitted ? null : savedId;
    } catch (e) {
      if (ownsForm()) {
        error = `Couldn’t save the task. ${loadErrorText(e)}`;
        toastError('Couldn’t save the task', e);
      }
    } finally {
      if (ownsForm()) busy = false;
    }
  }

  async function toggle(t: ScheduledTask): Promise<void> {
    try {
      await scheduledTasks.setEnabled(t.id, !t.enabled);
    } catch (e) {
      toastError(t.enabled ? `Couldn’t pause “${t.name}”` : `Couldn’t resume “${t.name}”`, e);
    }
  }

  async function runNow(t: ScheduledTask): Promise<void> {
    // Outward-facing: a run with a delivery destination posts its report
    // right now — say where before it goes.
    if (!await confirmRunAndDeliver(t.name, t.destination)) return;
    runningIds = { ...runningIds, [t.id]: true };
    try {
      await scheduledTasks.runNow(t.id);
      expandedId = t.id;
    } catch (e) {
      toastError(`Couldn’t start “${t.name}”`, e);
    } finally {
      const { [t.id]: _done, ...rest } = runningIds;
      runningIds = rest;
    }
  }

  async function convertToWorkflow(t: ScheduledTask): Promise<void> {
    // The new workflow gets a schedule trigger on the SAME cadence. Keeping the
    // task running too doubles every run — and every delivery — so pausing it
    // is the default; keeping both is an explicit choice.
    let disableTask = false;
    if (t.enabled) {
      const { value } = await confirmer.choose(
        `“${t.name}” becomes a workflow with a schedule trigger on the same cadence. If the task keeps running too, it runs twice — and delivers twice.`,
        {
          title: 'Convert to workflow',
          options: [
            { label: 'Convert and pause the task', value: 'pause', kind: 'primary' },
            { label: 'Convert, keep both running', value: 'keep', kind: 'normal' },
          ],
        },
      );
      if (!value) return;
      disableTask = value === 'pause';
    }
    busy = true;
    try {
      const res = await scheduledTasksApi.convertToWorkflow(t.id, disableTask);
      convertedWfId = res.workflow_id;
      announce('Created a workflow from this task.');
      wfLoaded = false; // the picker re-fetches and sees the new workflow
      if (disableTask && ws.currentId) void scheduledTasks.loadList(ws.currentId);
      // A missing trigger_id means the workflow has no schedule: say so
      // instead of claiming the conversion (and the pause) succeeded.
      const out = convertOutcome(res, t.name, disableTask);
      if (out.ok) toasts.success(out.title, out.body);
      else toasts.error(out.title, out.body);
    } catch (e) {
      toastError(`Couldn’t convert “${t.name}” to a workflow`, e);
    } finally {
      busy = false;
    }
  }

  async function remove(t: ScheduledTask): Promise<void> {
    const ok = await confirmer.ask(`Delete “${t.name}”? It stops running and can’t be restored.`, {
      title: 'Delete scheduled task',
    });
    if (!ok) return;
    try {
      await scheduledTasks.remove(t.id);
      if (expandedId === t.id) expandedId = null;
      toasts.success('Scheduled task deleted', t.name);
    } catch (e) {
      toastError(`Couldn’t delete “${t.name}”`, e);
    }
  }

  /** The row's ⋯ menu: everything past Run now / Runs (guidelines: >2 row actions → a menu). */
  function rowMenu(e: MouseEvent, t: ScheduledTask): void {
    const items: MenuItem[] = [
      { label: 'Edit…', icon: 'edit', action: () => startEdit(t) },
    ];
    // Resume is the leading button while paused, so it is not repeated here.
    if (t.enabled) items.push({ label: 'Pause', icon: 'pause', action: () => void toggle(t) });
    items.push(
      { label: 'Convert to workflow', icon: 'split', disabled: busy, action: () => void convertToWorkflow(t) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void remove(t) },
    );
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
  /** Stop a running run — asks first: the agent's work so far is not reported. */
  async function stopRun(r: ScheduledTaskRun, t: ScheduledTask): Promise<void> {
    const ok = await confirmer.ask(
      `Stop this run of “${t.name}”? Its agent session (or shell command / workflow run) is stopped and nothing is delivered. The next scheduled run is unaffected.`,
      { title: 'Stop run', confirmLabel: 'Stop run', cancelLabel: 'Keep running', danger: true },
    );
    if (!ok) return;
    try {
      await scheduledTasksApi.cancelRun(r.id);
      toasts.info('Stopping the run…', t.name);
    } catch (e) {
      toastError('Couldn’t stop the run', e);
    }
    void scheduledTasks.loadRuns(t.id);
  }

  /** "Workflow run" opens THAT run — not the task's current workflow (which
   *  may have been re-pointed since). The run names its own workflow. */
  async function openWorkflowRun(runId: string, fallbackWorkflowId: string | null | undefined): Promise<void> {
    let workflowId = fallbackWorkflowId ?? null;
    try {
      // The summary projection (no node bodies / checkpoints): only the run's
      // workflow id is needed — the task may have been re-targeted since —
      // and `page.openRun` fetches the full run itself (S17-309).
      const p = await api.get<{ run?: { workflow_id?: string } | null }>(
        `/workflow-runs/${encodeURIComponent(runId)}/progress`,
      );
      workflowId = p.run?.workflow_id || workflowId;
    } catch {
      /* fall back to the task's workflow */
    }
    if (!workflowId) {
      toasts.error('Couldn’t open the workflow run', 'This run did not name a workflow.');
      return;
    }
    if (!(await router.goChecked(`workflows/${encodeURIComponent(workflowId)}`))) return;
    try {
      const { workflowsPagePort } = await import('../../lib/uiCommands/workflows');
      const page = await workflowsPagePort.get(new AbortController().signal);
      if (await page.open(workflowId)) await page.openRun(workflowId, runId);
    } catch (e) {
      toastError('Couldn’t open the workflow run', e);
    }
  }

  function openSession(sessionId: string | null | undefined): void {
    if (sessionId) ws.navigateToSession(sessionId);
  }

  const reportSeq = latestOnly();
  async function viewReport(run: ScheduledTaskRun, taskName: string, plain = false): Promise<void> {
    reportRun = run;
    reportTaskName = taskName;
    reportRaw = plain;
    reportOpen = true;
    reportLoading = true;
    reportText = '';
    reportError = '';
    // Only the report last asked for may land (A's slow text used to fill
    // the modal opened for B).
    const t = reportSeq.begin();
    try {
      const text = await authedText(scheduledTasksApi.reportPath(run.id));
      if (t.current) reportText = text;
    } catch (e) {
      if (t.current) reportError = loadErrorText(e);
    } finally {
      if (t.current) reportLoading = false;
    }
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

  function loadedRuns(taskId: string): ScheduledTaskRun[] {
    return scheduledTasks.runsByTask[taskId] ?? [];
  }

  function runIsLive(status: string): boolean {
    return status === 'running' || status === 'pending';
  }

  /** In-flight Run now, or a loaded run that has not finished. */
  let wasStale = false; // rows show "Reconnecting…" inline (silent): say it once, at the edge
  $effect(() => { const stale = events.state !== 'connected' && list.some(taskRunLive); if (stale && !wasStale) announce('Reconnecting — live run status may be stale.'); wasStale = stale; });
  function taskRunLive(t: ScheduledTask): boolean {
    if (runningIds[t.id]) return true;
    return loadedRuns(t.id).some((r) => runIsLive(r.status));
  }

  /** Newest by `started_at` (the runs API is newest-first; don't trust that here). */
  function newestLoadedRun(t: ScheduledTask): ScheduledTaskRun | null {
    let newest: ScheduledTaskRun | null = null;
    for (const r of loadedRuns(t.id)) {
      if (!newest || (r.started_at ?? '') > (newest.started_at ?? '')) newest = r;
    }
    return newest;
  }

  async function refreshTasks(): Promise<void> {
    const id = ws.currentId;
    if (!id) return;
    await scheduledTasks.loadList(id);
    const open = expandedId;
    if (open) await scheduledTasks.loadRuns(open);
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
        <Icon name="chevronLeft" size={14} />
      </button>
    {/if}
  {/snippet}
  {#snippet actions()}
    <AutomateGuideButton current="scheduled-tasks" />
    {#if !(creating || editId) && ws.currentId}
      <button type="button" class="btn small" data-overflow="-1" data-icon="refresh" data-label="Refresh" title="Refresh" aria-label="Refresh" onclick={() => void refreshTasks()}>
        <Icon name="refresh" size={12} /> Refresh
      </button>
    {/if}
    {#if !(creating || editId) && ws.currentId && list.length > 0}
      <button class="btn small primary" onclick={startCreate}><Icon name="plus" size={12} /> New task</button>
    {/if}
  {/snippet}
</PageHeader>
<PageBody width="readable">
<div class="sched">
  {#snippet addWorkspace(page: boolean)}
    <EmptyState
      variant={page ? 'page' : 'panel'}
      icon="calendar"
      title="Add a workspace"
      body="Scheduled tasks belong to a workspace. Add your project folder, then create a task."
      actionLabel="Add workspace"
      actionIcon="plus"
      onaction={() => (ui.newWorkspaceOpen = true)}
    />
  {/snippet}
  {#if creating || editId}
    {#if !ws.currentId}
      {@render addWorkspace(false)}
    {/if}
    <form class="form" onsubmit={(e) => { e.preventDefault(); void save(); }}>
      {#if error}<div class="err" role="alert"><Icon name="warning" size={12} /> {error}</div>{/if}

      {#if !editId && presets.length}
        <label class="field">
          <span>Start from a preset</span>
          <select class="input" onchange={(e) => applyPreset((e.currentTarget as HTMLSelectElement).value)}>
            <option value="">Blank task</option>
            {#each presets as p}<option value={p.id}>{p.name}</option>{/each}
          </select>
        </label>
      {/if}

      <label class="field">
        <span>Name</span>
        <input id="sched-name" dir="auto" class="input" bind:value={fName} placeholder="Nightly ticket review"
          aria-invalid={fieldError?.id === 'sched-name'}
          aria-describedby={fieldError?.id === 'sched-name' ? 'sched-name-err' : undefined} />
        {#if fieldError?.id === 'sched-name'}<small id="sched-name-err" class="field-hint bad" role="alert">{fieldError.message}</small>{/if}
      </label>

      <div class="frow">
        <label class="field">
          <span>Type</span>
          <select class="input" bind:value={fKind}>
            <option value="agent_prompt">Run an agent</option>
            <option value="workflow">Hand off to a workflow</option>
          </select>
        </label>
        {#if fKind === 'agent_prompt'}
          <label class="field">
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
          <label class="field">
            <span>Workflow</span>
            <select id="sched-workflow" class="input" bind:value={fWorkflowId} disabled={wfLoading}
              aria-invalid={fieldError?.id === 'sched-workflow'}
              aria-describedby={fieldError?.id === 'sched-workflow' ? 'sched-workflow-err' : undefined}>
              <option value="">{wfLoading ? 'Loading workflows…' : wfOptions.length ? 'Choose a workflow…' : 'No workflows in this workspace'}</option>
              {#each wfOptions as w (w.id)}<option value={w.id}>{w.name || 'Untitled workflow'}</option>{/each}
              {#if fWorkflowId && !wfLoading && !wfOptions.some((w) => w.id === fWorkflowId)}
                <option value={fWorkflowId}>Unknown workflow ({fWorkflowId.slice(0, 8)}…)</option>
              {/if}
            </select>
            {#if fieldError?.id === 'sched-workflow'}<small id="sched-workflow-err" class="field-hint bad" role="alert">{fieldError.message}</small>{/if}
            {#if wfError}
              <small class="field-hint bad">Couldn’t load workflows. {wfError}
                <button type="button" class="btn small ghost" onclick={loadWorkflowOptions}>Retry</button></small>
            {/if}
          </label>
        {/if}
      </div>

      {#if fKind === 'agent_prompt' && !PROVIDERS.includes(fProvider)}
        <label class="field">
          <span>Custom provider slug</span>
          <input dir="ltr" class="input" bind:value={fProvider} placeholder="my-custom-agent (register it in Settings first)" />
        </label>
      {/if}

      {#if fKind === 'agent_prompt'}
        <!-- Hides itself when the provider has no model-flag template (e.g. shell). -->
        {#if ModelPickerLazy.error}
          <LoadState what="models" error={ModelPickerLazy.error} onretry={() => ModelPickerLazy.retry()} />
        {:else if ModelPickerLazy.component}
          <ModelPickerLazy.component provider={fProvider} value={fModel} onchange={(m) => (fModel = m)} />
        {:else}
          <LoadState what="models" loading />
        {/if}
      {/if}

      {#if fKind === 'workflow'}
        <p class="hint">The task launches this workflow on its cadence and reports the run outcome.</p>
      {:else if fProvider === 'shell'}
        <label class="field">
          <span>Shell command</span>
          <textarea dir="ltr" class="input mono" bind:value={fPrompt} rows="4" placeholder="e.g. df -h && uptime"></textarea>
        </label>
      {:else}
        <label class="field">
          <span>Prompt (the agent’s instructions)</span>
          <textarea dir="auto" class="input" bind:value={fPrompt} rows="6" placeholder="Go over every ticket updated in the last 24h…"></textarea>
        </label>
      {/if}

      <div class="frow">
        <label class="field">
          <span>Cadence</span>
          <select class="input" bind:value={fCadence}>
            <option value="interval">Interval</option>
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
            <option value="cron">Cron</option>
            <option value="once">Once</option>
          </select>
        </label>
        {#if fCadence === 'interval'}
          <label class="field">
            <span>Every (minutes, min 5)</span>
            <input id="sched-every" class="input" type="number" bind:value={fEveryMin} onblur={onIntervalBlur} oninput={onIntervalInput}
              aria-invalid={!!intervalError}
              aria-describedby={intervalError ? 'sched-every-err' : undefined} />
            {#if intervalError}<small id="sched-every-err" class="field-hint bad" role="alert">{intervalError}</small>{/if}
          </label>
        {:else if fCadence === 'once'}
          <label class="field">
            <span>Runs once at (in the timezone)</span>
            <input id="sched-run-at" class="input" type="datetime-local" bind:value={fRunAt}
              aria-invalid={!fRunAt}
              aria-describedby={fieldError?.id === 'sched-run-at' ? 'sched-run-at-err' : undefined} />
            {#if fieldError?.id === 'sched-run-at'}<small id="sched-run-at-err" class="field-hint bad" role="alert">{fieldError.message}</small>{/if}
          </label>
        {:else if fCadence === 'cron'}
          <label class="field">
            <span>Cron expression (5 fields)</span>
            <input id="sched-cron" dir="ltr" class="input mono" bind:value={fCronExpr} placeholder="0 9 * * 1"
              aria-invalid={cronFieldCount !== 5} aria-describedby="sched-cron-hint" />
            <small id="sched-cron-hint" class="field-hint" class:bad={cronFieldCount !== 5}>
              {cronFieldCount === 5
                ? 'minute · hour · day of month · month · day of week (0 or 7 = Sun)'
                : `Needs 5 fields (minute hour day month weekday) — has ${cronFieldCount}`}
            </small>
          </label>
        {:else}
          <label class="field">
            <span>At (24h, in the timezone)</span>
            <input class="input" type="time" bind:value={fAt} />
          </label>
          {#if fCadence === 'weekly'}
            <label class="field">
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
          <label class="field">
            <span>Timezone</span>
            <input id="sched-tz" dir="ltr" class="input" bind:value={fTimezone} placeholder="e.g. Europe/London" list="sched-tz-list"
              aria-invalid={!tzOk} aria-describedby={tzOk ? undefined : 'sched-tz-err'} />
            {#if !tzOk}<small id="sched-tz-err" class="field-hint bad">Unknown timezone — use an IANA name like Europe/London</small>{/if}
            {#if tzNames.length}
              <datalist id="sched-tz-list">{#each tzNames as z (z)}<option value={z}></option>{/each}</datalist>
            {/if}
          </label>
        {/if}
      </div>
      {#if shownPreview}
        <div class="fire-preview" class:bad={!!shownPreview.error} data-testid="sched-next-fires">
          {#if shownPreview.error}
            <Icon name="warning" size={12} /> {shownPreview.error}
          {:else if shownPreview.times.length}
            <span class="fp-h">Next fires ({shownPreview.tz}):</span>
            <ul>
              {#each shownPreview.times.slice(0, 3) as at (at)}<li>{fireLabel(at, shownPreview.tz)}</li>{/each}
            </ul>
          {:else}
            <span class="fp-h">This schedule has no upcoming run.</span>
          {/if}
        </div>
      {/if}

      <div class="frow">
        <!-- A div, not a <label>: the hints and their buttons sat inside the
             label and became part of the select's accessible name. -->
        <div class="field">
          <label for="sched-dest-type">Destination</label>
          <select id="sched-dest-type" class="input" bind:value={fDestType} aria-describedby="sched-dest-hint-ch sched-dest-hint-email">
            <option value="none">None (store only)</option>
            <option value="slack" disabled={destBlocked('slack')}>Slack{destBlocked('slack') ? ' — not set up' : ''}</option>
            <option value="telegram" disabled={destBlocked('telegram')}>Telegram{destBlocked('telegram') ? ' — not set up' : ''}</option>
            <option value="email" disabled={destBlocked('email')}>Email{destBlocked('email') ? ' — not set up' : ''}</option>
            <option value="webhook">HTTP webhook</option>
          </select>
          {#if destReady && (!destReady.slack || !destReady.telegram)}
            <span id="sched-dest-hint-ch" class="field-hint" class:bad={(fDestType === 'slack' && !destReady.slack) || (fDestType === 'telegram' && !destReady.telegram)}>
              {fDestType === 'slack' && !destReady.slack ? 'Slack isn’t set up for this workspace.' : fDestType === 'telegram' && !destReady.telegram ? 'Telegram isn’t set up for this workspace.' : 'Slack / Telegram need a workspace integration.'}
              <button type="button" class="btn small ghost" onclick={() => router.go('settings/channels')}>Set up in Settings → Slack, Telegram & webhooks</button>
            </span>
          {/if}
          {#if destReady && !destReady.email}
            <span id="sched-dest-hint-email" class="field-hint" class:bad={fDestType === 'email'}>
              Email needs a verified sender.
              <button type="button" class="btn small ghost" onclick={() => router.go('settings/sharing')}>Set up in Settings → Sharing</button>
            </span>
          {/if}
        </div>
        {#if fDestType === 'slack' || fDestType === 'telegram'}
          <label class="field">
            <span>Chat / channel id (optional)</span>
            <input dir="ltr" class="input" bind:value={fChatId} placeholder="defaults to the integration channel" />
          </label>
        {:else if fDestType === 'email'}
          <label class="field">
            <span>Send to (email)</span>
            <input id="sched-email" dir="ltr" class="input" type="email" bind:value={fEmailTo} placeholder="you@example.com"
              aria-invalid={fieldError?.id === 'sched-email'}
              aria-describedby={fieldError?.id === 'sched-email' ? 'sched-email-err' : undefined} />
            {#if fieldError?.id === 'sched-email'}<small id="sched-email-err" class="field-hint bad" role="alert">{fieldError.message}</small>{/if}
          </label>
        {:else if fDestType === 'webhook'}
          <label class="field">
            <span>Webhook URL</span>
            <input id="sched-webhook" dir="ltr" class="input" type="url" bind:value={fUrl} placeholder="https://…"
              aria-invalid={fieldError?.id === 'sched-webhook'}
              aria-describedby={fieldError?.id === 'sched-webhook' ? 'sched-webhook-err' : undefined} />
            {#if fieldError?.id === 'sched-webhook'}<small id="sched-webhook-err" class="field-hint bad" role="alert">{fieldError.message}</small>{/if}
          </label>
        {/if}
      </div>

      {#if fProvider === 'shell' && fKind === 'agent_prompt'}
        <label class="field">
          <span>Working dir (optional)</span>
          <PathField bind:value={fCwd}><input dir="ltr" class="input" bind:value={fCwd} placeholder="dir to run the command in" /></PathField>
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
          <label class="field">
            <span>Skill (optional, inlined)</span>
            <input dir="auto" class="input" bind:value={fSkill} placeholder="e.g. db-mysql" />
          </label>
          <label class="field">
            <span>Working dir (optional)</span>
            <PathField bind:value={fCwd}><input dir="ltr" class="input" bind:value={fCwd} placeholder="repo path — not a sandbox" /></PathField>
          </label>
        </div>

        <div class="frow">
          <label class="field">
            <span>Sandbox</span>
            <select class="input" bind:value={fSandbox}>
              <option value="none">Run in working dir</option>
              <option value="worktree">Isolated git worktree</option>
            </select>
          </label>
          <label class="field">
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
        <button type="submit" class="btn primary" disabled={busy || !ws.currentId}>
          {busy ? 'Saving…' : editId ? 'Save changes' : 'Create task'}
        </button>
      </div>
    </form>
  {:else if !ws.currentId}
    {@render addWorkspace(true)}
  {:else}
    {#if convertedWfId}
      <div class="notice">
        <span>Created a workflow from this task.</span>
        <span class="grow"></span>
        <button class="btn small" onclick={() => { const id = convertedWfId; convertedWfId = null; router.go(id ? `workflows/${id}` : 'workflows'); }}>Open workflow</button>
        <button class="btn small" onclick={() => { convertedWfId = null; void focusAppContent(); }}>Dismiss</button>
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
        >
          <AutomateGuide current="scheduled-tasks" />
        </EmptyState>
      {/snippet}
      <ul class="tasks">
        {#each list as t (t.id)}
          {@const live = taskRunLive(t)}
          {@const newest = newestLoadedRun(t)}
          <li class="task" data-task-id={t.id}>
            <div class="task-main">
              <div class="task-info">
                <div class="task-title">
                  <strong class="name" title={t.name}>{t.name}</strong>
                  {#if !t.enabled}
                    <StatusBadge tone="neutral" label="Paused" />
                  {/if}
                  {#if live}
                    <StatusBadge status={runStatus('running')} />
                  {:else}
                    {#if t.last_status}
                      <!-- Shared run vocabulary (lib/status.ts): ok → Succeeded, error → Failed.
                           A paused task still shows its last run. -->
                      <StatusBadge status={runStatus(t.last_status)} />
                    {/if}
                    {#if newest?.delivery_error}
                      <StatusBadge tone="warning" label="Not delivered" title={newest.delivery_error} />
                    {/if}
                  {/if}
                </div>
                <div class="meta">
                  <span>{cadenceLabel(t.schedule, t.timezone, { cronExpr: false })}</span>
                  {#if (t.schedule?.cadence as string) === 'cron'}<code class="cron">{t.schedule?.expr as string}</code><span>{t.timezone || 'UTC'}</span>{/if}
                  <span aria-hidden="true">·</span>
                  <span class="dest" title={destTitle(t)}>{destLabel(t)}</span>
                  <!-- When it fires next (ticks via RelTime; exact time on hover). -->
                  {#if t.enabled && t.next_run_at}<span aria-hidden="true">·</span><span>Next <RelTime iso={t.next_run_at} /></span>{/if}
                </div>
              </div>
              <div class="task-actions">
                {#if !t.enabled}
                  <button class="btn small" onclick={() => void toggle(t)} title="Resume this schedule">
                    <Icon name="play" size={12} /> Resume
                  </button>
                {/if}
                <button class="btn small" onclick={() => runNow(t)} disabled={!!runningIds[t.id]}
                  title={destLabel(t) === 'Report kept in Otto' ? 'Run once now' : `Run once now — ${destLabel(t).toLowerCase()}`}>
                  <Icon name="play" size={12} /> {runningIds[t.id] ? 'Starting…' : t.enabled ? 'Run now' : 'Run once'}
                </button>
                <button class="btn small ghost" onclick={() => toggleRuns(t)} aria-expanded={expandedId === t.id} aria-controls="runs-{t.id}">
                  <Icon name={expandedId === t.id ? 'chevronUp' : 'chevronDown'} size={12} /> Runs
                </button>
                <button class="icon-btn" onclick={(e) => rowMenu(e, t)} aria-label="More actions for {t.name}" title="More actions" aria-haspopup="menu">
                  <Icon name="more" size={14} />
                </button>
              </div>
            </div>
            {#if live && events.state !== 'connected'}
              <p class="reconnect">Reconnecting — this run’s status may be stale.</p>
            {/if}
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
                    {#if r.status === 'running'}
                      <button class="btn small danger" title="Stop this run (its agent session, shell command or workflow run is stopped)" onclick={() => void stopRun(r, t)}>Stop…</button>
                    {/if}
                    {#if (r.attempts ?? 1) > 1}<Badge tone="warn" label={plural(r.attempts ?? 1, 'attempt')} />{/if}
                    {#if r.delivered}<Badge tone="ok" label="Delivered" />{/if}
                    {#if r.skipped_delivery}<Badge label="No change" title="Not delivered: the report is unchanged since the last run" />{/if}
                    {#if r.delivery_error}<Badge tone="warn" label="Delivery failed" title={r.delivery_error ?? undefined} />{/if}
                    {#if r.proof_pack_id}
                      <button class="btn small" title="Open the proof pack for this run" onclick={() => router.go(`proof/${encodeURIComponent(r.proof_pack_id ?? '')}`)}>Proof</button>
                    {/if}
                    {#if r.workflow_run_id}
                      <button class="btn small" title={`Open the workflow this run launched (run ${r.workflow_run_id})`} onclick={() => void openWorkflowRun(r.workflow_run_id!, t.workflow_id)}>Workflow run</button>
                    {/if}
                    <!-- Why it failed / why it wasn't delivered, readable without
                         hovering (a failed run used to say only "No summary"). -->
                    {#if r.status === 'error' && r.error}
                      <p class="run-err" role="note"><Icon name="warning" size={12} /> {r.error}</p>
                    {:else if r.status === 'skipped' && r.error}
                      <!-- Why it was skipped (e.g. the workflow was still busy) —
                           neutral, not a failure (S3-306). -->
                      <p class="run-err note" role="note">{r.error}</p>
                    {/if}
                    {#if r.delivery_error}
                      <p class="run-err warn" role="note">Not delivered: {r.delivery_error}</p>
                    {/if}
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
      {#if reportLoading || reportError}
        <LoadState
          what="the report"
          loading={reportLoading}
          error={reportError}
          empty
          onretry={() => reportRun && viewReport(reportRun, reportTaskName, reportRaw)}
        />
      {:else if !reportText.trim()}
        <div class="muted">This run wrote an empty report.</div>
      {:else if reportRaw}
        <pre class="report">{reportText}</pre>
      {:else if reportRenderError}
        <LoadState what="formatted report" error={reportRenderError} empty onretry={() => reportRenderRetry++} />
      {:else if !reportHtml}
        <LoadState what="formatted report" loading empty />
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
          <button class="btn" onclick={async () => (await copyText(reportText)) ? toasts.success('Report copied') : toasts.error('Couldn’t copy the report', 'Select the text and copy it manually.')}>
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
  .cron { font-family: var(--font-mono); font-size: var(--fs-xs); padding: 0 4px; border-radius: var(--radius-s); background: var(--surface-2); color: var(--text); direction: ltr; }
  .task-actions { display: flex; align-items: center; gap: 4px; flex-shrink: 0; }
  .runs { margin-block-start: 10px; border-block-start: 1px solid var(--border); padding-block-start: 8px; display: flex; flex-direction: column; gap: 6px; }
  .run { display: flex; align-items: center; gap: 8px; font-size: var(--fs-s); flex-wrap: wrap; color: var(--text); min-height: 26px; }
  .run-when { color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .run-sum { flex: 1; min-width: 12ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .run-sum.none { color: var(--text-dim); font-style: italic; }
  .run-err { flex-basis: 100%; margin: 0; padding-inline-start: 4px; font-size: var(--fs-xs); color: var(--danger); overflow-wrap: anywhere; display: flex; gap: 4px; align-items: baseline; }
  .run-err.warn { color: var(--warning); }
  .run-err.note { color: var(--text-dim); }
  .form { display: flex; flex-direction: column; gap: 12px; max-width: 720px; }
  .frow { display: flex; gap: 12px; flex-wrap: wrap; }
  /* Shared .field (app.css); the form's gap spaces the rows, so no bottom margin. */
  .frow .field { flex: 1; min-width: 180px; }
  .field { margin-bottom: 0; font-size: var(--fs-s); color: var(--text); min-width: 0; }
  .field > span, .field > label { color: var(--text-dim); font-weight: 500; }
  .field :global(.input) { width: 100%; }
  .field :global(.mono) { font-family: var(--font-mono); }
  .field .field-hint { color: var(--text-dim); font-size: var(--fs-xs); display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .field .field-hint.bad { color: var(--danger); }
  .fire-preview { display: flex; align-items: baseline; gap: 6px; flex-wrap: wrap; font-size: var(--fs-xs); color: var(--text-dim); }
  .fire-preview.bad { color: var(--danger); align-items: center; }
  .fire-preview .fp-h { font-weight: 600; }
  .fire-preview ul { display: flex; flex-wrap: wrap; gap: 4px 12px; margin: 0; padding: 0; list-style: none; }
  .fire-preview li { font-variant-numeric: tabular-nums; }
  .toggles { display: flex; flex-direction: column; gap: 6px; margin: 4px 0; }
  .adv { border-block-start: 1px solid var(--border); padding-block-start: 10px; }
  .adv-toggle {
    display: flex; align-items: center; gap: 6px; width: 100%;
    background: none; border: none; padding: 2px 0; font: inherit; font-size: var(--fs-m); font-weight: 600;
    color: var(--text); cursor: pointer; text-align: start;
  }
  .adv-hint { flex: 1; min-width: 0; font-weight: 400; font-size: var(--fs-s); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .reconnect { margin: 8px 0 0; font-size: var(--fs-s); color: var(--warning); }
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
