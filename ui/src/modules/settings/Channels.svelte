<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import SectionIntro from './SectionIntro.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Channels settings page: per-workspace Slack + Telegram + Webhook integration config.
  import { api, baseUrl } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { Channel, Integration, UpsertIntegrationReq } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { confirmOutward } from '../../lib/confirmOutward';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { agentProviders } from '../../lib/providers';
  import { copyTextOrThrow } from '../../lib/clipboard';

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let integrations: Integration[] = $state([]);
  let loading = $state(false);
  let loadError = $state('');
  let testBusy: Channel | null = $state(null); // which channel is mid-test

  // Agent CLIs offered in the per-channel picker: a channel reply needs a
  // reasoning agent, so exclude the `shell` pseudo-provider (from /meta).
  const providers = $derived(agentProviders());

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
  const webhook = $derived(integrations.find((i) => i.channel === 'webhook') ?? null);

  // Human label for a channel kind (used in toasts, headings, confirm dialogs).
  const CHANNEL_LABELS: Record<Channel, string> = {
    slack: 'Slack',
    telegram: 'Telegram',
    webhook: 'Webhook',
  };
  const channelLabel = (c: Channel): string => CHANNEL_LABELS[c];

  // Agent CLI ids → the names people know them by.
  const CLI_NAMES: Record<string, string> = { claude: 'Claude Code', codex: 'Codex', agy: 'Gemini (agy)' };
  const cliName = (p: string): string => CLI_NAMES[p] ?? p.charAt(0).toUpperCase() + p.slice(1);

  // A first-time setup needs the secret: without one the row stays "Not set
  // up" after Save, which read as a silent failure.
  const needsSecret = $derived(
    editOpen && !integrations.find((i) => i.channel === editChannel)?.has_bot_token && fBotToken.trim() === '',
  );

  // The inbound URL an external system POSTs to (host = this Otto daemon).
  const webhookUrl = $derived(wsId ? `${baseUrl()}/api/v1/webhooks/${wsId}` : '');

  // Fill the key field with a fresh random secret. Shown once — copy it before
  // saving; after reload it's masked (the key lives only in the Keychain).
  function generateKey(): void {
    const bytes = new Uint8Array(24);
    crypto.getRandomValues(bytes);
    fBotToken = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
  }

  async function copyText(text: string): Promise<void> {
    if (!text) return;
    try {
      await copyTextOrThrow(text);
      toasts.success('Copied to the clipboard');
    } catch {
      toasts.error("Couldn't copy", 'Select the text and copy it manually.');
    }
  }

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
    loadError = '';
    try {
      integrations = await api.get<Integration[]>(`/workspaces/${id}/integrations`);
    } catch (e) {
      // Inline, not a toast: the cards would otherwise all read "Not
      // configured" — one Save away from overwriting a real integration.
      loadError = loadErrorText(e);
      integrations = [];
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
        `${channelLabel(editChannel)} integration saved`,
        updated.enabled ? 'Enabled' : 'Saved as disabled',
      );
    } catch (e) {
      toasts.error(`Couldn't save the ${channelLabel(editChannel)} integration`, loadErrorText(e));
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
    } catch (e) {
      toasts.error(`Couldn't ${intg.enabled ? 'disable' : 'enable'} ${channelLabel(intg.channel)}`, loadErrorText(e));
    }
  }

  // ---------------------------------------------------------------------------
  // Delete
  // ---------------------------------------------------------------------------

  async function remove(channel: Channel): Promise<void> {
    if (!wsId) return;
    const label = channelLabel(channel);
    const secretWord = channel === 'webhook' ? 'The key' : 'Tokens';
    if (!(await confirmer.ask(`Remove the ${label} integration? ${secretWord} will be deleted from the Keychain.`, { title: 'Remove integration', confirmLabel: 'Remove' }))) return;
    try {
      await api.del(`/workspaces/${wsId}/integrations/${channel}`);
      integrations = integrations.filter((i) => i.channel !== channel);
      toasts.success(`${label} integration removed`);
    } catch (e) {
      toasts.error(`Couldn't remove the ${label} integration`, loadErrorText(e));
    }
  }

  // ---------------------------------------------------------------------------
  // Send test message
  // ---------------------------------------------------------------------------

  async function sendTest(channel: Channel, intg: Integration): Promise<void> {
    if (!wsId) return;
    // Outward-facing: this posts to a real chat (or a real callback URL).
    const where =
      channel === 'webhook'
        ? `The reply callback URL ${intg.channel_id}`
        : `${channelLabel(channel)} ${channel === 'slack' ? 'channel' : 'chat'} ${intg.channel_id}`;
    if (
      !(await confirmOutward({
        verb: 'Send test message',
        where,
        what: '“Otto is connected ✅”',
        who: channel === 'webhook' ? 'Whatever system listens on that URL.' : 'Everyone in that chat.',
      }))
    )
      return;
    testBusy = channel;
    try {
      const resp = await api.post<{ ok: boolean; error?: string | null }>(
        `/workspaces/${wsId}/integrations/${channel}/test`,
        {},
      );
      if (resp.ok) {
        toasts.success('Test message sent', `Posted to the default ${channel === 'webhook' ? 'callback URL' : 'chat'}.`);
      } else {
        toasts.error("Couldn't send the test message", resp.error ?? 'The provider gave no reason.');
      }
    } catch (e) {
      toasts.error("Couldn't send the test message", loadErrorText(e));
    } finally {
      testBusy = null;
    }
  }

  // ---------------------------------------------------------------------------
  // Status line helpers
  // ---------------------------------------------------------------------------

  function statusLine(intg: Integration | null, channel: Channel): string {
    if (!intg || !intg.has_bot_token) return 'Not set up';
    const target = intg.channel_id
      ? `${channel === 'webhook' ? 'replies to' : 'default'} ${intg.channel_id}`
      : channel === 'webhook'
        ? 'no reply URL'
        : 'no default chat';
    const line = `${target} · ${intg.agent_reply ? 'the agent posts replies' : 'Otto posts replies'}`;
    return line.charAt(0).toUpperCase() + line.slice(1);
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('channels')} subtitle="Slack, Telegram and webhook bridges" />
  <PageBody width="readable">
  <SectionIntro>Configure Slack, Telegram and inbound webhook integrations per workspace. Tokens and webhook keys are stored in the <strong>macOS Keychain</strong>, never in Otto’s database.</SectionIntro>

  {#if !wsId}
    <!-- No workspace selected -->
    <EmptyState
      icon="plug"
      title="Select a workspace first"
      body="Integrations are per-workspace. Choose a workspace from the sidebar to configure channels."
    />
  {:else}
    <LoadState what="this workspace's channels" {loading} error={loadError} empty={!!loadError || (loading && integrations.length === 0)} onretry={() => wsId && void load(wsId)} rows={3}>
    <div class="channel-list">
      {#snippet channelCard(channel: Channel, intg: Integration | null, icon: IconName, label: string)}
        {@const configured = !!intg?.has_bot_token}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="channel-card card"
          class:off={configured && !intg?.enabled}
          oncontextmenu={(e) => ctxMenu.show(e, [
            { label: configured ? 'Edit…' : 'Set up…', icon: 'edit', action: () => openEdit(channel) },
            ...(intg && configured ? [
              { label: intg.enabled ? 'Disable' : 'Enable', icon: intg.enabled ? 'eyeOff' : 'eye', action: () => toggleEnabled(intg) },
              { separator: true },
              { label: 'Remove…', icon: 'trash', danger: true as const, action: () => remove(channel) },
            ] as const : []),
          ])}
        >
          <div class="ch-icon">
            <Icon name={icon} size={16} />
          </div>
          <div class="grow">
            <div class="ch-label">{label}</div>
            <div class="ch-status" title={statusLine(intg, channel)}>{statusLine(intg, channel)}</div>
          </div>
          <div class="ch-actions">
            {#if intg && configured}
              <label class="checkbox-row ch-enabled" title={intg.enabled ? `Stop the ${label} bridge` : `Start the ${label} bridge`}>
                <input
                  type="checkbox"
                  checked={intg.enabled}
                  onchange={async (e) => {
                    const el = e.currentTarget;
                    await toggleEnabled(intg);
                    // A failed save leaves the saved value — show it, not the click.
                    el.checked = integrations.find((i) => i.channel === channel)?.enabled ?? false;
                  }}
                />
                Enabled
              </label>
              <!-- Send a test message to confirm the token + default chat work. -->
              <button
                class="btn small ch-tool"
                title={intg.channel_id ? `Send “Otto is connected ✅” to ${intg.channel_id}` : `Set a default ${channel === 'webhook' ? 'reply URL' : 'chat'} first (Edit)`}
                disabled={testBusy === channel || !intg.channel_id}
                onclick={() => void sendTest(channel, intg)}
              >
                {testBusy === channel ? 'Sending…' : 'Test…'}
              </button>
            {/if}
            {#if configured}
              <button class="icon-btn ch-tool" title="Edit {label} integration" aria-label="Edit {label} integration" onclick={() => openEdit(channel)}>
                <Icon name="edit" size={14} />
              </button>
              <button class="icon-btn ch-tool" title="Remove {label} integration" aria-label="Remove {label} integration" onclick={() => remove(channel)}>
                <Icon name="trash" size={14} />
              </button>
            {:else}
              <button class="btn small ch-tool" onclick={() => openEdit(channel)}>Set up…</button>
            {/if}
          </div>
        </div>
      {/snippet}

      {@render channelCard('slack', slack, 'slack', 'Slack')}
      {@render channelCard('telegram', telegram, 'send', 'Telegram')}
      {@render channelCard('webhook', webhook, 'link', 'Webhook')}
    </div>
    </LoadState>
  {/if}
  </PageBody>
</div>

<!-- Edit modal -->
{#if editOpen}
  <Modal
    title={`Configure ${channelLabel(editChannel)}`}
    width={500}
    onclose={() => (editOpen = false)}
  >
    <!-- Enabled -->
    <label class="checkbox-row modal-check">
      <input id="ch-enabled" type="checkbox" bind:checked={fEnabled} />
      Enabled
    </label>

    {#if editChannel === 'webhook'}
      <!-- Inbound URL (read-only) — what the external system POSTs to. -->
      <div class="field">
        <label for="ch-url">Inbound URL</label>
        <div class="row-inline">
          <input id="ch-url" class="input mono inline-field" value={webhookUrl} readonly spellcheck="false" />
          <button class="btn small inline-btn" type="button" onclick={() => void copyText(webhookUrl)}>Copy</button>
        </div>
        <span class="hint">
          <code>POST</code> here with header <code>X-Otto-Webhook-Key: &lt;key&gt;</code> and a JSON
          body <code>{'{ "text": "…" }'}</code>. Reachable wherever this Otto daemon is — by default
          loopback only (<code>127.0.0.1</code>); expose it yourself if you need remote calls.
        </span>
      </div>
    {/if}

    <!-- Bot token / Webhook key -->
    <div class="field">
      <label for="ch-bot">{editChannel === 'webhook' ? 'Webhook key' : 'Bot token'}</label>
      <div class="row-inline">
        <input
          id="ch-bot"
          class="input mono inline-field"
          type={editChannel === 'webhook' ? 'text' : 'password'}
          bind:value={fBotToken}
          autocomplete="off"
          spellcheck="false"
          placeholder={integrations.find((i) => i.channel === editChannel)?.has_bot_token
            ? '•••••• (leave blank to keep)'
            : editChannel === 'slack'
              ? 'xoxb-…'
              : editChannel === 'telegram'
                ? '123456:ABC…'
                : 'paste a key or click Generate'}
        />
        {#if editChannel === 'webhook'}
          <button class="btn small inline-btn" type="button" onclick={generateKey}>Generate</button>
          <button class="btn small inline-btn" type="button" disabled={!fBotToken} onclick={() => void copyText(fBotToken)}>Copy</button>
        {/if}
      </div>
      {#if editChannel === 'webhook'}
        <span class="hint">
          The secret callers must send. Set your own or Generate one — copy it now, it's masked
          after save (stored only in the Keychain). Leave blank to keep the existing key.
        </span>
      {:else if integrations.find((i) => i.channel === editChannel)?.has_bot_token}
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

    <!-- Channel / chat ID  (webhook: default reply callback URL) -->
    <div class="field">
      <label for="ch-chid">
        {editChannel === 'slack'
          ? 'Default channel ID'
          : editChannel === 'telegram'
            ? 'Default chat ID'
            : 'Default reply callback URL'}
      </label>
      <input
        id="ch-chid"
        class="input mono"
        bind:value={fChannelId}
        spellcheck="false"
        autocomplete="off"
        placeholder={editChannel === 'slack'
          ? 'C0123…'
          : editChannel === 'telegram'
            ? '-100123456…'
            : 'https://example.com/otto-replies (optional)'}
      />
      {#if editChannel === 'webhook'}
        <span class="hint">
          Where the agent's reply is POSTed. A request may override it with <code>callback_url</code>.
          Leave blank for fire-and-forget (trigger only, no reply delivered).
        </span>
      {/if}
    </div>

    <!-- Allowed users -->
    <div class="field">
      <label for="ch-users">{editChannel === 'webhook' ? 'Allowed callers' : 'Allowed users'}</label>
      <input
        id="ch-users"
        class="input"
        bind:value={fAllowedUsers}
        spellcheck="false"
        autocomplete="off"
        placeholder={editChannel === 'webhook' ? 'ci-bot,deploy-bot' : 'U01234,U05678'}
      />
      <span class="hint">
        {#if editChannel === 'webhook'}
          Comma-separated caller ids, matched against the <code>user</code> field in the POST body.
          Leave blank to allow everyone.
        {:else}
          Comma-separated {channelLabel(editChannel)} user IDs. Leave blank to allow everyone.
        {/if}
      </span>
    </div>

    <!-- Preferred CLI -->
    <div class="field">
      <label for="ch-cli">Preferred CLI</label>
      <select id="ch-cli" class="input" bind:value={fPreferredCli}>
        <option value="">Use default agent</option>
        {#each providers as p (p)}
          <option value={p}>{cliName(p)}</option>
        {/each}
      </select>
      <span class="hint">
        Agent CLI spawned for replies in this channel. Defaults to the workspace
        default agent.
      </span>
    </div>

    <!-- Agent reply -->
    <label class="checkbox-row modal-check">
      <input id="ch-agent" type="checkbox" bind:checked={fAgentReply} />
      {editChannel === 'webhook'
        ? 'Relay only the agent\'s marked reply (⟦otto-send⟧)'
        : 'Agent posts the final reply itself'}
    </label>

    <!-- Reply instructions (only when agent reply on) -->
    {#if fAgentReply}
      <div class="field">
        <label for="ch-reply">Reply instructions</label>
        <textarea
          id="ch-reply"
          class="input reply-area"
          rows={4}
          bind:value={fReplyInstructions}
          placeholder="You are a helpful assistant replying on behalf of the Otto agent. Be concise and direct."
        ></textarea>
      </div>
    {/if}

    {#snippet footer()}
      <button class="btn" onclick={() => (editOpen = false)}>Cancel</button>
      <button
        class="btn primary"
        disabled={editBusy || needsSecret}
        title={needsSecret ? `Enter the ${editChannel === 'webhook' ? 'webhook key' : 'bot token'} first` : undefined}
        onclick={save}
      >
        {editBusy ? 'Saving…' : 'Save'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .channel-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: var(--settings-col);
  }
  .channel-card {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
  }
  .channel-card .grow {
    min-width: 0;
  }
  .ch-icon {
    width: 32px;
    height: 32px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .ch-label {
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .ch-status {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .channel-card.off .ch-label,
  .channel-card.off .ch-icon {
    opacity: 0.6;
  }
  .ch-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .ch-enabled {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin-inline-end: 4px;
    cursor: pointer;
  }
  .checkbox-row input {
    width: 15px;
    height: 15px;
    margin: 0;
    accent-color: var(--accent);
  }
  .modal-check {
    margin-bottom: 12px;
    cursor: pointer;
  }
  .reply-area {
    resize: vertical;
    font-family: inherit;
    line-height: 1.5;
  }
  /* Input + inline action button(s) on one row (webhook URL/key fields). */
  .row-inline {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .inline-field {
    flex: 1 1 auto;
    min-width: 0;
  }
  .inline-btn {
    flex-shrink: 0;
  }
  .hint code {
    font-size: var(--fs-xs);
    padding: 1px 4px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }

  /* Phone: the actions wrap under the name instead of squeezing it. */
  @media (max-width: 640px) {
    .channel-card {
      flex-wrap: wrap;
    }
    .ch-actions {
      width: 100%;
      justify-content: flex-end;
    }
    .ch-tool {
      min-height: 36px;
    }
  }
</style>
