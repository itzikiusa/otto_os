<script lang="ts">
  // "Can act as me in Slack" for one personal agent: which of the OWNER's
  // connected Slack workspaces (Settings → Channels → Act as me in Slack) the
  // agent may act in as them — read DMs, search, post (always approval-gated),
  // mark unread. Default none. Only the owner can change it.
  import { api } from '../../lib/api/client';
  import type { PersonalAgentSlackGrant } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { router } from '../../lib/router.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';

  interface Props { agentId: string }
  let { agentId }: Props = $props();
  const uid = $props.id();

  let grant = $state<PersonalAgentSlackGrant | null>(null);
  let picked = $state<string[]>([]);
  let loading = $state(true);
  let loadError = $state('');
  let saving = $state(false);
  const dirty = $derived(!!grant && [...picked].sort().join(',') !== [...grant.teams].sort().join(','));

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      const g = await api.get<Partial<PersonalAgentSlackGrant>>(`/personal-agents/${encodeURIComponent(agentId)}/slack-as-me`);
      // An older daemon (or a partial reply) may omit the lists — never crash the panel.
      grant = { ...g, teams: g.teams ?? [], available: g.available ?? [], is_owner: g.is_owner ?? false } as PersonalAgentSlackGrant;
      picked = [...grant.teams];
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void agentId;
    void load();
  });

  function toggle(id: string, on: boolean): void {
    picked = on ? [...picked.filter((t) => t !== id), id] : picked.filter((t) => t !== id);
  }

  async function save(): Promise<void> {
    saving = true;
    try {
      grant = await api.put<PersonalAgentSlackGrant>(`/personal-agents/${encodeURIComponent(agentId)}/slack-as-me`, { teams: picked });
      picked = [...grant.teams];
      toasts.success(picked.length ? 'This agent can now act as you in Slack' : 'This agent can no longer act as you in Slack');
    } catch (e) {
      toasts.error('Couldn’t save the Slack permission', loadErrorText(e));
    } finally {
      saving = false;
    }
  }
</script>

<section class="pa-panel" aria-labelledby="{uid}-h" data-testid="agent-slack-as-me">
  <h2 id="{uid}-h">Can act as me in Slack</h2>
  <p class="hint">
    Lets this agent use your Slack identity in the workspaces you tick: read your channels and DMs, search, mark messages
    unread, and post as you — every post still asks you first. Off by default.
  </p>
  <LoadState what="the Slack permission" {loading} error={loadError} variant="compact" rows={1} onretry={() => void load()}>
    {#if grant && !grant.is_owner}
      <p class="hint">Only this agent’s owner can change it. {grant.teams.length ? `Allowed in ${grant.teams.length} Slack workspace(s).` : 'Not allowed.'}</p>
    {:else if grant && grant.available.length === 0}
      <p class="hint">
        Connect a Slack workspace first:
        <button class="link" onclick={() => router.go('settings/channels')}>Settings → Channels → Act as me in Slack</button>.
      </p>
    {:else if grant}
      <ul class="teams">
        {#each grant.available as t (t.team_id)}
          <li>
            <label class="chk">
              <input type="checkbox" checked={picked.includes(t.team_id)} disabled={saving}
                onchange={(e) => toggle(t.team_id, (e.currentTarget as HTMLInputElement).checked)} />
              {t.team} <span class="meta">as {t.user}</span>
            </label>
          </li>
        {/each}
      </ul>
      <div class="actions">
        <button class="btn primary small" disabled={!dirty || saving} onclick={() => void save()}>{saving ? 'Saving…' : 'Save Slack permission'}</button>
      </div>
    {/if}
  </LoadState>
</section>

<style>
  .pa-panel { border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 12px 14px; color: var(--text); min-width: 0; }
  .pa-panel h2 { margin: 0 0 6px; font-size: var(--fs-l); font-weight: 600; }
  .hint { color: var(--text-dim); font-size: var(--fs-s); margin: 0 0 8px; }
  .teams { list-style: none; margin: 0 0 8px; padding: 0; display: flex; flex-direction: column; gap: 4px; }
  .chk { display: flex; align-items: center; gap: 6px; font-size: var(--fs-m); }
  .meta { color: var(--text-dim); font-size: var(--fs-s); }
  .actions { display: flex; justify-content: flex-end; }
  .link { border: 0; background: none; padding: 0; font: inherit; font-size: var(--fs-s); color: var(--accent-text); cursor: pointer; }
  .link:hover { text-decoration: underline; }
</style>
