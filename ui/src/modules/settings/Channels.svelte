<script lang="ts">
  // Channels settings page: per-workspace Slack + Telegram integration config.
  import { api } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { Channel, Integration, UpsertIntegrationReq } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let integrations: Integration[] = $state([]);
  let loading = $state(false);
  let testBusy: Channel | null = $state(null); // which channel is mid-test

  // Agent CLIs offered in the per-channel picker (from /meta).
  const providers = $derived(auth.meta?.providers ?? ['claude', 'codex', 'shell']);

  // Edit modal state
  let editOpen = $state(false);
  let editChannel: Channel = $state('slack');
  let editBusy = $state(false);

  // Form fields (reset per modal open)
  let fEnabled = $state(false);
  let fBotToken = $state('');
  let fAppToken = $state('');
  let fChannelId = $state('');
  let fAllowedUsers = $state('');
  let fAgentReply = $state(false);
  let fReplyInstructions = $state('');
  let fPreferredCli = $state('');

  // ---------------------------------------------------------------------------
  // Derived helpers
  // ---------------------------------------------------------------------------

  const wsId = $derived(ws.currentId);

  const slack = $derived(integrations.find((i) => i.channel === 'slack') ?? null);
  const telegram = $derived(integrations.find((i) => i.channel === 'telegram') ?? null);

  // ---------------------------------------------------------------------------
  // Load on workspace change
  // ---------------------------------------------------------------------------

  $effect(() => {
    if (wsId) {
      void load(wsId);
    }
  });

  async function load(id: string): Promise<void> {
    loading = true;
    try {
      integrations = await api.get<Integration[]>(`/workspaces/${id}/integrations`);
    } catch (e) {
      toasts.error('Could not load integrations', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Open edit modal
  // ---------------------------------------------------------------------------

  function openEdit(channel: Channel): void {
    editChannel = channel;
    const existing = integrations.find((i) => i.channel === channel);
    fEnabled = existing?.enabled ?? false;
    fBotToken = '';
    fAppToken = '';
    fChannelId = existing?.channel_id ?? '';
    fAllowedUsers = existing?.allowed_users ?? '';
    fAgentReply = existing?.agent_reply ?? false;
    fReplyInstructions = existing?.reply_instructions ?? '';
    fPreferredCli = existing?.preferred_cli ?? '';
    editOpen = true;
  }

  // ---------------------------------------------------------------------------
  // Save
  // ---------------------------------------------------------------------------

  async function save(): Promise<void> {
    if (!wsId) return;
    editBusy = true;
    try {
      const body: UpsertIntegrationReq = {
        enabled: fEnabled,
        allowed_users: fAllowedUsers.trim(),
        agent_reply: fAgentReply,
        reply_instructions: fReplyInstructions.trim(),
        channel_id: fChannelId.trim(),
        preferred_cli: fPreferredCli,
        bot_token: fBotToken !== '' ? fBotToken : null,
        ...(editChannel === 'slack'
          ? { app_token: fAppToken !== '' ? fAppToken : null }
          : {}),
      };
      const updated = await api.put<Integration>(
        `/workspaces/${wsId}/integrations/${editChannel}`,
        body,
      );
      integrations = [
        ...integrations.filter((i) => i.channel !== editChannel),
        updated,
      ];
      editOpen = false;
      toasts.success(
        `${editChannel === 'slack' ? 'Slack' : 'Telegram'} integration saved`,
        updated.enabled ? 'Enabled' : 'Disabled',
      );
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      editBusy = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Toggle enabled (quick toggle from card — keeps all existing values)
  // ---------------------------------------------------------------------------

  async function toggleEnabled(intg: Integration): Promise<void> {
    if (!wsId) return;
    try {
      const body: UpsertIntegrationReq = {
        enabled: !intg.enabled,
        allowed_users: intg.allowed_users,
        agent_reply: intg.agent_reply,
        reply_instructions: intg.reply_instructions,
        channel_id: intg.channel_id,
        preferred_cli: intg.preferred_cli,
        bot_token: null,  // keep existing
        ...(intg.channel === 'slack' ? { app_token: null } : {}),
      };
      const updated = await api.put<Integration>(
        `/workspaces/${wsId}/integrations/${intg.channel}`,
        body,
      );
      integrations = integrations.map((i) => (i.channel === updated.channel ? updated : i));
      toasts.info(
        `${intg.channel === 'slack' ? 'Slack' : 'Telegram'} ${updated.enabled ? 'enabled' : 'disabled'}`,
      );
    } catch (e) {
      toasts.error('Toggle failed', e instanceof Error ? e.message : String(e));
    }
  }

  // ---------------------------------------------------------------------------
  // Delete
  // ---------------------------------------------------------------------------

  async function remove(channel: Channel): Promise<void> {
    if (!wsId) return;
    const label = channel === 'slack' ? 'Slack' : 'Telegram';
    if (!(await confirmer.ask(`Remove the ${label} integration? Tokens will be deleted from the Keychain.`, { title: 'Remove integration', confirmLabel: 'Remove' }))) return;
    try {
      await api.del(`/workspaces/${wsId}/integrations/${channel}`);
      integrations = integrations.filter((i) => i.channel !== channel);
      toasts.info(`${label} integration removed`);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  // ---------------------------------------------------------------------------
  // Send test message
  // ---------------------------------------------------------------------------

  async function sendTest(channel: Channel): Promise<void> {
    if (!wsId) return;
    testBusy = channel;
    try {
      const resp = await api.post<{ ok: boolean; error?: string | null }>(
        `/workspaces/${wsId}/integrations/${channel}/test`,
        {},
      );
      if (resp.ok) {
        toasts.success('Test message sent', 'Otto is connected ✅ posted to the default chat.');
      } else {
        toasts.error('Test message failed', resp.error ?? 'Unknown error');
      }
    } catch (e) {
      toasts.error('Test message failed', e instanceof Error ? e.message : String(e));
    } finally {
      testBusy = null;
    }
  }

  // ---------------------------------------------------------------------------
  // Status line helpers
  // ---------------------------------------------------------------------------

  function statusLine(intg: Integration | null): string {
    if (!intg || !intg.has_bot_token) return 'not configured';
    const agentStr = intg.agent_reply ? 'agent-reply on' : 'agent-reply off';
    return `configured · ${agentStr}`;
  }
</script>

<div class="page">
  <!-- Header -->
  <div class="page-header">
    <div>
      <h1>Channels</h1>
      <div class="sub">
        Configure Slack and Telegram integrations for this workspace. Tokens are stored
        in the macOS Keychain.
      </div>
    </div>
  </div>

  {#if !wsId}
    <!-- No workspace selected -->
    <EmptyState
      icon="plug"
      title="Select a workspace first"
      body="Integrations are per-workspace. Choose a workspace from the sidebar to configure channels."
    />
  {:else if loading}
    <Skeleton rows={2} height={88} />
  {:else}
    <div class="channel-list">
      <!-- Slack card -->
      {#snippet channelCard(channel: Channel, intg: Integration | null, icon: string, label: string)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="channel-card card"
          oncontextmenu={(e) => ctxMenu.show(e, [
            { label: 'Edit', icon: 'edit', action: () => openEdit(channel) },
            ...(intg?.has_bot_token ? [
              { label: intg.enabled ? 'Disable' : 'Enable', icon: intg.enabled ? 'eye' : 'play', action: () => toggleEnabled(intg) },
            ] : []),
            ...(intg?.has_bot_token ? [
              { separator: true },
              { label: 'Remove', icon: 'trash', danger: true as const, action: () => remove(channel) },
            ] : []),
          ])}
        >
          <div class="ch-icon">
            <Icon name={icon} size={16} />
          </div>
          <div class="grow">
            <div class="ch-label">
              {label}
              {#if intg?.has_bot_token}
                <span class="chip">{intg.enabled ? 'enabled' : 'disabled'}</span>
              {/if}
            </div>
            <div class="ch-status dim">{statusLine(intg)}</div>
          </div>
          <div class="ch-actions">
            {#if intg?.has_bot_token}
              <!-- Enabled toggle -->
              <label class="toggle" title={intg.enabled ? 'Disable' : 'Enable'}>
                <input
                  type="checkbox"
                  checked={intg.enabled}
                  onchange={() => toggleEnabled(intg)}
                />
                <span class="toggle-track"></span>
              </label>
              <!-- Send test message to confirm the bot token + default chat work. -->
              <button
                class="btn sm"
                title="Send 'Otto is connected ✅' to the default chat"
                disabled={testBusy === channel || !intg.channel_id}
                onclick={() => void sendTest(channel)}
              >
                {testBusy === channel ? 'Sending…' : 'Test'}
              </button>
            {/if}
            <button class="btn sm" onclick={() => openEdit(channel)}>Edit</button>
            {#if intg?.has_bot_token}
              <button class="icon-btn" title="Remove integration" onclick={() => remove(channel)}>
                <Icon name="trash" size={13} />
              </button>
            {/if}
          </div>
        </div>
      {/snippet}

      {@render channelCard('slack', slack, 'slack', 'Slack')}
      {@render channelCard('telegram', telegram, 'bell', 'Telegram')}
    </div>
  {/if}
</div>

<!-- Edit modal -->
{#if editOpen}
  <Modal
    title={`Configure ${editChannel === 'slack' ? 'Slack' : 'Telegram'}`}
    width={500}
    onclose={() => (editOpen = false)}
  >
    <!-- Enabled -->
    <div class="field field-row">
      <label for="ch-enabled">Enabled</label>
      <input id="ch-enabled" type="checkbox" bind:checked={fEnabled} />
    </div>

    <!-- Bot token -->
    <div class="field">
      <label for="ch-bot">Bot token</label>
      <input
        id="ch-bot"
        class="input mono"
        type="password"
        bind:value={fBotToken}
        autocomplete="off"
        placeholder={integrations.find((i) => i.channel === editChannel)?.has_bot_token
          ? '•••••• (leave blank to keep)'
          : editChannel === 'slack'
            ? 'xoxb-…'
            : '123456:ABC…'}
      />
      {#if integrations.find((i) => i.channel === editChannel)?.has_bot_token}
        <span class="hint">Leave blank to keep the existing token.</span>
      {/if}
    </div>

    <!-- App token (Slack only) -->
    {#if editChannel === 'slack'}
      <div class="field">
        <label for="ch-app">App token <span class="dim">(Socket Mode)</span></label>
        <input
          id="ch-app"
          class="input mono"
          type="password"
          bind:value={fAppToken}
          autocomplete="off"
          placeholder={integrations.find((i) => i.channel === 'slack')?.has_app_token
            ? '•••••• (leave blank to keep)'
            : 'xapp-…'}
        />
        {#if integrations.find((i) => i.channel === 'slack')?.has_app_token}
          <span class="hint">Leave blank to keep the existing app-level token.</span>
        {:else}
          <span class="hint">Socket Mode app-level token; leave blank to keep existing.</span>
        {/if}
      </div>
    {/if}

    <!-- Channel / chat ID -->
    <div class="field">
      <label for="ch-chid">
        {editChannel === 'slack' ? 'Default channel ID' : 'Default chat ID'}
      </label>
      <input
        id="ch-chid"
        class="input mono"
        bind:value={fChannelId}
        spellcheck="false"
        autocomplete="off"
        placeholder={editChannel === 'slack' ? 'C0123…' : '-100123456…'}
      />
    </div>

    <!-- Allowed users -->
    <div class="field">
      <label for="ch-users">Allowed users</label>
      <input
        id="ch-users"
        class="input"
        bind:value={fAllowedUsers}
        spellcheck="false"
        autocomplete="off"
        placeholder="U01234,U05678"
      />
      <span class="hint">
        Comma-separated {editChannel === 'slack' ? 'Slack' : 'Telegram'} user IDs.
        Leave blank to allow everyone.
      </span>
    </div>

    <!-- Preferred CLI -->
    <div class="field">
      <label for="ch-cli">Preferred CLI</label>
      <select id="ch-cli" class="input" bind:value={fPreferredCli}>
        <option value="">Use default agent</option>
        {#each providers as p (p)}
          <option value={p}>{p}</option>
        {/each}
      </select>
      <span class="hint">
        Agent CLI spawned for replies in this channel. Defaults to the workspace
        default agent.
      </span>
    </div>

    <!-- Agent reply -->
    <div class="field field-row">
      <label for="ch-agent">Agent posts the final reply itself</label>
      <input id="ch-agent" type="checkbox" bind:checked={fAgentReply} />
    </div>

    <!-- Reply instructions (only when agent reply on) -->
    {#if fAgentReply}
      <div class="field">
        <label for="ch-reply">Reply instructions</label>
        <textarea
          id="ch-reply"
          class="input"
          rows={4}
          bind:value={fReplyInstructions}
          placeholder="You are a helpful assistant replying on behalf of the Otto agent. Be concise and direct."
        ></textarea>
      </div>
    {/if}

    {#snippet footer()}
      <button class="btn" onclick={() => (editOpen = false)}>Cancel</button>
      <button class="btn primary" disabled={editBusy} onclick={save}>
        {editBusy ? 'Saving…' : 'Save'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .channel-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 560px;
  }

  .channel-card {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 14px 16px;
  }

  .ch-icon {
    width: 32px;
    height: 32px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }

  .ch-label {
    font-size: 13px;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .ch-status {
    font-size: 11.5px;
    margin-top: 2px;
  }

  .ch-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }

  .btn.sm {
    font-size: 11.5px;
    height: 24px;
    padding: 0 10px;
  }

  /* Toggle switch */
  .toggle {
    display: flex;
    align-items: center;
    cursor: pointer;
    position: relative;
  }

  .toggle input {
    position: absolute;
    opacity: 0;
    width: 0;
    height: 0;
  }

  .toggle-track {
    width: 30px;
    height: 17px;
    border-radius: 9px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    position: relative;
    transition: background 140ms ease-out;
  }

  .toggle-track::after {
    content: '';
    position: absolute;
    top: 2px;
    inset-inline-start: 2px;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: var(--text-dim);
    transition: transform 140ms ease-out, background 140ms ease-out;
  }

  .toggle input:checked ~ .toggle-track {
    background: var(--accent);
    border-color: var(--accent);
  }

  .toggle input:checked ~ .toggle-track::after {
    transform: translateX(13px);
    background: #fff;
  }

  /* Inline label+checkbox row */
  .field-row {
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
  }

  .field-row label {
    margin-bottom: 0;
  }

  textarea.input {
    resize: vertical;
    font-family: inherit;
    line-height: 1.5;
  }
</style>
