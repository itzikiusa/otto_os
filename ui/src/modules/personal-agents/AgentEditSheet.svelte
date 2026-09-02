<script lang="ts">
  // Create/edit sheet for one personal agent: name, avatar, persona (soul),
  // provider + ModelPicker, browser toggle, delivery — mirrors the
  // scheduled-tasks destination form. Schedules are edited on the agent page.
  import Modal from '../../lib/components/Modal.svelte';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import { agentProvidersWith, defaultAgentProvider } from '../../lib/providers';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { PersonalAgent } from '../../lib/api/types';

  interface Props {
    /** null = create a new agent. */
    agent: PersonalAgent | null;
    onclose: () => void;
  }
  let { agent, onclose }: Props = $props();

  let busy = $state(false);
  let error = $state('');

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

  // Live registry (built-ins + custom), never dropping a saved custom slug.
  const PROVIDERS = $derived(agentProvidersWith(agent?.provider));

  function onProviderSelect(v: string): void {
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
      if (agent) await personalAgents.update(agent.id, body);
      else if (ws.currentId) await personalAgents.create(ws.currentId, body);
      onclose();
    } catch (e) {
      error = e instanceof Error ? e.message : 'Save failed';
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={agent ? `Edit ${agent.name}` : 'New personal agent'} width={620} {onclose}>
  <div class="sheet">
    {#if error}<div class="err" role="alert">{error}</div>{/if}

    <div class="row">
      <label class="fld grow">
        <span>Name</span>
        <input bind:value={fName} placeholder="Daily Recap" />
      </label>
      <label class="fld narrow">
        <span>Avatar (emoji)</span>
        <input bind:value={fAvatar} placeholder="📰" maxlength="8" />
      </label>
    </div>

    <label class="fld">
      <span>Persona (soul) — who this agent is, materialized into its workspace</span>
      <textarea bind:value={fSoul} rows="6" placeholder="You are a diligent chronicler…"></textarea>
    </label>

    <div class="row">
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
      <input bind:value={fCwd} placeholder="defaults to the agent's own workspace" />
    </label>

    <div class="row">
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

    <div class="toggles">
      <label class="chk">
        <input type="checkbox" bind:checked={fBrowser} />
        Browser use (attach the otto-browser MCP to runs and chat)
      </label>
      <label class="chk"><input type="checkbox" bind:checked={fEnabled} /> Enabled</label>
    </div>
  </div>

  {#snippet footer()}
    <button class="btn" disabled={busy} onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={busy} onclick={save}>{busy ? 'Saving…' : 'Save'}</button>
  {/snippet}
</Modal>

<style>
  .sheet { display: flex; flex-direction: column; gap: 0.75rem; }
  .err {
    background: color-mix(in srgb, var(--status-exited) 12%, transparent);
    color: var(--status-exited); padding: 0.5rem 0.75rem;
    border-radius: var(--radius-s); font-size: 0.85rem;
  }
  .row { display: flex; gap: 0.75rem; flex-wrap: wrap; }
  .row .fld { flex: 1; min-width: 180px; }
  .row .narrow { flex: 0 0 8rem; min-width: 8rem; }
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
  .toggles { display: flex; flex-direction: column; gap: 0.4rem; }
  .chk { display: flex; align-items: center; gap: 0.4rem; font-size: 0.85rem; color: var(--text); }
</style>
