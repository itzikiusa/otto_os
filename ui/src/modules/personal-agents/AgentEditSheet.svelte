<script lang="ts">
  import PathField from '../../lib/components/PathField.svelte';
  // Create/edit sheet for one personal agent: name, avatar, persona (soul),
  // provider + ModelPicker, browser toggle, delivery — mirrors the
  // scheduled-tasks destination form. Schedules are edited on the agent page.
  import Modal from '../../lib/components/Modal.svelte';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import { agentProvidersWith, defaultAgentProvider } from '../../lib/providers';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { ui } from '../../lib/stores/ui.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { PersonalAgent } from '../../lib/api/types';
  import { agentTemplates, templateById } from './templates';

  interface Props {
    /** null = create a new agent. */
    agent: PersonalAgent | null;
    /** Pre-select a template (id from `templates.ts`) when creating. */
    template?: string | null;
    onclose: () => void;
  }
  let { agent, template = null, onclose }: Props = $props();

  // Templates only apply to a brand-new agent: they pre-fill the form and, on
  // create, add the template's first schedule.
  const TEMPLATES = agentTemplates();
  // svelte-ignore state_referenced_locally
  let fTemplate = $state<string>(agent ? '' : (template ?? ''));
  let scheduleNote = $state('');

  function applyTemplate(id: string): void {
    fTemplate = id;
    const t = templateById(id);
    if (!t) {
      scheduleNote = '';
      return;
    }
    fName = t.name;
    fAvatar = t.avatar;
    fSoul = t.soul_md;
    scheduleNote = `Adds a schedule: every ${t.schedule.every_min} min — “${t.directive}”`;
  }

  let busy = $state(false);
  let error = $state('');
  let errorEl = $state<HTMLDivElement | null>(null);
  // Save is in the fixed footer: reveal errors even after scrolling a long form.
  $effect(() => { if (error) errorEl?.scrollIntoView({ block: 'nearest' }); });

  // The sheet is mounted fresh per open (parent {#if}-gates it), so capturing
  // the agent's initial values into the form state is intentional.
  // svelte-ignore state_referenced_locally
  const init = agent;
  const d = init?.delivery ?? {};
  const savedDest = (d.type as string) ?? 'none';

  let fName = $state(init?.name ?? '');
  let fAvatar = $state(init?.avatar ?? '');
  let fSoul = $state(init?.soul_md ?? '');
  let fProvider = $state(init?.provider || defaultAgentProvider());
  let fModel = $state(init?.model ?? ''); // '' = provider default
  let fCwd = $state(init?.cwd ?? '');
  let fBrowser = $state(init?.browser ?? false);
  let fEnabled = $state(init?.enabled ?? true);
  let fDestType = $state<'none' | 'slack' | 'telegram' | 'email' | 'webhook'>(
    ['slack', 'telegram', 'email', 'webhook'].includes(savedDest)
      ? (savedDest as 'slack' | 'telegram' | 'email' | 'webhook')
      : 'none',
  );
  let fChatId = $state((d.chat_id as string) ?? '');
  let fEmailTo = $state((d.to as string) ?? '');
  let fUrl = $state((d.url as string) ?? '');
  // Template pre-fill runs after every form field exists (they are `let`s above).
  // svelte-ignore state_referenced_locally
  if (!agent && template) applyTemplate(template);

  // Live registry (built-ins + custom), never dropping a saved custom slug.
  const PROVIDERS = $derived(agentProvidersWith(agent?.provider));

  function onProviderSelect(v: string): void {
    // A model id belongs to one provider — switching clears it, so a hidden
    // stale model (the picker hides for providers without a model flag) is
    // never saved against the new provider.
    if (v !== fProvider) fModel = '';
    if (v === 'custom') {
      if (PROVIDERS.includes(fProvider)) fProvider = '';
    } else {
      fProvider = v;
    }
  }

  function buildDelivery(): Record<string, unknown> {
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
    if (!agent && !ws.currentId) {
      error = 'Add or select a workspace before saving. Your changes are kept in this form.';
      return;
    }
    const body = {
      name: fName.trim(),
      avatar: fAvatar.trim(),
      soul_md: fSoul,
      provider: fProvider.trim(),
      model: fModel.trim(),
      cwd: fCwd.trim(),
      browser: fBrowser,
      delivery: buildDelivery(),
      enabled: fEnabled,
    };
    busy = true;
    try {
      if (agent) {
        await personalAgents.update(agent.id, body);
        toasts.success(`Saved ${body.name}`);
      } else if (ws.currentId) {
        const created = await personalAgents.create(ws.currentId, body);
        const t = templateById(fTemplate);
        if (t) {
          await personalAgents.createSchedule(created.id, {
            schedule: t.schedule,
            timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
            directive: t.directive,
            enabled: true,
          });
        }
        toasts.success(`Created ${body.name}`, body.enabled ? undefined : 'It’s paused — enable it to let its schedules fire.');
      }
      onclose();
    } catch (e) {
      error = `Couldn’t save the agent. ${loadErrorText(e)}`;
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={agent ? `Edit ${agent.name}` : 'New personal agent'} width={620} {onclose}>
  <div class="sheet">
    {#if error}<div class="err" role="alert" bind:this={errorEl}>{error}</div>{/if}
    {#if !agent && !ws.currentId}
      <button class="btn" onclick={() => (ui.newWorkspaceOpen = true)}>Add workspace</button>
    {/if}

    {#if !agent}
      <label class="fld">
        <span>Start from a template (optional)</span>
        <select value={fTemplate} onchange={(e) => applyTemplate((e.currentTarget as HTMLSelectElement).value)} data-testid="agent-template">
          <option value="">Blank agent</option>
          {#each TEMPLATES as t (t.id)}<option value={t.id}>{t.avatar} {t.title} — {t.description}</option>{/each}
        </select>
        {#if scheduleNote}<span class="note">{scheduleNote}</span>{/if}
      </label>
    {/if}

    <div class="fld-row">
      <label class="fld grow">
        <span>Name</span>
        <input bind:value={fName} placeholder="Daily Recap" />
      </label>
      <label class="fld narrow">
        <span>Avatar (emoji, optional)</span>
        <input bind:value={fAvatar} placeholder="📰" maxlength="8" />
      </label>
    </div>

    <label class="fld">
      <span>Persona (soul) — who this agent is, materialized into its workspace</span>
      <textarea bind:value={fSoul} rows="6" placeholder="You are a diligent chronicler…"></textarea>
    </label>

    <div class="fld-row">
      <label class="fld">
        <span>Provider</span>
        <select
          value={PROVIDERS.includes(fProvider) ? fProvider : 'custom'}
          onchange={(e) => onProviderSelect((e.currentTarget as HTMLSelectElement).value)}
        >
          {#each PROVIDERS as p (p)}<option value={p}>{p}</option>{/each}
          <option value="custom">Custom…</option>
        </select>
      </label>
      <div class="fld">
        <ModelPicker provider={fProvider} value={fModel} onchange={(m) => (fModel = m)} />
      </div>
    </div>

    {#if !PROVIDERS.includes(fProvider)}
      <label class="fld">
        <span>Custom provider slug</span>
        <input bind:value={fProvider} placeholder="my-custom-agent (register it in Settings first)" />
      </label>
    {/if}

    <label class="fld">
      <span>Working dir (optional — empty = a private per-agent folder)</span>
      <PathField bind:value={fCwd}><input bind:value={fCwd} placeholder="defaults to the agent's own workspace" /></PathField>
    </label>

    <div class="fld-row">
      <label class="fld">
        <span>Delivery</span>
        <select bind:value={fDestType}>
          <option value="none">None (reports on the agent page only)</option>
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
    {#if fDestType !== 'none'}
      <p class="note dim" data-testid="delivery-note">
        {#if fDestType === 'slack' || fDestType === 'telegram'}
          After every run, its report is posted to {fDestType === 'slack' ? 'Slack' : 'Telegram'}
          {fChatId.trim() ? `(${fChatId.trim()})` : '(the integration’s default channel)'} — everyone in that channel sees it.
        {:else if fDestType === 'email'}
          After every run, its report is emailed to {fEmailTo.trim() || 'the address above'}.
        {:else}
          After every run, its report is POSTed to {fUrl.trim() || 'the URL above'}.
        {/if}
        Unchanged reports aren’t re-sent.
      </p>
    {/if}

    <div class="toggles">
      <label class="chk">
        <input type="checkbox" bind:checked={fBrowser} />
        Browser use (attach the otto-browser MCP to runs and chat)
      </label>
      <label class="chk"><input type="checkbox" bind:checked={fEnabled} /> Enabled — its schedules fire on their cadence</label>
    </div>
  </div>

  {#snippet footer()}
    <button class="btn" disabled={busy} onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={busy} onclick={save}>{busy ? 'Saving…' : 'Save'}</button>
  {/snippet}
</Modal>

<style>
  .sheet { display: flex; flex-direction: column; gap: 12px; }
  .err {
    background: var(--danger-soft); color: var(--danger); padding: 8px 12px;
    border-radius: var(--radius-s); font-size: var(--fs-s);
  }
  .note { margin: 0; font-size: var(--fs-s); color: var(--accent-text); }
  .note.dim { color: var(--text-dim); }
  .fld-row { display: flex; gap: 12px; flex-wrap: wrap; }
  .fld-row .fld { flex: 1; min-width: 180px; }
  .fld-row .narrow { flex: 0 0 10rem; min-width: 10rem; }
  .fld { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-m); color: var(--text); }
  .fld span { color: var(--text-dim); font-size: var(--fs-s); }
  .fld input, .fld select, .fld textarea {
    background: var(--bg); color: var(--text); border: 1px solid var(--border);
    border-radius: var(--radius-s); padding: 6px 8px; font: inherit;
  }
  .fld input::placeholder, .fld textarea::placeholder { color: var(--text-dim); }
  .fld input:focus-visible, .fld select:focus-visible, .fld textarea:focus-visible {
    outline: 2px solid var(--accent); outline-offset: 1px;
  }
  .toggles { display: flex; flex-direction: column; gap: 6px; }
  .chk { display: flex; align-items: center; gap: 6px; font-size: var(--fs-m); color: var(--text); }
</style>
