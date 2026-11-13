<script lang="ts">
  // Threads-powered composer footer for `IdeAgentPanel.svelte`: banner stack,
  // smart IDE context bar, slash completion, Council & Ask-another-agent
  // delegation, image/PDF paste & file attach, dynamic ModelPickerPopover,
  // RuntimeAccessChip + Plan toggle, and Send / Queue / Stop CTA.
  import { untrack } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import {
    formatEffortShort,
    formatModelLabel,
    supportsEffortSelection,
    supportsInteractionMode,
  } from '../../../lib/providerModels';
  import { harnessLabel } from '../../../lib/providers';
  import {
    HOST_ONLY_SLASH_COMMANDS,
    INTERACTIVE_SLASH_COMMANDS,
    isImmediateSlashCommand,
    parseInteractiveSlashInput,
    type ComposerSlashCommand,
  } from '../../../lib/slashActions';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { uiControl } from '../../../lib/stores/uiControl.svelte';
  import { agentTasks } from '../../threads/runtime/agentTasks'; // ui-guards: allow — shared background task counter for RuntimeQueueBar
  import type { BannerBlock } from '../../threads/runtime/banners'; // ui-guards: allow — shared banner stack model
  import { council, councilAgentLabel } from '../../threads/runtime/councilStore.svelte'; // ui-guards: allow — shared multi-agent council state
  import CouncilStagePicker from '../../threads/runtime/CouncilStagePicker.svelte'; // ui-guards: allow — shared Ask-several council stage picker
  import RuntimeAccessChip from '../../threads/runtime/RuntimeAccessChip.svelte'; // ui-guards: allow — shared Supervised/Auto/Full access chip
  import RuntimeAskAgent from '../../threads/runtime/RuntimeAskAgent.svelte'; // ui-guards: allow — shared @agent peer delegation picker
  import RuntimeBannerStack from '../../threads/runtime/RuntimeBannerStack.svelte'; // ui-guards: allow — shared composer banner stack
  import RuntimeComposerImages from '../../threads/runtime/RuntimeComposerImages.svelte'; // ui-guards: allow — shared composer image/PDF chips
  import { mergeQueuedEdit } from '../../threads/runtime/runtimeComposer'; // ui-guards: allow — shared queued prompt edit helper
  import RuntimePlan from '../../threads/runtime/RuntimePlan.svelte'; // ui-guards: allow — shared live agent plan banner
  import RuntimeQueueBar from '../../threads/runtime/RuntimeQueueBar.svelte'; // ui-guards: allow — shared durable queue & revive bar
  import { isRuntimeProvider } from '../../threads/runtime/runtimeThreads.svelte'; // ui-guards: allow — shared runtime harness predicate
  import RuntimeUiControl from '../../threads/runtime/RuntimeUiControl.svelte'; // ui-guards: allow — shared UI control grant banner
  import { latestTurn } from '../../threads/runtime/timeline'; // ui-guards: allow — shared latest turn lookup for active plan
  import StageSlashPopup from '../../threads/StageSlashPopup.svelte'; // ui-guards: allow — shared slash command completion popup
  import { ide } from '../ideStore.svelte';
  import { ideAgentEngine as ideAgent } from './ideAgentEngine.svelte';
  import IdeContextBar, { showIdeContextMenu } from './IdeContextBar.svelte';

  let { onsent }: { onsent?: () => void } = $props();

  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));
  const RuntimeQuestions = lazyComponent(() => import('../../threads/runtime/RuntimeQuestions.svelte')); // ui-guards: allow — lazy structured question prompt card

  let addContextBtnEl = $state<HTMLButtonElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let agentBtnEl = $state<HTMLButtonElement | null>(null);
  let fileInputEl = $state<HTMLInputElement | null>(null);
  let askEl = $state<{ onKey: (e: KeyboardEvent) => boolean } | null>(null);
  let pickerOpen = $state(false);
  let cmdIdx = $state(0);
  let cmdDismissed = $state(false);
  const slashPopId = $props.id();

  $effect(() =>
    pickerOpen && !ideAgent.sessionId
      ? untrack(() =>
          council.watchModifiers({ provider: ideAgent.provider, model: ideAgent.model, effort: ideAgent.effort }),
        )
      : undefined,
  );

  const activeThread = $derived(ideAgent.activeThread);
  const runtimeSession = $derived(ideAgent.activeRuntimeSession);
  const activeProvider = $derived(activeThread?.provider ?? ideAgent.provider);
  const proj = $derived(ideAgent.projection);
  const closed = $derived(runtimeSession?.status === 'closed');
  const dormant = $derived(runtimeSession?.status === 'dormant');
  const needsRevive = $derived(closed || dormant);

  const agentChipLabel = $derived.by(() => {
    if (!ideAgent.sessionId && council.on) return councilAgentLabel(auth.meta);
    const m = ideAgent.effectiveModel;
    const base = m ? formatModelLabel(activeProvider, m, auth.meta) : harnessLabel(activeProvider);
    return supportsEffortSelection(activeProvider) && ideAgent.effort
      ? `${base} · ${formatEffortShort(ideAgent.effort)}`
      : base;
  });

  const hasDraftPayload = $derived(
    Boolean(ideAgent.composerText.trim()) || ideAgent.contexts.length > 0 || ideAgent.attachments.length > 0,
  );
  const canSend = $derived(hasDraftPayload && !ideAgent.sending && !ideAgent.reopening);

  const turn = $derived(proj ? latestTurn(proj) : null);
  const activePlan = $derived.by(() => {
    if (!proj || !turn) return null;
    const pl = proj.plans.findLast((x) => x.turn_id === turn.id);
    return pl && pl.steps.some((s) => s.status !== 'completed') ? pl : null;
  });
  const background = $derived.by(() => {
    const tasks = agentTasks(
      proj?.items ?? [],
      proj?.session?.background_tasks ?? runtimeSession?.background_tasks ?? [],
    );
    return tasks.running.filter((t) => t.background);
  });
  const uiAsk = $derived(
    Boolean(
      ideAgent.sessionId &&
        runtimeSession &&
        !closed &&
        !auth.isImpersonating &&
        runtimeSession.created_by === auth.me?.id &&
        uiControl.promptFor(ideAgent.sessionId),
    ),
  );
  const queueShown = $derived(
    Boolean(
      ideAgent.sessionId && ((background.length > 0 && !closed) || (proj?.queued.length ?? 0) > 0 || needsRevive),
    ),
  );
  const banners = $derived.by((): BannerBlock[] => [
    ...(uiAsk ? [{ key: 'ui', label: 'UI control request', urgent: true }] : []),
    ...(proj?.requests.some((r) => r.kind === 'question')
      ? [{ key: 'questions', label: 'questions', urgent: true }]
      : []),
    ...(activePlan ? [{ key: 'plan', label: 'plan' }] : []),
    ...(queueShown ? [{ key: 'queue', label: needsRevive ? 'revive' : 'queue' }] : []),
    ...(ideAgent.freshContext ? [{ key: 'fresh', label: 'fresh context' }] : []),
  ]);

  const slashSuggestions = $derived.by((): ComposerSlashCommand[] => {
    const m = /^\/([\w:-]*)$/.exec(ideAgent.composerText);
    if (!m || cmdDismissed) return [];
    const prefix = m[1].toLowerCase();
    return INTERACTIVE_SLASH_COMMANDS.filter((c) => !HOST_ONLY_SLASH_COMMANDS.has(c.name) && c.name.includes(prefix));
  });

  $effect(() => {
    void slashSuggestions.length;
    cmdIdx = 0;
  });
  $effect(() => {
    void ideAgent.composerText;
    cmdDismissed = false;
  });

  const contextSig = $derived(ideAgent.contexts.map((c) => `${c.id}:${c.snippet.length}`).join('|'));
  let prevContextSig: string | null = null;
  let prevContextCount = 0;
  $effect(() => {
    const sig = contextSig;
    const count = ideAgent.contexts.length;
    if (sig && (prevContextSig === null || (sig !== prevContextSig && count >= prevContextCount))) {
      requestAnimationFrame(() => composerEl?.focus());
    }
    prevContextSig = sig;
    prevContextCount = count;
  });

  function triggerSend(override?: string): void {
    onsent?.();
    void ideAgent.sendPrompt(override);
  }

  function handleSlashSubmit(): boolean {
    const slash = parseInteractiveSlashInput(ideAgent.composerText.trim());
    if (!slash) return false;
    switch (slash.kind) {
      case 'open-model-picker':
      case 'open-effort-picker':
        ideAgent.composerText = '';
        ModelPicker.prefetch();
        pickerOpen = true;
        return true;
      case 'set-model':
        ideAgent.model = slash.model;
        ideAgent.composerText = '';
        return true;
      case 'set-effort':
        ideAgent.effort = slash.effort;
        ideAgent.composerText = '';
        return true;
      case 'set-mode':
        ideAgent.setInteractionMode(slash.mode);
        ideAgent.composerText = slash.prompt ?? '';
        if (slash.prompt) {
          triggerSend(slash.prompt);
          ideAgent.composerText = '';
        }
        return true;
      case 'open-mode-picker':
        ideAgent.composerText = '';
        ModelPicker.prefetch();
        pickerOpen = true;
        return true;
      case 'fresh-context':
        ideAgent.freshContext = true;
        ideAgent.composerText = '';
        return true;
      case 'new-thread':
        ideAgent.startNewChat();
        ideAgent.composerText = '';
        return true;
      case 'toggle-terminal':
        ide.openBottomTab('terminal');
        ideAgent.composerText = '';
        return true;
      case 'send-prompt':
        ideAgent.composerText = '';
        triggerSend(slash.prompt);
        return true;
      default:
        return false;
    }
  }

  function acceptSlashCmd(c: ComposerSlashCommand): void {
    if (isImmediateSlashCommand(c.name)) {
      ideAgent.composerText = `/${c.name}`;
      if (!handleSlashSubmit() && canSend) triggerSend();
    } else {
      ideAgent.composerText = `/${c.name} `;
      cmdDismissed = true;
      composerEl?.focus();
    }
  }

  function onComposerKeydown(e: KeyboardEvent): void {
    if (e.isComposing || askEl?.onKey(e)) return;
    if (slashSuggestions.length > 0) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        cmdIdx = (cmdIdx + (e.key === 'ArrowDown' ? 1 : slashSuggestions.length - 1)) % slashSuggestions.length;
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        cmdDismissed = true;
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        e.preventDefault();
        acceptSlashCmd(slashSuggestions[cmdIdx]);
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      if (handleSlashSubmit()) return;
      if (canSend) triggerSend();
    }
  }

  function onPaste(e: ClipboardEvent): void {
    if (!ideAgent.canAttachImages) return;
    const allowPdf = ideAgent.canAttachFiles;
    const files = [...(e.clipboardData?.files ?? [])].filter(
      (f) => f.type.startsWith('image/') || (allowPdf && f.type === 'application/pdf'),
    );
    if (files.length === 0) return;
    e.preventDefault();
    void ideAgent.addAttachments(files);
  }
</script>

<footer class="ia-foot">
  {#if banners.length > 0 && ideAgent.sessionId}
    {@const sid = ideAgent.sessionId}
    <RuntimeBannerStack blocks={banners}>
      {#snippet block(key: string)}
        {#if key === 'ui'}
          <RuntimeUiControl sessionId={sid} />
        {:else if key === 'questions' && proj}
          <LazyMount
            lazy={RuntimeQuestions}
            what="the questions"
            variant="compact"
            props={{
              requests: proj.requests,
              sessionId: sid,
              agent: harnessLabel(activeProvider).replace(/ Code$/, ''),
              onanswered: () => composerEl?.focus(),
            }}
          />
        {:else if key === 'plan' && activePlan}
          <RuntimePlan plan={activePlan} />
        {:else if key === 'queue'}
          <RuntimeQueueBar
            sessionId={sid}
            {background}
            queued={proj?.queued ?? []}
            running={ideAgent.isWorking}
            canSteer={activeProvider === 'claude'}
            {closed}
            {dormant}
            reopening={ideAgent.reopening}
            onreopen={() => void ideAgent.reopenActiveSession()}
            onedit={(t) => {
              ideAgent.composerText = mergeQueuedEdit(ideAgent.composerText, t);
              composerEl?.focus();
            }}
          />
        {:else if key === 'fresh'}
          <div class="ia-fresh" use:announceShown={'Next message starts fresh'}>
            <span>Next message starts fresh</span>
            <button
              type="button"
              class="btn small ghost"
              onclick={() => {
                ideAgent.freshContext = false;
                composerEl?.focus();
              }}
            >
              Cancel
            </button>
          </div>
        {/if}
      {/snippet}
    </RuntimeBannerStack>
  {/if}

  <IdeContextBar onfocuscomposer={() => composerEl?.focus()} />

  <div class="ia-composer-box">
    {#if slashSuggestions.length > 0}
      <StageSlashPopup
        id={slashPopId}
        items={slashSuggestions}
        active={cmdIdx}
        onaccept={acceptSlashCmd}
        onhover={(i) => (cmdIdx = i)}
      />
    {/if}

    {#if !ideAgent.sessionId}
      <CouncilStagePicker part="seats" provider={ideAgent.provider} model={ideAgent.model} effort={ideAgent.effort} />
    {:else if isRuntimeProvider(activeProvider)}
      <RuntimeAskAgent
        bind:this={askEl}
        part="chips"
        sessionId={ideAgent.sessionId}
        provider={activeProvider}
        access={ideAgent.runtimeAccessMode}
        readOnly={Boolean(runtimeSession?.meta?.read_only)}
        text={ideAgent.composerText}
        onreplace={(t) => {
          ideAgent.composerText = t;
          composerEl?.focus();
        }}
      />
    {/if}

    <textarea
      bind:this={composerEl}
      bind:value={ideAgent.composerText}
      class="ia-textarea"
      rows={2}
      dir="auto"
      disabled={ideAgent.reopening}
      aria-label="Ask Agent or describe a code change"
      aria-autocomplete="list"
      aria-controls={slashSuggestions.length > 0 ? `${slashPopId}-list` : undefined}
      aria-activedescendant={slashSuggestions.length > 0 ? `${slashPopId}-opt-${cmdIdx}` : undefined}
      placeholder={closed
        ? 'Send a message to reopen this thread…'
        : ideAgent.isWorking
          ? 'Queue a follow-up for when this turn ends…'
          : 'Ask anything, type @agent to delegate, or / for commands…'}
      onkeydown={onComposerKeydown}
      onpaste={onPaste}
      data-testid="ide-agent-composer"
    ></textarea>

    {#if ideAgent.attachments.length > 0}
      <div class="ia-attach-row">
        <RuntimeComposerImages images={ideAgent.attachments} onremove={(i) => ideAgent.removeAttachment(i)} />
      </div>
    {/if}

    <div class="ia-composer-bar">
      <button
        bind:this={addContextBtnEl}
        type="button"
        class="icon-btn"
        aria-haspopup="menu"
        aria-label="Attach IDE context"
        title="Attach selection, file, diagnostic, or debug frame"
        onclick={() => showIdeContextMenu(addContextBtnEl, () => composerEl?.focus())}
        data-testid="ide-agent-add-context"
      >
        <Icon name="plus" size={13} />
      </button>

      {#if ideAgent.canAttachImages}
        <button
          type="button"
          class="icon-btn"
          aria-label={ideAgent.canAttachFiles ? 'Attach image or PDF' : 'Attach image'}
          title={ideAgent.canAttachFiles ? 'Attach image or PDF (or paste)' : 'Attach image (or paste)'}
          onclick={() => fileInputEl?.click()}
        >
          <Icon name="image" size={13} />
        </button>
        <input
          bind:this={fileInputEl}
          type="file"
          accept={ideAgent.canAttachFiles ? 'image/*,application/pdf' : 'image/*'}
          multiple
          hidden
          aria-label="Attach files"
          dir="ltr"
          onchange={(e) => {
            void ideAgent.addAttachments(e.currentTarget.files ?? []);
            e.currentTarget.value = '';
          }}
        />
      {/if}

      {#if !ideAgent.sessionId}
        <CouncilStagePicker
          part="toggle"
          provider={ideAgent.provider}
          model={ideAgent.model}
          effort={ideAgent.effort}
        />
      {:else if isRuntimeProvider(activeProvider)}
        <RuntimeAskAgent
          part="button"
          sessionId={ideAgent.sessionId}
          provider={activeProvider}
          access={ideAgent.runtimeAccessMode}
          disabled={closed}
        />
      {/if}

      <button
        bind:this={agentBtnEl}
        type="button"
        class="ia-agent-chip"
        aria-haspopup="dialog"
        aria-expanded={pickerOpen}
        aria-label="Agent and model: {agentChipLabel}"
        title="Switch provider, model, and reasoning effort"
        onpointerenter={() => ModelPicker.prefetch()}
        onclick={() => {
          ModelPicker.prefetch();
          pickerOpen = true;
        }}
        data-testid="ide-agent-provider"
      >
        {#if hasProviderIcon(activeProvider)}
          <ProviderIcon provider={activeProvider} size={12} />
        {:else}
          <Icon name="assistant" size={12} />
        {/if}
        <span class="ellipsis">{agentChipLabel}</span>
      </button>

      {#if isRuntimeProvider(activeProvider)}
        <RuntimeAccessChip
          mode={ideAgent.runtimeAccessMode}
          agent={harnessLabel(activeProvider)}
          cwd={ideAgent.resolveProjectCwd() ?? 'this folder'}
          disabled={closed}
          onpick={(m) => ideAgent.setRuntimeAccessMode(m)}
        />
      {/if}

      <button
        type="button"
        class="btn small ghost"
        class:ia-mode-plan={ideAgent.planMode}
        aria-pressed={ideAgent.planMode}
        title={ideAgent.planMode
          ? 'Plan mode: propose code blocks without modifying files directly'
          : 'Toggle Plan mode'}
        onclick={() => {
          ideAgent.planMode = !ideAgent.planMode;
        }}
        data-testid="ide-agent-mode"
      >
        Plan
      </button>

      <span class="grow"></span>

      {#if ideAgent.isWorking && !hasDraftPayload}
        <button
          type="button"
          class="btn small ia-stop"
          aria-label="Stop current agent turn"
          title="Stop turn"
          onclick={() => void ideAgent.stopTurn()}
          data-testid="ide-agent-stop"
        >
          <Icon name="stop" size={12} /> Stop
        </button>
      {:else}
        <button
          type="button"
          class="btn small ia-cta"
          disabled={!canSend}
          title={ideAgent.isWorking ? 'Queue follow-up (Enter)' : 'Send message (Enter)'}
          onclick={() => triggerSend()}
          data-testid="ide-agent-send"
        >
          <Icon name="send" size={12} /> {ideAgent.isWorking ? 'Queue' : 'Send'}
        </button>
      {/if}
    </div>
  </div>
</footer>

{#if pickerOpen && agentBtnEl && ModelPicker.component}
  <ModelPicker.component
    anchor={agentBtnEl}
    providers={ideAgent.availableProviders}
    provider={activeProvider}
    model={ideAgent.model}
    effectiveModel={ideAgent.effectiveModel}
    effort={ideAgent.effort}
    mode={ideAgent.interactionMode}
    onprovider={(p) => ideAgent.setProvider(p)}
    onmodel={(m) => {
      if (!ideAgent.sessionId) council.pick({ provider: ideAgent.provider, model: m, effort: ideAgent.effort });
      ideAgent.model = m;
    }}
    oneffort={supportsEffortSelection(activeProvider) ? (e) => (ideAgent.effort = e) : undefined}
    onmode={supportsInteractionMode(activeProvider) ? (m) => ideAgent.setInteractionMode(m) : undefined}
    onclose={() => (pickerOpen = false)}
  />
{/if}

<style>
  .ia-foot {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border-block-start: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
  }
  .ia-fresh {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ia-composer-box {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-block: 6px 4px;
    padding-inline: 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    transition:
      border-color var(--dur-fast) var(--ease-out),
      box-shadow var(--dur-fast) var(--ease-out);
  }
  .ia-composer-box:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 2px var(--accent-soft-strong);
  }
  .ia-textarea {
    inline-size: 100%;
    min-block-size: 48px;
    max-block-size: 160px;
    padding: 2px;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    line-height: 1.45;
    resize: vertical;
    unicode-bidi: plaintext;
    text-align: start;
  }
  .ia-textarea:focus {
    outline: none;
  }
  .ia-textarea::placeholder {
    color: var(--text-dim);
  }
  .ia-attach-row {
    padding-block: 2px;
  }
  .ia-composer-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    min-inline-size: 0;
  }
  .ia-agent-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    max-inline-size: 150px;
    block-size: 26px;
    padding-inline: 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .ia-agent-chip:hover {
    background: var(--hover);
    color: var(--text);
  }
  .ia-mode-plan {
    color: var(--warning);
    background: var(--warning-soft);
  }
  .ia-cta {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--accent-text);
  }
  .ia-cta:hover:not(:disabled) {
    background: var(--accent-soft-strong);
  }
  .ia-stop {
    background: var(--danger-soft);
    border-color: var(--danger);
    color: var(--danger);
  }
  @media (pointer: coarse) {
    .ia-agent-chip {
      min-block-size: var(--hit-min);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .ia-composer-box {
      transition: none;
    }
  }
</style>
