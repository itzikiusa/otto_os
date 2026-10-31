///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
import { announceShown } from '../../lib/announce.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { NO_WORKSPACE } from '../../lib/labels';
import { sectionLabel } from './sections';
import SectionIntro from './SectionIntro.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
// Channels settings page: per-workspace Slack + Telegram + Webhook integration config.
import { api, baseUrl } from '../../lib/api/client';
import { auth } from '../../lib/stores/auth.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type { Channel, Integration, ListenerStatus, RejectedSender, SlackDirectory, UpsertIntegrationReq } from '../../lib/api/types';
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
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import { pollWhileVisible, type Poller } from '../../lib/poll';
import { latestOnly } from '../../lib/latest';
import type { Tone } from '../../lib/status';
import AllowedIdsEditor from './AllowedIdsEditor.svelte';
import { pendingRejected, senderLabel, withAllowed } from './channelAllow';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let integrations: Integration[] = $state([]);
  // Slack names for the ids on the card and in the editor (members +
  // channels via the bot token). Null until loaded; ids show bare meanwhile.
  let slackDir = $state<SlackDirectory | null>(null);
  let slackDirLoading = $state(false);
  let slackDirError = $state('');
  const directoryLoads = latestOnly();
  const slackUser = $derived(new Map((slackDir?.users ?? []).map((u) => [u.id, u])));
  const slackChannel = $derived(new Map((slackDir?.channels ?? []).map((c) => [c.id, c])));
  async function loadSlackDir(id: string, refresh = false): Promise<void> {
    const ticket = directoryLoads.begin();
    slackDirLoading = true;
    slackDirError = '';
    try {
      const d = await api.get<SlackDirectory>(
        `/workspaces/${id}/integrations/slack/directory${refresh ? '?refresh=1' : ''}`,
      );
      if (ticket.current && wsId === id) slackDir = d;
    } catch (e) {
      // Manual IDs still work; make the name lookup failure recoverable.
      if (ticket.current && wsId === id) slackDirError = loadErrorText(e);
    } finally {
      if (ticket.current && wsId === id) slackDirLoading = false;
    }
  }
  /** "#general" for a known Slack channel id, else the id. */
  function slackChannelLabel(id: string): string {
    const c = slackChannel.get(id.trim());
    return c ? `#${c.name}` : id;
  }
  function pickSlackChannel(e: MouseEvent | KeyboardEvent): void {
    const items = (slackDir?.channels ?? [])
      .slice()
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((c) => ({
        label: `#${c.name}`,
        hint: c.is_private ? 'private' : undefined,
        title: c.id,
        checked: fChannelId.trim() === c.id,
        action: () => (fChannelId = c.id),
      }));
    ctxMenu.show(e, items, { filter: true, filterPlaceholder: 'Search channels…', maxVisible: 12 });
  }
  let loading = $state(false);
  let loadError = $state('');
  let testBusy: Channel | null = $state(null); // which channel is mid-test
  // Live listener health (GET …/integrations/status) — is the bot actually
  // connected, or silently reconnecting / rejected? Polled while visible.
  let statuses: ListenerStatus[] = $state([]);
  let statusPoll: Poller | null = null;

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
  /** Explicit "open to everyone" opt-in for a blank allow-list (otherwise a
   *  blank list admits nobody — the daemon fails closed). */
  let fOpenToAll = $state(false);
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
  const needsAllowedUser = $derived(
    editOpen && (editChannel as Channel) !== 'webhook' && fEnabled && fAllowedUsers.trim() === '' && !fOpenToAll,
  );
  let showWebhookKey = $state(false);

  // The inbound URL an external system POSTs to (host = this Otto daemon).
  const webhookUrl = $derived(wsId ? `${baseUrl()}/api/v1/webhooks/${wsId}` : '');

  // Fill the key field with a fresh random secret. Shown once — copy it before
  // saving; after reload it's masked (the key lives only in the Keychain).
  function generateKey(): void {
    const bytes = new Uint8Array(24);
    crypto.getRandomValues(bytes);
    fBotToken = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
    showWebhookKey = true;
  }

  async function copyNotify(text: string): Promise<void> {
    if (!text) return;
    try {
      await copyTextOrThrow(text);
      toasts.success('Copied to the clipboard');
    } catch {
      toasts.error("Couldn’t copy", 'Select the text and copy it manually.');
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

  $effect(() => {
    const id = wsId;
    if (!id) return;
    statuses = [];
    const p = pollWhileVisible(async (signal) => {
      const list = await api.get<ListenerStatus[]>(`/workspaces/${id}/integrations/status`, signal);
      if (wsId === id) statuses = list;
    }, { ms: 10_000 });
    statusPoll = p;
    return () => {
      p.stop();
      if (statusPoll === p) statusPoll = null;
    };
  });

  /** Re-read health soon after a config change (the daemon's supervisor
   *  respawns listeners on its ~15 s rescan, so read again a bit later too). */
  function refreshStatusSoon(): void {
    statusPoll?.now({ background: true });
    setTimeout(() => statusPoll?.now({ background: true }), 16_000);
  }

  // Generation guard + owner: after a workspace switch the previous
  // workspace's cards used to stay rendered (and clickable) during the reload —
  // toggling one PUT A's channel/allow-list/agent settings into B — and a slow
  // response for A could land over B's.
  const loads = latestOnly();
  let integrationsFor = '';
  async function load(id: string): Promise<void> {
    const t = loads.begin();
    if (integrationsFor !== id) {
      integrations = [];
      integrationsFor = id;
      directoryLoads.cancel();
      slackDir = null;
      slackDirError = '';
      slackDirLoading = false;
    }
    loading = true;
    loadError = '';
    try {
      const rows = await api.get<Integration[]>(`/workspaces/${id}/integrations`);
      if (!t.current || wsId !== id) return;
      integrations = rows;
      slackDir = null;
      if (rows.some((i) => i.channel === 'slack' && i.has_bot_token)) void loadSlackDir(id);
    } catch (e) {
      if (!t.current || wsId !== id) return;
      // Inline, not a toast: the cards would otherwise all read "Not
      // configured" — one Save away from overwriting a real integration.
      loadError = loadErrorText(e);
      integrations = [];
    } finally {
      if (t.current) loading = false;
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
    showWebhookKey = false;
    fChannelId = existing?.channel_id ?? '';
    fAllowedUsers = existing?.allowed_users ?? '';
    fAgentReply = existing?.agent_reply ?? false;
    fOpenToAll = existing?.open_to_all ?? false;
    fReplyInstructions = existing?.reply_instructions ?? '';
    fPreferredCli = existing?.preferred_cli ?? '';
    editOpen = true;
    if (channel === 'slack' && existing?.has_bot_token && wsId && !slackDir && !slackDirLoading && !slackDirError) void loadSlackDir(wsId);
  }

  // ---------------------------------------------------------------------------
  // Save
  // ---------------------------------------------------------------------------

  async function save(): Promise<void> {
    if (!wsId) return;
    const blank = fAllowedUsers.trim() === '';
    const listened = editChannel !== 'webhook';
    // The field error is inline under Allowed users (`needsAllowedUser`), and
    // Save is disabled while it shows — never a toast over an open form.
    if (listened && blank && fEnabled && !fOpenToAll) return;
    const wasOpen = integrations.find((i) => i.channel === editChannel)?.open_to_all ?? false;
    if (listened && blank && fOpenToAll && !wasOpen) {
      const ok = await confirmer.ask(
        `Anyone who can message this ${channelLabel(editChannel)} bot will be able to run an agent on this Mac as you — reading code and running commands — and receive its output.`,
        { title: 'Open the bot to everyone?', confirmLabel: 'Open to everyone', danger: true },
      );
      if (!ok) return;
    }
    editBusy = true;
    try {
      const body: UpsertIntegrationReq = {
        enabled: fEnabled,
        allowed_users: fAllowedUsers.trim(),
        open_to_all: listened && blank && fOpenToAll,
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
      // A new Slack bot token may see other people / channels: reload names.
      if (editChannel === 'slack' && fBotToken !== '') void loadSlackDir(wsId, true);
      integrations = [
        ...integrations.filter((i) => i.channel !== editChannel),
        updated,
      ];
      editOpen = false;
      refreshStatusSoon();
      toasts.success(
        `${channelLabel(editChannel)} integration saved`,
        updated.enabled ? 'Enabled' : 'Saved as disabled',
      );
    } catch (e) {
      toasts.error(`Couldn’t save the ${channelLabel(editChannel)} integration`, loadErrorText(e));
    } finally {
      editBusy = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Toggle enabled (quick toggle from card — keeps all existing values)
  // ---------------------------------------------------------------------------

  async function toggleEnabled(intg: Integration): Promise<void> {
    if (!wsId) return;
    if (!intg.enabled && intg.channel !== 'webhook' && intg.allowed_users.trim() === '' && !intg.open_to_all) {
      // Fail closed: enabling needs an allow-list (or the explicit opt-in).
      toasts.error(
        `Add an allowed user before enabling ${channelLabel(intg.channel)}`,
        'A blank list admits nobody.',
      );
      openEdit(intg.channel);
      return;
    }
    try {
      const body: UpsertIntegrationReq = {
        enabled: !intg.enabled,
        allowed_users: intg.allowed_users,
        open_to_all: intg.open_to_all,
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
      refreshStatusSoon();
    } catch (e) {
      toasts.error(`Couldn’t ${intg.enabled ? 'disable' : 'enable'} ${channelLabel(intg.channel)}`, loadErrorText(e));
    }
  }

  // ---------------------------------------------------------------------------
  // Allow a turned-away sender (S5-308) — the Telegram id the owner can't see
  // in the app, so the bot can be restricted instead of opened to everyone.
  // ---------------------------------------------------------------------------

  /** The sender whose "Allow" save is in flight (that button disables). */
  let allowing = $state<string | null>(null);

  async function allowSender(intg: Integration, r: RejectedSender): Promise<void> {
    if (!wsId || allowing) return;
    const label = channelLabel(intg.channel);
    const ok = await confirmer.ask(
      `${senderLabel(r)} will be able to message the ${label} bot and run an agent on this Mac as you — reading code and running commands — and receive its output.`,
      { title: `Allow ${senderLabel(r)}?`, confirmLabel: 'Allow', danger: true },
    );
    if (!ok) return;
    allowing = r.user;
    try {
      const body: UpsertIntegrationReq = {
        enabled: intg.enabled,
        allowed_users: withAllowed(intg.allowed_users, r.user),
        // A non-blank list replaces any "open to everyone" opt-in.
        open_to_all: false,
        agent_reply: intg.agent_reply,
        reply_instructions: intg.reply_instructions,
        channel_id: intg.channel_id,
        preferred_cli: intg.preferred_cli,
        bot_token: null, // keep existing
        ...(intg.channel === 'slack' ? { app_token: null } : {}),
      };
      const updated = await api.put<Integration>(`/workspaces/${wsId}/integrations/${intg.channel}`, body);
      integrations = integrations.map((i) => (i.channel === updated.channel ? updated : i));
      refreshStatusSoon();
      toasts.success(`${senderLabel(r)} can now message the ${label} bot`);
    } catch (e) {
      toasts.error(`Couldn’t allow ${senderLabel(r)}`, loadErrorText(e));
    } finally {
      allowing = null;
    }
  }

  // ---------------------------------------------------------------------------
  // Delete
  // ---------------------------------------------------------------------------

  /** Integrations whose removal is in flight, by channel — that row's button disables. */
  let removing = $state<Partial<Record<Channel, boolean>>>({});

  async function remove(channel: Channel): Promise<void> {
    if (!wsId || removing[channel]) return;
    const label = channelLabel(channel);
    const secretWord = channel === 'webhook' ? 'The key' : 'Tokens';
    if (!(await confirmer.ask(`Remove the ${label} integration? ${secretWord} will be deleted from the Keychain.`, { title: 'Remove integration?', confirmLabel: 'Remove', danger: true }))) return;
    removing = { ...removing, [channel]: true };
    try {
      await api.del(`/workspaces/${wsId}/integrations/${channel}`);
      integrations = integrations.filter((i) => i.channel !== channel);
      toasts.success(`${label} integration removed`);
    } catch (e) {
      toasts.error(`Couldn’t remove the ${label} integration`, loadErrorText(e));
    } finally {
      const next = { ...removing };
      delete next[channel];
      removing = next;
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
        toasts.error("Couldn’t send the test message", resp.error ?? 'The provider gave no reason.');
      }
    } catch (e) {
      toasts.error("Couldn’t send the test message", loadErrorText(e));
    } finally {
      testBusy = null;
    }
  }

  // ---------------------------------------------------------------------------
  // Status line helpers
  // ---------------------------------------------------------------------------

  interface Health {
    tone: Tone;
    label: string;
    /** Why — shown under the card for anything that needs attention. */
    detail?: string;
    since?: string;
    lastEvent?: string;
  }

  /** The listener-health badge for an ENABLED Slack/Telegram card (webhooks
   *  have no listener; a disabled card shows nothing). */
  function health(intg: Integration | null, channel: Channel): Health | null {
    if (!intg?.enabled || !intg.has_bot_token || channel === 'webhook') return null;
    const st = statuses.find((x) => x.channel === channel);
    if (!st) return { tone: 'neutral', label: 'Starting…' };
    const base = { since: st.since, lastEvent: st.last_event_at };
    switch (st.state) {
      case 'connected':
        return { ...base, tone: 'success', label: 'Connected' };
      case 'connecting':
        return { ...base, tone: 'info', label: 'Connecting…' };
      case 'reconnecting':
        return { ...base, tone: 'warning', label: 'Reconnecting', detail: st.detail ?? st.last_error };
      case 'failing':
        return { ...base, tone: 'danger', label: 'Not connected', detail: st.detail ?? st.last_error };
      case 'waiting_for_token':
        return { ...base, tone: 'warning', label: 'Waiting for token', detail: st.detail };
      case 'conflict':
        return { ...base, tone: 'danger', label: 'Token in use', detail: st.detail };
    }
  }

  /** Blank allowed users under the explicit opt-in = anyone who can message
   *  the bot drives an agent on this Mac. Webhooks are key-protected, so a
   *  blank caller list is fine. */
  function openToEveryone(intg: Integration | null, channel: Channel): boolean {
    return !!intg?.enabled && channel !== 'webhook' && intg.open_to_all && intg.allowed_users.trim() === '';
  }

  /** Blank allowed users WITHOUT the opt-in: the daemon admits nobody. */
  function admitsNobody(intg: Integration | null, channel: Channel): boolean {
    return !!intg?.enabled && channel !== 'webhook' && !intg.open_to_all && intg.allowed_users.trim() === '';
  }

  function statusLine(intg: Integration | null, channel: Channel): string {
    if (!intg || !intg.has_bot_token) return 'Not set up';
    const target = intg.channel_id
      ? `${channel === 'webhook' ? 'replies to' : 'default'} ${channel === 'slack' ? slackChannelLabel(intg.channel_id) : intg.channel_id}`
      : channel === 'webhook'
        ? 'no reply URL'
        : 'no default chat';
    const line = `${target} · ${intg.agent_reply ? 'the agent posts replies' : 'Otto posts replies'}`;
    return line.charAt(0).toUpperCase() + line.slice(1);
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('channels'),"subtitle":`Slack, Telegram and webhook bridges`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});                  { svelteHTML.createElement("strong", {});  }     SectionIntro}

  if(!wsId){
    
     { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {       "icon":`plug`,"title":NO_WORKSPACE,"body":`Integrations are per-workspace. Choose a workspace from the sidebar to configure channels.`,}});}
  }else{
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`this workspace's channels`,loading,"error":loadError,"empty":!!loadError || (loading && integrations.length === 0),"onretry":() => wsId && void load(wsId),"rows":3,}});
     { svelteHTML.createElement("div", { "class":`channel-list`,});
       const channelCard/*Ωignore_positionΩ*/ = (channel: Channel, intg: Integration | null, icon: IconName, label: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
        const configured = !!intg?.has_bot_token;
        const h = health(intg, channel);
        
         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {         "role":`presentation`,"class":`channel-card card`,"oncontextmenu":(e) => ctxMenu.show(e, [
            { label: configured ? 'Edit…' : 'Set up…', icon: 'edit', action: () => openEdit(channel) },
            ...(intg && configured ? [
              { label: intg.enabled ? 'Disable' : 'Enable', icon: intg.enabled ? 'eyeOff' : 'eye', action: () => toggleEnabled(intg) },
              { separator: true },
              { label: 'Remove…', icon: 'trash', danger: true as const, action: () => remove(channel) },
            ] as const : []),
          ]),});configured && !intg?.enabled;
           { svelteHTML.createElement("div", { "class":`ch-icon`,});
             { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":icon,"size":16,}});}
           }
           { svelteHTML.createElement("div", { "class":`grow`,});
             { svelteHTML.createElement("div", { "class":`ch-label-row`,});
               { svelteHTML.createElement("span", { "class":`ch-label`,});label; }
              if(h){
                 { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {        "tone":h.tone,"label":h.label,"testid":`channel-health-${channel}`,"title":h.detail ?? `${label} listener: ${h.label.toLowerCase()}`,}});}
              }
              if(openToEveryone(intg, channel)){
                 { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {        "tone":`warning`,"label":`Open to everyone`,"dot":false,"title":`No allowed users set: anyone who can message the bot can run an agent on this Mac. Edit to restrict it.`,}});}
              } else if (admitsNobody(intg, channel)){
                 { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {        "tone":`warning`,"label":`No allowed users`,"dot":false,"title":`The allow-list is blank, so every message is ignored. Edit to add user IDs.`,}});}
              }
             }
             { svelteHTML.createElement("div", {   "class":`ch-status`,"title":statusLine(intg, channel),});statusLine(intg, channel); }
            if(h?.detail){
               {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(h.detail)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`ch-health-detail tone-${h.tone}`,});
                
                h.detail;if(h.since){  { svelteHTML.createElement("span", {   "class":`dim`,"aria-hidden":`true`,});   { const $$_emiTleR4C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR4C({ target: __sveltets_2_any(), props: {  "iso":h.since,}});} }}
               }}
            } else if (h?.lastEvent && h.tone === 'success'){
               { svelteHTML.createElement("div", { "class":`ch-status`,});   { const $$_emiTleR3C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR3C({ target: __sveltets_2_any(), props: {  "iso":h.lastEvent,}});} }
            }
            if(intg && h){
              const turnedAway = pendingRejected(statuses.find((x) => x.channel === channel)?.rejected_senders, intg.allowed_users);
              if(turnedAway.length){
                 { svelteHTML.createElement("div", {   "class":`ch-rejected`,...__sveltets_2_empty({"data-testid":`channel-rejected-${channel}`}),});
                   { svelteHTML.createElement("div", { "class":`ch-status`,});        }
                   { svelteHTML.createElement("ul", {});
                       for(let r of __sveltets_2_ensureArray(turnedAway)){r.user;
                       { svelteHTML.createElement("li", {});
                         { svelteHTML.createElement("span", {   "class":`mono`,"dir":`ltr`,});channel === 'slack' && !r.name && slackUser.get(r.user) ? `${slackUser.get(r.user)?.name} (${r.user})` : senderLabel(r); }
                         { svelteHTML.createElement("span", { "class":`dim`,});  { const $$_emiTleR6C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR6C({ target: __sveltets_2_any(), props: {  "iso":r.at,}});} }
                         { svelteHTML.createElement("button", {         "class":`btn small`,"title":`Add ${r.user} to the ${label} allowed users`,"disabled":allowing !== null,"onclick":() => void allowSender(intg, r),});allowing === r.user ? 'Allowing…' : 'Allow…'; }
                       }
                    }
                   }
                 }
              }
            }
           }
           { svelteHTML.createElement("div", { "class":`ch-actions`,});
            if(intg && configured){
               { svelteHTML.createElement("label", {   "class":`checkbox-row ch-enabled`,"title":intg.enabled ? `Stop the ${label} bridge` : `Start the ${label} bridge`,});
                 { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":intg.enabled,"onchange":async (e) => {
                    const el = e.currentTarget;
                    await toggleEnabled(intg);
                    // A failed save leaves the saved value — show it, not the click.
                    el.checked = integrations.find((i) => i.channel === channel)?.enabled ?? false;
                  },});}
                
               }
              
               { svelteHTML.createElement("button", {         "class":`btn small ch-tool`,"title":intg.channel_id ? `Send “Otto is connected ✅” to ${intg.channel_id}` : `Set a default ${channel === 'webhook' ? 'reply URL' : 'chat'} first (Edit)`,"disabled":testBusy === channel || !intg.channel_id,"onclick":() => void sendTest(channel, intg),});
                testBusy === channel ? 'Sending…' : 'Test…';
               }
            }
            if(configured){
               { svelteHTML.createElement("button", {       "class":`icon-btn ch-tool`,"title":`Edit ${label} integration`,"aria-label":`Edit ${label} integration`,"onclick":() => openEdit(channel),});
                 { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":14,}});}
               }
               { svelteHTML.createElement("button", {             "class":`icon-btn ch-tool`,"title":removing[channel] ? `Removing ${label} integration…` : `Remove ${label} integration`,"aria-label":removing[channel] ? `Removing ${label} integration…` : `Remove ${label} integration`,"aria-busy":removing[channel] ? 'true' : undefined,"disabled":removing[channel],"onclick":() => remove(channel),});
                if(removing[channel]){ { svelteHTML.createElement("span", {   "class":`spinner`,"aria-hidden":`true`,}); }}else{ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}}
               }
            }else{
               { svelteHTML.createElement("button", {   "class":`btn small ch-tool`,"onclick":() => openEdit(channel),});  }
            }
           }
         }}
      };return __sveltets_2_any(0)};

      ;__sveltets_2_ensureSnippet(channelCard('slack', slack, 'slack', 'Slack'));
      ;__sveltets_2_ensureSnippet(channelCard('telegram', telegram, 'send', 'Telegram'));
      ;__sveltets_2_ensureSnippet(channelCard('webhook', webhook, 'link', 'Webhook'));
     }
     LoadState}
  }
   PageBody}
 }


if(editOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {       children:() => { return __sveltets_2_any(0); },"title":`Configure ${channelLabel(editChannel)}`,"width":500,"onclose":() => (editOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (editOpen = false),});  }
       { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":editBusy || needsSecret || needsAllowedUser,"title":needsSecret
          ? `Enter the ${editChannel === 'webhook' ? 'webhook key' : 'bot token'} first`
          : needsAllowedUser
            ? 'Add an allowed user, or check “Open to everyone”, first'
            : undefined,"onclick":save,});
        editBusy ? 'Saving…' : 'Save';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("label", { "class":`checkbox-row modal-check`,});
       { svelteHTML.createElement("input", {      "id":`ch-enabled`,"type":`checkbox`,"bind:checked":fEnabled,});/*Ωignore_startΩ*/() => fEnabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      
     }

    if(editChannel === 'webhook'){
      
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ch-url`,});  }
         { svelteHTML.createElement("div", { "class":`row-inline`,});
           { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`ch-url`,"class":`input mono inline-field`,"value":webhookUrl,"readonly":true,"spellcheck":`false`,});}
           { svelteHTML.createElement("button", {     "class":`btn small inline-btn`,"type":`button`,"onclick":() => void copyNotify(webhookUrl),});  }
         }
         { svelteHTML.createElement("span", { "class":`hint`,});
           { svelteHTML.createElement("code", {});  }     { svelteHTML.createElement("code", {});  }   
            { svelteHTML.createElement("code", {});'{ "text": "…" }'; }         
             { svelteHTML.createElement("code", {});  }        
         }
       }
    }

    
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ch-bot`,});editChannel === 'webhook' ? 'Webhook key' : 'Bot token'; }
       { svelteHTML.createElement("div", { "class":`row-inline`,});
         { svelteHTML.createElement("input", {               "id":`ch-bot`,"class":`input mono inline-field`,"type":editChannel === 'webhook' && showWebhookKey ? 'text' : 'password',"bind:value":fBotToken,"autocomplete":`off`,"spellcheck":`false`,"placeholder":integrations.find((i) => i.channel === editChannel)?.has_bot_token
            ? '•••••• (leave blank to keep)'
            : editChannel === 'slack'
              ? 'xoxb-…'
              : editChannel === 'telegram'
                ? '123456:ABC…'
                : 'paste a key or click Generate',});/*Ωignore_startΩ*/() => fBotToken = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(editChannel === 'webhook'){
           { svelteHTML.createElement("button", {     "class":`btn small inline-btn`,"type":`button`,"onclick":() => (showWebhookKey = !showWebhookKey),});showWebhookKey ? 'Hide' : 'Show'; }
           { svelteHTML.createElement("button", {     "class":`btn small inline-btn`,"type":`button`,"onclick":generateKey,});  }
           { svelteHTML.createElement("button", {       "class":`btn small inline-btn`,"type":`button`,"disabled":!fBotToken,"onclick":() => void copyNotify(fBotToken),});  }
        }
       }
      if(editChannel === 'webhook'){
         { svelteHTML.createElement("span", { "class":`hint`,});
                          
                       
         }
      } else if (integrations.find((i) => i.channel === editChannel)?.has_bot_token){
         { svelteHTML.createElement("span", { "class":`hint`,});       }
      }
     }

    
    if(editChannel === 'slack'){
      if(slackDirError){
         { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`Slack names`,"variant":`compact`,"error":slackDirError,"empty":true,"onretry":() => wsId && void loadSlackDir(wsId, true),}});}
         { svelteHTML.createElement("p", { "class":`hint`,});         }
      }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ch-app`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
         { svelteHTML.createElement("input", {             "id":`ch-app`,"class":`input mono`,"type":`password`,"bind:value":fAppToken,"autocomplete":`off`,"placeholder":integrations.find((i) => i.channel === 'slack')?.has_app_token
            ? '•••••• (leave blank to keep)'
            : 'xapp-…',});/*Ωignore_startΩ*/() => fAppToken = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(integrations.find((i) => i.channel === 'slack')?.has_app_token){
           { svelteHTML.createElement("span", { "class":`hint`,});        }
        }else{
           { svelteHTML.createElement("span", { "class":`hint`,});         }
        }
       }
    }

    
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ch-chid`,});
        editChannel === 'slack'
          ? 'Default channel ID'
          : editChannel === 'telegram'
            ? 'Default chat ID'
            : 'Default reply callback URL';
       }
       { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`ch-chid`,"class":`input mono`,"bind:value":fChannelId,"spellcheck":`false`,"autocomplete":`off`,"placeholder":editChannel === 'slack'
          ? 'C0123…'
          : editChannel === 'telegram'
            ? '-100123456…'
            : 'https://example.com/otto-replies (optional)',});/*Ωignore_startΩ*/() => fChannelId = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(editChannel === 'slack' && (slackDir?.channels.length ?? 0) > 0){
         { svelteHTML.createElement("div", { "class":`dir-row`,});
           { svelteHTML.createElement("button", {       "type":`button`,"class":`btn small`,"onclick":pickSlackChannel,"aria-haspopup":`menu`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}  
           }
          if(fChannelId.trim() && slackChannel.get(fChannelId.trim())){
             { svelteHTML.createElement("span", { "class":`dir-name`,});slackChannelLabel(fChannelId); }
          }
         }
      } else if (editChannel === 'slack' && slackDir?.channels_error){
         { svelteHTML.createElement("span", { "class":`hint`,});slackDir.channels_error; }
      }
      if(editChannel === 'webhook'){
         { svelteHTML.createElement("span", { "class":`hint`,});
                       { svelteHTML.createElement("code", {});  }
                  
         }
      }
     }

    
     { svelteHTML.createElement("div", { "class":`field`,});
      if(editChannel === 'webhook'){
         { svelteHTML.createElement("label", { "for":`ch-users`,});  }
         { svelteHTML.createElement("input", {              "dir":`auto`,"id":`ch-users`,"class":`input`,"bind:value":fAllowedUsers,"spellcheck":`false`,"autocomplete":`off`,"placeholder":`ci-bot,deploy-bot`,});/*Ωignore_startΩ*/() => fAllowedUsers = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", { "class":`hint`,});
                 { svelteHTML.createElement("code", {});  }     
                     
         }
      }else{
         { svelteHTML.createElement("div", { "class":`users-head`,});
           { svelteHTML.createElement("span", {   "class":`field-label`,"id":`ch-users-label`,});  }
          if(editChannel === 'slack' && slackDir){
             { svelteHTML.createElement("button", {         "type":`button`,"class":`btn ghost small`,"disabled":slackDirLoading,"onclick":() => wsId && void loadSlackDir(wsId, true),"title":`Refresh names from Slack`,});slackDirLoading ? 'Refreshing…' : 'Refresh names'; }
          }
         }
         { const $$_rotidEsdIdewollA2C = __sveltets_2_ensureComponent(AllowedIdsEditor); const $$_rotidEsdIdewollA2 = new $$_rotidEsdIdewollA2C({ target: __sveltets_2_any(), props: {           value:fAllowedUsers,"labelledby":`ch-users-label`,"people":editChannel === 'slack' ? slackDir?.users ?? [] : [],"label":channelLabel(editChannel),"placeholder":editChannel === 'slack' ? 'Member ID (U0123…)' : 'User ID',}});/*Ωignore_startΩ*/() => fAllowedUsers = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rotidEsdIdewollA2.$$bindings = 'value';}
         { svelteHTML.createElement("span", { "class":`hint`,});
          if(editChannel === 'slack'){
            if(slackDir?.users_error){slackDir.users_error; }            }else{
            channelLabel(editChannel);  }
               
         }
      }
      if(editChannel !== 'webhook' && fAllowedUsers.trim() === ''){
         { svelteHTML.createElement("label", { "class":`checkbox-row modal-check`,});
           { svelteHTML.createElement("input", {      "id":`ch-open`,"type":`checkbox`,"bind:checked":fOpenToAll,});/*Ωignore_startΩ*/() => fOpenToAll = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            
         }
        if(fOpenToAll){
           { svelteHTML.createElement("span", {   "class":`hint warn`,"role":`note`,});
                     editChannel === 'slack' ? 'channel it’s in, or by DM' : 'chat it’s in';        
           }
        } else if (needsAllowedUser){
           { svelteHTML.createElement("span", {     "class":`hint warn`,"id":`ch-users-err`,"role":`alert`,});
                           channelLabel(editChannel);
           }
        }
      }
     }

    
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ch-cli`,});  }
       { svelteHTML.createElement("select", {     "id":`ch-cli`,"class":`input`,"bind:value":fPreferredCli,});/*Ωignore_startΩ*/() => fPreferredCli = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});   }
           for(let p of __sveltets_2_ensureArray(providers)){p;
           { svelteHTML.createElement("option", { "value":p,});cliName(p); }
        }
       }
       { svelteHTML.createElement("span", { "class":`hint`,});
                   
         
       }
     }

    
     { svelteHTML.createElement("label", { "class":`checkbox-row modal-check`,});
       { svelteHTML.createElement("input", {      "id":`ch-agent`,"type":`checkbox`,"bind:checked":fAgentReply,});/*Ωignore_startΩ*/() => fAgentReply = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      editChannel === 'webhook'
        ? 'Relay only the agent\'s marked reply (⟦otto-send⟧)'
        : 'Agent posts the final reply itself';
     }

    
    if(fAgentReply){
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`ch-reply`,});  }
         { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`ch-reply`,"class":`input reply-area`,"rows":4,"bind:value":fReplyInstructions,"placeholder":`You are a helpful assistant replying on behalf of the Otto agent. Be concise and direct.`,});/*Ωignore_startΩ*/() => fReplyInstructions = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
    }

    
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Channels__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Channels__SvelteComponent_ = ReturnType<typeof Channels__SvelteComponent_>;
/*Ωignore_endΩ*/export default Channels__SvelteComponent_;