<script lang="ts">
  // "Act as me in Slack": the signed-in person's own Slack USER tokens
  // (xoxp-), one per Slack workspace, stored per Otto user in the Keychain.
  // Agents use them through the `otto_slack_*` tools with as=user — posts stay
  // approval-gated every time; a personal agent only acts in the workspaces
  // its owner ticks on the agent (Autonomy → Can act as me in Slack).
  import { api } from '../../lib/api/client';
  import { announceShown } from '../../lib/announce.svelte';
  import type { SlackMeTeam, SlackMeTestResp } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import SlackAsMeDefaults from './SlackAsMeDefaults.svelte';

  let teams = $state<SlackMeTeam[]>([]);
  let loading = $state(true);
  let loadError = $state('');
  let busy = $state<string | null>(null);
  /** The token sheet: `null` closed, `''` add, a team id = replace that one. */
  let sheet = $state<string | null>(null);
  let fToken = $state('');
  let sheetError = $state('');

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      teams = await api.get<SlackMeTeam[]>('/integrations/slack/me');
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  void load();

  function openSheet(teamId: string): void {
    sheet = teamId;
    fToken = '';
    sheetError = '';
  }

  async function saveToken(): Promise<void> {
    if (sheet === null) return;
    const token = fToken.trim();
    busy = 'sheet';
    sheetError = '';
    try {
      const t = sheet === ''
        ? await api.post<SlackMeTeam>('/integrations/slack/me', { token })
        : await api.put<SlackMeTeam>(`/integrations/slack/me/${encodeURIComponent(sheet)}`, { token });
      toasts.success(`Connected ${t.team} as ${t.user}`);
      sheet = null;
      fToken = '';
      await load();
    } catch (e) {
      sheetError = loadErrorText(e);
    } finally {
      busy = null;
    }
  }

  async function test(t: SlackMeTeam): Promise<void> {
    busy = t.team_id;
    try {
      const r = await api.post<SlackMeTestResp>(`/integrations/slack/me/${encodeURIComponent(t.team_id)}/test`);
      if (r.ok && r.team) toasts.success(`Slack says: ${r.team.user} in ${r.team.team}`);
      else toasts.error(`Slack rejected the ${t.team} token`, r.error ?? '');
      await load();
    } catch (e) {
      toasts.error(`Couldn’t test the ${t.team} token`, loadErrorText(e));
    } finally {
      busy = null;
    }
  }

  async function remove(t: SlackMeTeam): Promise<void> {
    const ok = await confirmer.ask(
      `Agents will no longer be able to read or post as you in ${t.team}. The token is deleted from the Keychain (revoke it in Slack too if you no longer need it).`,
      { title: `Disconnect ${t.team}?`, confirmLabel: 'Disconnect', danger: true },
    );
    if (!ok) return;
    busy = t.team_id;
    try {
      await api.del(`/integrations/slack/me/${encodeURIComponent(t.team_id)}`);
      toasts.success(`Disconnected ${t.team}`);
      await load();
    } catch (e) {
      toasts.error(`Couldn’t disconnect ${t.team}`, loadErrorText(e));
    } finally {
      busy = null;
    }
  }
</script>

<section class="as-me" aria-labelledby="slack-as-me-title" data-testid="slack-act-as-me">
  <div class="as-me-head">
    <div>
      <h2 id="slack-as-me-title" class="as-me-title">Act as me in Slack</h2>
      <p class="hint">
        Your own Slack user token (<span class="mono">xoxp-…</span>), one per Slack workspace — just for you, never shared with
        other Otto users. Agents can then read your channels and DMs, search, and post or mark messages unread
        <strong>as you</strong>. Every post asks you first; personal agents need your permission per agent.
      </p>
    </div>
    <button class="btn small primary" onclick={() => openSheet('')} data-testid="slack-as-me-add">
      <Icon name="plus" size={12} /> Add Slack workspace…
    </button>
  </div>

  <LoadState what="your Slack workspaces" {loading} error={loadError} variant="compact" empty={false} onretry={() => void load()} rows={1}>
    {#if teams.length === 0}
      <p class="hint empty">No Slack workspace connected yet. Paste your User OAuth Token to start.</p>
    {:else}
      <ul class="team-list">
        {#each teams as t (t.team_id)}
          <li class="team card" data-testid="slack-as-me-team">
            <div class="grow">
              <div class="team-name">
                {t.team}
                <span class="dim mono" dir="ltr">{t.url.replace(/^https?:\/\//, '').replace(/\/$/, '')}</span>
              </div>
              <div class="hint">as <strong>{t.user}</strong> <span class="mono" dir="ltr">({t.user_id})</span></div>
              {#if t.missing_scopes.length}
                <div class="missing">
                  <StatusBadge tone="warning" label="Missing scopes" dot={false}
                    title="Add these User Token Scopes in your Slack app, reinstall it, then Replace the token." />
                  <span class="mono small" dir="ltr">{t.missing_scopes.join(', ')}</span>
                </div>
              {/if}
            </div>
            <div class="team-actions">
              <button class="btn small" disabled={busy !== null} onclick={() => void test(t)} title="Check the token with Slack (auth.test)">
                {busy === t.team_id ? 'Testing…' : 'Test'}
              </button>
              <button class="btn small" disabled={busy !== null} onclick={() => openSheet(t.team_id)}>Replace…</button>
              <button class="icon-btn" disabled={busy !== null} onclick={() => void remove(t)}
                aria-label="Disconnect {t.team}" title="Disconnect {t.team}">
                <Icon name="trash" size={14} />
              </button>
            </div>
          </li>
        {/each}
      </ul>
      {#if teams.length > 1}<SlackAsMeDefaults {teams} />{/if}
    {/if}
  </LoadState>
</section>

{#if sheet !== null}
  <Modal title={sheet === '' ? 'Add a Slack workspace' : 'Replace the Slack token'} width={520} onclose={() => (sheet = null)}>
    <ol class="steps">
      <li>In your Slack app (api.slack.com/apps) → <strong>OAuth &amp; Permissions</strong> → <strong>User Token Scopes</strong>, add:
        <span class="mono small" dir="ltr">channels:history groups:history im:history mpim:history channels:read groups:read im:read mpim:read users:read search:read chat:write reactions:write channels:write groups:write im:write mpim:write</span>
        (the <span class="mono">*:write</span> ones are what marking unread needs).</li>
      <li>Install / reinstall the app to your workspace and copy the <strong>User OAuth Token</strong> (starts with <span class="mono">xoxp-</span>, not the <span class="mono">xoxb-</span> bot token). Keep token rotation off.</li>
    </ol>
    <label class="field-label" for="slack-as-me-token">User OAuth Token</label>
    <input id="slack-as-me-token" class="input mono" type="password" autocomplete="off" spellcheck="false"
      placeholder="xoxp-…" bind:value={fToken} data-testid="slack-as-me-token" />
    <p class="hint">Stored in the macOS Keychain under your Otto user. Otto checks it with Slack (auth.test) before saving.</p>
    {#if sheetError}<p class="error-text" use:announceShown={sheetError}>{sheetError}</p>{/if}
    {#snippet footer()}
      <button class="btn" onclick={() => (sheet = null)}>Cancel</button>
      <button class="btn primary" disabled={busy !== null || !fToken.trim().startsWith('xox')} onclick={() => void saveToken()}>
        {busy === 'sheet' ? 'Checking…' : sheet === '' ? 'Connect' : 'Replace'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .as-me {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: var(--settings-col);
    margin-block-start: 24px;
  }
  .as-me-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .as-me-title {
    font-size: var(--fs-m);
    font-weight: 600;
    margin: 0 0 4px;
  }
  .hint {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin: 0;
  }
  .empty {
    padding: 8px 0;
  }
  .team-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .team {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    flex-wrap: wrap;
  }
  .team .grow {
    flex: 1;
    min-width: 0;
  }
  .team-name {
    font-weight: 600;
    display: flex;
    gap: 8px;
    align-items: baseline;
    flex-wrap: wrap;
  }
  .dim {
    color: var(--text-dim);
    font-weight: 400;
    font-size: var(--fs-s);
  }
  .missing {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
    margin-block-start: 4px;
  }
  .small {
    font-size: var(--fs-s);
  }
  .mono {
    overflow-wrap: anywhere;
  }
  .team-actions {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .steps {
    margin: 0 0 12px;
    padding-inline-start: 18px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .field-label {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .error-text {
    color: var(--danger);
    font-size: var(--fs-s);
    margin: 6px 0 0;
  }
</style>
