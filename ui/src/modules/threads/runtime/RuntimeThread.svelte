<script lang="ts">
  // A Threads conversation on the structured agent runtime: the timeline
  // (your messages, the agent's streaming answer, a quiet work log of
  // reasoning / commands / edits / tools with approvals inline) over one
  // composer whose send turns into Stop while a turn runs; a message sent
  // meanwhile waits in the thread's durable queue (RuntimeQueueBar: steer it
  // in, send now). The agent's own turns run like any other; a closed thread
  // keeps its composer — sending reopens it (its agent resumes).
  // Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
  // apps/web/src/components/chat/MessagesTimeline.tsx + ThreadTimeline.tsx
  // (messages with work-log groups between them, follow the bottom),
  // ComposerPrimaryActions.tsx (send ↔ stop), QueuedRunsControl.tsx (one
  // queued follow-up), ThreadStatusLine.tsx ("Working for 12s"),
  // ThreadErrorBanner.tsx, and apps/web/src/session-logic.ts (`derivePhase`).
  // The composer's model / effort pick applies to the next turn and the ones
  // after it (ComposerModelEffort.tsx → the turn's model selection); pasted or
  // attached images go with the message (composerAttachmentFiles.ts).
  // Slash commands as in composerSlashCommandSearch.ts: the client's own
  // commands act on the view; any other `/command` goes to the agent as typed.
  // The access chip beside the model is T3's runtime mode (runtimeModeConfig.ts:
  // labels, icons, descriptions); it rides the next prompt and sticks. PDFs go
  // as file chips to the providers that take them (Claude).
  import { enterSends } from '../../../lib/enterSends';
  import { snapshotDraft, acknowledgeComposer } from '../../../lib/stores/runtimeDraftPersistence';
  import RuntimeDraftStatus from './RuntimeDraftStatus.svelte';
  import RuntimeContextChips from './RuntimeContextChips.svelte';
  import { assembleComposerMessage } from './composerContext';
  import { runtimeDrafts } from '../../../lib/stores/runtimeDrafts.svelte';
  import { setContext, untrack } from 'svelte';
  import { PATH_LINK_CTX } from '../../../lib/pathLinks/open';
  import { announceShown } from '../../../lib/announce.svelte';
  import { ApiError } from '../../../lib/api/client';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { harnessLabel } from '../../../lib/providers';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { formatEffortShort, formatModelLabel, supportsEffortSelection } from '../../../lib/providerModels';
  import { auth } from '../../../lib/stores/auth.svelte';
  import type { ApprovalDecision, PromptDelivery, RuntimeDraftContent } from '../../../lib/api/types';
  import { INTERACTIVE_SLASH_COMMANDS, isImmediateSlashCommand, parseInteractiveSlashInput, type ComposerSlashCommand } from '../../../lib/slashActions';
  import StageSlashPopup from '../StageSlashPopup.svelte';
  import { now } from '../../../lib/stores/now.svelte';
  import { toastError } from '../../../lib/toastError';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  const RuntimeQuestions = lazyComponent(() => import('./RuntimeQuestions.svelte'));
  import RuntimeQueueBar from './RuntimeQueueBar.svelte';
  import RuntimeTurnStatus from './RuntimeTurnStatus.svelte';
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { approvalOutcome, errorText, isRunHandoff, isRunning, latestTurn, messageAttribution, pendingApprovals, subagentChildren, timelineRows, turnOutcome, type WorkEntry } from './timeline';
  import { mergeQueuedEdit, readAttachments, runViewSlash } from './runtimeComposer';
  import { attachmentSupport, takesFile } from './attachmentSupport';
  import { availableSlash } from './hostCommands';
  import type { RuntimeHostCommands } from '../../../lib/runtimeThreadPort';
  import RuntimeComposerImages from './RuntimeComposerImages.svelte';
  import RuntimeFollow from './RuntimeFollow.svelte';
  import RuntimePlan from './RuntimePlan.svelte';
  import RuntimeAccessChip from './RuntimeAccessChip.svelte';
  import RuntimeApprovalRow from './RuntimeApprovalRow.svelte';
  import RuntimeAttachments from './RuntimeAttachments.svelte';
  import RuntimeLinkedThreads from './RuntimeLinkedThreads.svelte';
  import RuntimeUiControl from './RuntimeUiControl.svelte';
  import RuntimeBannerStack from './RuntimeBannerStack.svelte';
  import RuntimeWorkFold from './RuntimeWorkFold.svelte';
  import type { BannerBlock } from './banners';
  import { shouldFold } from './workLog';
  import { threadCall } from './agentRows';
  import { agentTasks } from './agentTasks';
  import RuntimeThreadCallRow from './RuntimeThreadCallRow.svelte';
  import { uiControl } from '../../../lib/stores/uiControl.svelte';
  import RuntimeWorkGroup from './RuntimeWorkGroup.svelte';
  import RuntimeSubagentCard from './RuntimeSubagentCard.svelte';
  import RuntimeWorkRow from './RuntimeWorkRow.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { personalAgentOf } from '../../../lib/personalAgentBadges';
  import { runtimeViews } from './runtimeView.svelte';
  import RuntimeAskAgent from './RuntimeAskAgent.svelte';
  import { askFromComposer, askTargets } from './askTargets.svelte';

  interface Props {
    sessionId: string;
    focused: boolean;
    /** Hosted outside Threads (Personal Agents' chat): a linked-thread chip switches the host's view. */
    onswitch?: (id: string) => void;
    /** A personal agent's chat: "Message <name>…", and its full access reads calm. */
    agentChat?: string;
    /** The host's view-level slash commands (lib/runtimeThreadPort.ts); unhandled ones are hidden. */
    hostCommands?: RuntimeHostCommands;
    /** Denser chrome for a narrow column (another module's embed). */
    compact?: boolean;
  }
  let { sessionId, focused, onswitch, agentChat, hostCommands, compact = false }: Props = $props();
  const ownerId = untrack(() => sessionId);
  const draft = untrack(() => runtimeDrafts.for(ownerId));
  /** The personal agent this thread runs as (its run notices open their report). */
  const agentChatId = $derived(personalAgentOf(runtimeThreads.get(sessionId)?.meta));

  $effect(() => {
    const id = sessionId;
    return untrack(() => runtimeViews.open(id));
  });

  const session = $derived(runtimeThreads.get(sessionId));
  // Paths in this thread's messages resolve against ITS folder (Markdown.svelte).
  setContext(PATH_LINK_CTX, () => ({ sessionId, cwd: session?.cwd, workspaceId: session?.workspace_id }));
  const p = $derived(runtimeViews.projections[sessionId] ?? null);
  const loadError = $derived(runtimeViews.loadError[sessionId] ?? null);
  /** A later resync failed: LoadState's quiet "Couldn’t refresh · Retry" bar over the timeline. */
  const staleError = $derived(runtimeViews.staleError[sessionId] ?? null);
  const rows = $derived(p ? timelineRows(p.items) : []);
  const approvals = $derived(new Map((p ? pendingApprovals(p) : []).map((a) => [a.requestId, a] as const)));
  const requests = $derived(new Map((p?.requests ?? []).map((r) => [r.id, r] as const)));
  const turn = $derived(p ? latestTurn(p) : null);
  const closed = $derived(session?.status === 'closed');
  const dormant = $derived(session?.status === 'dormant');
  const needsRevive = $derived(closed || dormant);
  /** Each subagent card's own activity. */
  const children = $derived(p ? subagentChildren(p.items) : new Map<string, never[]>());
  /** Background work still pending (the live session entity first). */
  const tasks = $derived(agentTasks(p?.items ?? [], p?.session?.background_tasks ?? session?.background_tasks ?? []));
  const background = $derived(tasks.running.filter((t) => t.background));
  /** The agent couldn't start (a refused handshake answers the prompt 400;
   *  the session goes dormant with `last_error`) — shown inline, T3 Code's
   *  ThreadErrorBanner, until the next prompt starts it. */
  const sessionError = $derived.by(() => {
    const e = draft.startError ?? (p?.session && (p.session.status === 'stopped' || p.session.status === 'error') ? p.session.last_error : null);
    // Said once: the timeline's own error row already carries the same text.
    const last = p?.items.findLast((i) => !i.parent_item_id);
    const said = last?.type === 'error' ? last.failure.message.trim() : '';
    if (!e || /no conversation found/i.test(e)) return null;
    return said && (e.includes(said) || said.includes(e.trim())) ? null : errorText(e);
  });
  const plan = $derived.by(() => {
    if (!p || !turn) return null;
    const pl = p.plans.findLast((x) => x.turn_id === turn.id);
    return pl && pl.steps.some((s) => s.status !== 'completed') ? pl : null;
  });

  // ── Above the composer: one stack, one order (RuntimeBannerStack) ──────
  const uiAsk = $derived(!!session && !closed && !auth.isImpersonating && session.created_by === auth.me?.id && !!uiControl.promptFor(sessionId));
  const queueShown = $derived((background.length > 0 && !closed) || (p?.queued.length ?? 0) > 0 || needsRevive);
  const banners = $derived.by((): BannerBlock[] => [
    ...(sessionError ? [{ key: 'error', label: 'error', urgent: true }] : []),
    ...(uiAsk ? [{ key: 'ui', label: 'UI control request', urgent: true }] : []),
    ...(p?.requests.some((r) => r.kind === 'question') ? [{ key: 'questions', label: 'questions', urgent: true }] : []),
    ...(plan ? [{ key: 'plan', label: 'plan' }] : []),
    ...(queueShown ? [{ key: 'queue', label: needsRevive ? 'revive' : 'queue' }] : []),
    ...(draft.freshContext ? [{ key: 'fresh', label: 'fresh context' }] : []),
  ]);
  const pendingIds = $derived(new Set(approvals.keys()));

  let scroller = $state<HTMLDivElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);

  const running = $derived((p ? isRunning(p) : false) || draft.sending || (draft.awaitSeq !== null && (p?.lastSequence ?? 0) < draft.awaitSeq));
  // An agent chat that answered needs no "Partly done · Send again": the
  // failed step's row already says what went wrong in words.
  const outcome = $derived.by(() => {
    const o = p && !running ? turnOutcome(p, turn) : null;
    return o?.kind === 'partial' && agentChatId ? null : o;
  });
  const ownTurn = $derived(turn?.origin === 'agent');
  const workingFor = $derived.by(() => {
    if (!running || approvals.size > 0 || !turn?.started_at || turn.status !== 'running') return null;
    const s = Math.max(0, Math.floor((now() - Date.parse(turn.started_at)) / 1000));
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
  });

  // ── Model / effort for the next turn ──────────────────────────────────
  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));
  let pickerOpen = $state(false);
  let agentEl = $state<HTMLButtonElement | null>(null);
  const provider = $derived(session?.provider ?? '');
  /** "Claude", for "Withdrawn by Claude". */
  const agentName = $derived(harnessLabel(provider).replace(/ Code$/, ''));
  const agentLabel = $derived.by(() => {
    const m = draft.model ?? session?.model ?? null;
    const label = m ? formatModelLabel(provider, m, auth.meta) : harnessLabel(provider);
    return draft.effort ? `${label} · ${formatEffortShort(draft.effort)}` : label;
  });

  // ── Access mode for the next turn: sticky per thread ────────────────
  const accessMode = $derived(draft.pickedMode ?? session?.access_mode ?? 'approval-required');

  // ── Images and PDFs (paste or attach; sent with the next message) ─────
  const canImages = $derived(attachmentSupport(provider).images);
  const canFiles = $derived(attachmentSupport(provider).files);
  const takes = (f: File): boolean => takesFile(provider, f);
  let fileEl = $state<HTMLInputElement | null>(null);
  async function addImages(files: Iterable<File>): Promise<void> {
    const read = await readAttachments(files, takes);
    if (read.length && runtimeDrafts.owns(ownerId, draft)) draft.images = [...draft.images, ...read];
  }
  function onPaste(e: ClipboardEvent): void {
    const files = [...(e.clipboardData?.files ?? [])].filter(takes);
    if (!canImages || files.length === 0) return;
    e.preventDefault();
    void addImages(files);
  }

  async function send(t: string, delivery?: PromptDelivery, sent: RuntimeDraftContent = snapshotDraft(draft)): Promise<void> {
    if (!runtimeDrafts.owns(ownerId, draft) || draft.recovered === false) return;
    draft.sending = true;
    const pics = sent.images.filter((f) => f.mime.startsWith('image/'));
    const docs = sent.images.filter((f) => !f.mime.startsWith('image/'));
    try {
      draft.awaitSeq = (
        await runtimeThreads.prompt(ownerId, {
          text: assembleComposerMessage(t, sent.context),
          ...(sent.model ? { model: sent.model } : {}),
          ...(sent.effort ? { effort: sent.effort } : {}),
          ...(pics.length ? { images: pics.map(({ mime, data_base64 }) => ({ mime, data_base64 })) } : {}),
          ...(docs.length ? { files: docs } : {}),
          // Only a person's pick changes the mode; otherwise the session keeps its own.
          ...(sent.access_mode ? { access_mode: sent.access_mode } : {}),
          ...(sent.fresh_context ? { fresh_context: true } : {}),
          // While a turn runs it waits in the thread's queue (never interleaved).
          ...(delivery ? { delivery } : {}),
        })
      ).sequence;
      if (!runtimeDrafts.owns(ownerId, draft)) return;
      acknowledgeComposer(draft, sent);
      draft.flushSave();
      if (delivery) draft.awaitSeq = null;
      draft.startError = null;
    } catch (e) {
      if (!runtimeDrafts.owns(ownerId, draft)) return;
      if (e instanceof ApiError && (e.status === 400 || e.status === 409)) draft.startError = e.message;
      else toastError('Couldn’t send message', e);

    } finally {
      draft.sending = false;
    }
  }

  /** Revive without sending a message or replacing the conversation. */
  async function reopen(): Promise<boolean> {
    if (!needsRevive || draft.reopening) return !needsRevive;
    draft.reopening = true;
    const ok = await runtimeThreads.reopen(ownerId).then(
      () => { draft.startError = null; return true; },
      (e: unknown) => { draft.startError = e instanceof Error ? e.message : 'Couldn’t revive the thread. Try again.'; return false; },
    );
    draft.reopening = false;
    return ok;
  }

  function submit(): void {
    let t = draft.text.trim();
    if ((!t && draft.images.length === 0 && draft.context.length === 0) || draft.reopening || draft.sending || draft.recovered === false) return;
    const intent = parseInteractiveSlashInput(t);
    if (intent?.kind === 'send-prompt') t = intent.prompt;
    else if (intent && runViewSlash(intent, sessionId, hostCommands)) {
      draft.text = '';
      return;
    }
    // Agents picked as chips ("Ask another agent", `@codex`) get it instead, in new linked threads.
    if (askTargets.for(ownerId).length) return void askFromComposer(ownerId, draft, assembleComposerMessage(t, draft.context), draft.pickedMode);
    const submitted = snapshotDraft(draft);
    // A closed thread reopens first; while a turn runs it waits in the queue.
    if (needsRevive && !submitted.fresh_context) void reopen().then((ok) => (ok ? send(t, undefined, submitted) : undefined));
    else void send(t, running && !submitted.fresh_context ? 'queue' : undefined, submitted);
  }

  // ── Slash commands: the ones that act on this view (and its host) ──────
  const SLASH = $derived(availableSlash(INTERACTIVE_SLASH_COMMANDS, hostCommands));
  const popId = $props.id();
  let cmdIdx = $state(0);
  let cmdDismissed = $state(false);
  const suggestions = $derived.by(() => {
    const prefix = /^\/([\w:-]*)$/.exec(draft.text)?.[1].toLowerCase();
    return prefix === undefined || cmdDismissed ? [] : SLASH.filter((c) => c.name.includes(prefix));
  });
  $effect(() => {
    void suggestions.length;
    cmdIdx = 0;
  });
  $effect(() => {
    void draft.text;
    cmdDismissed = false;
  });

  function acceptCmd(c: ComposerSlashCommand): void {
    if (isImmediateSlashCommand(c.name)) {
      draft.text = `/${c.name}`;
      submit();
    } else {
      draft.text = `/${c.name} `;
      cmdDismissed = true;
      composerEl?.focus();
    }
  }

  async function stop(): Promise<void> {
    if (draft.stopping) return;
    draft.stopping = true;
    try {
      await runtimeThreads.interrupt(ownerId);
    } catch (e) {
      toastError('Couldn’t stop the turn', e);
    } finally {
      draft.stopping = false;
    }
  }

  async function respond(requestId: string, decision: ApprovalDecision): Promise<void> {
    try {
      await runtimeThreads.respond(ownerId, requestId, decision);
    } catch (e) {
      toastError('Couldn’t answer the approval', e);
    }
  }

  let askEl = $state<{ onKey: (e: KeyboardEvent) => boolean } | null>(null);
  function onKeydown(e: KeyboardEvent): void {
    if (e.isComposing || askEl?.onKey(e)) return;
    if (suggestions.length > 0) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        cmdIdx = (cmdIdx + (e.key === 'ArrowDown' ? 1 : suggestions.length - 1)) % suggestions.length;
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        cmdDismissed = true;
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        e.preventDefault();
        acceptCmd(suggestions[cmdIdx]);
        return;
      }
    }
    if (enterSends(e)) {
      e.preventDefault();
      submit();
    }
  }

  // Focus the composer when the pane becomes the focused one (not per event).
  $effect(() => {
    const el = composerEl;
    if (focused && el) untrack(() => el.focus({ preventScroll: true }));
  });
</script>

{#snippet entryRow({ item, approval }: WorkEntry)}
  {@const pending = item.type === 'approval_request' ? approvals.get(item.request_id) : undefined}
  {@const call = item.type === 'dynamic_tool' && !item.error && !approval ? threadCall(item) : null}
  {#if item.type === 'subagent'}
    <RuntimeSubagentCard {item} activity={children.get(item.id) ?? []} output={p?.output} />
  {:else if call}
    <RuntimeThreadCallRow {item} {call} />
  {:else if pending}
    <RuntimeApprovalRow approval={pending} onrespond={respond} />
  {:else}
    {@const ask = approval ?? (item.type === 'approval_request' ? item : undefined)}
    {@const gate = approval ? approvals.get(approval.request_id) : undefined}
    <RuntimeWorkRow
      {item}
      output={p?.output[item.id]}
      pending={gate}
      note={ask && !gate ? approvalOutcome(ask, requests.get(ask.request_id), agentName) : null}
      onrespond={respond}
      agentId={agentChatId}
      cwd={session?.cwd}
    />
  {/if}
{/snippet}

<section class="rt" class:compact aria-label="Conversation" data-testid="runtime-thread-{sessionId}">
  <!-- In Threads the header's Related threads chip carries the group; an
       embed (Personal Agents, a PR, Connections) gets a chip of its own. -->
  {#if onswitch}<RuntimeLinkedThreads {sessionId} {onswitch} />{/if}
  <div class="rt-scroll" bind:this={scroller}>
    <div class="rt-col">
      <LoadState what="conversation" loading={!p && !loadError} error={p ? staleError : loadError} empty={!p} onretry={() => runtimeViews.retry(sessionId)}>
        {#if rows.length === 0}
          <EmptyState
            icon="threads"
            title="Start the conversation"
            body="Replies, tool calls and approvals stream here as the agent works."
          />
        {/if}
        {#each rows as row (row.kind === 'message' ? row.item.id : row.id)}
          {#if row.kind === 'session'}
            <div class="rt-day" role="separator" aria-label="New session" data-testid="runtime-session-boundary"><span>New session</span></div>
          {:else if row.kind === 'day'}
            <div class="rt-day" role="separator" aria-label={row.label}><span>{row.label}</span></div>
          {:else if row.kind === 'message' && row.item.type === 'user_message' && isRunHandoff(row.item.text)}
            <!-- A routine's report handed to the agent: its result card and the agent's reply show it. -->
          {:else if row.kind === 'message' && row.item.type === 'user_message' && messageAttribution(row.item.text)}
            {@const a = messageAttribution(row.item.text)!}
            <div class="rt-from" data-testid="rt-from"><span class="rt-from-h"><Icon name="send" size={12} /> From {a.from}</span><Markdown md={a.body} /></div>
          {:else if row.kind === 'message' && row.item.type === 'user_message'}
            <div class="rt-user">
              {#if row.item.attachments?.length}<RuntimeAttachments {sessionId} attachments={row.item.attachments} />{/if}
              {#if row.item.text}<div class="rt-bubble" dir="auto">{row.item.text}</div>{/if}
            </div>
          {:else if row.kind === 'message' && row.item.type === 'assistant_message'}
            <div class="rt-assistant" class:streaming={row.item.streaming} dir="auto">
              <Markdown md={row.item.text} />
            </div>
          {:else if row.kind === 'work'}
            {@const flat = row.parts.flatMap((part) => (part.kind === 'group' ? part.entries : [part.entry]))}
            <div class="rt-work">
              <RuntimeWorkFold id={row.id} entries={flat} fold={!!p && shouldFold(flat, p.turns, pendingIds)}>
                {#each row.parts as part (part.kind === 'group' ? part.id : part.entry.item.id)}
                  {#if part.kind === 'item'}
                    {@render entryRow(part.entry)}
                  {:else}
                    <RuntimeWorkGroup id={part.id} entries={part.entries} cwd={session?.cwd} open={part.entries.some((e) => e.approval && approvals.has(e.approval.request_id))}>
                      {#each part.entries as e (e.item.id)}{@render entryRow(e)}{/each}
                    </RuntimeWorkGroup>
                  {/if}
                {/each}
              </RuntimeWorkFold>
            </div>
          {/if}
        {/each}
        <RuntimeTurnStatus {workingFor} {ownTurn} {outcome} disabled={closed} onresend={(t) => void send(t, undefined, { ...snapshotDraft(draft), text: t, images: [], context: [], fresh_context: false })} />
      </LoadState>
    </div>
  </div>
  <RuntimeFollow {scroller} seq={p?.lastSequence} messageIds={rows.flatMap((r) => (r.kind === 'message' ? [r.item.id] : []))} />

  <div class="rt-dock">
    <RuntimeBannerStack blocks={banners}>
      {#snippet block(key: string)}
        {#if key === 'error' && sessionError}
          <p class="rt-banner" dir="auto" use:announceShown={{ text: sessionError, assertive: true }}><Icon name="warning" size={13} />{sessionError}</p>
        {:else if key === 'ui'}
          <RuntimeUiControl {sessionId} />
        {:else if key === 'questions' && p}
          <LazyMount lazy={RuntimeQuestions} what="the questions" variant="compact" props={{ requests: p.requests, sessionId, agent: agentName, onanswered: () => composerEl?.focus() }} />
        {:else if key === 'plan' && plan}
          <RuntimePlan {plan} />
        {:else if key === 'queue'}
          <RuntimeQueueBar
            {sessionId}
            {background}
            onshowtasks={hostCommands?.showTasks ? () => hostCommands.showTasks!(sessionId) : undefined}
            queued={p?.queued ?? []}
            {running}
            canSteer={provider === 'claude'}
            {closed}
            {dormant}
            reopening={draft.reopening}
            onreopen={() => void reopen()}
            onedit={(t) => {
              draft.text = mergeQueuedEdit(draft.text, t);
              composerEl?.focus();
            }}
          />
        {:else if key === 'fresh'}
          <div class="rt-fresh" data-testid="runtime-fresh-notice" use:announceShown={'Next message starts fresh'}>
            <span>Next message starts fresh</span>
            <button type="button" class="btn small ghost" onclick={() => { draft.freshContext = false; composerEl?.focus(); }}>Cancel</button>
          </div>
        {/if}
      {/snippet}
    </RuntimeBannerStack>
    <div class="rt-composer">
      {#if suggestions.length > 0}
        <StageSlashPopup id={popId} items={suggestions} active={cmdIdx} onaccept={acceptCmd} onhover={(i) => (cmdIdx = i)} />
      {/if}
      <RuntimeContextChips context={draft.context} onremove={(id) => { draft.context = draft.context.filter(c => c.id !== id); }} onfocus={() => composerEl?.focus()} />
      {#if !agentChat}<RuntimeAskAgent bind:this={askEl} part="chips" {sessionId} {provider} access={accessMode} readOnly={!!session?.meta?.read_only} text={draft.text} onreplace={(t) => { draft.text = t; composerEl?.focus(); }} />{/if}
      <textarea
        bind:this={composerEl}
        bind:value={draft.text}
        dir="auto"
        rows={2}
        disabled={draft.reopening}
        placeholder={closed
          ? 'Send a message to reopen this thread'
          : approvals.size > 0
            ? Array.from(approvals.values()).some((request) => request.kind === 'question')
              ? 'Answer the question above'
              : 'Waiting for your approval above'
            : running
              ? 'Queue a follow-up for when this turn ends'
              : agentChat ? `Message ${agentChat}…` : 'Ask anything. / commands'}
        aria-label="Message"
        aria-autocomplete="list"
        aria-controls={suggestions.length > 0 ? `${popId}-list` : undefined}
        aria-activedescendant={suggestions.length > 0 ? `${popId}-opt-${cmdIdx}` : undefined}
        data-testid="runtime-composer"
        onkeydown={onKeydown}
        onpaste={onPaste}
      ></textarea>
      {#if draft.images.length > 0}
        <RuntimeComposerImages images={draft.images} onremove={(i) => (draft.images = draft.images.filter((_, j) => j !== i))} />
      {/if}
      <div class="rt-foot">
        <RuntimeDraftStatus {draft} />
        {#if canImages}
        <button
          type="button"
          class="icon-btn"
          aria-label={canFiles ? 'Attach an image or PDF' : 'Attach an image'}
          title={canFiles ? 'Attach an image or PDF (or paste one)' : 'Attach an image (or paste one)'}
          data-testid="runtime-attach"
          disabled={draft.reopening}
          onclick={() => fileEl?.click()}
        >
          <Icon name="image" size={14} />
        </button>
        <input
          bind:this={fileEl}
          type="file"
          accept={canFiles ? 'image/*,application/pdf' : 'image/*'}
          multiple
          hidden
          onchange={(e) => {
            void addImages(e.currentTarget.files ?? []);
            e.currentTarget.value = '';
          }}
        />
        {/if}
        {#if !agentChat}<RuntimeAskAgent part="button" {sessionId} {provider} access={accessMode} disabled={closed || !session} />{/if}
        <span class="rt-grow">
          <!-- A personal agent's model and access are its settings (Settings ›
               About); a per-turn picker here disagreed with them. -->
          {#if !agentChat}
          <button
            bind:this={agentEl}
            type="button"
            class="rt-agent"
            aria-haspopup="dialog"
            aria-expanded={pickerOpen}
            aria-label="Model for the next turn: {agentLabel}"
            title="Model and effort for the next turn"
            data-testid="runtime-model"
            disabled={closed || !session}
            onpointerenter={() => ModelPicker.prefetch()}
            onclick={() => (pickerOpen = true)}
          >
            {#if session && hasProviderIcon(session.provider)}<ProviderIcon provider={session.provider} size={13} />{/if}
            <span class="ellipsis">{agentLabel}</span>
          </button>
          <RuntimeAccessChip
            mode={accessMode}
            agent={harnessLabel(provider)}
            cwd={session?.cwd ?? 'this folder'}
            disabled={closed || !session}
            onpick={(m) => (draft.pickedMode = m)}
          />
          {/if}
        </span>
        {#if running && !draft.text.trim() && draft.images.length === 0 && draft.context.length === 0}
          <button
            type="button"
            class="rt-send stop"
            aria-label="Stop the turn"
            title="Stop"
            data-testid="runtime-stop"
            disabled={draft.stopping}
            onclick={() => void stop()}
          >
            <Icon name="stop" size={14} />
          </button>
        {:else}
          <button
            type="button"
            class="rt-send"
            aria-label={running ? 'Queue message' : 'Send message'}
            title={running ? 'Queue (↵)' : 'Send (↵)'}
            data-testid="runtime-send"
            disabled={!draft.recovered || draft.reopening || draft.sending || (!draft.text.trim() && draft.images.length === 0 && draft.context.length === 0)}
            onclick={submit}
          >
            <Icon name="arrowUp" size={16} />
          </button>
        {/if}
      </div>
    </div>
  </div>
</section>

{#if pickerOpen && agentEl && ModelPicker.component}
  <ModelPicker.component
    anchor={agentEl}
    providers={[provider]}
    {provider}
    lockProvider
    model={draft.model ?? session?.model ?? null}
    effort={draft.effort}
    mode="full"
    onmodel={(m) => (draft.model = m)}
    oneffort={supportsEffortSelection(provider) ? (e) => (draft.effort = e) : undefined}
    onclose={() => (pickerOpen = false)}
  />
{/if}

<style>
  .rt {
    display: flex;
    flex-direction: column;
    min-block-size: 0;
    block-size: 100%;
    min-inline-size: 0;
    container-type: inline-size;
    background: var(--bg);
  }
  .rt-scroll {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .rt-col,
  .rt-dock {
    inline-size: 100%;
    margin-inline: auto;
    padding-inline: clamp(12px, 3.2cqi, 48px);
  }
  /* A narrow column (another module's embed): tighter gutters. */
  .rt.compact .rt-col,
  .rt.compact .rt-dock {
    padding-inline: 8px;
  }
  .rt.compact .rt-col {
    padding-block: 8px 12px;
  }
  .rt.compact .rt-dock {
    padding-block-end: 8px;
  }
  /* T3's MessagesTimeline rhythm: a turn opens with clear air above the
     user bubble; the answer and its work log sit close beneath it. */
  .rt-col {
    display: flex;
    flex-direction: column;
    padding-block: 16px 24px;
  }
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
  .rt-assistant {
    padding-block-end: 12px;
  }
  .rt-assistant :global(:is(code, a.ref-chip)) {
    unicode-bidi: isolate;
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
    min-width: 0;
    color: var(--text);
  }
  .rt-assistant.streaming :global(.md > :last-child)::after {
    content: '▍';
    margin-inline-start: 2px;
    color: var(--text-dim);
  }
  .rt-day { display: flex; align-items: center; gap: 10px; margin-block: 8px 14px; color: var(--text-dim); font-size: var(--fs-s); }
  .rt-day::before, .rt-day::after { content: ''; flex: 1; border-block-start: 1px solid var(--border); }
  .rt-from { margin-block: 12px; padding: 8px 12px; border-inline-start: 3px solid var(--accent-solid); border-radius: var(--radius-m); background: var(--surface-2); min-width: 0; }
  .rt-from-h { display: inline-flex; align-items: center; gap: 4px; font-size: var(--fs-s); font-weight: 600; color: var(--accent-text); }
  .rt-work {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding-block-end: 10px;
  }
  .rt-dock {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-block: 0 12px;
  }
  .rt-banner {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 6px 10px;
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
  .rt-fresh {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rt-composer {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
  }
  .rt-composer:focus-within {
    border-color: var(--border-strong);
  }
  .rt-composer textarea {
    inline-size: 100%;
    min-block-size: 44px;
    max-block-size: 220px;
    resize: none;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1.55;
    unicode-bidi: plaintext;
    text-align: start;
    outline: none;
    field-sizing: content;
  }
  .rt-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .rt-grow {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .rt-agent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-inline-size: 100%;
    min-width: 0;
    block-size: 28px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    font: inherit;
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rt-agent:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .rt-send {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    inline-size: 30px;
    block-size: 30px;
    border: 0;
    border-radius: 50%;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    cursor: pointer;
  }
  .rt-send.stop {
    background: var(--text);
    color: var(--bg);
  }
  .rt-send:disabled {
    background: var(--surface-3);
    color: var(--text-dim);
    cursor: default;
  }
</style>
