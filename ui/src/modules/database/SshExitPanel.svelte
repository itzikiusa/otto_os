<script lang="ts">
  // Under an ended SSH / custom-CLI terminal: WHY it ended and the way back.
  // A dead `ssh` used to leave only "Failed (exit 255)" — no reason, no
  // Reconnect. For ssh profiles this runs the connection's Test probe (a
  // quick non-interactive `ssh … exit`, diagnosed server-side) and shows its
  // summary, the hint on what to change, and the raw detail on demand.
  //
  // `extra` is the slot for further recovery actions (a later "Diagnose with
  // an agent"); it renders after Reconnect / Edit connection.
  import type { Snippet } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { api } from '../../lib/api/client';
  import { loadErrorText } from '../../lib/loadError';
  import type { Connection, TestConnectionResp } from '../../lib/api/types';

  interface Props {
    /** The tab's profile (null when it was deleted while the tab stayed open). */
    conn: Connection | null;
    /** Tab label — the profile name when the tab was opened. */
    name: string;
    kind: string;
    onreconnect: () => void;
    /** Absent when the viewer may not edit the profile. */
    onedit?: (c: Connection) => void;
    extra?: Snippet<[{ result: Probe | null }]>;
  }
  let { conn, name, kind, onreconnect, onedit, extra }: Props = $props();

  /** `detail` (raw ssh stderr) is newer than the rest of the response: read
   *  it, and `hint`, as optional. */
  type Probe = TestConnectionResp & { detail?: string | null };

  let result = $state<Probe | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let seq = 0;

  // By id: a connections reload hands over new objects for the same profile.
  const connId = $derived(conn?.id ?? null);
  const probes = $derived(kind === 'ssh' && connId != null);

  async function probe(id: string | null): Promise<void> {
    if (!id) return;
    const mine = ++seq;
    loading = true;
    error = null;
    result = null;
    try {
      const r = await api.post<Probe>(`/connections/${id}/test`, {});
      if (mine === seq) result = r;
    } catch (e) {
      if (mine === seq) error = loadErrorText(e);
    } finally {
      if (mine === seq) loading = false;
    }
  }

  // Once per profile shown (switching between two ended tabs re-probes).
  $effect(() => {
    if (probes) void probe(connId);
  });
</script>

<section class="exit-panel" aria-label="Session ended" data-testid="ssh-exit-panel">
  <div class="head">
    <Icon name="warning" size={14} />
    <span class="title ellipsis">Session to {name} ended</span>
  </div>
  {#if probes}
    <LoadState what="the connection check" errorTitle="Couldn’t check the connection" {loading} {error} empty={!result} onretry={() => void probe(connId)} variant="compact" rows={2}>
      {#if result}
        {#if result.ok}
          <p class="msg">The connection test passes now, so the shell exited or the link dropped. Reconnect to start a new session.</p>
        {:else}
          <p class="msg err" data-testid="ssh-exit-message">{result.message || 'The connection test failed without a reason.'}</p>
        {/if}
        {#if result.hint}<p class="msg hint" data-testid="ssh-exit-hint">{result.hint}</p>{/if}
        {#if result.warn_key_perms}<p class="msg hint">{result.warn_key_perms}</p>{/if}
        {#if result.detail}
          <details class="detail">
            <summary>Details</summary>
            <pre class="mono">{result.detail}</pre>
          </details>
        {/if}
      {/if}
    </LoadState>
  {/if}
  <div class="actions">
    <button class="btn small primary" onclick={onreconnect} data-testid="ssh-exit-reconnect">
      <Icon name="refresh" size={12} />Reconnect
    </button>
    {#if conn && onedit}
      {@const c = conn}
      <button class="btn small" onclick={() => onedit(c)}>
        <Icon name="edit" size={12} />Edit connection
      </button>
    {/if}
    {@render extra?.({ result })}
  </div>
</section>

<style>
  .exit-panel {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-block-start: 1px solid var(--border);
    background: var(--surface);
    font-size: var(--fs-s);
    max-height: 40%;
    overflow-y: auto;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    color: var(--danger);
    font-weight: 600;
  }
  .title {
    min-width: 0;
  }
  .msg {
    margin: 0;
    line-height: 1.4;
    overflow-wrap: anywhere;
  }
  .msg.err {
    color: var(--text);
  }
  .msg.hint {
    color: var(--text-dim);
  }
  .detail summary {
    cursor: pointer;
    color: var(--text-dim);
  }
  .detail pre {
    margin: 4px 0 0;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 160px;
    overflow-y: auto;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
</style>
