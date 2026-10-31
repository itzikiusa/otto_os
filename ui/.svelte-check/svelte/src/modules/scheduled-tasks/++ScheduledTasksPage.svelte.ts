///<reference types="svelte" />
;
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
import { announce, focusAppContent, announceShown } from '../../lib/announce.svelte';
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
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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

;
async () => {

 { svelteHTML.createElement("div", { "class":`sched-page`,});
  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":creating || editId ? (editId ? `Edit “${list.find((t) => t.id === editId)?.name ?? 'task'}”` : 'New scheduled task') : 'Scheduled Tasks',"subtitle":creating || editId ? undefined : 'Run an agent on a cadence and deliver its report',leading:() => { async ()/*Ωignore_positionΩ*/ => {
    if(creating || editId){
       { svelteHTML.createElement("button", {         "class":`icon-btn`,"title":`Back to Scheduled Tasks`,"aria-label":`Back to Scheduled Tasks`,"disabled":busy,"onclick":closeForm,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":14,}});}
       }
    }
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
     { const $$_nottuBediuGetamotuA0C = __sveltets_2_ensureComponent(AutomateGuideButton); new $$_nottuBediuGetamotuA0C({ target: __sveltets_2_any(), props: {  "current":`scheduled-tasks`,}});}
    if(!(creating || editId) && ws.currentId){
       { svelteHTML.createElement("button", {               "type":`button`,"class":`btn small`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`refresh`}),...__sveltets_2_empty({"data-label":`Refresh`}),"title":`Refresh`,"aria-label":`Refresh`,"onclick":() => void refreshTasks(),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} 
       }
    }
    if(!(creating || editId) && ws.currentId && list.length > 0){
       { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":startCreate,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
 { svelteHTML.createElement("div", { "class":`sched`,});
   const addWorkspace/*Ωignore_positionΩ*/ = (page: boolean)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
     { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {               "variant":page ? 'page' : 'panel',"icon":`calendar`,"title":`Add a workspace`,"body":`Scheduled tasks belong to a workspace. Add your project folder, then create a task.`,"actionLabel":`Add workspace`,"actionIcon":`plus`,"onaction":() => (ui.newWorkspaceOpen = true),}});}
  };return __sveltets_2_any(0)};
  if(creating || editId){
    if(!ws.currentId){
      ;__sveltets_2_ensureSnippet(addWorkspace(false));
    }
     { svelteHTML.createElement("form", {   "class":`form`,"onsubmit":(e) => { e.preventDefault(); void save(); },});
      if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: error, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`err`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} error; }}}

      if(!editId && presets.length){
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});    }
           { svelteHTML.createElement("select", {   "class":`input`,"onchange":(e) => applyPreset((e.currentTarget as HTMLSelectElement).value),});
             { svelteHTML.createElement("option", {"value":"",});  }
              for(let p of __sveltets_2_ensureArray(presets)){ { svelteHTML.createElement("option", { "value":p.id,});p.name; }}
           }
         }
      }

       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("input", {              "id":`sched-name`,"dir":`auto`,"class":`input`,"bind:value":fName,"placeholder":`Nightly ticket review`,"aria-invalid":fieldError?.id === 'sched-name',"aria-describedby":fieldError?.id === 'sched-name' ? 'sched-name-err' : undefined,});/*Ωignore_startΩ*/() => fName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(fieldError?.id === 'sched-name'){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('small'),({ text: fieldError.message, assertive: true })));{ svelteHTML.createElement("small", __sveltets_2_union($$action_0), {     "id":`sched-name-err`,"class":`field-hint bad`,});fieldError.message; }}}
       }

       { svelteHTML.createElement("div", { "class":`frow`,});
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", {   "class":`input`,"bind:value":fKind,});/*Ωignore_startΩ*/() => fKind = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":`agent_prompt`,});   }
             { svelteHTML.createElement("option", { "value":`workflow`,});     }
           }
         }
        if(fKind === 'agent_prompt'){
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("select", {       "class":`input`,"value":PROVIDERS.includes(fProvider) ? fProvider : 'custom',"onchange":(e) => onProviderSelect((e.currentTarget as HTMLSelectElement).value),});
                for(let p of __sveltets_2_ensureArray(PROVIDERS)){ { svelteHTML.createElement("option", { "value":p,});p; }}
               { svelteHTML.createElement("option", { "value":`custom`,});  }
             }
           }
        }else{
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("select", {           "id":`sched-workflow`,"class":`input`,"bind:value":fWorkflowId,"disabled":wfLoading,"aria-invalid":fieldError?.id === 'sched-workflow',"aria-describedby":fieldError?.id === 'sched-workflow' ? 'sched-workflow-err' : undefined,});/*Ωignore_startΩ*/() => fWorkflowId = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", {"value":"",});wfLoading ? 'Loading workflows…' : wfOptions.length ? 'Choose a workflow…' : 'No workflows in this workspace'; }
                 for(let w of __sveltets_2_ensureArray(wfOptions)){w.id; { svelteHTML.createElement("option", { "value":w.id,});w.name || 'Untitled workflow'; }}
              if(fWorkflowId && !wfLoading && !wfOptions.some((w) => w.id === fWorkflowId)){
                 { svelteHTML.createElement("option", { "value":fWorkflowId,});  fWorkflowId.slice(0, 8);  }
              }
             }
            if(fieldError?.id === 'sched-workflow'){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('small'),({ text: fieldError.message, assertive: true })));{ svelteHTML.createElement("small", __sveltets_2_union($$action_0), {     "id":`sched-workflow-err`,"class":`field-hint bad`,});fieldError.message; }}}
            if(wfError){
               { svelteHTML.createElement("small", { "class":`field-hint bad`,});   wfError;
                 { svelteHTML.createElement("button", {     "type":`button`,"class":`btn small ghost`,"onclick":loadWorkflowOptions,});  } }
            }
           }
        }
       }

      if(fKind === 'agent_prompt' && !PROVIDERS.includes(fProvider)){
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});   }
           { svelteHTML.createElement("input", {        "dir":`ltr`,"class":`input`,"bind:value":fProvider,"placeholder":`my-custom-agent (register it in Settings first)`,});/*Ωignore_startΩ*/() => fProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
      }

      if(fKind === 'agent_prompt'){
        
        if(ModelPickerLazy.error){
           { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {      "what":`models`,"error":ModelPickerLazy.error,"onretry":() => ModelPickerLazy.retry(),}});}
        } else if (ModelPickerLazy.component){
           { const $$_tnenopmoc_yzaLrekciPledoM4C = __sveltets_2_ensureComponent(ModelPickerLazy.component); new $$_tnenopmoc_yzaLrekciPledoM4C({ target: __sveltets_2_any(), props: {      "provider":fProvider,"value":fModel,"onchange":(m) => (fModel = m),}});}
        }else{
            { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {  "what":`models`,"loading":true,}});}
        }
      }

      if(fKind === 'workflow'){
         { svelteHTML.createElement("p", { "class":`hint`,});             }
      } else if (fProvider === 'shell'){
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("textarea", {         "dir":`ltr`,"class":`input mono`,"bind:value":fPrompt,"rows":4,"placeholder":`e.g. df -h && uptime`,});/*Ωignore_startΩ*/() => fPrompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         }
      }else{
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});    }
           { svelteHTML.createElement("textarea", {         "dir":`auto`,"class":`input`,"bind:value":fPrompt,"rows":6,"placeholder":`Go over every ticket updated in the last 24h…`,});/*Ωignore_startΩ*/() => fPrompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         }
      }

       { svelteHTML.createElement("div", { "class":`frow`,});
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", {   "class":`input`,"bind:value":fCadence,});/*Ωignore_startΩ*/() => fCadence = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":`interval`,});  }
             { svelteHTML.createElement("option", { "value":`daily`,});  }
             { svelteHTML.createElement("option", { "value":`weekly`,});  }
             { svelteHTML.createElement("option", { "value":`cron`,});  }
             { svelteHTML.createElement("option", { "value":`once`,});  }
           }
         }
        if(fCadence === 'interval'){
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});    }
             { svelteHTML.createElement("input", {                "id":`sched-every`,"class":`input`,"type":`number`,"bind:value":fEveryMin,"onblur":onIntervalBlur,"oninput":onIntervalInput,"aria-invalid":!!intervalError,"aria-describedby":intervalError ? 'sched-every-err' : undefined,});/*Ωignore_startΩ*/() => fEveryMin = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            if(intervalError){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('small'),({ text: intervalError, assertive: true })));{ svelteHTML.createElement("small", __sveltets_2_union($$action_0), {     "id":`sched-every-err`,"class":`field-hint bad`,});intervalError; }}}
           }
        } else if (fCadence === 'once'){
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});      }
             { svelteHTML.createElement("input", {            "id":`sched-run-at`,"class":`input`,"type":`datetime-local`,"bind:value":fRunAt,"aria-invalid":!fRunAt,"aria-describedby":fieldError?.id === 'sched-run-at' ? 'sched-run-at-err' : undefined,});/*Ωignore_startΩ*/() => fRunAt = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            if(fieldError?.id === 'sched-run-at'){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('small'),({ text: fieldError.message, assertive: true })));{ svelteHTML.createElement("small", __sveltets_2_union($$action_0), {     "id":`sched-run-at-err`,"class":`field-hint bad`,});fieldError.message; }}}
           }
        } else if (fCadence === 'cron'){
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});    }
             { svelteHTML.createElement("input", {              "id":`sched-cron`,"dir":`ltr`,"class":`input mono`,"bind:value":fCronExpr,"placeholder":`0 9 * * 1`,"aria-invalid":cronFieldCount !== 5,"aria-describedby":`sched-cron-hint`,});/*Ωignore_startΩ*/() => fCronExpr = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("small", {    "id":`sched-cron-hint`,"class":`field-hint`,});cronFieldCount !== 5;
              cronFieldCount === 5
                ? 'minute · hour · day of month · month · day of week (0 or 7 = Sun)'
                : `Needs 5 fields (minute hour day month weekday) — has ${cronFieldCount}`;
             }
           }
        }else{
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});     }
             { svelteHTML.createElement("input", {      "class":`input`,"type":`time`,"bind:value":fAt,});/*Ωignore_startΩ*/() => fAt = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
          if(fCadence === 'weekly'){
             { svelteHTML.createElement("label", { "class":`field`,});
               { svelteHTML.createElement("span", {});  }
               { svelteHTML.createElement("select", {   "class":`input`,"bind:value":fWeekday,});/*Ωignore_startΩ*/() => fWeekday = __sveltets_2_any(null);/*Ωignore_endΩ*/
                   for(let d of __sveltets_2_ensureArray((['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday']))){let i = 1;
                   { svelteHTML.createElement("option", { "value":i,});d; }
                }
               }
             }
          }
        }
        if(fCadence !== 'interval'){
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("input", {                "id":`sched-tz`,"dir":`ltr`,"class":`input`,"bind:value":fTimezone,"placeholder":`e.g. Europe/London`,"list":`sched-tz-list`,"aria-invalid":!tzOk,"aria-describedby":tzOk ? undefined : 'sched-tz-err',});/*Ωignore_startΩ*/() => fTimezone = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            if(!tzOk){ { svelteHTML.createElement("small", {   "id":`sched-tz-err`,"class":`field-hint bad`,});         }}
            if(tzNames.length){
               { svelteHTML.createElement("datalist", { "id":`sched-tz-list`,});   for(let z of __sveltets_2_ensureArray(tzNames)){z; { svelteHTML.createElement("option", { "value":z,}); }} }
            }
           }
        }
       }
      if(shownPreview){
         { svelteHTML.createElement("div", {    "class":`fire-preview`,...__sveltets_2_empty({"data-testid":`sched-next-fires`}),});!!shownPreview.error;
          if(shownPreview.error){
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} shownPreview.error;
          } else if (shownPreview.times.length){
             { svelteHTML.createElement("span", { "class":`fp-h`,});  shownPreview.tz;  }
             { svelteHTML.createElement("ul", {});
                 for(let at of __sveltets_2_ensureArray((shownPreview.times.slice(0, 3)))){at; { svelteHTML.createElement("li", {});fireLabel(at, shownPreview.tz); }}
             }
          }else{
             { svelteHTML.createElement("span", { "class":`fp-h`,});      }
          }
         }
      }

       { svelteHTML.createElement("div", { "class":`frow`,});
        
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`sched-dest-type`,});  }
           { svelteHTML.createElement("select", {       "id":`sched-dest-type`,"class":`input`,"bind:value":fDestType,"aria-describedby":`sched-dest-hint-ch sched-dest-hint-email`,});/*Ωignore_startΩ*/() => fDestType = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":`none`,});   }
             { svelteHTML.createElement("option", {   "value":`slack`,"disabled":destBlocked('slack'),}); destBlocked('slack') ? ' — not set up' : ''; }
             { svelteHTML.createElement("option", {   "value":`telegram`,"disabled":destBlocked('telegram'),}); destBlocked('telegram') ? ' — not set up' : ''; }
             { svelteHTML.createElement("option", {   "value":`email`,"disabled":destBlocked('email'),}); destBlocked('email') ? ' — not set up' : ''; }
             { svelteHTML.createElement("option", { "value":`webhook`,});  }
           }
          if(destReady && (!destReady.slack || !destReady.telegram)){
             { svelteHTML.createElement("span", {    "id":`sched-dest-hint-ch`,"class":`field-hint`,});(fDestType === 'slack' && !destReady.slack) || (fDestType === 'telegram' && !destReady.telegram);
              fDestType === 'slack' && !destReady.slack ? 'Slack isn’t set up for this workspace.' : fDestType === 'telegram' && !destReady.telegram ? 'Telegram isn’t set up for this workspace.' : 'Slack / Telegram need a workspace integration.';
               { svelteHTML.createElement("button", {     "type":`button`,"class":`btn small ghost`,"onclick":() => router.go('settings/channels'),});         }
             }
          }
          if(destReady && !destReady.email){
             { svelteHTML.createElement("span", {    "id":`sched-dest-hint-email`,"class":`field-hint`,});fDestType === 'email';
                  
               { svelteHTML.createElement("button", {     "type":`button`,"class":`btn small ghost`,"onclick":() => router.go('settings/sharing'),});      }
             }
          }
         }
        if(fDestType === 'slack' || fDestType === 'telegram'){
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});     }
             { svelteHTML.createElement("input", {        "dir":`ltr`,"class":`input`,"bind:value":fChatId,"placeholder":`defaults to the integration channel`,});/*Ωignore_startΩ*/() => fChatId = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
        } else if (fDestType === 'email'){
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});   }
             { svelteHTML.createElement("input", {                "id":`sched-email`,"dir":`ltr`,"class":`input`,"type":`email`,"bind:value":fEmailTo,"placeholder":`you@example.com`,"aria-invalid":fieldError?.id === 'sched-email',"aria-describedby":fieldError?.id === 'sched-email' ? 'sched-email-err' : undefined,});/*Ωignore_startΩ*/() => fEmailTo = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            if(fieldError?.id === 'sched-email'){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('small'),({ text: fieldError.message, assertive: true })));{ svelteHTML.createElement("small", __sveltets_2_union($$action_0), {     "id":`sched-email-err`,"class":`field-hint bad`,});fieldError.message; }}}
           }
        } else if (fDestType === 'webhook'){
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("input", {                "id":`sched-webhook`,"dir":`ltr`,"class":`input`,"type":`url`,"bind:value":fUrl,"placeholder":`https://…`,"aria-invalid":fieldError?.id === 'sched-webhook',"aria-describedby":fieldError?.id === 'sched-webhook' ? 'sched-webhook-err' : undefined,});/*Ωignore_startΩ*/() => fUrl = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            if(fieldError?.id === 'sched-webhook'){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('small'),({ text: fieldError.message, assertive: true })));{ svelteHTML.createElement("small", __sveltets_2_union($$action_0), {     "id":`sched-webhook-err`,"class":`field-hint bad`,});fieldError.message; }}}
           }
        }
       }

      if(fProvider === 'shell' && fKind === 'agent_prompt'){
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});   }
           { const $$_dleiFhtaP5C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP5 = new $$_dleiFhtaP5C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },value:fCwd,}});/*Ωignore_startΩ*/() => fCwd = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP5.$$bindings = 'value'; { svelteHTML.createElement("input", {        "dir":`ltr`,"class":`input`,"bind:value":fCwd,"placeholder":`dir to run the command in`,});/*Ωignore_startΩ*/() => fCwd = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
         }
      }

       { svelteHTML.createElement("label", { "class":`checkbox-row`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":fEnabled,});/*Ωignore_startΩ*/() => fEnabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}        }

       { svelteHTML.createElement("div", { "class":`adv`,});
         { svelteHTML.createElement("button", {         "type":`button`,"class":`adv-toggle`,"aria-expanded":showAdvanced,"aria-controls":`sched-advanced`,"onclick":() => (showAdvanced = !showAdvanced),});
           { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":showAdvanced ? 'chevronDown' : 'chevronRight',"size":12,}});} 
           { svelteHTML.createElement("span", { "class":`adv-hint`,});fKind === 'agent_prompt' && fProvider !== 'shell' ? 'skill, working dir, sandbox, retries, delivery rules' : 'delivery rules'; }
         }
      if(showAdvanced){
       { svelteHTML.createElement("div", {   "class":`adv-body`,"id":`sched-advanced`,});
      if(fKind === 'agent_prompt' && fProvider !== 'shell'){
         { svelteHTML.createElement("div", { "class":`frow`,});
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});   }
             { svelteHTML.createElement("input", {        "dir":`auto`,"class":`input`,"bind:value":fSkill,"placeholder":`e.g. db-mysql`,});/*Ωignore_startΩ*/() => fSkill = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});   }
             { const $$_dleiFhtaP8C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP8 = new $$_dleiFhtaP8C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },value:fCwd,}});/*Ωignore_startΩ*/() => fCwd = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP8.$$bindings = 'value'; { svelteHTML.createElement("input", {        "dir":`ltr`,"class":`input`,"bind:value":fCwd,"placeholder":`repo path — not a sandbox`,});/*Ωignore_startΩ*/() => fCwd = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
           }
         }

         { svelteHTML.createElement("div", { "class":`frow`,});
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("select", {   "class":`input`,"bind:value":fSandbox,});/*Ωignore_startΩ*/() => fSandbox = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", { "value":`none`,});    }
               { svelteHTML.createElement("option", { "value":`worktree`,});   }
             }
           }
           { svelteHTML.createElement("label", { "class":`field`,});
             { svelteHTML.createElement("span", {});    }
             { svelteHTML.createElement("input", {          "class":`input`,"type":`number`,"min":`0`,"max":`5`,"bind:value":fMaxRetries,});/*Ωignore_startΩ*/() => fMaxRetries = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
         }
      }

       { svelteHTML.createElement("div", { "class":`toggles`,});
         { svelteHTML.createElement("label", { "class":`checkbox-row`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":fNotifyOnChange,});/*Ωignore_startΩ*/() => fNotifyOnChange = __sveltets_2_any(null);/*Ωignore_endΩ*/}        }
         { svelteHTML.createElement("label", { "class":`checkbox-row`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":fAttachProof,});/*Ωignore_startΩ*/() => fAttachProof = __sveltets_2_any(null);/*Ωignore_endΩ*/}        }
       }
       }
      }
       }

       { svelteHTML.createElement("div", { "class":`actions`,});
         { svelteHTML.createElement("button", {       "type":`button`,"class":`btn`,"disabled":busy,"onclick":closeForm,});  }
         { svelteHTML.createElement("button", {     "type":`submit`,"class":`btn primary`,"disabled":busy || !ws.currentId,});
          busy ? 'Saving…' : editId ? 'Save changes' : 'Create task';
         }
       }
     }
  } else if (!ws.currentId){
    ;__sveltets_2_ensureSnippet(addWorkspace(true));
  }else{
    if(convertedWfId){
       { svelteHTML.createElement("div", { "class":`notice`,});
         { svelteHTML.createElement("span", {});      }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { const id = convertedWfId; convertedWfId = null; router.go(id ? `workflows/${id}` : 'workflows'); },});  }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => { convertedWfId = null; void focusAppContent(); },});  }
       }
    }

     { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL3 = new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`scheduled tasks`,"variant":`page`,"loading":scheduledTasks.loadingList,"error":scheduledTasks.listError,"empty":list.length === 0,"onretry":() => ws.currentId && void scheduledTasks.loadList(ws.currentId),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`calendar`,"title":`No scheduled tasks yet`,"body":`Create one to run an agent on a cadence and deliver its report.`,"actionLabel":`New task`,"actionIcon":`plus`,"onaction":startCreate,}});
           { const $$_ediuGetamotuA1C = __sveltets_2_ensureComponent(AutomateGuide); new $$_ediuGetamotuA1C({ target: __sveltets_2_any(), props: {  "current":`scheduled-tasks`,}});}
         EmptyState}
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL3.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("ul", { "class":`tasks`,});
           for(let t of __sveltets_2_ensureArray(list)){t.id;
          const live = taskRunLive(t);
          const newest = newestLoadedRun(t);
           { svelteHTML.createElement("li", {   "class":`task`,...__sveltets_2_empty({"data-task-id":t.id}),});
             { svelteHTML.createElement("div", { "class":`task-main`,});
               { svelteHTML.createElement("div", { "class":`task-info`,});
                 { svelteHTML.createElement("div", { "class":`task-title`,});
                   { svelteHTML.createElement("strong", {   "class":`name`,"title":t.name,});t.name; }
                  if(!t.enabled){
                     { const $$_egdaBsutatS9C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS9C({ target: __sveltets_2_any(), props: {    "tone":`neutral`,"label":`Paused`,}});}
                  }
                  if(live){
                     { const $$_egdaBsutatS9C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS9C({ target: __sveltets_2_any(), props: {  "status":runStatus('running'),}});}
                  }else{
                    if(t.last_status){
                      
                       { const $$_egdaBsutatS9C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS9C({ target: __sveltets_2_any(), props: {  "status":runStatus(t.last_status),}});}
                    }
                    if(newest?.delivery_error){
                       { const $$_egdaBsutatS9C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS9C({ target: __sveltets_2_any(), props: {      "tone":`warning`,"label":`Not delivered`,"title":newest.delivery_error,}});}
                    }
                  }
                 }
                 { svelteHTML.createElement("div", { "class":`meta`,});
                   { svelteHTML.createElement("span", {});cadenceLabel(t.schedule, t.timezone, { cronExpr: false }); }
                  if((t.schedule?.cadence as string) === 'cron'){ { svelteHTML.createElement("code", { "class":`cron`,});t.schedule?.expr as string; } { svelteHTML.createElement("span", {});t.timezone || 'UTC'; }}
                   { svelteHTML.createElement("span", { "aria-hidden":`true`,});  }
                   { svelteHTML.createElement("span", {   "class":`dest`,"title":destTitle(t),});destLabel(t); }
                  
                  if(t.enabled && t.next_run_at){ { svelteHTML.createElement("span", { "aria-hidden":`true`,});  } { svelteHTML.createElement("span", {});  { const $$_emiTleR10C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR10C({ target: __sveltets_2_any(), props: {  "iso":t.next_run_at,}});} }}
                 }
               }
               { svelteHTML.createElement("div", { "class":`task-actions`,});
                if(!t.enabled){
                   { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void toggle(t),"title":`Resume this schedule`,});
                     { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} 
                   }
                }
                 { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => runNow(t),"disabled":!!runningIds[t.id],"title":destLabel(t) === 'Report kept in Otto' ? 'Run once now' : `Run once now — ${destLabel(t).toLowerCase()}`,});
                   { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} runningIds[t.id] ? 'Starting…' : t.enabled ? 'Run now' : 'Run once';
                 }
                 { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":() => toggleRuns(t),"aria-expanded":expandedId === t.id,"aria-controls":`runs-${t.id}`,});
                   { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":expandedId === t.id ? 'chevronUp' : 'chevronDown',"size":12,}});} 
                 }
                 { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":(e) => rowMenu(e, t),"aria-label":`More actions for ${t.name}`,"title":`More actions`,"aria-haspopup":`menu`,});
                   { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
                 }
               }
             }
            if(live && events.state !== 'connected'){
               { svelteHTML.createElement("p", { "class":`reconnect`,});        }
            }
            if(expandedId === t.id){
               { svelteHTML.createElement("div", {   "class":`runs`,"id":`runs-${t.id}`,});
                 { const $$_etatSdaoL7C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL7 = new $$_etatSdaoL7C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`runs`,"variant":`compact`,"loading":!scheduledTasks.runsByTask[t.id] && !scheduledTasks.runsError[t.id],"error":scheduledTasks.runsError[t.id],"empty":(scheduledTasks.runsByTask[t.id] ?? []).length === 0,"onretry":() => void scheduledTasks.loadRuns(t.id),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("div", { "class":`muted`,});        }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL7.$$prop_def;/*Ωignore_endΩ*/
                  
                   for(let r of __sveltets_2_ensureArray(scheduledTasks.runsByTask[t.id] ?? [])){r.id;
                   { svelteHTML.createElement("div", { "class":`run`,});
                     { const $$_egdaBsutatS9C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS9C({ target: __sveltets_2_any(), props: {  "status":runStatus(r.status),}});}
                     { svelteHTML.createElement("span", { "class":`run-when`,}); { const $$_emiTleR10C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR10C({ target: __sveltets_2_any(), props: {  "iso":r.started_at,}});} }
                     { svelteHTML.createElement("span", {    "class":`run-sum`,"title":r.summary || undefined,});!r.summary;r.summary || 'No summary'; }
                    if(r.report_rel){
                       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => viewReport(r, t.name),});  }
                    }
                    if(r.session_id){
                       { svelteHTML.createElement("button", {     "class":`btn small`,"title":`Open the agent session this run drove`,"onclick":() => openSession(r.session_id),});  }
                    }
                    if(r.status === 'running'){
                       { svelteHTML.createElement("button", {     "class":`btn small danger`,"title":`Stop this run (its agent session, shell command or workflow run is stopped)`,"onclick":() => void stopRun(r, t),});  }
                    }
                    if((r.attempts ?? 1) > 1){ { const $$_egdaB9C = __sveltets_2_ensureComponent(Badge); new $$_egdaB9C({ target: __sveltets_2_any(), props: {    "tone":`warn`,"label":plural(r.attempts ?? 1, 'attempt'),}});}}
                    if(r.delivered){ { const $$_egdaB9C = __sveltets_2_ensureComponent(Badge); new $$_egdaB9C({ target: __sveltets_2_any(), props: {    "tone":`ok`,"label":`Delivered`,}});}}
                    if(r.skipped_delivery){ { const $$_egdaB9C = __sveltets_2_ensureComponent(Badge); new $$_egdaB9C({ target: __sveltets_2_any(), props: {    "label":`No change`,"title":`Not delivered: the report is unchanged since the last run`,}});}}
                    if(r.delivery_error){ { const $$_egdaB9C = __sveltets_2_ensureComponent(Badge); new $$_egdaB9C({ target: __sveltets_2_any(), props: {      "tone":`warn`,"label":`Delivery failed`,"title":r.delivery_error ?? undefined,}});}}
                    if(r.proof_pack_id){
                       { svelteHTML.createElement("button", {     "class":`btn small`,"title":`Open the proof pack for this run`,"onclick":() => router.go(`proof/${encodeURIComponent(r.proof_pack_id ?? '')}`),});  }
                    }
                    if(r.workflow_run_id){
                       { svelteHTML.createElement("button", {     "class":`btn small`,"title":`Open the workflow this run launched (run ${r.workflow_run_id})`,"onclick":() => void openWorkflowRun(r.workflow_run_id!, t.workflow_id),});  }
                    }
                    
                    if(r.status === 'error' && r.error){
                       { svelteHTML.createElement("p", {   "class":`run-err`,"role":`note`,}); { const $$_nocI10C = __sveltets_2_ensureComponent(Icon); new $$_nocI10C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} r.error; }
                    } else if (r.status === 'skipped' && r.error){
                      
                       { svelteHTML.createElement("p", {   "class":`run-err note`,"role":`note`,});r.error; }
                    }
                    if(r.delivery_error){
                       { svelteHTML.createElement("p", {   "class":`run-err warn`,"role":`note`,});  r.delivery_error; }
                    }
                   }
                }
                 LoadState}
               }
            }
           }
        }
       }
     LoadState}
  }

  if(reportOpen){
     { const $$_ladoM3C = __sveltets_2_ensureComponent(Modal); const $$_ladoM3 = new $$_ladoM3C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":reportTaskName ? `Report — ${reportTaskName}` : 'Report',"width":760,"onclose":() => (reportOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
        if(reportText.trim() && !reportLoading && !reportError){
           { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Report view`,});
             { svelteHTML.createElement("button", {      "type":`button`,"aria-pressed":!reportRaw,"onclick":() => (reportRaw = false),});!reportRaw;  }
             { svelteHTML.createElement("button", {      "type":`button`,"aria-pressed":reportRaw,"onclick":() => (reportRaw = true),});reportRaw;  }
           }
           { svelteHTML.createElement("button", {   "class":`btn`,"onclick":async () => (await copyText(reportText)) ? toasts.success('Report copied') : toasts.error('Couldn’t copy the report', 'Select the text and copy it manually.'),});
             { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} 
           }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
        }
         { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (reportOpen = false),});  }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM3.$$prop_def;/*Ωignore_endΩ*/
      if(reportLoading || reportError){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {          "what":`the report`,"loading":reportLoading,"error":reportError,"empty":true,"onretry":() => reportRun && viewReport(reportRun, reportTaskName, reportRaw),}});}
      } else if (!reportText.trim()){
         { svelteHTML.createElement("div", { "class":`muted`,});      }
      } else if (reportRaw){
         { svelteHTML.createElement("pre", { "class":`report`,});reportText; }
      } else if (reportRenderError){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {      "what":`formatted report`,"error":reportRenderError,"empty":true,"onretry":() => reportRenderRetry++,}});}
      } else if (!reportHtml){
          { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {  "what":`formatted report`,"loading":true,"empty":true,}});}
      }else{
        
         { svelteHTML.createElement("div", { "class":`md-body report-md`,}); reportHtml; }
      }
      
     Modal}
  }
 }
 PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ScheduledTasksPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ScheduledTasksPage__SvelteComponent_ = ReturnType<typeof ScheduledTasksPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default ScheduledTasksPage__SvelteComponent_;