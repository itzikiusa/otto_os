<script lang="ts">
  // Triggers configuration panel: list, add, toggle and delete workflow
  // triggers (schedule / webhook / event).  Shown in the workflow inspector
  // sidebar when the "Triggers" tab is active.
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import type { WorkflowTrigger, TriggerKind } from '../../lib/api/types';

  interface Props {
    workflowId: string;
    workflowName?: string;
    triggers: WorkflowTrigger[];
    ontriggers?: (ts: WorkflowTrigger[]) => void;
  }

  let { workflowId, workflowName = '', triggers = $bindable([]), ontriggers }: Props = $props();

  // A copy-paste Slack message that triggers THIS workflow by name.
  const slackSnippet = $derived(
    `@otto\n` +
      `Action: Workflow\n` +
      `Name: ${workflowName || '<workflow name>'}\n` +
      `Msg: what you want done — instructions for the agents\n` +
      `Jira ticket: GS-1111\n` +
      `Working Directory: ~/path/to/repo\n` +
      `Relevant Info: ~/path/a, ~/path/b\n` +
      `Goals:\n` +
      `  - 100% test coverage (services)\n` +
      `  - under 2 minutes runtime`,
  );
  let copied = $state(false);
  async function copySlack(): Promise<void> {
    try {
      await navigator.clipboard.writeText(slackSnippet);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      toasts.error('Copy failed', 'Select the text and copy manually.');
    }
  }

  // ---- state for the "add trigger" form ----------------------------------
  let adding = $state(false);
  let newKind = $state<TriggerKind>('schedule');
  // Schedule params
  let cadence = $state<'interval' | 'daily' | 'weekly'>('interval');
  let everyMin = $state(60);
  let atTime = $state('09:00');
  let weekday = $state(0);
  // Event params
  let eventKind = $state('ReviewChanged');
  // Webhook has no extra params (token is auto-generated server-side).

  let saving = $state(false);

  async function load(): Promise<void> {
    try {
      const ts = await api.get<WorkflowTrigger[]>(`/workflows/${workflowId}/triggers`);
      triggers = ts;
      ontriggers?.(ts);
    } catch (e) {
      toasts.error('Could not load triggers', e instanceof Error ? e.message : String(e));
    }
  }

  $effect(() => {
    if (workflowId) void load();
  });

  function buildSpec(): Record<string, unknown> {
    switch (newKind) {
      case 'schedule':
        if (cadence === 'interval') return { cadence, every_min: everyMin, enabled: true };
        if (cadence === 'daily') return { cadence, at: atTime, enabled: true };
        return { cadence, at: atTime, weekday, enabled: true };
      case 'event':
        return { event_kind: eventKind };
      case 'webhook':
      default:
        return {};
    }
  }

  async function addTrigger(): Promise<void> {
    if (saving) return;
    saving = true;
    try {
      const t = await api.post<WorkflowTrigger>(`/workflows/${workflowId}/triggers`, {
        kind: newKind,
        spec: buildSpec(),
        enabled: true,
      });
      triggers = [...triggers, t];
      ontriggers?.(triggers);
      adding = false;
      toasts.success('Trigger added');
    } catch (e) {
      toasts.error('Could not add trigger', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function toggle(t: WorkflowTrigger): Promise<void> {
    try {
      const updated = await api.patch<WorkflowTrigger>(`/workflow-triggers/${t.id}`, {
        enabled: !t.enabled,
      });
      triggers = triggers.map((x) => (x.id === t.id ? updated : x));
      ontriggers?.(triggers);
    } catch (e) {
      toasts.error('Could not update trigger', e instanceof Error ? e.message : String(e));
    }
  }

  async function remove(t: WorkflowTrigger): Promise<void> {
    try {
      await api.del(`/workflow-triggers/${t.id}`);
      triggers = triggers.filter((x) => x.id !== t.id);
      ontriggers?.(triggers);
      toasts.success('Trigger removed');
    } catch (e) {
      toasts.error('Could not remove trigger', e instanceof Error ? e.message : String(e));
    }
  }

  function describeSpec(t: WorkflowTrigger): string {
    const s = t.spec as Record<string, unknown>;
    if (t.kind === 'schedule') {
      const c = s.cadence as string | undefined;
      if (c === 'interval') return `every ${s.every_min ?? 60} min`;
      if (c === 'daily') return `daily at ${s.at ?? '09:00'} UTC`;
      if (c === 'weekly') {
        const days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
        const wd = typeof s.weekday === 'number' ? (days[s.weekday] ?? 'Mon') : 'Mon';
        return `weekly ${wd} at ${s.at ?? '09:00'} UTC`;
      }
      return 'custom schedule';
    }
    if (t.kind === 'webhook') {
      const tok = s.token as string | undefined;
      return tok ? `token: ${tok.slice(0, 8)}…` : 'token pending';
    }
    if (t.kind === 'event') return `on ${s.event_kind ?? 'event'}`;
    return '';
  }

  function kindIcon(kind: TriggerKind): string {
    if (kind === 'schedule') return 'clock';
    if (kind === 'webhook') return 'zap';
    return 'bell';
  }
</script>

<div class="tp">
  <div class="tp-head">
    <span class="tp-title">Triggers</span>
    <button class="btn ghost small" onclick={() => (adding = !adding)}>
      <Icon name="plus" size={12} /> Add
    </button>
  </div>

  {#if adding}
    <div class="add-form">
      <label class="fl">
        <span>Kind</span>
        <select bind:value={newKind}>
          <option value="schedule">Schedule</option>
          <option value="webhook">Webhook</option>
          <option value="event">Event</option>
        </select>
      </label>

      {#if newKind === 'schedule'}
        <label class="fl">
          <span>Cadence</span>
          <select bind:value={cadence}>
            <option value="interval">Interval</option>
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
          </select>
        </label>
        {#if cadence === 'interval'}
          <label class="fl">
            <span>Every (min)</span>
            <input type="number" min="1" max="10080" bind:value={everyMin} />
          </label>
        {:else}
          <label class="fl">
            <span>At (UTC HH:MM)</span>
            <input type="text" placeholder="09:00" bind:value={atTime} />
          </label>
          {#if cadence === 'weekly'}
            <label class="fl">
              <span>Weekday</span>
              <select bind:value={weekday}>
                {#each ['Mon','Tue','Wed','Thu','Fri','Sat','Sun'] as d, i (i)}
                  <option value={i}>{d}</option>
                {/each}
              </select>
            </label>
          {/if}
        {/if}
      {:else if newKind === 'event'}
        <label class="fl">
          <span>Event kind</span>
          <input type="text" placeholder="ReviewChanged" bind:value={eventKind} />
        </label>
        <p class="hint">The workflow starts whenever this daemon event fires.</p>
      {:else}
        <p class="hint">
          A unique token will be auto-generated. Call
          <code>POST /workflows/{workflowId}/webhook/&#123;token&#125;</code>
          from any external system to start a run.
        </p>
      {/if}

      <div class="add-btns">
        <button class="btn primary small" disabled={saving} onclick={addTrigger}>
          {saving ? 'Saving…' : 'Save trigger'}
        </button>
        <button class="btn ghost small" onclick={() => (adding = false)}>Cancel</button>
      </div>
    </div>
  {/if}

  {#if triggers.length === 0 && !adding}
    <p class="empty">No triggers — the workflow only runs manually.</p>
  {/if}

  {#each triggers as t (t.id)}
    <div class="trig-row" class:disabled={!t.enabled}>
      <span class="trig-ic"><Icon name={kindIcon(t.kind)} size={13} /></span>
      <div class="trig-body">
        <span class="trig-kind">{t.kind}</span>
        <span class="trig-spec">{describeSpec(t)}</span>
      </div>
      <button
        class="toggle"
        title={t.enabled ? 'Disable' : 'Enable'}
        onclick={() => toggle(t)}
      >
        {t.enabled ? 'on' : 'off'}
      </button>
      <button class="row-del" title="Delete" onclick={() => remove(t)}>
        <Icon name="trash" size={12} />
      </button>
    </div>
  {/each}

  <!-- Trigger from Slack: a copy-paste message that starts this workflow by name. -->
  <div class="slack-trig">
    <div class="st-head">
      <span class="tp-title">Trigger from Slack</span>
      <button class="btn ghost small" onclick={copySlack}>
        <Icon name={copied ? 'check' : 'copy'} size={12} /> {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
    <p class="st-hint">
      Post this in a Slack channel where the Otto bot is configured for this
      workspace. The bot matches the workflow by <strong>Name</strong> and starts a run.
    </p>
    <pre class="st-snip">{slackSnippet}</pre>
  </div>
</div>

<style>
  .slack-trig {
    margin-top: 8px;
    padding-top: 10px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .st-head {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .st-head .tp-title {
    flex: 1;
  }
  .st-hint {
    margin: 0;
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .st-snip {
    margin: 0;
    padding: 8px 10px;
    background: var(--bg, #0d0f13);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 11px;
    line-height: 1.5;
    white-space: pre-wrap;
    color: var(--text);
    overflow-x: auto;
  }
  .tp {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-top: 1px solid var(--border);
  }
  .tp-head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 4px;
  }
  .tp-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    flex: 1;
  }
  .add-form {
    background: var(--surface-raised, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .fl {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .fl span {
    font-size: 11px;
    color: var(--text-dim);
    font-weight: 600;
  }
  .fl input,
  .fl select {
    font: inherit;
    font-size: 12.5px;
    padding: 4px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--input-bg, var(--bg));
    color: var(--text);
  }
  .add-btns {
    display: flex;
    gap: 6px;
  }
  .hint {
    font-size: 11px;
    color: var(--text-dim);
    margin: 0;
  }
  code {
    font-family: var(--mono);
    font-size: 11px;
    background: var(--surface-raised, var(--surface));
    padding: 1px 4px;
    border-radius: 3px;
  }
  .trig-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .trig-row.disabled {
    opacity: 0.45;
  }
  .trig-ic {
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .trig-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .trig-kind {
    font-size: 11px;
    font-weight: 600;
    color: var(--text);
    text-transform: capitalize;
  }
  .trig-spec {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .toggle {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
    background: none;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 2px 6px;
    cursor: pointer;
    flex-shrink: 0;
  }
  .toggle:hover {
    background: var(--hover);
  }
  .row-del {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    padding: 4px;
    flex-shrink: 0;
  }
  .row-del:hover {
    color: var(--danger, #e04c4c);
  }
  .empty {
    font-size: 12px;
    color: var(--text-dim);
    margin: 4px 0;
  }
</style>
