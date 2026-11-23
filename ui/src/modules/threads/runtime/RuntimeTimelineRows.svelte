<script lang="ts">
  // Timeline message & work-log rows for `RuntimeThread.svelte` (shared 100%
  // between the main Threads module and `EmbeddedRuntimeThread` in IDE / PRs /
  // Personal Agents). Strips `<ide_context>` and file-context envelopes into
  // clickable badges so user bubbles stay clean in every surface.
  import type { Snippet } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { copyWithToast } from '../../../lib/clipboard';
  import type { ApprovalDecision, ProviderTurn, RuntimeRequest, TurnItem } from '../../../lib/api/types';
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import { activatePathLink } from '../../../lib/pathLinks/open';
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { threadCall } from './agentRows';
  import { parseUserMessageItemWithBadges, type ParsedContextBadge } from './mentionMatcher';
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
    workRowEntries,
    type PendingApproval,
    type TimelineRow,
    type WorkEntry,
    type WorkPart,
  } from './timeline';
  import RuntimeTurnChanges from './RuntimeTurnChanges.svelte';
  import { turnChanges, type FileDelta } from './turnChanges';
  import { earlierSteps, formatDuration, shouldFold, visibleTools } from './workLog';
  import { threadPrefs } from './threadPrefs.svelte';
  import { turnUsageText } from './turnUsage';
  import { handoffSummary, handoffTitle } from './providerSwitch';

  interface Props {
    sessionId: string;
    projection: RuntimeProjection;
    rows?: TimelineRow[];
    agentName: string;
    agentChatId?: string | null;
    cwd?: string | null;
    workspaceId?: string | null;
    onrespond: (requestId: string, decision: ApprovalDecision) => Promise<void>;
    /** A failed turn's error row while its failure block (RuntimeTurnStatus) says it. */
    hideItemId?: string | null;
    /** "Edit as new message": put a past message of the person's back in the
     *  composer. Sending it starts a NEW turn; history is never rewritten. */
    onedituser?: (item: Extract<TurnItem, { type: 'user_message' }>) => void;
    onopenpath?: (path: string, line?: number) => void;
    formatAssistantText?: (item: Extract<TurnItem, { type: 'assistant_message' }>) => string;
    assistantFooter?: Snippet<[Extract<TurnItem, { type: 'assistant_message' }>]>;
    /** Open the thread's Changes surface on a file (the turn changes card). */
    onopendiff?: (path: string) => void;
  }

  let {
    sessionId,
    projection,
    rows: providedRows,
    agentName,
    agentChatId = null,
    cwd = null,
    workspaceId = null,
    onrespond,
    hideItemId = null,
    onedituser,
    onopenpath,
    formatAssistantText,
    assistantFooter,
    onopendiff,
  }: Props = $props();

  /** Edit as new message: only on the person's latest message (less chrome). */
  const lastUserId = $derived.by(() => {
    for (let i = rows.length - 1; i >= 0; i--) {
      const r = rows[i];
      if (r.kind === 'message' && r.item.type === 'user_message' && !messageAttribution(r.item.text)) return r.item.id;
    }
    return null;
  });
  /** "From …" rows the reader opened past their first three lines. */
  const openedFrom = new SvelteSet<string>();
  const longBody = (t: string): boolean => t.length > 280 || t.split('\n', 5).length > 3;
  /** The last work stretch of each finished turn carries its token count in
   *  its "Worked for …" line (the latest turn says it on its own status line). */
  const tokensByRow = $derived.by(() => {
    const lastRow = new Map<string, string>();
    for (const r of rows) {
      if (r.kind !== 'work') continue;
      const tid = workRowEntries(r)[0]?.item.turn_id;
      if (tid) lastRow.set(tid, r.id);
    }
    const latest = turns.reduce<ProviderTurn | null>((b, t) => (!b || t.ordinal > b.ordinal ? t : b), null);
    const out = new Map<string, string>();
    for (const [tid, rowId] of lastRow) {
      const t = turnById.get(tid);
      const text = t && t !== latest ? turnUsageText(t) : '';
      if (text) out.set(rowId, text);
    }
    return out;
  });

  /** A finished turn that changed files gets its changes card after its last row. */
  const changesByRow = $derived.by(() => {
    const last = new Map<string, string>();
    for (const r of rows) {
      const tid = r.kind === 'message' ? r.item.turn_id : r.kind === 'work' ? workRowEntries(r)[0]?.item.turn_id : null;
      if (tid) last.set(tid, r.kind === 'message' ? r.item.id : r.id);
    }
    const out = new Map<string, FileDelta[]>();
    for (const [tid, key] of last) {
      const t = turnById.get(tid);
      if (!t || t.status === 'running' || t.status === 'pending') continue;
      const files = turnChanges(projection.items, tid, cwd);
      if (files.length) out.set(key, files);
    }
    return out;
  });

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

  const EMPTY_APPROVALS = new Map<string, PendingApproval>();
  const EMPTY_PENDING_IDS = new Set<string>();
  const EMPTY_REQUESTS = new Map<string, RuntimeRequest>();
  const EMPTY_TURNS = new Map<string, ProviderTurn>();
  const EMPTY_ACTIVITY: TurnItem[] = [];

  const turns = $derived(projection.turns);
  const turnById = $derived(turns.length === 0 ? EMPTY_TURNS : new Map(turns.map((t) => [t.id, t] as const)));
  const outputMap = $derived(projection.output);
  const rawRequests = $derived(projection.requests);
  // Tool calls the person chose to hide (⋯ › View) are left out; failures and approvals stay.
  const rows = $derived(visibleTools(providedRows ?? timelineRows(projection.items, turns), threadPrefs.tools));
  const approvals = $derived.by(() => {
    if (rawRequests.length === 0 || !rawRequests.some((r) => r.status === 'pending')) return EMPTY_APPROVALS;
    const pending = pendingApprovals(projection);
    return pending.length === 0 ? EMPTY_APPROVALS : new Map(pending.map((a) => [a.requestId, a] as const));
  });
  const pendingIds = $derived(approvals.size === 0 ? EMPTY_PENDING_IDS : new Set(approvals.keys()));
  const requests = $derived(rawRequests.length === 0 ? EMPTY_REQUESTS : new Map(rawRequests.map((r) => [r.id, r] as const)));
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
  {#if item.id === hideItemId}
    <!-- Said once, by the failed turn's block under the timeline. -->
  {:else if item.type === 'subagent'}
    <RuntimeSubagentCard {sessionId} {item} activity={children.get(item.id) ?? EMPTY_ACTIVITY} output={outputMap} />
  {:else if call}
    <RuntimeThreadCallRow {item} {call} />
  {:else if pending}
    <RuntimeApprovalRow approval={pending} {onrespond} />
  {:else}
    {@const ask = approval ?? (item.type === 'approval_request' ? item : undefined)}
    {@const gate = approval ? approvals.get(approval.request_id) : undefined}
    <RuntimeWorkRow
      {sessionId}
      {item}
      output={outputMap[item.id]}
      pending={gate}
      note={ask && !gate ? approvalOutcome(ask, requests.get(ask.request_id), agentName) : null}
      {onrespond}
      agentId={agentChatId ?? undefined}
      cwd={cwd ?? undefined}
    />
  {/if}
{/snippet}

{#snippet workPart(part: WorkPart)}
  {#if part.kind === 'item'}
    {@render entryRow(part.entry)}
  {:else}
    <RuntimeWorkGroup id={part.id} entries={part.entries} cwd={cwd ?? undefined} open={part.entries.some((e) => e.approval && approvals.has(e.approval.request_id))}>
      {#each part.entries as e (e.item.id)}{@render entryRow(e)}{/each}
    </RuntimeWorkGroup>
  {/if}
{/snippet}

{#each rows as row (row.kind === 'message' ? row.item.id : row.id)}
  {#if row.kind === 'session' && row.handoff}
    <details class="rt-handoff" data-testid="runtime-handoff">
      <summary class="rt-day" aria-label={handoffTitle(row.handoff)}><span>{handoffTitle(row.handoff)}</span></summary>
      <p class="rt-handoff-note">{handoffSummary(row.handoff)}</p>
      {#if row.handoff.text}<pre class="rt-handoff-text" dir="auto">{row.handoff.text}</pre>{/if}
    </details>
  {:else if row.kind === 'session'}
    <div class="rt-day" role="separator" aria-label="New session" data-testid="runtime-session-boundary"><span>New session</span></div>
  {:else if row.kind === 'agent_turn'}
    <div class="rt-day rt-agent-turn" role="separator" aria-label={row.label} data-testid="runtime-agent-turn-divider">
      <Icon name="clock" size={12} /><span>{row.label}</span>
    </div>
  {:else if row.kind === 'day'}
    <div class="rt-day" role="separator" aria-label={row.label}><span>{row.label}</span></div>
  {:else if row.kind === 'setting'}
    <div class="rt-day rt-setting" class:refused={row.change.refused} role="note" aria-label={row.change.text} data-testid="runtime-setting-change" data-setting={row.change.setting}>
      <Icon name={row.change.refused ? 'info' : 'gear'} size={12} /><span dir="auto">{row.change.text}</span>
    </div>
  {:else if row.kind === 'message' && row.item.type === 'user_message' && isRunHandoff(row.item.text)}
    <!-- A routine's report handed to the agent: its result card and the agent's reply show it. -->
  {:else if row.kind === 'message' && row.item.type === 'user_message' && messageAttribution(row.item.text)}
    {@const a = messageAttribution(row.item.text)!}
    {@const peerIso = row.item.started_at ?? row.item.updated_at}
    {@const peerClock = formatMessageClock(peerIso)}
    {@const foldable = longBody(a.body)}
    {@const folded = foldable && !openedFrom.has(row.item.id)}
    <div class="rt-from" class:reply={a.reply} data-testid="rt-from" data-reply={a.reply ? 'true' : undefined} data-rt-msg={row.item.id}>
      <span class="rt-from-h">
        <Icon name="send" size={12} /> {a.reply ? 'Reply from' : 'From'} {a.from}
        {#if peerClock && peerIso}
          <time class="rt-msg-time" datetime={peerIso} title={formatMessageTooltip(peerIso)}>{peerClock}</time>
        {/if}
      </span>
      <div class="rt-from-body" class:folded><Markdown md={a.body} /></div>
      {#if foldable}
        <button type="button" class="rt-more" aria-expanded={!folded} onclick={() => (folded ? openedFrom.add(row.item.id) : openedFrom.delete(row.item.id))}>{folded ? 'More' : 'Less'}</button>
      {/if}
    </div>
  {:else if row.kind === 'message' && row.item.type === 'user_message'}
    {@const parsed = parseUserMessageItemWithBadges(row.item)}
    {@const userIso = row.item.started_at ?? row.item.updated_at}
    {@const userClock = formatMessageClock(userIso)}
    <div class="rt-user" data-rt-msg={row.item.id}>
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
      <div class="rt-msg-foot">
        {#if onedituser && row.item.id === lastUserId && (row.item.text || row.item.attachments?.length)}
          {@const userItem = row.item}
          <button type="button" class="rt-msg-act" title="Put this message back in the composer — sending it starts a new message; the conversation above stays as it is" data-testid="runtime-user-edit" onclick={() => onedituser?.(userItem)}>
            <Icon name="edit" size={12} /><span>Edit as new message</span>
          </button>
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
    </div>
  {:else if row.kind === 'message' && row.item.type === 'assistant_message'}
    {@const mdText = formatAssistantText ? formatAssistantText(row.item) : row.item.text}
    {@const startIso = row.item.started_at ?? row.item.updated_at}
    {@const endIso = row.item.completed_at ?? row.item.updated_at}
    {@const activeIso = row.item.streaming ? startIso : endIso}
    {@const assistantClock = formatMessageClock(activeIso)}
    {@const elapsedMs = !row.item.streaming && row.item.started_at && row.item.completed_at ? Date.parse(row.item.completed_at) - Date.parse(row.item.started_at) : 0}
    <div class="rt-assistant" class:streaming={row.item.streaming} dir="auto" data-rt-msg={row.item.id}>
      {#if mdText || !assistantFooter}<Markdown md={mdText} streaming={row.item.streaming} />{/if}
      {#if assistantFooter}{@render assistantFooter(row.item)}{/if}
      <div class="rt-msg-foot">
        {#if assistantClock && activeIso}
          <time
            class="rt-msg-time"
            datetime={activeIso}
            title={formatMessageTooltip(startIso, row.item.completed_at ?? (row.item.streaming ? null : row.item.updated_at), row.item.streaming)}
            data-testid="runtime-assistant-time"
          >{row.item.streaming ? `Started ${assistantClock}` : elapsedMs >= 1000 ? `${assistantClock} · ${formatDuration(elapsedMs)}` : assistantClock}</time>
        {/if}
        {#if !row.item.streaming && mdText}
          {@const copyText = mdText}
          <button type="button" class="rt-msg-act" aria-label="Copy reply" title="Copy reply" data-testid="runtime-copy-reply" onclick={() => void copyWithToast(copyText, 'reply')}>
            <Icon name="copy" size={12} /><span>Copy</span>
          </button>
        {/if}
      </div>
    </div>
  {:else if row.kind === 'work'}
    {@const flat = workRowEntries(row)}
    {@const fold = shouldFold(flat, turns, pendingIds, turnById)}
    {@const earlier = fold ? 0 : earlierSteps(row.parts, pendingIds)}
    <div class="rt-work">
      <RuntimeWorkFold id={row.id} entries={flat} {fold} tokens={tokensByRow.get(row.id) ?? ''}>
        {#if earlier}
          <!-- A running turn: the steps before the live one fold to one line. -->
          <details class="rt-earlier" data-testid="runtime-earlier-steps">
            <summary>{earlier} earlier steps <Icon name="chevronRight" size={12} /></summary>
            {#each row.parts.slice(0, earlier) as part (part.kind === 'group' ? part.id : part.entry.item.id)}{@render workPart(part)}{/each}
          </details>
        {/if}
        {#each row.parts.slice(earlier) as part (part.kind === 'group' ? part.id : part.entry.item.id)}{@render workPart(part)}{/each}
      </RuntimeWorkFold>
    </div>
  {/if}
  {#if changesByRow.get(row.kind === 'message' ? row.item.id : row.id)}
    <RuntimeTurnChanges files={changesByRow.get(row.kind === 'message' ? row.item.id : row.id)!} {onopendiff} />
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
  /* Turns breathe (--sp-9 between them); a turn's parts sit --sp-5 apart. */
  .rt-user:not(:first-child) {
    margin-block-start: var(--sp-9);
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
  /* The person's message: a calm right-aligned bubble in reading type. */
  .rt-bubble {
    max-inline-size: 82%;
    padding-block: var(--sp-3);
    padding-inline: var(--sp-5);
    border-radius: var(--radius-l);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-l);
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    unicode-bidi: plaintext;
  }
  /* The agent's answer: plain prose in reading type, no bubble, no gutter.
     Wide content (code, tables) scrolls inside the column, never widens it. */
  .rt-assistant {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    min-inline-size: 0;
    padding-block: var(--sp-2) var(--sp-5);
    color: var(--text);
    font-size: var(--fs-l);
    line-height: 1.6;
  }
  .rt-assistant :global(.md.md-body:not(.small)) {
    font-size: var(--fs-l);
    line-height: 1.6;
  }
  .rt-assistant :global(:is(pre, table)) {
    max-inline-size: 100%;
    overflow-x: auto;
  }
  .rt-assistant :global(table) {
    display: block;
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
  /* A provider switch: the divider opens on what was handed over. */
  .rt-handoff > summary {
    list-style: none;
    cursor: pointer;
    min-block-size: 32px;
  }
  .rt-handoff > summary::-webkit-details-marker {
    display: none;
  }
  .rt-handoff > summary span {
    max-inline-size: 80%;
    overflow-wrap: anywhere;
    text-align: center;
  }
  .rt-handoff > summary:hover,
  .rt-handoff[open] > summary {
    color: var(--text);
  }
  .rt-handoff > summary:focus-visible {
    outline: 2px solid var(--accent-line);
    outline-offset: 2px;
    border-radius: var(--radius-m);
  }
  .rt-handoff-note {
    margin: 0 0 6px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rt-handoff-text {
    margin: 0 0 14px;
    padding: 8px 10px;
    max-block-size: 320px;
    overflow: auto;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  @media (pointer: coarse) {
    .rt-handoff > summary {
      min-block-size: 44px;
    }
  }
  .rt-setting {
    font-size: var(--fs-xs);
    text-align: center;
  }
  .rt-setting.refused {
    color: var(--text);
  }
  .rt-setting span {
    max-inline-size: 80%;
    overflow-wrap: anywhere;
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
  .rt-msg-foot > .rt-msg-time {
    align-self: center;
    padding-inline: 4px;
  }
  .rt-from-h > .rt-msg-time {
    align-self: center;
    margin-inline-start: 4px;
  }
  .rt-from-body.folded {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    overflow: hidden;
  }
  .rt-more,
  .rt-msg-act {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .rt-more:hover,
  .rt-msg-act:hover {
    background: var(--hover);
    color: var(--text);
  }
  .rt-more:focus-visible,
  .rt-msg-act:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .rt-msg-foot {
    display: flex;
    align-items: center;
    gap: 6px;
    min-block-size: 20px;
  }
  .rt-user > .rt-msg-foot {
    align-self: flex-end;
  }
  /* A message's time and actions show on hover / focus with a pointer
     (still in the accessibility tree), always on touch. */
  @media (hover: hover) {
    .rt-msg-act,
    .rt-msg-foot > .rt-msg-time {
      opacity: 0;
    }
    :is(.rt-user, .rt-assistant):is(:hover, :focus-within) :is(.rt-msg-act, .rt-msg-foot > .rt-msg-time),
    .rt-msg-act:focus-visible {
      opacity: 1;
    }
  }
  @media (hover: hover) and (prefers-reduced-motion: no-preference) {
    .rt-msg-act,
    .rt-msg-foot > .rt-msg-time {
      transition: opacity var(--dur-fast) ease;
    }
  }
  @media (pointer: coarse) {
    .rt-more,
    .rt-msg-act {
      min-block-size: var(--hit-min);
      padding-inline: 10px;
    }
  }
  .rt-earlier > summary {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    min-block-size: 28px;
    color: var(--text-dim);
    font-size: var(--fs-m);
    list-style: none;
    cursor: pointer;
  }
  .rt-earlier > summary::-webkit-details-marker {
    display: none;
  }
  .rt-earlier > summary:hover {
    color: var(--text);
  }
  .rt-earlier > summary:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .rt-earlier[open] > summary > :global(svg) {
    transform: rotate(90deg);
  }
  .rt-work {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 0;
    padding-block-end: var(--sp-4);
  }
</style>
