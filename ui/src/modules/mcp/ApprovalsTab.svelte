<script lang="ts">
  // The approval queue — dangerous tool calls and `otto.ask_human_approval`
  // requests waiting on a human. Shows the redacted args (never the full/secret
  // values; the server binds the hash of the FULL args). Approve/Deny with an
  // optional note. The requester cannot approve their own request (enforced
  // server-side). Polls every few seconds and on tab refocus so a new pending
  // request appears without a manual reload.
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import type { McpApproval } from '../../lib/api/types';
  import McpPill from './McpPill.svelte';

  let approvals = $state<McpApproval[]>([]);
  let loading = $state(false);
  let busy = $state<Record<string, boolean>>({});
  let notes = $state<Record<string, string>>({});
  let showAll = $state(false);

  async function load(): Promise<void> {
    loading = true;
    try {
      approvals = await mcpCpApi.cpApprovals(showAll ? undefined : 'pending');
    } catch (e) {
      toasts.error('Failed to load approvals', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  // Poll every 5s, and immediately whenever the tab/window regains focus.
  $effect(() => {
    void showAll; // re-load when the filter flips
    void load();
    const t = setInterval(() => void load(), 5000);
    const onVis = (): void => {
      if (document.visibilityState === 'visible') void load();
    };
    document.addEventListener('visibilitychange', onVis);
    window.addEventListener('focus', onVis);
    return () => {
      clearInterval(t);
      document.removeEventListener('visibilitychange', onVis);
      window.removeEventListener('focus', onVis);
    };
  });

  async function decide(a: McpApproval, approved: boolean): Promise<void> {
    busy = { ...busy, [a.id]: true };
    try {
      await mcpCpApi.cpDecide(a.id, { approved, note: notes[a.id]?.trim() || null });
      toasts.success(approved ? 'Approved' : 'Denied', a.title);
      await load();
    } catch (e) {
      toasts.error('Decision failed', e instanceof Error ? e.message : String(e));
    } finally {
      const n = { ...busy };
      delete n[a.id];
      busy = n;
    }
  }

  function prettyArgs(json: string): string {
    try {
      return JSON.stringify(JSON.parse(json), null, 2);
    } catch {
      return json;
    }
  }
</script>

<div class="appr">
  <div class="bar">
    <span class="count">{approvals.length} {showAll ? 'total' : 'pending'}</span>
    <span class="grow"></span>
    <label class="check">
      <input type="checkbox" bind:checked={showAll} />
      <span>Show decided too</span>
    </label>
    <button class="btn small" onclick={() => void load()} title="Refresh"><Icon name="refresh" size={13} /></button>
  </div>

  {#if loading && approvals.length === 0}
    <p class="muted pad">Loading…</p>
  {:else if approvals.length === 0}
    <div class="empty">
      <Icon name="check" size={24} />
      <p>{showAll ? 'No approvals yet.' : 'Nothing waiting on you — the queue is clear.'}</p>
    </div>
  {:else}
    <div class="list">
      {#each approvals as a (a.id)}
        <div class="card">
          <div class="chead">
            <span class="kind">{a.kind === 'human_ask' ? 'human ask' : 'tool call'}</span>
            <span class="title">{a.title}</span>
            {#if a.risk_label}<McpPill kind="risk" value={a.risk_label} small />{/if}
            <McpPill kind="status" value={a.status} small />
            <span class="grow"></span>
            <span class="when">{new Date(a.created_at).toLocaleString()}</span>
          </div>
          <div class="meta">
            {#if a.server_name || a.tool}
              <span class="route mono">{a.server_name ?? '—'}{a.tool ? ` → ${a.tool}` : ''}</span>
            {/if}
            {#if a.requested_by}<span class="by">requested by {a.requested_by}{a.requested_by_kind ? ` (${a.requested_by_kind})` : ''}</span>{/if}
            {#if a.expires_at}<span class="by">expires {new Date(a.expires_at).toLocaleString()}</span>{/if}
          </div>
          {#if a.detail}<p class="detail">{a.detail}</p>{/if}
          {#if a.args_redacted_json && a.args_redacted_json !== '{}'}
            <pre class="args">{prettyArgs(a.args_redacted_json)}</pre>
          {/if}

          {#if a.status === 'pending'}
            <div class="actions">
              <input
                class="note"
                placeholder="Note (optional)"
                value={notes[a.id] ?? ''}
                oninput={(e) => (notes = { ...notes, [a.id]: (e.currentTarget as HTMLInputElement).value })}
              />
              <button class="btn small ok" disabled={busy[a.id]} onclick={() => void decide(a, true)}>
                {busy[a.id] ? '…' : 'Approve'}
              </button>
              <button class="btn small danger" disabled={busy[a.id]} onclick={() => void decide(a, false)}>
                {busy[a.id] ? '…' : 'Deny'}
              </button>
            </div>
          {:else}
            <div class="decided">
              {a.status}{a.decided_by ? ` by ${a.decided_by}` : ''}
              {#if a.decision_note}· “{a.decision_note}”{/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .appr {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .count {
    font-size: 12px;
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--text);
  }
  .list {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-m, 8px);
    background: var(--surface);
    padding: 10px 12px;
  }
  .chead {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .kind {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    border-radius: 4px;
    padding: 1px 6px;
  }
  .title {
    font-size: 13.5px;
    font-weight: 600;
  }
  .when {
    font-size: 11px;
    color: var(--text-dim);
  }
  .meta {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    margin-top: 6px;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .route {
    color: var(--text);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .detail {
    margin: 8px 0 0;
    font-size: 12.5px;
    color: var(--text);
  }
  .args {
    margin: 8px 0 0;
    font-family: var(--font-mono);
    font-size: 11.5px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    padding: 8px;
    max-height: 220px;
    overflow: auto;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
  }
  .note {
    flex: 1;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 6px);
    color: var(--text);
    padding: 6px 9px;
    font-size: 12.5px;
  }
  .btn.ok {
    color: var(--status-working, #28c840);
  }
  .btn.danger {
    color: var(--status-exited, #ff5f57);
  }
  .decided {
    margin-top: 8px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .muted {
    color: var(--text-dim);
  }
  .pad {
    padding: 16px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    color: var(--text-dim);
    text-align: center;
    padding: 40px 24px;
  }
</style>
