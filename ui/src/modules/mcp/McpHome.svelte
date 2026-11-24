<script lang="ts">
  // The MCP Control Plane's landing (#/mcp): what needs you first — each
  // waiting approval as an inbox row (who asks, the request in words, what it
  // will do) with Approve as the one primary and Deny beside it — and, when
  // nothing waits, four sentences in the reading column with one link each
  // (mcpHome.ts). On a phone a row opens a bottom sheet with 44 px actions.
  // It reads the pending approvals and servers the page loads; the tool
  // catalog, the auto-approve rules and the denied requests are its own three.
  // One notice line for an outage, one Retry.
  import { untrack } from 'svelte';
  import ApprovalActions from '../../lib/components/ApprovalActions.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import InboxFold from '../../lib/components/InboxFold.svelte';
  import InboxRow from '../../lib/components/InboxRow.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import type { McpApproval, McpOttoServerStatus, McpServerDetail } from '../../lib/api/types';
  import { loadErrorText } from '../../lib/loadError';
  import { router } from '../../lib/router.svelte';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { waitingWord, wordTone } from '../../lib/statusWords';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { canDecideTriageApproval, requesterMayDecideApproval, triageBlockedReason } from '../../shell/triageModel';
  import { mcpCpExtraApi } from './cp-api';
  import { argsLine, pendingOf, summarySentences } from './mcpHome';

  interface Props {
    wsId: string | null;
    approvals: McpApproval[] | null;
    approvalsError: string | null;
    servers: McpServerDetail[];
    serversLoaded: boolean;
    serversError: string | null;
    /** Reload what the page owns (approvals, servers). */
    onretry: () => void;
    /** A decision landed: refresh the approvals now. */
    ondecided: () => void;
  }
  let { wsId, approvals, approvalsError, servers, serversLoaded, serversError, onretry, ondecided }: Props = $props();

  let status = $state<McpOttoServerStatus | null>(null);
  let rules = $state<number | null>(null);
  /** Denied requests (newest 200) — "N calls denied in the last 7 days". */
  let denied = $state<McpApproval[]>([]);
  let ownError = $state<string | null>(null);
  async function load(): Promise<void> {
    ownError = null;
    const [s, r, d] = await Promise.allSettled([mcpCpApi.cpOttoServer(), mcpCpExtraApi.autoApproveRules(), mcpCpApi.cpApprovals('denied')]);
    if (s.status === 'fulfilled') status = s.value;
    if (r.status === 'fulfilled') rules = r.value.rules.filter((x) => x.enabled).length;
    if (d.status === 'fulfilled') denied = d.value;
    const failed = [s, r, d].find((x) => x.status === 'rejected') as PromiseRejectedResult | undefined;
    if (failed) ownError = loadErrorText(failed.reason);
  }
  $effect(() => untrack(() => void load()));

  /** One outage, one line (the first failure says why). */
  const outage = $derived(approvalsError ?? serversError ?? ownError);
  function retry(): void {
    onretry();
    void load();
  }

  const pending = $derived(pendingOf(approvals ?? []));
  $effect(() => {
    for (const a of pending) if (a.server_id) void resourceAccess.load('mcp_server', a.server_id, a.tool ?? undefined);
  });
  const canDecide = (a: McpApproval): boolean =>
    canDecideTriageApproval(a, auth.me?.id, (id, tool) => resourceAccess.can('mcp_server', id, 'approve', 'mcp', 'admin', tool), () => auth.can('mcp', 'admin'));
  const KIND_WORD: Record<string, string> = { mcp_server: 'An MCP server', gateway: 'The gateway', agent: 'An agent', user: 'A person' };
  /** Who asks, in words: the requesting session's title, else what kind of caller. */
  function who(a: McpApproval): string {
    const sid = a.requested_by_session_id;
    const title = sid ? ws.getSession(sid)?.title : null;
    if (title) return title;
    if (sid) return `Session ${sid.slice(0, 8)}`;
    if (a.requested_by === auth.me?.id && !requesterMayDecideApproval(a)) return 'You';
    return KIND_WORD[a.requested_by_kind ?? ''] ?? 'Someone';
  }

  let busy = $state<Record<string, 'approve' | 'deny'>>({});
  let open = $state<McpApproval | null>(null);
  async function decide(a: McpApproval, approved: boolean, note?: string | null): Promise<void> {
    if (!canDecide(a) || busy[a.id]) return;
    busy = { ...busy, [a.id]: approved ? 'approve' : 'deny' };
    try {
      await mcpCpApi.cpDecide(a.id, { approved, note: note?.trim() || null });
      toasts.success(approved ? 'Approved' : 'Denied', a.title);
      if (open?.id === a.id) open = null;
      ondecided();
    } catch (e) {
      toastError('Couldn’t record the decision', e);
    } finally {
      const next = { ...busy };
      delete next[a.id];
      busy = next;
    }
  }

  const sentences = $derived(
    summarySentences({
      servers: wsId && serversLoaded && !serversError ? servers : null,
      tools: status?.tools ?? null,
      rules,
      approvalsOn: status?.require_approval_dangerous ?? true,
      approvals: denied,
      now: Date.now(),
    }),
  );
  const word = (a: McpApproval) => waitingWord([a.kind === 'human_ask' ? 'question' : 'tool_call']);
  function prettyArgs(json: string): string {
    try {
      return JSON.stringify(JSON.parse(json), null, 2);
    } catch {
      return json;
    }
  }
</script>

<div class="home read-col" data-testid="mcp-home">
  {#if outage}
    <!-- One outage, one line, one Retry (it reloads everything this view reads). -->
    <div data-testid="mcp-outage"><LoadState what="the MCP control plane" variant="compact" error={outage} empty onretry={retry} /></div>
  {/if}

  {#if approvals === null && !approvalsError}
    <LoadState what="approvals" loading empty rows={2} />
  {:else if pending.length > 0}
    <InboxFold label="Waiting for you">
      {#each pending as a (a.id)}
        <div class="appr" data-testid="mcp-home-approval">
          <InboxRow
            onPage
            strong
            title={a.detail ?? a.title}
            word={word(a)}
            tone={wordTone(word(a))}
            tip="Open the request"
            onclick={() => (open = a)}
          >
            {#snippet context()}<span class="who ellipsis" dir="auto">{who(a)} asks</span>{/snippet}
            {#snippet line3()}<span class="ellipsis mono">{argsLine(a)}</span>{#if a.expires_at}<span class="exp">· expires {rel(a.expires_at)}</span>{/if}{/snippet}
          </InboxRow>
          <div class="appr-act">
            <ApprovalActions busy={busy[a.id] ?? null} disabled={!canDecide(a)} disabledReason={triageBlockedReason(a, auth.me?.id)}
              denyTarget="this {a.kind === 'human_ask' ? 'request' : 'tool call'}" onapprove={() => decide(a, true)} ondeny={(reason) => decide(a, false, reason)} />
          </div>
        </div>
      {/each}
    </InboxFold>
  {:else}
    <section class="calm read-prose" aria-label="Summary" data-testid="mcp-summary">
      <p class="lead"><Icon name="check" size={16} /> Nothing waits on you.</p>
      {#each sentences as s (s.testid)}
        <p data-testid={s.testid}>{s.text} <button class="link" onclick={() => router.go(s.to)}>{s.link}</button></p>
      {/each}
    </section>
  {/if}
</div>

{#if open}
  {@const a = open}
  <Modal bottom title={a.tool ?? a.title} width={520} onclose={() => (open = null)}>
    <dl class="facts">
      <dt>Who asks</dt><dd dir="auto">{who(a)}</dd>
      <dt>What it will do</dt><dd dir="auto">{a.detail ?? a.title}</dd>
      {#if a.server_name}<dt>Server</dt><dd>{a.server_name}</dd>{/if}
      {#if a.expires_at}<dt>Expires</dt><dd>{rel(a.expires_at)}</dd>{/if}
    </dl>
    {#if a.args_redacted_json && a.args_redacted_json !== '{}'}<pre class="args">{prettyArgs(a.args_redacted_json)}</pre>{/if}
    <p class="dim small">Values are redacted. To stop being asked for this tool, use “Approve &amp; always allow…” in <button class="link" onclick={() => { open = null; router.go('mcp/activity'); }}>Activity</button>.</p>
    {#snippet footer()}
      <ApprovalActions size="normal" busy={busy[a.id] ?? null} disabled={!canDecide(a)} disabledReason={triageBlockedReason(a, auth.me?.id)}
        denyTarget="this {a.kind === 'human_ask' ? 'request' : 'tool call'}" onapprove={() => decide(a, true)} ondeny={(reason) => decide(a, false, reason)} />
    {/snippet}
  </Modal>
{/if}

<style>
  .home {
    display: flex;
    flex-direction: column;
    gap: var(--sp-6);
    padding-block: var(--sp-6);
  }
  .appr {
    display: flex;
    align-items: center;
    gap: var(--sp-5);
    border-block-end: 1px solid var(--separator);
  }
  .appr > :global(.tr-row) {
    flex: 1;
    min-inline-size: 0;
  }
  .appr-act {
    flex: none;
  }
  .who {
    min-inline-size: 0;
  }
  .exp {
    flex: none;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .calm p {
    margin: 0 0 var(--sp-5);
  }
  .lead {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    font-weight: 600;
  }
  .lead > :global(svg) {
    color: var(--success);
  }
  .link {
    border: 0;
    padding: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .facts {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: var(--sp-3) var(--sp-5);
    margin: 0 0 var(--sp-5);
    font-size: var(--fs-m);
  }
  .facts dt {
    color: var(--text-dim);
  }
  .facts dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .args {
    max-block-size: 220px;
    overflow: auto;
    margin: 0 0 var(--sp-5);
    padding: var(--sp-4);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: var(--fs-s);
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
  }
  /* Phone: the row is one tap target; Approve / Deny live in its sheet. */
  @media (max-width: 640px) {
    .appr-act {
      display: none;
    }
  }
  @media (pointer: coarse) {
    .link {
      min-block-size: 44px;
    }
  }
</style>
