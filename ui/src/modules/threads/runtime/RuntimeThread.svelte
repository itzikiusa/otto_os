<script lang="ts">
  // Structured agent runtime thread view shared between Threads and IDE Agent dock.
  import { setContext, untrack, type Snippet } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import { ApiError } from '../../../lib/api/client';
  import type { ApprovalDecision, PromptDelivery, RuntimeAccessMode, RuntimeDraftContent, TurnItem } from '../../../lib/api/types';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { enterSends, isTouchFirst } from '../../../lib/enterSends';
  import { isMacPlatform } from '../../../lib/keys';
  import { toasts } from '../../../lib/toast.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { PATH_LINK_CTX } from '../../../lib/pathLinks/open';
  import { personalAgentOf } from '../../../lib/personalAgentBadges';
  import { formatEffortShort, formatModelLabel, supportsEffortSelection } from '../../../lib/providerModels';
  import { harnessLabel, runtimeBaselineCaps, sessionProviderSlug } from '../../../lib/providers';
  import type { RuntimeComposerTestIds, RuntimeHostCommands } from '../../../lib/runtimeThreadPort';
  import { INTERACTIVE_SLASH_COMMANDS, isImmediateSlashCommand, parseInteractiveSlashInput, type ComposerSlashCommand } from '../../../lib/slashActions';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { now } from '../../../lib/stores/now.svelte';
  import { acknowledgeComposer, snapshotDraft } from '../../../lib/stores/runtimeDraftPersistence';
  import { composerViewState } from '../../../lib/stores/runtimeDraftCursor';
  import { runtimeDrafts } from '../../../lib/stores/runtimeDrafts.svelte';
  import { ui } from '../../../lib/stores/ui.svelte';
  import { uiControl } from '../../../lib/stores/uiControl.svelte';
  import { toastError } from '../../../lib/toastError';
  import StageSlashPopup from '../StageSlashPopup.svelte';
  import { EmojiAssistView } from '../../../lib/components/emojiAssistLazy.svelte';
  import { agentTasks } from './agentTasks';
  import { askFromComposer, askTargets } from './askTargets.svelte';
  import { attachmentSupport, capsOf, takesFile } from './attachmentSupport';
  import { addComposerContext, assembleComposerMessage } from './composerContext';
  import { availableSlash, slashSuggestions } from './hostCommands';
  import RuntimeAccessChip from './RuntimeAccessChip.svelte';
  import RuntimeAskAgent from './RuntimeAskAgent.svelte';
  import { attentionItems, attentionPillText } from './attentionDock';
  import RuntimeAttentionDock from './RuntimeAttentionDock.svelte';
  import RuntimeComposerActions from './RuntimeComposerActions.svelte';
  import RuntimeComposerAttach from './RuntimeComposerAttach.svelte';
  import { isAltSend, midTurnDelivery, primaryAction } from './composerSend';
  import { threadPrefs } from './threadPrefs.svelte';
  import RuntimeModelChip from './RuntimeModelChip.svelte';
  import RuntimeBannerStack from './RuntimeBannerStack.svelte';
  import {
    appendDraftAttachments, buildOptimisticUserMessage, buildRuntimeBanners, collectMessageIds,
    composerPlaceholderText, deriveActivePlan, deriveSessionError, handleSlashMenuKey, isPlanToggleKey, mapChildThreadSummaries,
    pasteDraftAttachments, resolveEffectiveDraftModel, resolvePromptModelOverride, restoreOptimisticDraft,
    runViewSlash, withOptimisticItem, type OptimisticMessageState,
  } from './runtimeComposer';
  import { buildFailedTurnResendDraft, restoreFailedTurnToDraft, restorePoppedQueuedMessageToDraft } from './runtimeRetryRestore';
  import RuntimeComposerImages from './RuntimeComposerImages.svelte';
  import RuntimeContextChips from './RuntimeContextChips.svelte';
  import RuntimeDraftStatus from './RuntimeDraftStatus.svelte';
  import RuntimeFollow from './RuntimeFollow.svelte';
  import RuntimeStreamTools from './RuntimeStreamTools.svelte';
  import RuntimeFreshStop from './RuntimeFreshStop.svelte';
  import { interruptTurn } from './taskStops.svelte';
  import RuntimeHistoryBar from './RuntimeHistoryBar.svelte';
  import RuntimeLinkedThreads from './RuntimeLinkedThreads.svelte';
  import RuntimePlan from './RuntimePlan.svelte';
  import RuntimePlanToggle from './RuntimePlanToggle.svelte';
  import RuntimeQueueBar from './RuntimeQueueBar.svelte';
  import { linkedChildren } from './council';
  import { secretRequests } from '../../../lib/stores/secretRequests.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import RuntimeTimelineRows from './RuntimeTimelineRows.svelte';
  import RuntimeTurnStatus from './RuntimeTurnStatus.svelte';
  import RuntimeUiControl from './RuntimeUiControl.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { formatLiveElapsed } from './liveStatus';
  import { renderWindow, timelineRowKey } from './follow';
  import { sendRefusal } from './queueRows';
  import { isRunning, latestTurn, pendingApprovals, timelineRows, turnOutcome } from './timeline';
  import { composerStopKey, isStopChord } from './stopKeys';
  import { failureView, turnErrorItem } from './turnFailure';

  const RuntimeQuestions = lazyComponent(() => import('./RuntimeQuestions.svelte'));
  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));

  export type { RuntimeComposerTestIds } from '../../../lib/runtimeThreadPort';

  interface Props {
    sessionId: string;
    focused: boolean;
    onswitch?: (id: string) => void;
    agentChat?: string;
    hostCommands?: RuntimeHostCommands;
    compact?: boolean;
    accessModeOverride?: RuntimeAccessMode | null;
    onaccessmodechange?: (mode: RuntimeAccessMode) => void;
    fallbackCwd?: string | null;
    hasExternalContext?: boolean;
    /** `fresh`: this send starts a fresh conversation (an embed re-adds its context). */
    transformOutgoingText?: (text: string, fresh?: boolean) => string;
    transformRestoredQueuedText?: (text: string) => string;
    onExternalContextConsumed?: () => void;
    onopenpath?: (path: string, line?: number) => void;
    formatAssistantText?: (item: Extract<TurnItem, { type: 'assistant_message' }>) => string;
    assistantFooter?: Snippet<[Extract<TurnItem, { type: 'assistant_message' }>]>;
    composerTopSlot?: Snippet<[{ focusComposer: () => void }]>;
    composerFootLeadSlot?: Snippet<[{ focusComposer: () => void }]>;
    testIds?: RuntimeComposerTestIds;
  }

  let {
    sessionId, focused, onswitch, agentChat, hostCommands, compact = false, accessModeOverride = null,
    onaccessmodechange, fallbackCwd = null, hasExternalContext = false, transformOutgoingText,
    transformRestoredQueuedText, onExternalContextConsumed, onopenpath, formatAssistantText,
    assistantFooter, composerTopSlot, composerFootLeadSlot, testIds,
  }: Props = $props();

  const ownerId = untrack(() => sessionId);
  const draft = untrack(() => runtimeDrafts.for(ownerId));
  const agentChatId = $derived(personalAgentOf(runtimeThreads.get(sessionId)?.meta));

  $effect(() => {
    const id = sessionId;
    return untrack(() => runtimeViews.open(id));
  });

  const session = $derived(runtimeThreads.get(sessionId));
  const effectiveCwd = $derived(session?.cwd ?? fallbackCwd ?? null);
  setContext(PATH_LINK_CTX, () => ({ sessionId, cwd: effectiveCwd, workspaceId: session?.workspace_id }));

  const p = $derived(runtimeViews.projections[sessionId] ?? null);
  const loadError = $derived(runtimeViews.loadError[sessionId] ?? null);
  const staleError = $derived(runtimeViews.staleError[sessionId] ?? null);
  const loadingOlder = $derived(!!runtimeViews.loadingOlder[sessionId]);
  let optimisticMessage = $state<OptimisticMessageState | null>(null);
  const effectiveItems = $derived(p ? withOptimisticItem(p.items, optimisticMessage) : optimisticMessage ? [optimisticMessage.item] : []);
  const rows = $derived(p || optimisticMessage ? timelineRows(effectiveItems, p?.turns ?? []) : []);
  let rowCap = $state(200);
  let holdFrom = $state<string | null>(null);
  const visibleRows = $derived(renderWindow(rows, rowCap, holdFrom, timelineRowKey));
  let prevMsgIds: string[] = [];
  const messageIds = $derived.by(() => (prevMsgIds = collectMessageIds(rows, prevMsgIds)));
  const approvals = $derived(new Map((p ? pendingApprovals(p) : []).map((a) => [a.requestId, a] as const)));
  const turn = $derived(p ? latestTurn(p) : null);
  const closed = $derived(session?.status === 'closed');
  const dormant = $derived(session?.status === 'dormant');
  const needsRevive = $derived(closed || dormant);
  const childThreads = $derived(mapChildThreadSummaries(linkedChildren(Object.values(runtimeThreads.byId), sessionId)));
  const tasks = $derived(
    agentTasks(
      p?.items ?? [],
      p?.session?.background_tasks ?? session?.background_tasks ?? [],
      { output: p?.output ?? {}, childThreads },
    ),
  );
  const background = $derived(tasks.running.filter((t) => t.background));
  const sessionError = $derived(deriveSessionError(draft.startError, p));
  const plan = $derived(deriveActivePlan(p, turn));
  const uiAsk = $derived(!!session && !closed && !auth.isImpersonating && session.created_by === auth.me?.id && !!uiControl.promptFor(sessionId));
  // The owner's own sign-in answers a private secret request.
  const secretAsk = $derived(session && !auth.isImpersonating && session.created_by === auth.me?.id ? (secretRequests.bySession[sessionId] ?? null) : null);
  $effect(() => void secretRequests.load());
  // What the agent waits on the person for: answered in the attention dock.
  const attention = $derived(attentionItems(p?.requests ?? [], secretAsk));

  let scroller = $state<HTMLDivElement | null>(null);
  let streamEl = $state<HTMLDivElement | null>(null);
  let followRef = $state<{ releaseBottom: () => void } | null>(null);
  let composerBoxEl = $state<HTMLDivElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);

  const running = $derived((p ? isRunning(p) : false) || draft.sending || (draft.awaitSeq !== null && (p?.lastSequence ?? 0) < draft.awaitSeq));
  const outcome = $derived.by(() => {
    const o = p && !running ? turnOutcome(p, turn) : null;
    return o?.kind === 'partial' && agentChatId ? null : o;
  });
  const ownTurn = $derived(turn?.origin === 'agent');
  const workingFor = $derived.by(() => {
    if (!running || approvals.size > 0 || !turn?.started_at || (turn.status !== 'running' && turn.status !== 'pending')) return null;
    const s = Math.max(0, Math.floor((now() - Date.parse(turn.started_at)) / 1000));
    return formatLiveElapsed(s);
  });

  let pickerOpen = $state(false);
  let agentEl = $state<HTMLButtonElement | null>(null);
  const provider = $derived(session ? sessionProviderSlug(session) : '');
  const agentName = $derived(harnessLabel(provider).replace(/ Code$/, ''));
  const chosenModel = $derived(resolveEffectiveDraftModel(draft.model, session?.model));
  const agentLabel = $derived.by(() => {
    const label = chosenModel ? formatModelLabel(provider, chosenModel, auth.meta) : harnessLabel(provider);
    const effort = draft.effort ?? session?.effort ?? null; // the pick, else what the thread stored
    return effort ? `${label} · ${formatEffortShort(effort)}` : label;
  });

  // A failed turn is ONE block (RuntimeTurnStatus): its error row and the
  // dormant "Agent stopped" line would only repeat it.
  const failedError = $derived(outcome?.kind === 'failed' && p ? turnErrorItem(p.items, turn?.id) : null);
  const lastError = $derived(p?.session?.last_error ? { class: '', message: p.session.last_error } : null);
  const failure = $derived(outcome?.kind === 'failed' ? failureView(failedError?.failure ?? lastError, harnessLabel(provider)) : null);
  const failedQuiet = $derived(!!failure && dormant);
  const queueShown = $derived((background.length > 0 && !closed) || (p?.queued.length ?? 0) > 0 || (needsRevive && !failedQuiet));
  const banners = $derived(buildRuntimeBanners({
    sessionError, uiAsk, queueShown, needsRevive, freshContext: draft.freshContext,
    hasQuestions: Boolean(p?.requests.some((r) => r.kind === 'question' && r.status !== 'pending')),
    hasPlan: Boolean(plan),
  }));
  const accessMode = $derived(accessModeOverride ?? draft.pickedMode ?? session?.access_mode ?? 'approval-required');
  // What the agent takes, as the daemon reports it (live first).
  const caps = $derived(capsOf(p?.session?.capabilities ?? session?.capabilities, runtimeBaselineCaps(provider, auth.meta)));
  const canImages = $derived(attachmentSupport(caps).images);
  const planOn = $derived(!!caps.plan && !!runtimeThreads.planIds[ownerId]); // every prompt, until turned off
  const canFiles = $derived(attachmentSupport(caps).files);
  const takes = (f: File): boolean => takesFile(caps, f);
  /** "Stop background work and start fresh": `next` = the next fresh send asks the daemon to
   *  stop it first; `tried` = it was asked and still refused (an older daemon) — the reason shows. */
  let freshStopBg = $state<'' | 'next' | 'tried'>('');

  $effect(() => {
    if (optimisticMessage && p && effectiveItems === p.items) optimisticMessage = null;
  });

  async function send(
    t: string,
    delivery?: PromptDelivery,
    sent: RuntimeDraftContent = snapshotDraft(draft),
    usedExternal = false,
    optimistic = false,
  ): Promise<void> {
    if (!runtimeDrafts.owns(ownerId, draft) || draft.recovered === false) return;
    draft.sending = true;
    const pics = sent.images.filter((f) => f.mime.startsWith('image/'));
    const docs = sent.images.filter((f) => !f.mime.startsWith('image/'));
    const assembled = assembleComposerMessage(t, sent.context);
    const finalPrompt =
      typeof transformOutgoingText === 'function' ? transformOutgoingText(assembled, sent.fresh_context) : assembled;
    const effectiveAccess =
      (typeof accessModeOverride !== 'undefined' && accessModeOverride) || sent.access_mode;
    const sessionModel = typeof session !== 'undefined' ? session?.model : null;
    const modelOverride =
      typeof resolvePromptModelOverride === 'function'
        ? resolvePromptModelOverride(sent.model, sessionModel)
        : sent.model || undefined;
    try {
      const resp = await runtimeThreads.prompt(
        ownerId,
        {
          text: finalPrompt,
          ...(modelOverride !== undefined ? { model: modelOverride } : {}),
          ...(sent.effort ? { effort: sent.effort } : {}),
          ...(pics.length ? { images: pics.map(({ name, mime, data_base64 }) => ({ ...(name ? { name } : {}), mime, data_base64 })) } : {}),
          ...(docs.length ? { files: docs } : {}),
          ...(effectiveAccess ? { access_mode: effectiveAccess } : {}),
          ...(sent.fresh_context ? { fresh_context: true, ...(typeof freshStopBg !== 'undefined' && freshStopBg === 'next' ? { stop_background: true } : {}) } : {}),
          ...(delivery ? { delivery } : {}),
        },
        sent.images,
      );
      draft.awaitSeq = resp.queued_id ? null : resp.sequence;
      if (delivery === 'steer' && resp.queued_id) toasts.info('Queued instead', 'The agent couldn’t take it mid-turn; it goes when this turn ends.');
      if (resp.queued_id && typeof optimisticMessage !== 'undefined') optimisticMessage = null;
      if (!runtimeDrafts.owns(ownerId, draft)) return;
      acknowledgeComposer(draft, sent);
      if (usedExternal && typeof onExternalContextConsumed === 'function') onExternalContextConsumed();
      draft.flushSave();
      draft.startError = null;
      if (typeof freshStopBg !== 'undefined') freshStopBg = '';
    } catch (e) {
      if (typeof optimisticMessage !== 'undefined') optimisticMessage = null;
      if (!runtimeDrafts.owns(ownerId, draft)) return;
      if (optimistic && typeof restoreOptimisticDraft === 'function') restoreOptimisticDraft(draft, sent);
      if (e instanceof ApiError && (e.status === 400 || e.status === 409 || e.status === 413)) {
        draft.startError = typeof sendRefusal === 'function' ? sendRefusal(e.status, e.message) : e.message;
      } else toastError('Couldn’t send message', e);
    } finally {
      draft.sending = false;
      if (typeof freshStopBg !== 'undefined' && freshStopBg === 'next') freshStopBg = 'tried';
    }
  }

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

  function submit(chosen?: PromptDelivery): void {
    let t = draft.text.trim();
    const extCtx = typeof hasExternalContext !== 'undefined' && Boolean(hasExternalContext);
    if ((!t && draft.images.length === 0 && draft.context.length === 0 && !extCtx) || draft.reopening || draft.sending || draft.recovered === false) {
      return;
    }
    const intent = parseInteractiveSlashInput(t);
    if (intent?.kind === 'send-prompt') t = intent.prompt;
    else if (intent && runViewSlash(intent, sessionId, hostCommands)) {
      draft.text = '';
      return;
    }
    if (!t && extCtx) t = 'Inspect the attached context and propose improvements.';
    if (askTargets.for(ownerId).length) {
      const assembled = assembleComposerMessage(t, draft.context);
      const payload =
        typeof transformOutgoingText === 'function' ? transformOutgoingText(assembled, draft.freshContext) : assembled;
      if (extCtx && typeof onExternalContextConsumed === 'function') onExternalContextConsumed();
      const effectiveAccess =
        (typeof accessModeOverride !== 'undefined' && accessModeOverride) || draft.pickedMode;
      return void askFromComposer(ownerId, draft, payload, effectiveAccess);
    }
    const submitted = snapshotDraft(draft);
    const delivery: PromptDelivery | undefined = !submitted.fresh_context ? (chosen ?? 'queue') : undefined;
    acknowledgeComposer(draft, submitted);
    if (typeof composerEl !== 'undefined' && composerEl) composerEl.style.height = '';
    if (!running && !submitted.fresh_context && typeof buildOptimisticUserMessage === 'function') {
      const assembled = assembleComposerMessage(t, submitted.context);
      const finalPrompt =
        typeof transformOutgoingText === 'function' ? transformOutgoingText(assembled, submitted.fresh_context) : assembled;
      const lastOrd = typeof p !== 'undefined' && p && p.items.length > 0 ? p.items[p.items.length - 1].ordinal : 0;
      optimisticMessage = buildOptimisticUserMessage(finalPrompt, lastOrd, submitted.images);
    }
    const mustReopenFirst = (typeof closed === 'boolean' ? closed : needsRevive) && !submitted.fresh_context;
    if (mustReopenFirst) {
      void reopen().then((ok) => {
        if (ok) return send(t, delivery, submitted, extCtx, true);
        if (typeof optimisticMessage !== 'undefined') optimisticMessage = null;
        if (typeof restoreOptimisticDraft === 'function' && runtimeDrafts.owns(ownerId, draft)) {
          restoreOptimisticDraft(draft, submitted);
        }
      });
    } else {
      void send(t, delivery, submitted, extCtx, true);
    }
  }

  const SLASH = $derived(availableSlash(INTERACTIVE_SLASH_COMMANDS, hostCommands));
  const popId = $props.id();
  let cmdIdx = $state(0);
  let cmdDismissed = $state(false);
  const hasDraftText = $derived(draft.text.trim().length > 0);
  const canSendOrQueue = $derived(
    hasDraftText || draft.images.length > 0 || draft.context.length > 0 || hasExternalContext,
  );
  const composerStateArg = $derived({ key: `rt:${ownerId}`, hasText: draft.text.length > 0 });
  // Otto's view commands, then the agent's own (`commands.updated`, hostCommands.ts).
  const suggestions = $derived(cmdDismissed ? [] : slashSuggestions(draft.text, SLASH, p?.commands ?? [], agentName, hostCommands));
  $effect(() => {
    void suggestions.length;
    if (untrack(() => cmdIdx) !== 0) cmdIdx = 0;
  });
  $effect(() => {
    void draft.text;
    if (untrack(() => cmdDismissed)) cmdDismissed = false;
  });

  function acceptCmd(c: ComposerSlashCommand): void {
    if (c.source !== 'agent' && isImmediateSlashCommand(c.name)) {
      draft.text = `/${c.name}`;
      submit();
    } else {
      draft.text = `/${c.name} `;
      cmdDismissed = true;
      composerEl?.focus();
    }
  }

  /** While a turn runs: the person's mid-turn delivery, or (`alt`, ⌘↵) the other one (composerSend.ts). */
  function midTurn(alt: boolean): PromptDelivery | undefined {
    if (!running) return undefined;
    const d = midTurnDelivery(threadPrefs.midTurn, alt, !!caps.steer);
    if (d.refused && alt) toasts.info(`${agentName} can’t take a message mid-turn`, 'Queued — it goes when this turn ends.');
    return d.delivery;
  }

  async function stop(): Promise<void> {
    if (draft.stopping) return;
    draft.stopping = true;
    try {
      await interruptTurn(ownerId);
    } catch (e) {
      toastError('Couldn’t stop the turn', e);
    } finally {
      draft.stopping = false;
    }
  }

  const respond = (requestId: string, decision: ApprovalDecision): Promise<void> =>
    runtimeThreads.respond(ownerId, requestId, decision).catch((e: unknown) => void toastError('Couldn’t answer the approval', e));

  let askEl = $state<{ onKey: (e: KeyboardEvent) => boolean } | null>(null);
  let emojiEl = $state<{ onKey: (e: KeyboardEvent) => boolean } | null>(null);
  let dockEl = $state<{ onKey: (e: KeyboardEvent, composerEmpty: boolean) => boolean } | null>(null);
  const escState = { lastEscAt: -Infinity };
  function onKeydown(e: KeyboardEvent): void {
    if (e.isComposing || askEl?.onKey(e) || emojiEl?.onKey(e)) return;
    if (caps.plan && !suggestions.length && isPlanToggleKey(e)) return void (e.preventDefault(), (runtimeThreads.planIds[ownerId] = !planOn));
    if (handleSlashMenuKey(e, suggestions, cmdIdx, (i) => (cmdIdx = i), () => (cmdDismissed = true), acceptCmd)) return;
    if (dockEl?.onKey(e, !canSendOrQueue)) return;
    if (!ui.overlayOpen && composerStopKey(e, { running, composerEmpty: !canSendOrQueue, now: performance.now() }, escState)) {
      e.preventDefault();
      e.stopPropagation();
      return void stop();
    }
    if (enterSends(e)) {
      e.preventDefault();
      submit(midTurn(isAltSend(e, isMacPlatform(), isTouchFirst())));
    }
  }

  const focusComposer = (): void => {
    composerEl?.focus({ preventScroll: true });
  };
  /** ⌘. stops the turn from anywhere in this pane (the composer handles its own keys). */
  function onWindowKey(e: KeyboardEvent): void {
    const pane = scroller?.parentElement;
    const here = !!pane && (pane.contains(document.activeElement) || (focused && document.activeElement === document.body));
    if (here && running && !ui.overlayOpen && isStopChord(e)) {
      e.preventDefault();
      void stop();
    }
  }
  /** The dock's "Show": the request's row in the timeline. */
  function jumpTo(requestId: string): void {
    followRef?.releaseBottom();
    scroller?.querySelector(`[data-testid="runtime-approval-${CSS.escape(requestId)}"]`)?.scrollIntoView({ block: 'center' });
  }

  $effect(() => {
    const el = composerEl;
    if (focused && el) untrack(() => el.focus({ preventScroll: true }));
  });
</script>

<svelte:window onkeydown={onWindowKey} />
<section class="rt" class:compact aria-label="Conversation" data-testid="runtime-thread-{sessionId}">
  {#if onswitch}<RuntimeLinkedThreads {sessionId} {onswitch} />{/if}
  <div class="rt-scroll" class:isolated={!ui.overlayOpen} bind:this={scroller}>
    <div class="rt-stream" bind:this={streamEl} data-runtime-stream>
      <div class="rt-col">
        <LoadState what="conversation" loading={!p && !loadError} error={p ? staleError : loadError} empty={!p} onretry={() => runtimeViews.retry(sessionId)}>
          {#if rows.length === 0}
            <EmptyState
              icon="threads"
              title="Start the conversation"
              body="Replies, tool calls and approvals stream here as the agent works."
            />
          {:else if p}
            <RuntimeHistoryBar
              {sessionId}
              hasOlder={p.hasOlder}
              olderTurns={p.olderTurns}
              {loadingOlder}
              totalRows={rows.length}
              {scroller}
              {streamEl}
              onreleasebottom={() => followRef?.releaseBottom()}
              bind:rowCap
            />
            <RuntimeTimelineRows
              {sessionId}
              projection={p}
              rows={visibleRows}
              {agentName}
              {agentChatId}
              cwd={effectiveCwd}
              workspaceId={session?.workspace_id}
              onrespond={respond}
              hideItemId={failedError?.id ?? null}
              onedituser={closed ? undefined : (m) => void restoreFailedTurnToDraft({ sessionId, draft, text: m.text, attachments: m.attachments, transformText: transformRestoredQueuedText, focusComposer })}
              {onopenpath}
              {formatAssistantText}
              {assistantFooter}
            />
          {/if}
          <RuntimeTurnStatus
            projection={p}
            {workingFor}
            {ownTurn}
            background={closed ? [] : background}
            {outcome}
            {failure}
            {agentName}
            revive={failedQuiet && !draft.reopening ? () => void reopen() : undefined}
            disabled={closed}
            onedit={(t, atts) => void restoreFailedTurnToDraft({ sessionId, draft, text: t, attachments: atts, transformText: transformRestoredQueuedText, focusComposer })}
            onresend={async (t, atts) => {
              try {
                const r = await buildFailedTurnResendDraft({ sessionId, draft, text: t, attachments: atts, transformText: transformRestoredQueuedText });
                // A dormant thread revives first (like a normal submit); restored IDE context chips are consumed by the resend.
                if (!(await reopen())) return;
                await send(r.text, undefined, r.sent, typeof transformRestoredQueuedText === 'function');
              } catch (e) { toastError('Couldn’t resend message', e); }
            }}
          />
        </LoadState>
      </div>
    </div>
  </div>
  <RuntimeFollow bind:this={followRef} {scroller} seq={p?.lastSequence} {messageIds} needs={attentionPillText(attention)} firstRow={visibleRows[0] ? timelineRowKey(visibleRows[0]) : null} bind:holdFrom />
  <RuntimeStreamTools root={streamEl} {rows} bind:rowCap bind:text={draft.text} composer={composerEl} />

  <div class="rt-dock">
    <RuntimeBannerStack blocks={banners}>
      {#snippet block(key: string)}
        {#if key === 'error' && sessionError}
          <p class="rt-banner" dir="auto" use:announceShown={{ text: sessionError, assertive: true }}><Icon name="warning" size={13} />{sessionError}</p>
          <RuntimeFreshStop show={draft.freshContext && background.some((b) => b.stopId) && freshStopBg === ''} count={background.length} onstop={() => { freshStopBg = 'next'; submit(); }} />
        {:else if key === 'ui'}
          <RuntimeUiControl {sessionId} />
        {:else if key === 'questions' && p}
          <LazyMount lazy={RuntimeQuestions} what="the questions" variant="compact" props={{ requests: p.requests, sessionId, agent: agentName, onanswered: focusComposer }} />
        {:else if key === 'plan' && plan}
          <RuntimePlan {plan} />
        {:else if key === 'queue'}
          <RuntimeQueueBar
            {sessionId}
            {background}
            onshowtasks={hostCommands?.showTasks ? () => hostCommands.showTasks!(sessionId) : undefined}
            queued={p?.queued ?? []}
            {running}
            canSteer={!!caps.steer}
            {closed}
            dormant={dormant && !failedQuiet}
            reopening={draft.reopening}
            onreopen={() => void reopen()}
            onedit={(t, restoredAtts, editedSessionId) => restorePoppedQueuedMessageToDraft({ activeSessionId: sessionId, editedSessionId, text: t, restoredAtts, transformText: transformRestoredQueuedText, focusComposer })}
          />
        {:else if key === 'fresh'}
          <div class="rt-fresh" data-testid="runtime-fresh-notice" use:announceShown={'Next message starts fresh'}>
            <span>Next message starts fresh</span>
            <button type="button" class="btn small ghost" onclick={() => { draft.freshContext = false; focusComposer(); }}>Cancel</button>
          </div>
        {/if}
      {/snippet}
    </RuntimeBannerStack>

    {#if p && attention.length > 0}
      <RuntimeAttentionDock
        bind:this={dockEl}
        {sessionId}
        items={attention}
        requests={p.requests}
        {approvals}
        secret={secretAsk}
        agent={agentName}
        onrespond={respond}
        onanswered={focusComposer}
        onjump={jumpTo}
      />
    {/if}
    {#if composerTopSlot}{@render composerTopSlot({ focusComposer })}{/if}

    <div class="rt-composer" style:border-color={planOn ? 'color-mix(in srgb, var(--warning) 42%, var(--border))' : undefined} bind:this={composerBoxEl}>
      {#if suggestions.length > 0}
        <StageSlashPopup id={popId} items={suggestions} active={cmdIdx} onaccept={acceptCmd} onhover={(i) => (cmdIdx = i)} />
      {/if}
      {#if EmojiAssistView.component}<EmojiAssistView.component bind:this={emojiEl} textareaEl={composerEl} />{/if}
      <RuntimeContextChips context={draft.context} onremove={(id) => { draft.context = draft.context.filter((c) => c.id !== id); }} onfocus={focusComposer} />
      <RuntimeAskAgent
        bind:this={askEl}
        part="chips"
        {sessionId}
        {provider}
        access={accessMode}
        cwd={effectiveCwd}
        textareaEl={composerEl}
        anchorEl={composerBoxEl}
        allowAgents={!agentChat}
        readOnly={!!session?.meta?.read_only}
        text={draft.text}
        onreplace={(t) => { draft.text = t; focusComposer(); }}
        onaddcontext={(ctx) => { draft.context = addComposerContext(draft.context, ctx); }}
      />
      <textarea
        bind:this={composerEl}
        bind:value={draft.text}
        use:composerViewState={composerStateArg}
        dir="auto"
        rows={2}
        spellcheck="false"
        {...{ autocorrect: 'off' }}
        autocapitalize="off"
        autocomplete="off"
        disabled={draft.reopening}
        placeholder={composerPlaceholderText(closed, approvals, running, agentChat, planOn, threadPrefs.midTurn === 'steer' && !!caps.steer)}
        aria-label="Message"
        aria-autocomplete="list"
        aria-controls={suggestions.length > 0 ? `${popId}-list` : undefined}
        aria-activedescendant={suggestions.length > 0 ? `${popId}-opt-${cmdIdx}` : undefined}
        data-testid={testIds?.composer ?? 'runtime-composer'}
        onkeydown={onKeydown}
        onpaste={(e) => pasteDraftAttachments(e, canImages, ownerId, draft, takes)}
      ></textarea>
      {#if draft.images.length > 0}
        <RuntimeComposerImages images={draft.images} onremove={(i) => (draft.images = draft.images.filter((_, j) => j !== i))} />
      {/if}
      <div class="rt-foot">
        {#if composerFootLeadSlot}{@render composerFootLeadSlot({ focusComposer })}{/if}
        <RuntimeDraftStatus {draft} />
        {#if canImages}<RuntimeComposerAttach {canFiles} testId={testIds?.attach ?? 'runtime-attach'} disabled={draft.reopening} onfiles={(f) => void appendDraftAttachments(ownerId, draft, f, takes)} />{/if}
        {#if !agentChat}<RuntimeAskAgent part="button" {sessionId} {provider} access={accessMode} disabled={closed || !session} />{/if}
        <span class="rt-grow">
          {#if !agentChat}
            <RuntimeModelChip
              bind:el={agentEl}
              {provider}
              label={agentLabel}
              expanded={pickerOpen}
              disabled={closed || !session}
              showIcon={!!session}
              testId={testIds?.model ?? 'runtime-model'}
              onprefetch={() => ModelPicker.prefetch()}
              onopen={() => {
                if (ModelPicker.error) ModelPicker.retry();
                pickerOpen = true;
              }}
            />
            <RuntimeAccessChip
              readOnly={!!session?.meta?.read_only}
              mode={accessMode}
              agent={harnessLabel(provider)}
              cwd={effectiveCwd ?? 'this folder'}
              disabled={closed || !session}
              onpick={(m) => {
                draft.pickedMode = m;
                onaccessmodechange?.(m);
              }}
            />
            {#if caps.plan}<RuntimePlanToggle id={ownerId} on={planOn} disabled={closed || !session} />{/if}
          {/if}
        </span>
        <RuntimeComposerActions
          action={primaryAction({ running, canSend: canSendOrQueue, pref: threadPrefs.midTurn, canSteer: !!caps.steer })}
          canSteer={!!caps.steer}
          agent={agentName}
          {running}
          sendDisabled={!draft.recovered || draft.reopening || draft.sending || !canSendOrQueue}
          stopping={draft.stopping}
          waiting={attention.length > 0}
          stopTestId={testIds?.stop ?? 'runtime-stop'}
          sendTestId={testIds?.send ?? 'runtime-send'}
          onstop={() => void stop()}
          onsend={() => submit(midTurn(false))}
        />
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
    model={chosenModel}
    effectiveModel={session?.model ?? null}
    effort={draft.effort}
    mode="full"
    onmodel={(m) => (draft.model = m)}
    oneffort={caps.efforts?.length ? (e) => (draft.effort = e) : undefined}
    effortLevels={caps.efforts ?? []}
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
    overflow-anchor: none;
  }
  .rt-scroll.isolated {
    contain: layout style paint;
  }
  .rt-col,
  .rt-dock {
    inline-size: 100%;
    margin-inline: auto;
    padding-inline: clamp(12px, 3.2cqi, 48px);
  }
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
  .rt-col {
    display: flex;
    flex-direction: column;
    padding-block: 16px 24px;
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
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
  }
  /* A visible focus ring (the shared accent line), not just a border shade. */
  .rt-composer:focus-within {
    border-color: var(--accent-line);
    box-shadow: 0 0 0 1px var(--accent-line);
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
    min-inline-size: 0;
  }
  .rt-grow {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: 1;
    min-inline-size: 0;
  }
  /* A-9: a narrow embed (PR dock, 280–360 px) keeps both chips readable —
     the chips take their own row above the foot's buttons, never just "…". */
  @container (max-inline-size: 380px) {
    .rt-foot { flex-wrap: wrap; row-gap: 4px; }
    .rt-grow { order: -1; flex-basis: 100%; flex-wrap: wrap; }
  }
</style>
