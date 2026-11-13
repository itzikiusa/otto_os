<script lang="ts">
  // Structured runtime timeline for `IdeAgentPanel.svelte`, reusing Otto Threads'
  // real timeline grouping, Markdown renderer (wired to open file links directly
  // in the IDE editor), work-log folds, subagent cards, linked threads, and
  // inline approvals — layered with 1-click IDE Diff Preview & Apply cards.
  import { setContext } from 'svelte';
  import type { ApprovalDecision, TurnItem } from '../../../lib/api/types';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { PATH_LINK_CTX } from '../../../lib/pathLinks/open';
  import { harnessLabel } from '../../../lib/providers';
  import { now } from '../../../lib/stores/now.svelte';
  import { toastError } from '../../../lib/toastError';
  import { CONV_CTX, type PreviewReq } from '../../agents/conversation/context'; // ui-guards: allow — wire Markdown file links directly to IDE editor
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared sanitized Threads chat markdown renderer
  import { threadCall } from '../../threads/runtime/agentRows'; // ui-guards: allow — shared structured runtime thread-call detector
  import type { RuntimeProjection } from '../../threads/runtime/projection'; // ui-guards: allow — shared structured runtime projection
  import RuntimeApprovalRow from '../../threads/runtime/RuntimeApprovalRow.svelte'; // ui-guards: allow — shared structured runtime inline approval row
  import RuntimeAttachments from '../../threads/runtime/RuntimeAttachments.svelte'; // ui-guards: allow — shared user message image/PDF attachments
  import RuntimeLinkedThreads from '../../threads/runtime/RuntimeLinkedThreads.svelte'; // ui-guards: allow — shared linked peer/child thread switcher
  import { runtimeThreads } from '../../threads/runtime/runtimeThreads.svelte'; // ui-guards: allow — shared structured runtime responder
  import RuntimeSubagentCard from '../../threads/runtime/RuntimeSubagentCard.svelte'; // ui-guards: allow — shared subagent work card
  import RuntimeThreadCallRow from '../../threads/runtime/RuntimeThreadCallRow.svelte'; // ui-guards: allow — shared peer thread call row
  import RuntimeTurnStatus from '../../threads/runtime/RuntimeTurnStatus.svelte'; // ui-guards: allow — shared turn working/outcome status line
  import RuntimeWorkFold from '../../threads/runtime/RuntimeWorkFold.svelte'; // ui-guards: allow — shared collapsible work-log fold
  import RuntimeWorkGroup from '../../threads/runtime/RuntimeWorkGroup.svelte'; // ui-guards: allow — shared grouped tool-call rows
  import RuntimeWorkRow from '../../threads/runtime/RuntimeWorkRow.svelte'; // ui-guards: allow — shared tool/command/edit/thinking row
  import {
    approvalOutcome,
    isRunHandoff,
    isRunning,
    latestTurn,
    messageAttribution,
    pendingApprovals,
    subagentChildren,
    timelineRows,
    turnOutcome,
    type WorkEntry,
  } from '../../threads/runtime/timeline'; // ui-guards: allow — shared structured runtime timeline builder
  import { shouldFold } from '../../threads/runtime/workLog'; // ui-guards: allow — shared work-log fold policy
  import { ide } from '../ideStore.svelte';
  import {
    ideAgent,
    parseIdeContextChipsFromPrompt,
    stripIdeContextBlocks,
    type IdeCodeProposal,
    type ParsedPromptContextSummary,
  } from '../store/ideAgentController.svelte';
  import IdeProposalCard from './IdeProposalCard.svelte';

  interface Props {
    sessionId: string;
    projection: RuntimeProjection;
  }
  let { sessionId, projection }: Props = $props();

  const session = $derived(runtimeThreads.get(sessionId));
  const effectiveCwd = $derived(session?.cwd ?? ideAgent.resolveProjectCwd());
  const agentName = $derived(harnessLabel(session?.provider ?? ideAgent.provider).replace(/ Code$/, ''));

  // Route every file reference & code-block Open action in `Markdown.svelte` directly to the IDE editor!
  setContext(PATH_LINK_CTX, () => ({
    sessionId,
    cwd: effectiveCwd,
    workspaceId: session?.workspace_id ?? ideAgent.activeWorkspaceId,
  }));
  setContext(CONV_CTX, {
    get sessionId() { return sessionId; },
    get cwd() { return effectiveCwd; },
    readonly: false,
    openPreview(req: PreviewReq) {
      if (req.kind === 'file') {
        void ide.openFile(ide.resolvePath(req.path), { line: req.line ?? 1, preview: false });
      } else if (req.kind === 'code' && req.file) {
        void ide.openFile(ide.resolvePath(req.file), { preview: false });
      }
    },
  });

  const rows = $derived(timelineRows(projection.items));
  const approvals = $derived(new Map(pendingApprovals(projection).map((a) => [a.requestId, a] as const)));
  const pendingIds = $derived(new Set(approvals.keys()));
  const requests = $derived(new Map(projection.requests.map((r) => [r.id, r] as const)));
  const children = $derived(subagentChildren(projection.items));
  const turn = $derived(latestTurn(projection));
  const running = $derived(isRunning(projection) || ideAgent.sending);
  const outcome = $derived(!running ? turnOutcome(projection, turn) : null);
  const ownTurn = $derived(turn?.origin === 'agent');
  const closed = $derived(session?.status === 'closed');
  const workingFor = $derived.by(() => {
    if (!running || approvals.size > 0 || !turn?.started_at || turn.status !== 'running') return null;
    const s = Math.max(0, Math.floor((now() - Date.parse(turn.started_at)) / 1000));
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
  });

  function parseUserMessage(raw: string): { badges: ParsedPromptContextSummary[]; prose: string } {
    const badges = parseIdeContextChipsFromPrompt(raw);
    if (badges.length === 0 && /<ide_context\b/i.test(raw)) {
      badges.push({ kind: 'file', label: 'Editor context' });
    }
    return { badges, prose: stripIdeContextBlocks(raw) };
  }

  function proposalsFor(item: Extract<TurnItem, { type: 'assistant_message' }>): IdeCodeProposal[] {
    const map = ideAgent.proposalsByMessageId;
    return (map[item.id]?.length ? map[item.id] : map[item.message_id]) ?? [];
  }

  function formatAssistantMarkdown(raw: string, proposals: readonly IdeCodeProposal[]): string {
    const cleaned = stripIdeContextBlocks(raw.replace(/^echo:\s*(?=<ide_context\b)/i, ''));
    if (!cleaned || proposals.length === 0) return cleaned;
    const codeSet = new Set(proposals.map((p) => p.code.trim()));
    return cleaned
      .replace(/^(?:[ \t]*|echo:[ \t]*)```([^\r\n`]*)\r?\n([\s\S]*?)^[ \t]*```[ \t]*$/gm, (full, _info, body: string) =>
        codeSet.has(body.replace(/\r\n/g, '\n').replace(/\n$/, '').trim()) ? '' : full,
      )
      .replace(/\n{3,}/g, '\n\n')
      .trim();
  }

  function jumpTo(path?: string, line?: number): void {
    if (path) void ide.openFile(ide.resolvePath(path), { line: line ?? 1, preview: false });
  }

  async function respond(requestId: string, decision: ApprovalDecision): Promise<void> {
    try {
      await runtimeThreads.respond(sessionId, requestId, decision);
    } catch (e) {
      toastError('Couldn’t answer the approval', e);
    }
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
    <RuntimeApprovalRow approval={pending} onrespond={respond} />
  {:else}
    {@const ask = approval ?? (item.type === 'approval_request' ? item : undefined)}
    {@const gate = approval ? approvals.get(approval.request_id) : undefined}
    <RuntimeWorkRow
      {item}
      output={projection.output[item.id]}
      pending={gate}
      note={ask && !gate ? approvalOutcome(ask, requests.get(ask.request_id), agentName) : null}
      onrespond={respond}
      cwd={effectiveCwd}
    />
  {/if}
{/snippet}

<RuntimeLinkedThreads {sessionId} onswitch={(id) => void ideAgent.selectSession(id)} />

<div class="irt-col">
  {#each rows as row (row.kind === 'message' ? row.item.id : row.id)}
    {#if row.kind === 'session'}
      <div class="irt-day" role="separator" aria-label="New session"><span>New session</span></div>
    {:else if row.kind === 'day'}
      <div class="irt-day" role="separator" aria-label={row.label}><span>{row.label}</span></div>
    {:else if row.kind === 'message' && row.item.type === 'user_message' && isRunHandoff(row.item.text)}
      <!-- Routine report handoff already surfaces in its card/reply. -->
    {:else if row.kind === 'message' && row.item.type === 'user_message' && messageAttribution(row.item.text)}
      {@const a = messageAttribution(row.item.text)!}
      <div class="irt-from">
        <span class="irt-from-h"><Icon name="send" size={12} /> From {a.from}</span>
        <Markdown md={a.body} />
      </div>
    {:else if row.kind === 'message' && row.item.type === 'user_message'}
      {@const parsed = parseUserMessage(row.item.text)}
      <article class="irt-user">
        {#if row.item.attachments?.length}
          <RuntimeAttachments {sessionId} attachments={row.item.attachments} />
        {/if}
        {#if parsed.badges.length > 0}
          <div class="irt-badges" dir="ltr">
            {#each parsed.badges as b, bi (`${bi}:${b.label}`)}
              {#if b.path}
                <button
                  type="button"
                  class="chip accent"
                  title="{b.path}{b.startLine ? `:${b.startLine}` : ''}"
                  onclick={() => jumpTo(b.path, b.startLine)}
                >
                  {b.label}
                </button>
              {:else}
                <Badge tone="accent" variant="outline" label={b.label} />
              {/if}
            {/each}
          </div>
        {/if}
        {#if parsed.prose}
          <div class="irt-bubble" dir="auto">{parsed.prose}</div>
        {/if}
      </article>
    {:else if row.kind === 'message' && row.item.type === 'assistant_message'}
      {@const proposals = proposalsFor(row.item)}
      {@const mdText = formatAssistantMarkdown(row.item.text, proposals)}
      <article class="irt-assistant" class:streaming={row.item.streaming} dir="auto">
        {#if mdText}
          <Markdown md={mdText} />
        {:else if row.item.streaming && proposals.length === 0}
          <div class="irt-streaming">
            <span class="spinner" style:--spinner-size="12px" aria-hidden="true"></span>
            <span>Writing response…</span>
          </div>
        {/if}
        {#each proposals as proposal (proposal.id)}
          <IdeProposalCard {proposal} />
        {/each}
      </article>
    {:else if row.kind === 'work'}
      {@const flat = row.parts.flatMap((part) => (part.kind === 'group' ? part.entries : [part.entry]))}
      <div class="irt-work">
        <RuntimeWorkFold id={row.id} entries={flat} fold={shouldFold(flat, projection.turns, pendingIds)}>
          {#each row.parts as part (part.kind === 'group' ? part.id : part.entry.item.id)}
            {#if part.kind === 'item'}
              {@render entryRow(part.entry)}
            {:else}
              <RuntimeWorkGroup
                id={part.id}
                entries={part.entries}
                cwd={effectiveCwd}
                open={part.entries.some((e) => e.approval && approvals.has(e.approval.request_id))}
              >
                {#each part.entries as e (e.item.id)}{@render entryRow(e)}{/each}
              </RuntimeWorkGroup>
            {/if}
          {/each}
        </RuntimeWorkFold>
      </div>
    {/if}
  {/each}

  <RuntimeTurnStatus
    {workingFor}
    {ownTurn}
    {outcome}
    disabled={closed}
    onresend={(t) => void ideAgent.sendPrompt(t)}
  />
</div>

<style>
  .irt-col {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-inline-size: 0;
  }
  .irt-day {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-block: 4px 8px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .irt-day::before,
  .irt-day::after {
    content: '';
    flex: 1;
    border-block-start: 1px solid var(--border);
  }
  .irt-from {
    padding: 8px 10px;
    border-inline-start: 3px solid var(--accent-solid);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    min-inline-size: 0;
  }
  .irt-from-h {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--accent-text);
  }
  .irt-user {
    align-self: flex-end;
    max-inline-size: 92%;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 6px;
  }
  .irt-badges {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 4px;
  }
  .irt-bubble {
    max-inline-size: 100%;
    padding: 8px 12px;
    border: 1px solid color-mix(in srgb, var(--accent) 34%, var(--border));
    border-radius: var(--radius-m);
    background: color-mix(in srgb, var(--accent) 14%, var(--surface-2));
    color: var(--text);
    font-size: var(--fs-s);
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    unicode-bidi: plaintext;
  }
  .irt-assistant {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-inline-size: 0;
    color: var(--text);
    font-size: var(--fs-s);
  }
  .irt-assistant :global(:is(code, a.ref-chip)) {
    unicode-bidi: isolate;
  }
  .irt-assistant.streaming :global(.md > :last-child)::after {
    content: '▍';
    margin-inline-start: 2px;
    color: var(--text-dim);
  }
  .irt-streaming {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .irt-work {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 0;
  }
</style>
