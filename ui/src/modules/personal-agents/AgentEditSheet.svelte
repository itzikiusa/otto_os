<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  import { untrack } from 'svelte';
  import PathField from '../../lib/components/PathField.svelte';
  // Create/edit sheet for one personal agent: name, avatar, persona (soul),
  // provider + ModelPicker, browser toggle, delivery — mirrors the
  // scheduled-tasks destination form. Schedules are edited on the agent page.
  import Modal from '../../lib/components/Modal.svelte';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import { modelsForProvider } from '../../lib/providerModels';
  import { auth } from '../../lib/stores/auth.svelte';
  import { agentProvidersWith, defaultAgentProvider, harnessLabel } from '../../lib/providers';
  import { canCarryPersona, personaDefault, personaProviders, personaRefusal } from './personaProviders';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { ui } from '../../lib/stores/ui.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { PersonalAgent } from '../../lib/api/types';
  import { agentTemplates, templateById } from './templates';
  import { paSheetPort } from '../../lib/uiCommands/personalAgentsPort';
  import type { PaCreateFill, PaScheduleArg } from '../../lib/uiCommands/personalAgentsPlan';
  import { cadenceLabel } from './cadence';
  import AgentAvatar from './AgentAvatar.svelte';
  import { CHARACTERS } from './characters';

  interface Props {
    /** null = create a new agent. */
    agent: PersonalAgent | null;
    /** Pre-select a template (id from `templates.ts`) when creating. */
    template?: string | null;
    /** Editing: `about` = name and avatar; `setup` = provider,
     *  folder, where results go, browser (the model is the Setup card's own
     *  select). Creating shows everything. */
    section?: 'about' | 'setup';
    onclose: () => void;
  }
  let { agent, template = null, section = 'about', onclose }: Props = $props();
  const showAbout = $derived(!agent || section === 'about');
  const showSetup = $derived(!agent || section === 'setup');
  /** Prefix for the label↔control ids (several sheets can be mounted). */
  const uid = $props.id();

  // Templates only apply to a brand-new agent: they pre-fill the form and, on
  // create, add the template's first schedule.
  const TEMPLATES = agentTemplates();
  // svelte-ignore state_referenced_locally
  let fTemplate = $state<string>(agent ? '' : (template ?? ''));
  let scheduleNote = $state('');
  let createdAgent = $state<PersonalAgent | null>(null);
  let pendingSchedule: Parameters<typeof personalAgents.createSchedule>[1] | null = null;
  /** A schedule an agent filled in (UI control), replacing the template's. */
  let scheduleOverride = $state<PaScheduleArg | null>(null);

  function applyTemplate(id: string): void {
    if (createdAgent) return;
    fTemplate = id;
    const t = templateById(id);
    if (!t) {
      scheduleNote = '';
      return;
    }
    fName = t.name;
    fAvatar = t.avatar;
    fSoul = t.soul_md;
    const s = t.schedule;
    if (!s) {
      scheduleNote = t.primary ? 'Becomes your main agent — set up its routines by chatting with it.' : '';
      return;
    }
    const when = s.cadence === 'daily' ? `daily at ${s.at}` : `every ${s.every_min} min`;
    scheduleNote = `Adds a schedule: ${when} — “${t.directive}”`;
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
  let fProvider = $state(init?.provider || personaDefault(defaultAgentProvider()));
  let fModel = $state(init?.model ?? ''); // '' = provider default
  let fCwd = $state(init?.cwd ?? '');
  let fBrowser = $state(init?.browser ?? false);
  const fEnabled = init?.enabled ?? true;
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

  // Live registry, only the CLIs that can load a persona (personaProviders.ts);
  // a saved other one shows as itself, with why it can't run.
  const PROVIDERS = $derived(personaProviders(agentProvidersWith(agent?.provider)));

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

  type DestType = 'none' | 'slack' | 'telegram' | 'email' | 'webhook';

  function deliveryTarget(type: DestType, chat: string, email: string, url: string): string {
    switch (type) {
      case 'slack':
      case 'telegram':
        return chat.trim();
      case 'email':
        return email.trim();
      case 'webhook':
        return url.trim();
      default:
        return '';
    }
  }

  // Mounted fresh per open — untrack keeps the saved destination, not a live binding.
  const initialDestType = untrack(() => fDestType);
  const initialDestTarget = untrack(() => deliveryTarget(fDestType, fChatId, fEmailTo, fUrl));

  /** True when save would start or retarget an outward report channel. */
  function deliversReports(): boolean {
    if (fDestType === 'none') return false;
    const target = deliveryTarget(fDestType, fChatId, fEmailTo, fUrl);
    return fDestType !== initialDestType || target !== initialDestTarget;
  }

  function deliveryConfirm(): string {
    const target = deliveryTarget(fDestType, fChatId, fEmailTo, fUrl);
    switch (fDestType) {
      case 'slack':
      case 'telegram': {
        const where = fDestType === 'slack' ? 'Slack' : 'Telegram';
        const dest = target ? `${where} (${target})` : `${where} (the integration’s default channel)`;
        return `Every run’s report will be posted to ${dest}. Everyone in that channel can see it. Unchanged reports aren’t re-sent.`;
      }
      case 'email':
        return `Every run’s report will be emailed to ${target || 'the address you entered'}. That recipient is who sees it. Unchanged reports aren’t re-sent.`;
      case 'webhook':
        return `Every run’s report will be POSTed to ${target || 'the webhook URL'}. Whoever controls that URL can read it. Unchanged reports aren’t re-sent.`;
      default:
        return '';
    }
  }

  function saveLabel(): string {
    if (busy) return 'Saving…';
    if (deliversReports()) return 'Save and deliver reports';
    return 'Save';
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
    if (busy) return;
    error = '';
    if (!fName.trim()) {
      error = 'Name is required.';
      return;
    }
    if (!agent && !createdAgent && !ws.currentId) {
      error = 'Add or select a workspace before saving. Your changes are kept in this form.';
      return;
    }
    if (deliversReports()) {
      const ok = await confirmer.ask(deliveryConfirm(), {
        title: 'Deliver run reports?',
        confirmLabel: 'Save and deliver',
      });
      if (!ok) return;
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
      } else if (createdAgent || ws.currentId) {
        if (!createdAgent) {
          const t = templateById(fTemplate);
          const o = scheduleOverride;
          pendingSchedule = o ? {
            schedule: o.schedule, timezone: o.timezone || Intl.DateTimeFormat().resolvedOptions().timeZone,
            directive: o.directive?.trim() || t?.directive || '', enabled: o.enabled ?? true,
            permission: o.permission, idempotency_key: crypto.randomUUID(),
          } : t?.schedule ? {
            schedule: t.schedule, timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
            directive: t.directive, enabled: true, idempotency_key: crypto.randomUUID(),
          } : null;
          createdAgent = await personalAgents.create(ws.currentId!, { ...body, ...(t?.primary ? { primary: true } : {}) });
        } else {
          createdAgent = await personalAgents.update(createdAgent.id, body);
        }
        if (pendingSchedule) {
          await personalAgents.createSchedule(createdAgent.id, pendingSchedule);
          pendingSchedule = null;
        }
        toasts.success(`Created ${body.name}`);
      }
      onclose();
    } catch (e) {
      error = createdAgent
        ? `Agent created; its schedule couldn’t be added. Save again to finish setup for ${createdAgent.name}. ${loadErrorText(e)}`
        : `Couldn’t save the agent. ${loadErrorText(e)}`;
    } finally {
      busy = false;
    }
  }

  // Agent UI control (lib/uiCommands/personalAgents.ts): fill the new-agent
  // form where the user sees it, then run this same Save.
  $effect(() => {
    if (agent) return;
    return paSheetPort.bind({
      fill(f: PaCreateFill) {
        if (f.template !== undefined) applyTemplate(f.template);
        if (f.name !== undefined) fName = f.name;
        if (f.avatar !== undefined) fAvatar = f.avatar;
        if (f.soul_md !== undefined) fSoul = f.soul_md;
        if (f.provider !== undefined) { fProvider = f.provider; fModel = ''; }
        if (f.model !== undefined) fModel = f.model;
        if (f.schedule) {
          scheduleOverride = f.schedule;
          scheduleNote = `Adds a schedule: ${cadenceLabel(f.schedule.schedule, f.schedule.timezone ?? '', { cronWords: true })}`;
        }
      },
      async submit() {
        await save();
        return { agent: createdAgent, error: error || (createdAgent ? '' : 'The form was not saved.') };
      },
      close: () => onclose(),
    });
  });
</script>

<Modal title={agent ? (section === 'setup' ? `${agent.name} · Setup` : `Edit ${agent.name}`) : 'New agent'} width={620} {onclose}>
  <div class="sheet" data-pa-sheet>
    {#if error}<div class="err" use:announceShown={{ text: error, assertive: true }} bind:this={errorEl}>{error}</div>{/if}
    {#if !agent && !ws.currentId}
      <button class="btn" onclick={() => (ui.newWorkspaceOpen = true)}>Add workspace</button>
    {/if}

    {#if !agent}
      <div class="field">
        <label for="{uid}-template">Start from a template (optional)</label>
        <select id="{uid}-template" class="input" disabled={busy || !!createdAgent} value={fTemplate} onchange={(e) => applyTemplate((e.currentTarget as HTMLSelectElement).value)} data-testid="agent-template">
          <option value="">Blank agent</option>
          {#each TEMPLATES as t (t.id)}<option value={t.id}>{t.title} — {t.description}</option>{/each}
        </select>
        {#if scheduleNote}<span class="note">{scheduleNote}</span>{/if}
      </div>
    {/if}

    {#if showAbout}
    <div class="who">
      <AgentAvatar avatar={fAvatar} name={fName || '?'} size={44} />
      <div class="field grow">
        <label for="{uid}-name">Name</label>
        <input dir="auto" id="{uid}-name" class="input" bind:value={fName} placeholder="e.g. PR watcher" />
      </div>
    </div>
    <fieldset class="avatars">
      <legend>Avatar</legend>
      {#each CHARACTERS as c (c.id)}
        <button type="button" class="glyph" class:on={fAvatar === `char:${c.id}`} aria-label="Avatar: {c.label}" title={c.label} aria-pressed={fAvatar === `char:${c.id}`} onclick={() => (fAvatar = `char:${c.id}`)}>
          <img src={c.url} alt="" width="22" height="22" loading="lazy" decoding="async" />
        </button>
      {/each}
    </fieldset>

    {#if !agent}
    <!-- Editing an existing agent, "What it does" is edited in place on its
         About page; this sheet keeps the name and avatar. -->
    <div class="field">
      <label for="{uid}-soul">What it does</label>
      <textarea dir="auto" id="{uid}-soul" class="input" bind:value={fSoul} rows="6" placeholder="You watch my pull requests and tell me when one needs me…"></textarea>
    </div>
    {/if}

    {/if}

    {#if showSetup}
    <div class="field-row">
      <div class="field">
        <label for="{uid}-provider">Provider</label>
        <select
          id="{uid}-provider"
          class="input"
          value={PROVIDERS.includes(fProvider) ? fProvider : 'custom'}
          onchange={(e) => onProviderSelect((e.currentTarget as HTMLSelectElement).value)}
        >
          {#each PROVIDERS as p (p)}<option value={p}>{harnessLabel(p)}</option>{/each}
          {#if !PROVIDERS.includes(fProvider)}<option value="custom" disabled>{harnessLabel(fProvider.trim() || 'custom')}</option>{/if}
        </select>
      </div>
      {#if !agent}
        <div class="cell">
          <ModelPicker provider={fProvider} value={fModel} extra={modelsForProvider(fProvider, auth.meta)} onchange={(m) => (fModel = m)} hint="Every run and chat of this agent uses it. Empty = the provider’s default." />
        </div>
      {/if}
    </div>

    {#if !canCarryPersona(fProvider)}
      <p class="note warn" role="note" data-testid="persona-provider-note">{personaRefusal(fProvider)}</p>
    {/if}

    <div class="field">
      <label for="{uid}-cwd">Working folder (optional — empty = a private per-agent folder)</label>
      <PathField bind:value={fCwd}><input dir="ltr" id="{uid}-cwd" class="input" bind:value={fCwd} placeholder="defaults to the agent’s own workspace" /></PathField>
    </div>

    <div class="field-row">
      <div class="field">
        <label for="{uid}-dest">Also send results to</label>
        <select id="{uid}-dest" class="input" bind:value={fDestType}>
          <option value="none">Its chat only</option>
          <option value="slack">Slack</option>
          <option value="telegram">Telegram</option>
          <option value="email">Email</option>
          <option value="webhook">HTTP webhook</option>
        </select>
      </div>
      {#if fDestType === 'slack' || fDestType === 'telegram'}
        <div class="field">
          <label for="{uid}-chat">Chat / channel id (optional)</label>
          <input dir="auto" id="{uid}-chat" class="input" bind:value={fChatId} placeholder="defaults to the integration channel" />
        </div>
      {:else if fDestType === 'email'}
        <div class="field">
          <label for="{uid}-email">Send to (email)</label>
          <input dir="ltr" id="{uid}-email" class="input" bind:value={fEmailTo} placeholder="you@example.com" />
        </div>
      {:else if fDestType === 'webhook'}
        <div class="field">
          <label for="{uid}-url">Webhook URL</label>
          <input dir="ltr" id="{uid}-url" class="input" bind:value={fUrl} placeholder="https://…" />
        </div>
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
        Browser use (give runs and chat the Otto browser tool)
      </label>
    </div>
    {/if}
  </div>

  {#snippet footer()}
    <button class="btn" disabled={busy} onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={busy} onclick={save}>{saveLabel()}</button>
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
  .note.warn { color: var(--warning); }
  /* Global .field/.input; the sheet's own gap spaces them, rows share a line. */
  .sheet .field { margin-bottom: 0; }
  .field-row { display: flex; gap: 12px; flex-wrap: wrap; }
  .field-row > .field, .field-row > .cell { flex: 1; min-width: 180px; }
  .who { display: flex; align-items: flex-end; gap: 12px; }
  .who > .grow { flex: 1; margin: 0; }
  .avatars { display: flex; flex-wrap: wrap; gap: 6px; border: 0; padding: 0; margin: 0; }
  .avatars legend { font-size: var(--fs-s); color: var(--text-dim); margin-block-end: 4px; }
  .glyph { inline-size: 30px; block-size: 30px; border-radius: var(--radius-m); border: 1px solid var(--border); background: var(--surface-2); cursor: pointer; padding: 2px; display: grid; place-items: center; }
  .glyph.on { border-color: var(--accent-line-strong); background: var(--accent-soft); }
  .glyph:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .toggles { display: flex; flex-direction: column; gap: 6px; }
  .chk { display: flex; align-items: center; gap: 6px; font-size: var(--fs-m); color: var(--text); }
</style>
