<script lang="ts">
  // Structured agent runtime thread view shared between Threads and IDE Agent dock.
  import { setContext, tick, untrack, type Snippet } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import { ApiError } from '../../../lib/api/client';
  import type { ApprovalDecision, PromptDelivery, RuntimeAccessMode, RuntimeDraftContent, TurnItem } from '../../../lib/api/types';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { enterSends } from '../../../lib/enterSends';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { PATH_LINK_CTX } from '../../../lib/pathLinks/open';
  import { personalAgentOf } from '../../../lib/personalAgentBadges';
  import { formatEffortShort, formatModelLabel, supportsEffortSelection } from '../../../lib/providerModels';
  import { harnessLabel, runtimeBaselineCaps, sessionProviderSlug } from '../../../lib/providers';
  import type { RuntimeHostCommands } from '../../../lib/runtimeThreadPort';
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
  import { agentTasks } from './agentTasks';
  import { askFromComposer, askTargets } from './askTargets.svelte';
  import { attachmentSupport, capsOf, takesFile } from './attachmentSupport';
  import { addComposerContext, assembleComposerMessage } from './composerContext';
  import { availableSlash } from './hostCommands';
  import RuntimeAccessChip from './RuntimeAccessChip.svelte';
  import RuntimeAskAgent from './RuntimeAskAgent.svelte';
  import RuntimeBannerStack from './RuntimeBannerStack.svelte';
  import {
    appendDraftAttachments, buildOptimisticUserMessage, buildRuntimeBanners, collectMessageIds,
    composerPlaceholderText, deriveActivePlan, deriveSessionError, handleSlashMenuKey, mapChildThreadSummaries,
    pasteDraftAttachments, resolveEffectiveDraftModel, resolvePromptModelOverride, restoreOptimisticDraft,
    runViewSlash, withOptimisticItem, type OptimisticMessageState,
  } from './runtimeComposer';
  import { buildFailedTurnResendDraft, restoreFailedTurnToDraft, restorePoppedQueuedMessageToDraft } from './runtimeRetryRestore';
  import RuntimeComposerImages from './RuntimeComposerImages.svelte';
  import RuntimeContextChips from './RuntimeContextChips.svelte';
  import RuntimeDraftStatus from './RuntimeDraftStatus.svelte';
  import RuntimeFollow from './RuntimeFollow.svelte';
  import SelectionQuote from '../../../lib/components/SelectionQuote.svelte';
  import { withQuote } from '../../../lib/quote/quote';
  import { registerFindProvider } from '../../../lib/findProviders';
  import { threadFindProvider } from './threadFind';
  import RuntimeHistoryBar from './RuntimeHistoryBar.svelte';
  import RuntimeLinkedThreads from './RuntimeLinkedThreads.svelte';
  import RuntimePlan from './RuntimePlan.svelte';
  import RuntimeQueueBar from './RuntimeQueueBar.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import RuntimeTimelineRows from './RuntimeTimelineRows.svelte';
  import RuntimeTurnStatus from './RuntimeTurnStatus.svelte';
  import RuntimeUiControl from './RuntimeUiControl.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { formatLiveElapsed } from './liveStatus';
  import { isRunning, latestTurn, pendingApprovals, timelineRows, turnOutcome } from './timeline';

  const RuntimeQuestions = lazyComponent(() => import('./RuntimeQuestions.svelte'));
  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));

  export type { RuntimeComposerTestIds } from '../../../lib/runtimeThreadPort';
  import type { RuntimeComposerTestIds } from '../../../lib/runtimeThreadPort';

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
    transformOutgoingText?: (text: string) => string;
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
  const visibleRows = $derived(rows.length > rowCap ? rows.slice(rows.length - rowCap) : rows);
  let prevMsgIds: string[] = [];
  const messageIds = $derived.by(() => (prevMsgIds = collectMessageIds(rows, prevMsgIds)));
  const approvals = $derived(new Map((p ? pendingApprovals(p) : []).map((a) => [a.requestId, a] as const)));
  const turn = $derived(p ? latestTurn(p) : null);
  const closed = $derived(session?.status === 'closed');
  const dormant = $derived(session?.status === 'dormant');
  const needsRevive = $derived(closed || dormant);
  const childThreads = $derived(mapChildThreadSummaries(runtimeThreads.byId, sessionId));
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
  const queueShown = $derived((background.length > 0 && !closed) || (p?.queued.length ?? 0) > 0 || needsRevive);
  const banners = $derived(buildRuntimeBanners({
    sessionError,
    uiAsk,
    hasQuestions: Boolean(p?.requests.some((r) => r.kind === 'question')),
    hasPlan: Boolean(plan),
    queueShown,
    needsRevive,
    freshContext: draft.freshContext,
  }));

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
    return draft.effort ? `${label} · ${formatEffortShort(draft.effort)}` : label;
  });

  const accessMode = $derived(accessModeOverride ?? draft.pickedMode ?? session?.access_mode ?? 'approval-required');
  // What the agent takes, as the daemon reports it (live first).
  const caps = $derived(capsOf(p?.session?.capabilities ?? session?.capabilities, runtimeBaselineCaps(provider, auth.meta)));
  const canImages = $derived(attachmentSupport(caps).images);
  const canFiles = $derived(attachmentSupport(caps).files);
  const takes = (f: File): boolean => takesFile(caps, f);
  let fileEl = $state<HTMLInputElement | null>(null);

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
      typeof transformOutgoingText === 'function' ? transformOutgoingText(assembled) : assembled;
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
          ...(sent.fresh_context ? { fresh_context: true } : {}),
          ...(delivery ? { delivery } : {}),
        },
        sent.images,
      );
      draft.awaitSeq = resp.queued_id ? null : resp.sequence;
      if (resp.queued_id && typeof optimisticMessage !== 'undefined') optimisticMessage = null;
      if (!runtimeDrafts.owns(ownerId, draft)) return;
      acknowledgeComposer(draft, sent);
      if (usedExternal && typeof onExternalContextConsumed === 'function') onExternalContextConsumed();
      draft.flushSave();
      draft.startError = null;
    } catch (e) {
      if (typeof optimisticMessage !== 'undefined') optimisticMessage = null;
      if (!runtimeDrafts.owns(ownerId, draft)) return;
      if (optimistic && typeof restoreOptimisticDraft === 'function') restoreOptimisticDraft(draft, sent);
      if (e instanceof ApiError && (e.status === 400 || e.status === 409)) draft.startError = e.message;
      else toastError('Couldn’t send message', e);
    } finally {
      draft.sending = false;
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

  function submit(): void {
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
        typeof transformOutgoingText === 'function' ? transformOutgoingText(assembled) : assembled;
      if (extCtx && typeof onExternalContextConsumed === 'function') onExternalContextConsumed();
      const effectiveAccess =
        (typeof accessModeOverride !== 'undefined' && accessModeOverride) || draft.pickedMode;
      return void askFromComposer(ownerId, draft, payload, effectiveAccess);
    }
    const submitted = snapshotDraft(draft);
    const delivery: PromptDelivery | undefined = !submitted.fresh_context ? 'queue' : undefined;
    acknowledgeComposer(draft, submitted);
    if (typeof composerEl !== 'undefined' && composerEl) composerEl.style.height = '';
    if (!running && !submitted.fresh_context && typeof buildOptimisticUserMessage === 'function') {
      const assembled = assembleComposerMessage(t, submitted.context);
      const finalPrompt =
        typeof transformOutgoingText === 'function' ? transformOutgoingText(assembled) : assembled;
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
  const suggestions = $derived.by(() => {
    if (cmdDismissed || !draft.text.startsWith('/')) return [];
    const prefix = /^\/([\w:-]*)$/.exec(draft.text)?.[1].toLowerCase();
    return prefix === undefined ? [] : SLASH.filter((c) => c.name.includes(prefix));
  });
  $effect(() => {
    void suggestions.length;
    if (untrack(() => cmdIdx) !== 0) cmdIdx = 0;
  });
  $effect(() => {
    void draft.text;
    if (untrack(() => cmdDismissed)) cmdDismissed = false;
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
    if (handleSlashMenuKey(e, suggestions, cmdIdx, (i) => (cmdIdx = i), () => (cmdDismissed = true), acceptCmd)) return;
    if (enterSends(e)) {
      e.preventDefault();
      submit();
    }
  }

  const focusComposer = (): void => {
    composerEl?.focus({ preventScroll: true });
  };

  // ⌘F finds in the whole loaded thread, not just its mounted rows.
  $effect(() =>
    registerFindProvider(
      threadFindProvider({ root: () => streamEl, rows: () => rows, cap: () => rowCap, setCap: (n) => (rowCap = n) }),
    ),
  );

  // Quote a passage of an answer: it lands in the composer, caret below it.
  function quoteIntoComposer(text: string): void {
    draft.text = withQuote(draft.text, text);
    void tick().then(() => {
      focusComposer();
      composerEl?.setSelectionRange(draft.text.length, draft.text.length);
    });
  }

  $effect(() => {
    const el = composerEl;
    if (focused && el) untrack(() => el.focus({ preventScroll: true }));
  });
</script>

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
  <RuntimeFollow bind:this={followRef} {scroller} seq={p?.lastSequence} {messageIds} />
  <SelectionQuote root={streamEl} within=".rt-assistant" onquote={quoteIntoComposer} />

  <div class="rt-dock">
    <RuntimeBannerStack blocks={banners}>
      {#snippet block(key: string)}
        {#if key === 'error' && sessionError}
          <p class="rt-banner" dir="auto" use:announceShown={{ text: sessionError, assertive: true }}><Icon name="warning" size={13} />{sessionError}</p>
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
            {dormant}
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

    {#if composerTopSlot}{@render composerTopSlot({ focusComposer })}{/if}

    <div class="rt-composer" bind:this={composerBoxEl}>
      {#if suggestions.length > 0}
        <StageSlashPopup id={popId} items={suggestions} active={cmdIdx} onaccept={acceptCmd} onhover={(i) => (cmdIdx = i)} />
      {/if}
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
        placeholder={composerPlaceholderText(closed, approvals, running, agentChat)}
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
        {#if canImages}
          <button
            type="button"
            class="icon-btn"
            aria-label={canFiles ? 'Attach an image or PDF' : 'Attach an image'}
            title={canFiles ? 'Attach an image or PDF (or paste one)' : 'Attach an image (or paste one)'}
            data-testid={testIds?.attach ?? 'runtime-attach'}
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
              void appendDraftAttachments(ownerId, draft, e.currentTarget.files ?? [], takes);
              e.currentTarget.value = '';
            }}
          />
        {/if}
        {#if !agentChat}<RuntimeAskAgent part="button" {sessionId} {provider} access={accessMode} disabled={closed || !session} />{/if}
        <span class="rt-grow">
          {#if !agentChat}
            <button
              bind:this={agentEl}
              type="button"
              class="rt-agent"
              aria-haspopup="dialog"
              aria-expanded={pickerOpen}
              aria-label="Model for the next turn: {agentLabel}"
              title="Model and effort for the next turn"
              data-testid={testIds?.model ?? 'runtime-model'}
              disabled={closed || !session}
              onpointerenter={() => ModelPicker.prefetch()}
              onclick={() => {
                if (ModelPicker.error) ModelPicker.retry();
                pickerOpen = true;
              }}
            >
              {#if session && hasProviderIcon(provider)}<ProviderIcon {provider} size={13} />{/if}
              <span class="ellipsis">{agentLabel}</span>
            </button>
            <RuntimeAccessChip
              mode={accessMode}
              agent={harnessLabel(provider)}
              cwd={effectiveCwd ?? 'this folder'}
              disabled={closed || !session}
              onpick={(m) => {
                draft.pickedMode = m;
                onaccessmodechange?.(m);
              }}
            />
          {/if}
        </span>
        {#if running && !canSendOrQueue}
          <button
            type="button"
            class="rt-send stop"
            aria-label="Stop the turn"
            title="Stop"
            data-testid={testIds?.stop ?? 'runtime-stop'}
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
            data-testid={testIds?.send ?? 'runtime-send'}
            disabled={!draft.recovered || draft.reopening || draft.sending || !canSendOrQueue}
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
    min-inline-size: 0;
  }
  .rt-grow {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: 1;
    min-inline-size: 0;
  }
  .rt-agent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-inline-size: 100%;
    min-inline-size: 0;
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
