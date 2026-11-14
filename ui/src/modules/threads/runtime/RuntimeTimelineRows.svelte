<script lang="ts">
  // Timeline message & work-log rows for `RuntimeThread.svelte` (shared 100%
  // between the main Threads module and `EmbeddedRuntimeThread` in IDE / PRs /
  // Personal Agents). Strips `<ide_context>` and file-context envelopes into
  // clickable badges so user bubbles stay clean in every surface.
  import type { Snippet } from 'svelte';
  import type { ApprovalDecision, TurnItem } from '../../../lib/api/types';
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import { activatePathLink } from '../../../lib/pathLinks/open';
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { threadCall } from './agentRows';
  import { parseUserMessageWithBadges, type ParsedContextBadge } from './mentionMatcher';
  import type { RuntimeProjection } from './projection';
  import RuntimeApprovalRow from './RuntimeApprovalRow.svelte';
  import RuntimeAttachments from './RuntimeAttachments.svelte';
  import RuntimeSubagentCard from './RuntimeSubagentCard.svelte';
  import RuntimeThreadCallRow from './RuntimeThreadCallRow.svelte';
  import RuntimeWorkFold from './RuntimeWorkFold.svelte';
  import RuntimeWorkGroup from './RuntimeWorkGroup.svelte';
  import RuntimeWorkRow from './RuntimeWorkRow.svelte';
  import {
    approvalOutcome,
    formatMessageClock,
    formatMessageTooltip,
    isRunHandoff,
    messageAttribution,
    pendingApprovals,
    subagentChildren,
    timelineRows,
    type WorkEntry,
  } from './timeline';
  import { formatDuration, shouldFold } from './workLog';

  interface Props {
    sessionId: string;
    projection: RuntimeProjection;
    agentName: string;
    agentChatId?: string | null;
    cwd?: string | null;
    workspaceId?: string | null;
    onrespond: (requestId: string, decision: ApprovalDecision) => Promise<void>;
    onopenpath?: (path: string, line?: number) => void;
    formatAssistantText?: (item: Extract<TurnItem, { type: 'assistant_message' }>) => string;
    assistantFooter?: Snippet<[Extract<TurnItem, { type: 'assistant_message' }>]>;
  }

  let {
    sessionId,
    projection,
    agentName,
    agentChatId = null,
    cwd = null,
    workspaceId = null,
    onrespond,
    onopenpath,
    formatAssistantText,
    assistantFooter,
  }: Props = $props();

  const BADGE_ICONS: Record<ParsedContextBadge['kind'], IconName> = {
    selection: 'cursor',
    file: 'file',
    folder: 'folder',
    symbol: 'zap',
    diagnostic: 'warning',
    test_failure: 'flask',
    debug_frame: 'bug',
    run_output: 'terminal',
  };

  const rows = $derived(timelineRows(projection.items, projection.turns));
  const approvals = $derived(new Map(pendingApprovals(projection).map((a) => [a.requestId, a] as const)));
  const pendingIds = $derived(new Set(approvals.keys()));
  const requests = $derived(new Map(projection.requests.map((r) => [r.id, r] as const)));
  const children = $derived(subagentChildren(projection.items));

  function handleBadgeClick(b: ParsedContextBadge): void {
    if (!b.path) return;
    if (onopenpath) {
      onopenpath(b.path, b.line);
      return;
    }
    void activatePathLink({ path: b.path, line: b.line }, { sessionId, cwd, workspaceId });
  }
</script>

{#snippet entryRow({ item, approval }: WorkEntry)}
  {@const pending = item.type === 'approval_request' ? approvals.get(item.request_id) : undefined}
  {@const call = item.type === 'dynamic_tool' && !item.error && !approval ? threadCall(item) : null}
  {#if item.type === 'subagent'}
    <RuntimeSubagentCard {item} activity={children.get(item.id) ?? []} output={projection.output} />
  {:else if call}
    <RuntimeThreadCallRow {item} {call} />
  {:else if pending}
    <RuntimeApprovalRow approval={pending} {onrespond} />
  {:else}
    {@const ask = approval ?? (item.type === 'approval_request' ? item : undefined)}
    {@const gate = approval ? approvals.get(approval.request_id) : undefined}
    <RuntimeWorkRow
      {item}
      output={projection.output[item.id]}
      pending={gate}
      note={ask && !gate ? approvalOutcome(ask, requests.get(ask.request_id), agentName) : null}
      {onrespond}
      agentId={agentChatId ?? undefined}
      cwd={cwd ?? undefined}
    />
  {/if}
{/snippet}

{#each rows as row (row.kind === 'message' ? row.item.id : row.id)}
  {#if row.kind === 'session'}
    <div class="rt-day" role="separator" aria-label="New session" data-testid="runtime-session-boundary"><span>New session</span></div>
  {:else if row.kind === 'agent_turn'}
    <div class="rt-day rt-agent-turn" role="separator" aria-label={row.label} data-testid="runtime-agent-turn-divider">
      <Icon name="clock" size={12} /><span>{row.label}</span>
    </div>
  {:else if row.kind === 'day'}
    <div class="rt-day" role="separator" aria-label={row.label}><span>{row.label}</span></div>
  {:else if row.kind === 'message' && row.item.type === 'user_message' && isRunHandoff(row.item.text)}
    <!-- A routine's report handed to the agent: its result card and the agent's reply show it. -->
  {:else if row.kind === 'message' && row.item.type === 'user_message' && messageAttribution(row.item.text)}
    {@const a = messageAttribution(row.item.text)!}
    {@const peerIso = row.item.started_at ?? row.item.updated_at}
    {@const peerClock = formatMessageClock(peerIso)}
    <div class="rt-from" data-testid="rt-from">
      <span class="rt-from-h">
        <Icon name="send" size={12} /> From {a.from}
        {#if peerClock && peerIso}
          <time class="rt-msg-time" datetime={peerIso} title={formatMessageTooltip(peerIso)}>{peerClock}</time>
        {/if}
      </span>
      <Markdown md={a.body} />
    </div>
  {:else if row.kind === 'message' && row.item.type === 'user_message'}
    {@const parsed = parseUserMessageWithBadges(row.item.text)}
    {@const userIso = row.item.started_at ?? row.item.updated_at}
    {@const userClock = formatMessageClock(userIso)}
    <div class="rt-user">
      {#if row.item.attachments?.length}<RuntimeAttachments {sessionId} attachments={row.item.attachments} />{/if}
      {#if parsed.badges.length > 0}
        <div class="rt-ctx-badges">
          {#each parsed.badges as b, bi (`${bi}:${b.label}`)}
            {#if b.path}
              <button
                type="button"
                class="rt-ctx-badge"
                dir="ltr"
                title={b.path}
                onclick={() => handleBadgeClick(b)}
              >
                <Icon name={BADGE_ICONS[b.kind] ?? 'file'} size={12} />
                <span class="ellipsis">{b.label}</span>
              </button>
            {:else}
              <span class="rt-ctx-badge static" dir="ltr">
                <Icon name={BADGE_ICONS[b.kind] ?? 'file'} size={12} />
                <span class="ellipsis">{b.label}</span>
              </span>
            {/if}
          {/each}
        </div>
      {/if}
      {#if parsed.prose || (parsed.badges.length === 0 && row.item.text)}
        <div class="rt-bubble" dir="auto">{parsed.badges.length > 0 ? parsed.prose : row.item.text}</div>
      {/if}
      {#if userClock && userIso}
        <time
          class="rt-msg-time"
          datetime={userIso}
          title={formatMessageTooltip(userIso)}
          data-testid="runtime-user-time"
        >{userClock}</time>
      {/if}
    </div>
  {:else if row.kind === 'message' && row.item.type === 'assistant_message'}
    {@const mdText = formatAssistantText ? formatAssistantText(row.item) : row.item.text}
    {@const startIso = row.item.started_at ?? row.item.updated_at}
    {@const endIso = row.item.completed_at ?? row.item.updated_at}
    {@const activeIso = row.item.streaming ? startIso : endIso}
    {@const assistantClock = formatMessageClock(activeIso)}
    {@const elapsedMs = !row.item.streaming && row.item.started_at && row.item.completed_at ? Date.parse(row.item.completed_at) - Date.parse(row.item.started_at) : 0}
    <div class="rt-assistant" class:streaming={row.item.streaming} dir="auto">
      {#if mdText || !assistantFooter}<Markdown md={mdText} />{/if}
      {#if assistantFooter}{@render assistantFooter(row.item)}{/if}
      {#if assistantClock && activeIso}
        <time
          class="rt-msg-time"
          datetime={activeIso}
          title={formatMessageTooltip(startIso, row.item.completed_at ?? (row.item.streaming ? null : row.item.updated_at), row.item.streaming)}
          data-testid="runtime-assistant-time"
        >{row.item.streaming ? `Started ${assistantClock}` : elapsedMs >= 1000 ? `${assistantClock} · ${formatDuration(elapsedMs)}` : assistantClock}</time>
      {/if}
    </div>
  {:else if row.kind === 'work'}
    {@const flat = row.parts.flatMap((part) => (part.kind === 'group' ? part.entries : [part.entry]))}
    <div class="rt-work">
      <RuntimeWorkFold id={row.id} entries={flat} fold={shouldFold(flat, projection.turns, pendingIds)}>
        {#each row.parts as part (part.kind === 'group' ? part.id : part.entry.item.id)}
          {#if part.kind === 'item'}
            {@render entryRow(part.entry)}
          {:else}
            <RuntimeWorkGroup id={part.id} entries={part.entries} cwd={cwd ?? undefined} open={part.entries.some((e) => e.approval && approvals.has(e.approval.request_id))}>
              {#each part.entries as e (e.item.id)}{@render entryRow(e)}{/each}
            </RuntimeWorkGroup>
          {/if}
        {/each}
      </RuntimeWorkFold>
    </div>
  {/if}
{/each}

<style>
  .rt-user {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 6px;
    padding-block-end: 12px;
  }
  .rt-user:not(:first-child) {
    margin-block-start: 20px;
  }
  .rt-ctx-badges {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 4px;
    max-inline-size: 92%;
  }
  .rt-ctx-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-inline-size: 240px;
    padding: 2px 8px;
    border: 1px solid var(--accent-line);
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent-text);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .rt-ctx-badge:hover:not(.static) {
    background: var(--accent-soft-strong);
  }
  .rt-ctx-badge.static {
    cursor: default;
  }
  .rt-bubble {
    max-inline-size: min(80%, 78ch);
    padding: 8px 14px;
    border: 1px solid color-mix(in srgb, var(--accent) 34%, var(--border));
    border-radius: var(--radius-l);
    background: color-mix(in srgb, var(--accent) 16%, var(--surface));
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    unicode-bidi: plaintext;
  }
  .rt-assistant {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-inline-size: 0;
    padding-block-end: 12px;
    color: var(--text);
  }
  .rt-assistant :global(:is(code, a.ref-chip)) {
    unicode-bidi: isolate;
  }
  .rt-assistant.streaming :global(.md > :last-child)::after {
    content: '▍';
    margin-inline-start: 2px;
    color: var(--text-dim);
  }
  .rt-day {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-block: 8px 14px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rt-day::before,
  .rt-day::after {
    content: '';
    flex: 1;
    border-block-start: 1px solid var(--border);
  }
  .rt-from {
    margin-block: 12px;
    padding: 8px 12px;
    border-inline-start: 3px solid var(--accent-solid);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    min-inline-size: 0;
  }
  .rt-from-h {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--accent-text);
  }
  .rt-msg-time {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 400;
    line-height: 1.2;
    font-variant-numeric: tabular-nums;
    align-self: flex-start;
  }
  .rt-user > .rt-msg-time {
    align-self: flex-end;
    padding-inline: 4px;
  }
  .rt-from-h > .rt-msg-time {
    align-self: center;
    margin-inline-start: 4px;
  }
  .rt-work {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 0;
    padding-block-end: 10px;
  }
</style>
