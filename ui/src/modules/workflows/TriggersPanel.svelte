<script lang="ts">
  // Triggers configuration panel: list, add, toggle and delete workflow
  // triggers (schedule / webhook / event).  Shown in the workflow inspector
  // sidebar when the "Triggers" tab is active.
  import { onDestroy } from 'svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { WorkflowTrigger, TriggerKind } from '../../lib/api/types';
  import { buildTriggerSpec, defaultTriggerForm, formFromTrigger, EVENT_KINDS } from './triggerForm';
  import { copyTextOrThrow } from '../../lib/clipboard';

  interface Props {
    workflowId: string;
    workflowName?: string;
    triggers: WorkflowTrigger[];
    ontriggers?: (ts: WorkflowTrigger[]) => void;
  }

  let { workflowId, workflowName = '', triggers = $bindable([]), ontriggers }: Props = $props();

  // A copy-paste chat (Slack/Telegram) message that triggers THIS workflow by
  // name. Name is pre-filled; the rest are placeholders the user fills in. Copy
  // it, paste it into a channel where the bot lives, and pin it for reuse. The
  // bot mention is per-workspace (set in the Slack/Telegram client), so it's a
  // placeholder here — the parser keys off `Action: Workflow`, not the mention.
  const chatSnippet = $derived(
    `@<your-bot>\n` +
      `Action: Workflow\n` +
      `Name: ${workflowName || '<workflow name>'}\n` +
      `Msg: what you want done — instructions for the agents\n` +
      `Jira ticket:\n` +
      `Working Directory: ~/path/to/repo\n` +
      `Branch: <base branch>, create wt from it\n` +
      `Relevant Info: ~/other/repo\n` +
      `Goals:\n` +
      `  - goal 1\n` +
      `  - under 20 minutes`,
  );
  let alive = true;
  onDestroy(() => { alive = false; });
  let copied = $state(false);
  async function copyChat(): Promise<void> {
    try {
      await copyTextOrThrow(chatSnippet);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {
      toasts.error('Couldn’t copy to the clipboard', 'Select the text and copy it manually.');
    }
  }

  let adding = $state(false);
  let form = $state(defaultTriggerForm());
  let editingId = $state<string | null>(null);
  let originalSpec: Record<string, unknown> = {};
  let preview = $state<string[] | null>(null);
  let previewTimezone = $state('UTC');
  let previewing = $state(false);
  function edit(t?: WorkflowTrigger): void {
    editingId = t?.id ?? null;
    originalSpec = t ? { ...(t.spec as Record<string, unknown>) } : {};
    form = t ? formFromTrigger(t) : defaultTriggerForm();
    preview = null;
    adding = true;
  }
  async function previewTrigger(): Promise<void> {
    previewing = true;
    const timezone = form.timezone;
    try {
      const result = await api.post<{ next_fire_times: string[] }>(`/workflows/${workflowId}/triggers/preview`, { kind: form.kind, spec: buildSpec() });
      if (!alive) return;
      previewTimezone = timezone;
      preview = result.next_fire_times;
    } catch (e) { toasts.error('Couldn’t preview the trigger', e instanceof Error ? e.message : String(e)); }
    finally { previewing = false; }
  }

  // A preview describes the form as it WAS: any later edit hides it, so a stale
  // "Next fires" list never sits under a changed cadence/timezone.
  $effect(() => {
    JSON.stringify(form);
    preview = null;
  });

  let saving = $state(false);

  // A failed load is shown inline with Retry (never as "No triggers", which
  // would claim the workflow only runs manually when we simply don't know).
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  async function load(): Promise<void> {
    loading = true;
    try {
      const ts = await api.get<WorkflowTrigger[]>(`/workflows/${workflowId}/triggers`);
      if (!alive) return;
      triggers = ts;
      loadError = null;
      ontriggers?.(ts);
    } catch (e) {
      if (alive) loadError = loadErrorText(e);
    } finally {
      if (alive) loading = false;
    }
  }

  const KIND_LABEL: Record<string, string> = { schedule: 'Schedule', webhook: 'Webhook', event: 'Event', chat: 'Chat binding' };

  $effect(() => {
    if (workflowId) void load();
  });

  function buildSpec(): Record<string, unknown> { return buildTriggerSpec(form, originalSpec); }

  async function addTrigger(): Promise<void> {
    if (saving) return;
    saving = true;
    try {
      const t = editingId
        ? await api.patch<WorkflowTrigger>(`/workflow-triggers/${editingId}`, { spec: buildSpec() })
        : await api.post<WorkflowTrigger>(`/workflows/${workflowId}/triggers`, { kind: form.kind, spec: buildSpec(), enabled: true });
      if (!alive) return;
      triggers = editingId ? triggers.map((old) => old.id === t.id ? t : old) : [...triggers, t];
      ontriggers?.(triggers);
      adding = false;
      toasts.success(editingId ? 'Trigger updated' : 'Trigger added');
    } catch (e) {
      toasts.error(editingId ? 'Couldn’t update the trigger' : 'Couldn’t add the trigger', e instanceof Error ? e.message : String(e));
    } finally {
      saving = false;
    }
  }

  async function toggle(t: WorkflowTrigger): Promise<void> {
    try {
      const updated = await api.patch<WorkflowTrigger>(`/workflow-triggers/${t.id}`, {
        enabled: !t.enabled,
      });
      if (!alive) return;
      triggers = triggers.map((x) => (x.id === t.id ? updated : x));
      ontriggers?.(triggers);
    } catch (e) {
      toasts.error('Couldn’t update the trigger', e instanceof Error ? e.message : String(e));
    }
  }

  // Deleting is irreversible — a webhook trigger's token goes with it, so any
  // external caller breaks — so it asks first, like deleting the workflow.
  // (Pausing is the reversible option: the on/off toggle.)
  async function remove(t: WorkflowTrigger): Promise<void> {
    const ok = await confirmer.ask(
      `Delete this ${t.kind} trigger (${describeSpec(t)})?${t.kind === 'webhook' ? ' Its webhook URL stops working.' : ''} To pause it instead, switch it off.`,
      { title: 'Delete trigger', confirmLabel: 'Delete trigger' },
    );
    if (!ok) return;
    try {
      await api.del(`/workflow-triggers/${t.id}`);
      if (!alive) return;
      triggers = triggers.filter((x) => x.id !== t.id);
      ontriggers?.(triggers);
      toasts.success('Trigger removed');
    } catch (e) {
      toasts.error('Couldn’t remove the trigger', e instanceof Error ? e.message : String(e));
    }
  }

  function describeSpec(t: WorkflowTrigger): string {
    const s = t.spec as Record<string, unknown>;
    if (t.kind === 'schedule') {
      const c = s.cadence as string | undefined;
      if (c === 'interval') return `every ${s.every_min ?? 60} min`;
      if (c === 'daily') return `daily at ${s.at ?? '09:00'} ${s.timezone ?? 'UTC'}`;
      if (c === 'weekly') {
        const days = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
        const wd = typeof s.weekday === 'number' ? (days[s.weekday] ?? 'Mon') : 'Mon';
        return `weekly ${wd} at ${s.at ?? '09:00'} ${s.timezone ?? 'UTC'}`;
      }
      if (c === 'cron') return `${s.expr ?? ''} (${s.timezone ?? 'UTC'})`;
      return 'custom schedule';
    }
    if (t.kind === 'webhook') {
      const tok = s.token as string | undefined;
      return tok ? `token: ${tok.slice(0, 8)}…` : 'token pending';
    }
    if (t.kind === 'event') return `on ${s.event_kind ?? 'event'}`;
    if (t.kind === 'chat') {
      const ch = (s.channel as string | undefined) ?? '?';
      const chat = (s.chat as string | undefined) ?? '?';
      const thread = s.thread ? ` · thread ${s.thread}` : '';
      const mention = s.mention_only ? ' · @mention only' : '';
      return `${ch}: ${chat}${thread}${mention}`;
    }
    return '';
  }

  function kindIcon(kind: TriggerKind): IconName {
    if (kind === 'schedule') return 'clock';
    if (kind === 'webhook') return 'zap';
    if (kind === 'chat') return 'comment';
    return 'bell';
  }
</script>

<div class="tp">
  <div class="tp-head">
    <span class="tp-title">Triggers</span>
    <button class="btn ghost small" onclick={() => edit()}>
      <Icon name="plus" size={12} /> Add
    </button>
  </div>

  {#if adding}
    <div class="add-form">
      <label class="fl">
        <span>Kind</span>
        <select bind:value={form.kind} disabled={editingId !== null}>
          <option value="schedule">Schedule</option>
          <option value="webhook">Webhook</option>
          <option value="event">Event</option>
          <option value="chat">Chat binding</option>
        </select>
      </label>

      {#if form.kind === 'schedule'}
        <label class="fl">
          <span>Cadence</span>
          <select bind:value={form.cadence}>
            <option value="interval">Interval</option>
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
            <option value="cron">Cron</option>
          </select>
        </label>
        {#if form.cadence === 'interval'}
          <label class="fl">
            <span>Every (min)</span>
            <input type="number" min="1" max="10080" bind:value={form.everyMin} />
          </label>
        {:else if form.cadence === 'cron'}
          <label class="fl"><span>Cron (5 fields)</span><input bind:value={form.cron} placeholder="0 9 * * 1-5" /></label>
        {:else}
          <label class="fl">
            <span>At (HH:MM, in the timezone below)</span>
            <input type="text" placeholder="09:00" bind:value={form.atTime} />
          </label>
          {#if form.cadence === 'weekly'}
            <label class="fl">
              <span>Weekday</span>
              <select bind:value={form.weekday}>
                {#each ['Mon','Tue','Wed','Thu','Fri','Sat','Sun'] as d, i (i)}
                  <option value={i}>{d}</option>
                {/each}
              </select>
            </label>
          {/if}
        {/if}
        <label class="fl"><span>Timezone (IANA)</span><input bind:value={form.timezone} placeholder="Asia/Jerusalem" /></label>
        <label class="fl"><span>Run prompt</span><textarea bind:value={form.prompt} rows="3"></textarea></label>
      {:else if form.kind === 'event'}
        <label class="fl">
          <span>Event kind</span>
          <select bind:value={form.eventKind}>{#each EVENT_KINDS as [value, label] (value)}<option {value}>{label}</option>{/each}</select>
        </label>
        <label class="fl"><span>Match fields (JSON)</span><textarea rows="3" bind:value={form.filter}></textarea></label>
        <p class="hint">All specified fields must match the event.</p>
      {:else if form.kind === 'chat'}
        <label class="fl">
          <span>Channel</span>
          <select bind:value={form.chatChannel}>
            <option value="slack">Slack</option>
            <option value="telegram">Telegram</option>
          </select>
        </label>
        <label class="fl">
          <span>Chat id</span>
          <input type="text" placeholder="C0123456 (Slack) / -100987654321 (Telegram)" bind:value={form.chatId} />
        </label>
        <label class="fl">
          <span>Thread (optional)</span>
          <input type="text" placeholder="thread ts — leave blank to match any thread" bind:value={form.chatThread} />
        </label>
        <label class="chk-row" class:disabled={form.chatChannel === 'telegram'}>
          <input
            type="checkbox"
            bind:checked={form.chatMentionOnly}
            disabled={form.chatChannel === 'telegram'}
          />
          <span>Only when the bot is @mentioned{form.chatChannel === 'telegram' ? ' (Slack only)' : ''}</span>
        </label>
        <p class="hint">Any message in this channel/chat starts the workflow — no keyword needed.</p>
      {:else}
        <p class="hint">
          A unique token will be auto-generated. Call
          <code>POST /workflows/{workflowId}/webhook/&#123;token&#125;</code>
          from any external system to start a run.
        </p>
      {/if}

      <label class="fl"><span>Send results to</span><select bind:value={form.resultChannel}><option value="">No chat delivery</option><option value="slack">Slack</option><option value="telegram">Telegram</option></select></label>
      {#if form.resultChannel}
        <label class="fl"><span>Channel / chat ID</span><input bind:value={form.resultChat} /></label>
        <label class="fl"><span>Thread (optional)</span><input bind:value={form.resultThread} /></label>
      {/if}
      <label class="fl"><span>Result webhook (optional)</span><input type="url" bind:value={form.resultWebhook} placeholder="https://example.com/result" /></label>
      <button class="btn small" disabled={previewing} onclick={previewTrigger}>{previewing ? 'Checking…' : 'Preview / validate'}</button>
      {#if preview}
        <div class="hint">{#if preview.length}Next fires ({previewTimezone}):<ul>{#each preview as at}<li>{new Date(at).toLocaleString(undefined, { timeZone: previewTimezone })}</li>{/each}</ul>{:else}Trigger configuration is valid.{/if}</div>
      {/if}
      <div class="add-btns">
        <button class="btn ghost small" onclick={() => (adding = false)}>Cancel</button>
        <button class="btn primary small" disabled={saving} onclick={addTrigger}>
          {saving ? 'Saving…' : editingId ? 'Save trigger' : 'Add trigger'}
        </button>
      </div>
    </div>
  {/if}

  <LoadState
    what="triggers"
    variant="compact"
    {loading}
    error={loadError}
    empty={triggers.length === 0}
    onretry={() => void load()}
  >
    {#snippet emptyView()}
      {#if !adding}<p class="empty">No triggers — the workflow only runs manually.</p>{/if}
    {/snippet}
  {#each triggers as t (t.id)}
    <div class="trig-row" class:disabled={!t.enabled}>
      <span class="trig-ic"><Icon name={kindIcon(t.kind)} size={13} /></span>
      <div class="trig-body">
        <span class="trig-kind">{KIND_LABEL[t.kind] ?? t.kind}</span>
        <span class="trig-spec" title={describeSpec(t)}>{describeSpec(t)}</span>
      </div>
      <button
        class="toggle"
        aria-pressed={t.enabled}
        title={t.enabled ? 'On — click to pause this trigger' : 'Off — click to enable this trigger'}
        onclick={() => toggle(t)}
      >
        {t.enabled ? 'On' : 'Off'}
      </button>
      <button class="icon-btn" title="Edit trigger" aria-label="Edit trigger" onclick={() => edit(t)}><Icon name="edit" size={12} /></button>
      <button class="row-del" title="Delete trigger…" aria-label="Delete trigger…" onclick={() => remove(t)}>
        <Icon name="trash" size={12} />
      </button>
    </div>
  {/each}
  </LoadState>

  <!-- Trigger from chat: a copy-paste message that starts this workflow by name.
       Copy → paste into Slack/Telegram → pin it for one-click reuse. -->
  <div class="chat-trig">
    <div class="st-head">
      <span class="tp-title">Trigger from chat</span>
      <button class="btn ghost small" onclick={copyChat}>
        <Icon name={copied ? 'check' : 'copy'} size={12} /> {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
    <p class="st-hint">
      Post this in a Slack/Telegram channel where the Otto bot is configured for
      this workspace, then <strong>pin it</strong> to reuse it. The bot matches the
      workflow by <strong>Name</strong> and starts a run.
    </p>
    <pre class="st-snip">{chatSnippet}</pre>
    <p class="st-hint">
      Reply <code>help</code> in the thread for all commands;
      <code>status</code> / <code>skip</code> / <code>abort</code> control a running run.
    </p>
  </div>
</div>

<style>
  .chat-trig {
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
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .st-snip {
    margin: 0;
    padding: 8px 10px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
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
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    flex: 1;
  }
  .add-form {
    background: var(--surface);
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
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-weight: 600;
  }
  .fl input,
  .fl select,
  .fl textarea {
    font: inherit;
    font-size: var(--fs-m);
    padding: 4px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
  }
  .chk-row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text);
    cursor: pointer;
  }
  .chk-row.disabled {
    color: var(--text-dim);
    cursor: default;
  }
  .chk-row input {
    margin: 0;
  }
  .add-btns {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin: 0;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    background: var(--surface);
    padding: 1px 4px;
    border-radius: var(--radius-s);
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
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text);
    text-transform: capitalize;
  }
  .trig-spec {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .toggle {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    background: none;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 2px 6px;
    cursor: pointer;
    flex-shrink: 0;
  }
  .toggle[aria-pressed='true'] {
    color: var(--success);
    background: var(--success-soft);
    border-color: transparent;
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
  .row-del:hover,
  .row-del:focus-visible {
    color: var(--danger);
  }
  .empty {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin: 4px 0;
  }
</style>
