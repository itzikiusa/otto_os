<script lang="ts">
  import { toastError } from '../../lib/toastError';
  // The approval queue — dangerous tool calls and `otto.ask_human_approval`
  // requests waiting on a human. Shows the redacted args (never the full/secret
  // values; the server binds the hash of the FULL args). Approve/Deny with the
  // card's one optional note (Deny does not ask a second time). The requester
  // cannot approve their own DIRECT request (enforced server-side) — but a request raised by their own agent session /
  // MCP client on their behalf is exactly what they're meant to decide, since
  // every Otto session authorizes as its owner (mirrors
  // `AGENT_REQUESTER_KINDS` in crates/otto-state/src/mcp_control.rs). Polls
  // every few seconds and on tab refocus so a new pending request appears
  // without a manual reload. "Approve & always allow…" on an `otto.*` tool
  // call opens the auto-approve rule form prefilled with that tool and the
  // requesting session (else its workspace), then approves this request.
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import AgentByline from '../../lib/components/AgentByline.svelte';
  import ApprovalActions from '../../lib/components/ApprovalActions.svelte';
  import ApprovalOutcome from '../../lib/components/ApprovalOutcome.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { liveQuery } from '../../lib/live';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import type { McpApproval, McpAutoApproveList } from '../../lib/api/types';
  import McpPill from './McpPill.svelte';
  import AutoApproveForm from './AutoApproveForm.svelte';
  import { mcpCpExtraApi } from './cp-api';

  let approvals = $state<McpApproval[]>([]);
  const AGENT_REQUESTER_KINDS = ['mcp_server', 'gateway', 'agent'];
  const requesterMayDecide = (a: McpApproval): boolean => AGENT_REQUESTER_KINDS.includes(a.requested_by_kind ?? '');
  const canDecide = (a: McpApproval): boolean => {
    // Not your own direct request (an agent raising it on your behalf is fine)…
    const allowedRequester = a.requested_by !== auth.me?.id || requesterMayDecide(a);
    // …and approve access on the server (or MCP admin for server-less asks).
    const hasAccess = a.server_id
      ? resourceAccess.can('mcp_server', a.server_id, 'approve', 'mcp', 'admin', a.tool ?? undefined)
      : auth.can('mcp', 'admin');
    return allowedRequester && hasAccess;
  };
  $effect(() => {
    for (const approval of approvals) {
      if (approval.server_id) void resourceAccess.load('mcp_server', approval.server_id, approval.tool ?? undefined);
    }
  });
  let loading = $state(false);
  /** Failed load — inline with Retry, never the empty state. */
  let loadError = $state<string | null>(null);
  let busy = $state<Record<string, 'approve' | 'deny'>>({});
  let notes = $state<Record<string, string>>({});
  let showAll = $state(false);
  /** Told after a decision so the page header's pending badge updates now, not on its next poll. */
  let { ondecided }: { ondecided?: () => void } = $props();
  /** Why Approve/Deny is greyed out — a disabled button never goes unexplained. */
  const decideBlockedReason = (a: McpApproval): string =>
    a.requested_by === auth.me?.id && !requesterMayDecide(a)
      ? 'You raised this request yourself — another admin has to decide it'
      : 'You don’t have approve access to this server';

  /** The approval whose "always allow" rule form is open, with the catalog it needs. */
  let allowFor = $state<{ approval: McpApproval; catalog: McpAutoApproveList['categories'] } | null>(null);
  let allowLoading = $state<string | null>(null);
  /** Only governed `otto.*` tool calls can be covered by an auto-approve rule. */
  const canAlwaysAllow = (a: McpApproval): boolean =>
    a.kind === 'tool_call' && !a.server_id && !!a.tool?.startsWith('otto.') && canDecide(a);
  const sessionTitle = (id: string): string | null =>
    ws.getSession(id)?.title ?? null;
  const requesterLabel = (a: McpApproval): string =>
    a.requested_by === auth.me?.id ? 'you' : (a.requested_by ?? 'unknown');

  async function openAlwaysAllow(a: McpApproval): Promise<void> {
    allowLoading = a.id;
    try {
      const list = await mcpCpExtraApi.autoApproveRules();
      allowFor = { approval: a, catalog: list.categories };
    } catch (e) {
      toasts.error('Couldn’t load the auto-approve catalog', e instanceof Error ? e.message : String(e));
    } finally {
      allowLoading = null;
    }
  }

  function openSession(id: string): void {
    ws.navigateToSession(id);
  }

  let loadSeq = 0;
  async function load(): Promise<boolean> {
    const seq = ++loadSeq;
    loading = true;
    try {
      const list = await mcpCpApi.cpApprovals(showAll ? undefined : 'pending');
      if (seq !== loadSeq) return true; // a newer load (filter flip) owns the view
      approvals = list;
      loadError = null;
      return true;
    } catch (e) {
      if (seq === loadSeq) loadError = loadErrorText(e);
      return false;
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  // Event-fed: reload when `mcp_approval_changed` says the list changed (plus
  // a safety net / reconnect resync); the old 5 s poll runs only while the
  // event socket is down. Same chain rules (no overlap, paused while hidden,
  // backoff), and immediately whenever the window regains focus.
  $effect(() => {
    void showAll; // re-load when the filter flips
    const p = liveQuery({ run: () => load(), on: ['mcp_approval_changed'], fallbackMs: 5000 });
    const onFocus = (): void => p.now();
    window.addEventListener('focus', onFocus);
    return () => {
      p.stop();
      window.removeEventListener('focus', onFocus);
    };
  });

  async function decide(a: McpApproval, approved: boolean, reason?: string | null): Promise<void> {
    if (!canDecide(a)) return;
    busy = { ...busy, [a.id]: approved ? 'approve' : 'deny' };
    try {
      await mcpCpApi.cpDecide(a.id, { approved, note: reason?.trim() || notes[a.id]?.trim() || null });
      toasts.success(approved ? 'Approved' : 'Denied', a.title);
      ondecided?.();
      await load();
    } catch (e) {
      toastError('Couldn’t decision', e);
    } finally {
      const n = { ...busy };
      delete n[a.id];
      busy = n;
    }
  }

  /** `approved` → “Approved” — the raw enum never reaches the screen. */
  const statusLabel = (status: string): string => {
    const t = status.replace(/_/g, ' ');
    return t.charAt(0).toUpperCase() + t.slice(1);
  };

  function prettyArgs(json: string): string {
    try {
      return JSON.stringify(JSON.parse(json), null, 2);
    } catch {
      return json;
    }
  }
</script>

<div class="appr" data-testid="mcp-approvals">
  <div class="bar">
    <h2>Approvals</h2>
    <span class="count chip" class:warn={!showAll && approvals.length > 0}>{approvals.length} {showAll ? 'total' : 'pending'}</span>
    <span class="grow"></span>
    <label class="check">
      <input type="checkbox" bind:checked={showAll} />
      <span>Show decided too</span>
    </label>
    <button class="btn small" onclick={() => void load()} title="Refresh" aria-label="Refresh approvals"><Icon name="refresh" size={13} /></button>
  </div>

  <!-- One wrapper owns every state: skeleton on first load, inline error + Retry,
       and — when a REFRESH fails with approvals on screen — the stale bar, so a
       security queue never silently shows old data as current. -->
  <LoadState what="approvals" {loading} error={loadError} empty={approvals.length === 0} onretry={() => void load()}>
    {#snippet emptyView()}
      <div class="empty">
        <Icon name="check" size={24} />
        <p>{showAll ? 'No approvals yet.' : 'Nothing waiting on you — the queue is clear.'}</p>
      </div>
    {/snippet}
    <div class="list">
      {#each approvals as a (a.id)}
        <div class="card">
          <div class="chead">
            <span class="chip">{a.kind === 'human_ask' ? 'Human ask' : 'Tool call'}</span>
            <span class="title">{a.title}</span>
            {#if a.risk_label}<McpPill kind="risk" value={a.risk_label} small />{/if}
            <McpPill kind="status" value={a.status} small />
            <span class="grow"></span>
            <span class="when" title={new Date(a.created_at).toLocaleString()}>{rel(a.created_at)}</span>
          </div>
          <div class="meta">
            {#if a.server_name || a.tool}
              <span class="route mono">{a.server_name ?? '—'}{a.tool ? ` → ${a.tool}` : ''}</span>
            {/if}
            {#if a.requested_by_session_id}
              {@const sid = a.requested_by_session_id}
              <span class="by">
                from session
                <button class="link" type="button" onclick={() => openSession(sid)} title="Open the requesting session" data-testid="mcp-approval-session">
                  {sessionTitle(sid) ?? `${sid.slice(0, 8)}…`}
                </button>
                ·
                {#if requesterMayDecide(a)}<AgentByline name={requesterLabel(a)} />{:else}{requesterLabel(a)}{/if}
              </span>
            {:else if a.requested_by}
              <span class="by">
                requested by
                {#if requesterMayDecide(a)}<AgentByline name={requesterLabel(a)} />{:else}{requesterLabel(a)}{/if}
                {a.requested_by_kind ? ` (${a.requested_by_kind})` : ''}
              </span>
            {/if}
            {#if a.expires_at}<span class="by" title={new Date(a.expires_at).toLocaleString()}>expires {rel(a.expires_at)}</span>{/if}
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
              <ApprovalActions
                busy={busy[a.id] ?? null}
                disabled={!canDecide(a)}
                disabledReason={decideBlockedReason(a)}
                denyTarget="this {a.kind === 'human_ask' ? 'request' : 'tool call'}"
                askReason={false}
                onapprove={() => decide(a, true)}
                ondeny={(reason) => decide(a, false, reason)}
              >
                {#snippet extra()}
                  {#if canAlwaysAllow(a)}
                    <button
                      class="btn small"
                      disabled={busy[a.id] !== undefined || allowLoading === a.id}
                      title="Approve this call and stop asking for {a.tool} — opens the auto-approve rule form"
                      data-testid="mcp-approve-always"
                      onclick={() => void openAlwaysAllow(a)}
                    >
                      {allowLoading === a.id ? '…' : 'Approve & always allow…'}
                    </button>
                  {/if}
                {/snippet}
              </ApprovalActions>
            </div>
          {:else if a.status === 'approved' || a.status === 'denied'}
            <div class="decided">
              <ApprovalOutcome outcome={a.status} by={a.decided_by} at={a.decided_at} note={a.decision_note} />
            </div>
          {:else if a.status === 'expired'}
            <div class="decided">
              <ApprovalOutcome outcome="expired" at={a.expires_at ?? a.decided_at} note={a.decision_note} />
            </div>
          {:else}
            <div class="decided">
              {statusLabel(a.status)}{a.decided_by ? ` by ${a.decided_by}` : ''}
              {#if a.decision_note}· “{a.decision_note}”{/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  </LoadState>
</div>

{#if allowFor}
  {@const a = allowFor.approval}
  {@const sid = a.requested_by_session_id ?? null}
  <AutoApproveForm
    catalog={allowFor.catalog}
    preset={{ kind: 'tool', target: a.tool ?? '' }}
    presetScope={sid
      ? { scope: 'session', session_id: sid, session_label: sessionTitle(sid) ?? `Session ${sid.slice(0, 8)}…` }
      : a.workspace_id
        ? { scope: 'workspace', workspace_id: a.workspace_id }
        : null}
    title="Approve & always allow {a.tool}"
    saveLabel="Create rule & approve"
    onclose={() => (allowFor = null)}
    onsaved={() => decide(a, true)}
  />
{/if}

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
  h2 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .count {
    font-size: var(--fs-s);
  }
  .grow {
    flex: 1;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-m);
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
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 10px 12px;
  }
  .chead {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .title {
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .when {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .meta {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    margin-top: 6px;
    font-size: var(--fs-s);
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
    font-size: var(--fs-m);
    color: var(--text);
  }
  .args {
    margin: 8px 0 0;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
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
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 6px 9px;
    font-size: var(--fs-m);
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
    text-decoration: underline;
  }
  .link:focus-visible {
    outline: 2px solid var(--accent-solid);
    outline-offset: 2px;
  }
  .decided {
    margin-top: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
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
