<script lang="ts">
  // The "Send on Slack as me" dialog (see lib/slackSend.svelte.ts). Mounted
  // once by the app shell; renders while a request is open. Recipient search
  // lists people and channels inline (in flow — no floating popup to clamp),
  // the footer states where the message goes and that it posts as you, and
  // Send is the confirmation. Nothing is sent before Send.
  import { api } from '../../api/client';
  import type { SlackConv, SlackDirUser, SlackMeTeam, SlackPosted, SlackTeamsResp } from '../../api/types';
  import { slackSend } from '../../slackSend.svelte';
  import { ws } from '../../stores/workspace.svelte';
  import { router } from '../../router.svelte';
  import { toasts } from '../../toast.svelte';
  import { loadErrorText } from '../../loadError';
  import { openExternal } from '../../external';
  import Modal from '../Modal.svelte';
  import Icon from '../Icon.svelte';
  import LoadState from '../LoadState.svelte';

  type Target = { kind: 'person' | 'channel'; id: string; label: string };

  const req = $derived(slackSend.req);
  const wsId = $derived(ws.currentId);
  let teams = $state<SlackMeTeam[]>([]);
  let teamId = $state('');
  let loading = $state(true);
  let loadError = $state('');
  let query = $state('');
  let target = $state<Target | null>(null);
  let text = $state('');
  let people = $state<SlackDirUser[]>([]);
  let channels = $state<SlackConv[]>([]);
  let searching = $state(false);
  let sending = $state(false);
  let sendError = $state('');

  const base = (id: string): string => `/workspaces/${encodeURIComponent(id)}/integrations/slack`;
  const team = $derived(teams.find((t) => t.team_id === teamId) ?? null);

  // A fresh request: reset the form from it and load the connected teams.
  $effect(() => {
    const r = req;
    const id = wsId;
    if (!r) return;
    query = r.to ?? '';
    text = r.text ?? '';
    target = null;
    sendError = '';
    loading = true;
    loadError = '';
    if (!id) {
      loading = false;
      loadError = 'Open a workspace first — the message goes out through its Slack settings.';
      return;
    }
    let live = true;
    api.get<SlackTeamsResp>(`${base(id)}/teams`)
      .then((r2) => {
        if (!live) return;
        teams = r2.teams;
        teamId = r2.default_team ?? (r2.teams.length === 1 ? r2.teams[0].team_id : '');
      })
      .catch((e) => { if (live) loadError = loadErrorText(e); })
      .finally(() => { if (live) loading = false; });
    return () => { live = false; };
  });

  // Recipient search (debounced), as you, in the picked team.
  $effect(() => {
    const q = query.trim().replace(/^[#@]/, '');
    const id = wsId;
    const t = teamId;
    if (!id || !t || target || q.length < 2) {
      people = [];
      channels = [];
      return;
    }
    let live = true;
    const timer = setTimeout(() => {
      searching = true;
      const qs = `as=user&team=${encodeURIComponent(t)}&q=${encodeURIComponent(q)}`;
      Promise.all([
        api.get<{ users: SlackDirUser[] }>(`${base(id)}/users?${qs}`).then((r) => r.users.filter((u) => !u.is_bot).slice(0, 6)).catch(() => []),
        api.get<{ conversations: SlackConv[] }>(`${base(id)}/conversations?${qs}&types=public_channel,private_channel&limit=20`)
          .then((r) => r.conversations.slice(0, 6)).catch(() => []),
      ]).then(([u, c]) => {
        if (!live) return;
        people = u;
        channels = c;
      }).finally(() => { if (live) searching = false; });
    }, 250);
    return () => { live = false; clearTimeout(timer); };
  });

  function pick(t: Target): void {
    target = t;
    query = '';
  }
  /** A typed `#channel` / `@handle` is a valid recipient as is. */
  const typed = $derived.by((): Target | null => {
    const q = query.trim();
    if (/^#[\w.-]{1,80}$/.test(q)) return { kind: 'channel', id: q, label: q };
    if (/^@[\w.-]{1,80}$/.test(q)) return { kind: 'person', id: q, label: q };
    return null;
  });
  const recipient = $derived(target ?? typed);
  const canSend = $derived(!!wsId && !!team && !!recipient && text.trim() !== '' && !sending);

  async function send(): Promise<void> {
    const id = wsId;
    const r = recipient;
    if (!id || !team || !r || !canSend) return;
    sending = true;
    sendError = '';
    try {
      const posted = await api.post<SlackPosted>(`${base(id)}/messages`, { as: 'user', team: team.team_id, channel: r.id, text: text.trim() });
      const link = posted.permalink;
      toasts.push('success', `Sent to ${r.label}`, `As ${team.user} in ${team.team}`, 8000,
        link ? { action: { label: 'Open in Slack', run: () => openExternal(link) } } : {});
      slackSend.close();
    } catch (e) {
      sendError = loadErrorText(e);
    } finally {
      sending = false;
    }
  }

  function goSettings(): void {
    slackSend.close();
    router.go('settings/channels');
  }
</script>

{#if req}
  <Modal title={req.title ?? 'Send on Slack as me'} width={560} onclose={() => slackSend.close()}>
    <div class="ss" data-testid="slack-send">
      {#if req.context}<p class="ss-context">{req.context}</p>{/if}
      <LoadState what="your Slack workspaces" {loading} error={loadError} variant="compact" empty={false} rows={2}>
        {#if teams.length === 0}
          <div class="ss-empty">
            <p>Connect your Slack user token to send messages as you.</p>
            <button class="btn small" onclick={goSettings}>Open Settings → Channels</button>
          </div>
        {:else}
          {#if teams.length > 1}
            <label class="ss-label" for="ss-team">Slack workspace</label>
            <select id="ss-team" class="input" bind:value={teamId}>
              <option value="" disabled>Pick a Slack workspace</option>
              {#each teams as t (t.team_id)}<option value={t.team_id}>{t.team}</option>{/each}
            </select>
          {/if}

          <label class="ss-label" for="ss-to">To</label>
          {#if target}
            <div class="ss-chip">
              <Icon name={target.kind === 'person' ? 'user' : 'comment'} size={12} />
              <span class="ss-chip-label">{target.kind === 'channel' && !target.label.startsWith('#') ? `#${target.label}` : target.label}</span>
              <button class="ss-chip-x" aria-label="Change the recipient" title="Change the recipient" onclick={() => (target = null)}><Icon name="x" size={12} /></button>
            </div>
          {:else}
            <input id="ss-to" class="input" dir="auto" autocomplete="off" placeholder="A person or #channel" bind:value={query} disabled={!teamId} />
            {#if people.length || channels.length}
              <ul class="ss-results" aria-label="Matching people and channels">
                {#each people as u (u.id)}
                  <li><button class="ss-result" onclick={() => pick({ kind: 'person', id: u.id, label: u.name })}>
                    <Icon name="user" size={12} /> <span class="ss-name">{u.name}</span> <span class="ss-dim" dir="ltr">@{u.handle}</span>
                  </button></li>
                {/each}
                {#each channels as c (c.id)}
                  <li><button class="ss-result" onclick={() => pick({ kind: 'channel', id: c.id, label: `#${c.name}` })}>
                    <Icon name="comment" size={12} /> <span class="ss-name">#{c.name}</span>{#if c.kind === 'private'}<span class="ss-dim">private</span>{/if}
                  </button></li>
                {/each}
              </ul>
            {:else if searching}
              <p class="ss-hint">Searching…</p>
            {:else if query.trim().length >= 2 && !typed}
              <p class="ss-hint">No one matches “{query.trim()}”. Type a <span class="mono">#channel</span> or <span class="mono">@handle</span> to send to it directly.</p>
            {/if}
          {/if}

          <label class="ss-label" for="ss-text">Message</label>
          <textarea id="ss-text" class="input" dir="auto" rows="5" bind:value={text}></textarea>
          {#if sendError}<p class="error-text">{sendError}</p>{/if}
        {/if}
      </LoadState>
    </div>
    {#snippet footer()}
      {#if team && recipient}
        <span class="ss-where">Posts <strong>as you ({team.user})</strong> to <strong>{recipient.label}</strong> in {team.team}</span>
      {/if}
      <button class="btn" onclick={() => slackSend.close()}>Cancel</button>
      <button class="btn primary" disabled={!canSend} onclick={() => void send()}>
        <Icon name="send" size={12} /> {sending ? 'Sending…' : 'Send'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .ss { display: flex; flex-direction: column; gap: 6px; }
  .ss-context { margin: 0 0 4px; font-size: var(--fs-s); color: var(--text-dim); }
  .ss-label { font-size: var(--fs-s); font-weight: 600; margin-block-start: 6px; }
  .ss textarea { resize: vertical; min-block-size: 96px; }
  .ss-results { list-style: none; margin: 0; padding: 4px; border: 1px solid var(--border); border-radius: var(--radius-s); max-block-size: 220px; overflow-y: auto; }
  .ss-result { display: flex; align-items: center; gap: 6px; inline-size: 100%; padding: 6px 8px; border: none; background: none; color: var(--text); font-size: var(--fs-s); text-align: start; border-radius: var(--radius-s); cursor: pointer; }
  .ss-result:hover, .ss-result:focus-visible { background: var(--hover); }
  .ss-name { font-weight: 600; }
  .ss-dim, .ss-hint { color: var(--text-dim); font-size: var(--fs-xs); }
  .ss-hint { margin: 0; font-size: var(--fs-s); }
  .ss-chip { display: inline-flex; align-items: center; gap: 6px; align-self: flex-start; padding-block: 4px; padding-inline: 10px 6px; border-radius: var(--radius-m); background: var(--accent-soft); color: var(--accent-text); font-size: var(--fs-s); font-weight: 600; }
  .ss-chip-x { display: inline-flex; border: none; background: none; color: inherit; cursor: pointer; padding: 2px; border-radius: var(--radius-s); }
  .ss-chip-x:hover { background: var(--hover); }
  .ss-empty { display: flex; flex-direction: column; align-items: flex-start; gap: 8px; font-size: var(--fs-s); }
  .ss-empty p { margin: 0; }
  .ss-where { flex: 1; font-size: var(--fs-xs); color: var(--text-dim); }
</style>
