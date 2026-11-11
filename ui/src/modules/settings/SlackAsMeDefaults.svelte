<script lang="ts">
  // Which connected Slack workspace "act as me" uses when a send doesn't name
  // one: your global default, and an override for the current Otto workspace
  // (the override wins). Per person — other Otto users keep their own. Shown
  // under "Act as me in Slack" once two or more Slack workspaces are connected.
  import { api } from '../../lib/api/client';
  import type { SlackAsMeDefault, SlackMeTeam } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';

  interface Props {
    teams: SlackMeTeam[];
  }
  let { teams }: Props = $props();

  const wsId = $derived(ws.currentId);
  const wsName = $derived(ws.current?.name ?? 'this workspace');
  let cur = $state<SlackAsMeDefault | null>(null);
  let error = $state('');
  let busy = $state(false);

  const url = (id: string | null): string =>
    id ? `/workspaces/${encodeURIComponent(id)}/integrations/slack/as-me-default` : '/integrations/slack/as-me-default';

  $effect(() => {
    const id = wsId;
    void teams.length; // re-read when a team is connected / disconnected
    let live = true;
    api.get<SlackAsMeDefault>(url(id))
      .then((d) => { if (live) { cur = d; error = ''; } })
      .catch((e) => { if (live) error = loadErrorText(e); });
    return () => { live = false; };
  });

  async function save(level: 'global' | 'workspace', team: string): Promise<void> {
    busy = true;
    try {
      const body = { team_id: team || null };
      if (level === 'global') await api.put(url(null), body);
      cur = level === 'workspace' && wsId ? await api.put<SlackAsMeDefault>(url(wsId), body) : await api.get<SlackAsMeDefault>(url(wsId));
    } catch (e) {
      toastError('Couldn’t save the default Slack workspace', e);
    } finally {
      busy = false;
    }
  }
  const nameOf = (id: string | null | undefined): string => teams.find((t) => t.team_id === id)?.team ?? '—';
</script>

<div class="defaults" data-testid="slack-as-me-defaults">
  <div class="sd-row">
    <label class="lbl" for="slack-default-global">Default Slack workspace</label>
    <select id="slack-default-global" class="input" disabled={busy || !cur} value={cur?.global ?? ''}
      onchange={(e) => void save('global', (e.currentTarget as HTMLSelectElement).value)}>
      <option value="">None — ask each time</option>
      {#each teams as t (t.team_id)}<option value={t.team_id}>{t.team}</option>{/each}
    </select>
  </div>
  {#if wsId}
    <div class="sd-row">
      <label class="lbl" for="slack-default-ws">In {wsName}</label>
      <select id="slack-default-ws" class="input" disabled={busy || !cur} value={cur?.workspace ?? ''}
        onchange={(e) => void save('workspace', (e.currentTarget as HTMLSelectElement).value)}>
        <option value="">Use the default{cur?.global ? ` (${nameOf(cur.global)})` : ''}</option>
        {#each teams as t (t.team_id)}<option value={t.team_id}>{t.team}</option>{/each}
      </select>
    </div>
  {/if}
  {#if error}
    <p class="error-text">{error}</p>
  {:else if cur}
    <p class="hint">Sends from {wsName} go out as you in <strong>{nameOf(cur.effective)}</strong> unless you pick another workspace. The workspace choice wins over the default.</p>
  {/if}
</div>

<style>
  .defaults { display: flex; flex-direction: column; gap: 6px; }
  .sd-row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .lbl { min-inline-size: 180px; font-size: var(--fs-s); color: var(--text-dim); }
  .sd-row select { min-inline-size: 220px; max-inline-size: 100%; }
  .hint { margin: 0; font-size: var(--fs-s); color: var(--text-dim); }
</style>
