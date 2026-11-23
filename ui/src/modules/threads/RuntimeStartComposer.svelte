<script lang="ts">
  // The ONE start composer: Threads' new-thread stage and every embed's
  // draft state (a pull request's agent, a connection's diagnosis) render
  // it, so a person picks the agent the same way everywhere — attach on the
  // start, one agent chip (harness · model · effort, ModelPickerPopover), the
  // access chip where the host names a mode, a round send on the end, `@`
  // file/folder & agent completion, the `/` popup and (Threads) the Council
  // picker. The host does the starting (`onlaunch`); pure rules live in
  // startComposer.ts.
  import { untrack, type Snippet } from 'svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import type { RuntimeAccessMode } from '../../lib/api/types';
  import { EmojiAssistView } from '../../lib/components/emojiAssistLazy.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { enterSends } from '../../lib/enterSends';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import {
    formatEffortShort,
    formatModelLabel,
    nextInteractionModeToggle,
    supportsEffortSelection,
    supportsInteractionMode,
    type EffortLevel,
    type InteractionMode,
  } from '../../lib/providerModels';
  import { agentProviders, defaultAgentProvider, orderThreadHarnesses, providerReadiness, runtimeBaselineCaps } from '../../lib/providers';
  import { isImmediateSlashCommand, parseInteractiveSlashInput, type ComposerSlashCommand } from '../../lib/slashActions';
  import { auth } from '../../lib/stores/auth.svelte';
  import { composerViewState, getTransientComposerDraft, saveTransientComposerDraft } from '../../lib/stores/runtimeDraftCursor';
  import { toasts } from '../../lib/toast.svelte';
  import { STAGE_SLASH_COMMANDS } from './newThreadMenus';
  import { addComposerContext, assembleComposerMessage, fileMentionContext, type ComposerContext } from './runtime/composerContext';
  import CouncilStagePicker from './runtime/CouncilStagePicker.svelte';
  import { council, councilAgentLabel } from './runtime/councilStore.svelte';
  import { handleSlashMenuKey } from './runtime/runtimeComposer';
  import RuntimeAccessChip from './runtime/RuntimeAccessChip.svelte';
  import RuntimeContextChips from './runtime/RuntimeContextChips.svelte';
  import type { PickedAgentMention, PickedFileMention } from './runtime/RuntimeMentionMenu.svelte';
  import { terminalTakesEffort } from '../../lib/providerRegistry';
  import { isRuntimeProvider } from './runtime/runtimeThreads.svelte';
  import StageAttach from './runtime/StageAttach.svelte';
  import { StageAttachments } from './runtime/stageAttachments.svelte';
  import { capsOf } from './runtime/attachmentSupport';
  import { readStartPick, writeStartPick } from '../../lib/startPick';
  import { initialProvider, startHarnesses, type StartLaunch, type StartMeta } from './startComposer';
  import { MentionMenuView, StageSlashView } from './threadLazy.svelte';
  import { buildModeMenu } from './threadPickerMenus';
  import { harnessLabel } from './threadsHelpers';

  const ModelPicker = lazyComponent(() => import('../../lib/components/ModelPickerPopover.svelte'));

  interface Props {
    /** The folder the thread works in ('' = none yet): `@` completion and the file picker. */
    cwd: string;
    /** The folder a full-access confirm names when `cwd` is empty. */
    pickerStart?: string;
    /** The unsent text survives a reload under this key. */
    draftKey: string;
    /** A narrow column (an embed): denser, chips shrink to icons. */
    compact?: boolean;
    /** Only agents that run a structured thread and are installed (embeds). */
    runtimeOnly?: boolean;
    /** Ask several agents at once (Threads). */
    allowCouncil?: boolean;
    /** The agent it starts on (else the person's default). */
    defaultProvider?: string;
    /** Remember the last agent · model · effort under this scope (Threads:
     *  per workspace; an embed: per host kind). Access is never remembered. */
    rememberScope?: string;
    /** The access chip's mode (Threads: the person's pick, else the saved
     *  default); null hides the chip. Shown for a structured agent only. */
    accessMode?: RuntimeAccessMode | null;
    onaccess?: (mode: RuntimeAccessMode) => void;
    placeholder?: string;
    inputLabel?: string;
    /** The host's error under the box (a folder that can't be used…). */
    error?: string;
    onlaunch: (req: StartLaunch) => Promise<boolean>;
    /** Escape in an empty box; true = the host handled it. */
    onescape?: () => boolean;
    below?: Snippet<[StartMeta]>;
    /** Text to open with, replacing the box ("Continue with this answer"); `onprefilled` takes it. */
    prefill?: string | null;
    onprefilled?: () => void;
  }
  let {
    cwd, pickerStart = '', draftKey, compact = false, runtimeOnly = false, allowCouncil = true, defaultProvider, rememberScope,
    accessMode = null, onaccess, placeholder, inputLabel = 'Opening prompt for new thread', error = '', onlaunch, onescape, below, prefill = null, onprefilled,
  }: Props = $props();

  const ready = (p: string): boolean => providerReadiness(p).available;
  const harnesses = $derived(startHarnesses(orderThreadHarnesses(agentProviders()), runtimeOnly, (p) => isRuntimeProvider(p, auth.meta), ready));
  const councilOn = $derived(allowCouncil && council.on);
  const councilReady = $derived(allowCouncil && council.ready);

  let provider = $state('');
  let selectedModel = $state<string | null>(null);
  let selectedEffort = $state<EffortLevel | null>(null);
  let interactionMode = $state<InteractionMode>('full');
  let prompt = $state('');
  let stageContext = $state<ComposerContext[]>([]);
  let ownError = $state('');
  let busy = $state(false);
  let composerBoxEl = $state<HTMLDivElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let agentEl = $state<HTMLButtonElement | null>(null);
  let mentionEl = $state<{ onKey: (e: KeyboardEvent) => boolean } | null>(null);
  let emojiEl = $state<{ onKey: (e: KeyboardEvent) => boolean } | null>(null);

  const runtime = $derived(isRuntimeProvider(provider, auth.meta));
  // A runtime thread's attachments, plan mode and effort levels come from what
  // its agent reports (`MetaResp.runtime_capabilities`), never from its name.
  const stageCaps = $derived(capsOf(null, runtimeBaselineCaps(provider, auth.meta)));
  const files = new StageAttachments(() => stageCaps, () => runtime && !councilOn);
  const modeSupported = $derived(supportsInteractionMode(provider, auth.meta) && !runtime);
  const planSupported = $derived(runtime && !!stageCaps.plan);
  const takesEffort = $derived(runtime ? (stageCaps.efforts?.length ?? 0) > 0 : terminalTakesEffort(provider, auth.meta, supportsEffortSelection(provider, auth.meta)));
  const planShown = $derived(interactionMode === 'plan' && (modeSupported || planSupported));
  const viewStateArg = $derived({ key: draftKey, hasText: prompt.length > 0 });
  let hydratedKey = '';
  $effect(() => {
    const key = draftKey;
    if (key !== hydratedKey) {
      hydratedKey = key;
      const saved = getTransientComposerDraft(key);
      if (saved && !untrack(() => prompt)) prompt = saved;
    }
  });
  $effect(() => {
    const key = draftKey;
    const val = prompt;
    if (key === hydratedKey) saveTransientComposerDraft(key, val);
  });
  const agentLabel = $derived.by(() => {
    const meta = auth.meta;
    if (councilOn) return councilAgentLabel(meta);
    const model = selectedModel ? formatModelLabel(provider, selectedModel, meta) : harnessLabel(provider);
    return takesEffort && selectedEffort ? `${model} · ${formatEffortShort(selectedEffort)}` : model;
  });

  $effect(() => {
    if (!provider || (harnesses.length > 0 && !harnesses.includes(provider))) {
      const def = defaultProvider ?? defaultAgentProvider();
      provider = initialProvider(harnesses, def, ready) || (runtimeOnly ? '' : def);
    }
  });

  // The last pick for this scope, once the offered agents are known; then
  // every change is kept (an agent no longer installed falls back above).
  const pickStore = (): Storage | null => {
    try {
      return typeof localStorage === 'undefined' ? null : localStorage;
    } catch {
      return null;
    }
  };
  let restoredScope = '';
  $effect(() => {
    const scope = rememberScope;
    const offered = harnesses;
    if (!scope || scope === restoredScope || offered.length === 0) return;
    restoredScope = scope;
    const pick = readStartPick(pickStore(), scope, offered, ready);
    if (pick) {
      provider = pick.provider;
      selectedModel = pick.model;
      selectedEffort = pick.effort as EffortLevel | null;
    }
  });
  $effect(() => {
    const scope = rememberScope;
    const pick = { provider, model: selectedModel, effort: selectedEffort };
    if (scope && scope === restoredScope && pick.provider) writeStartPick(pickStore(), scope, pick);
  });

  function selectProvider(next: string): void {
    if (provider === next) return;
    provider = next;
    selectedModel = null;
    selectedEffort = null;
  }

  const SUPPORTS_FIELD_SIZING =
    typeof CSS !== 'undefined' && typeof CSS.supports === 'function' && CSS.supports('field-sizing', 'content');

  function autoGrow(el: HTMLTextAreaElement): void {
    if (SUPPORTS_FIELD_SIZING) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(240, Math.max(compact ? 40 : 56, el.scrollHeight))}px`;
  }

  /** Focus the box (the caret at the end when asked). */
  export function focus(caretToEnd = false): void {
    queueMicrotask(() => {
      if (!composerEl) return;
      autoGrow(composerEl);
      composerEl.focus();
      if (caretToEnd) composerEl.setSelectionRange(prompt.length, prompt.length);
    });
  }

  // A prefilled draft lands once, the caret after it; nothing is sent.
  $effect(() => {
    const t = prefill;
    if (t == null) return;
    untrack(() => {
      prompt = t;
      focus(true);
      onprefilled?.();
    });
  });

  let pickerOpen = $state(false);
  function openModelMenu(): void {
    if (agentEl) pickerOpen = true;
  }
  const openEffortMenu = openModelMenu;
  $effect(() => (pickerOpen && allowCouncil ? untrack(() => council.watchModifiers({ provider, model: selectedModel, effort: selectedEffort })) : undefined));

  function openModeMenu(anchor?: HTMLElement): void {
    const target = anchor ?? agentEl;
    if (!target || !modeSupported) return;
    ctxMenu.showAt(target, buildModeMenu(interactionMode, (m) => (interactionMode = m)), { align: 'end' });
  }

  /** Send what is in the box again (the host's Retry after a failed start). */
  export function retry(): Promise<void> {
    return launch(true);
  }

  /** The text in the box (the host names a branch from it). */
  export function draftText(): string {
    return prompt;
  }

  /** Start with a message the host wrote (a suggestion, a menu ask) and the
   *  composer's picks; `access` overrides the chip for this start. */
  export function launchText(text: string, access?: RuntimeAccessMode): Promise<boolean> {
    return run({ withPrompt: true, message: text, titleSeed: text, raw: true, access, background: false });
  }

  async function run(o: { withPrompt: boolean; message: string; titleSeed: string; raw: boolean; access?: RuntimeAccessMode; background: boolean }): Promise<boolean> {
    if (busy) return false;
    const readiness = providerReadiness(provider);
    if (!provider || !readiness.available) {
      toasts.error(`Couldn’t start ${harnessLabel(provider)}`, readiness.message);
      return false;
    }
    ownError = '';
    busy = true;
    try {
      const ok = await onlaunch({
        provider,
        runtime,
        council: o.withPrompt && councilReady,
        background: o.background,
        model: selectedModel,
        effort: takesEffort ? selectedEffort : null,
        interactionMode,
        plan: planSupported && interactionMode === 'plan',
        withPrompt: o.withPrompt,
        message: o.message,
        titleSeed: o.titleSeed,
        attachments: o.withPrompt ? files.list : [],
        accessMode: o.access ?? (runtime ? accessMode : null),
        raw: o.raw,
      });
      if (ok && !o.raw) {
        saveTransientComposerDraft(draftKey, '');
        prompt = '';
        stageContext = [];
        files.picked = [];
      }
      return ok;
    } finally {
      busy = false;
    }
  }

  async function launch(withPrompt: boolean, overridePrompt?: string, background = false): Promise<void> {
    if (busy) return;
    const rawPrompt = overridePrompt ?? (withPrompt ? prompt.trim() : '');
    if (withPrompt && overridePrompt === undefined) {
      const slash = parseInteractiveSlashInput(rawPrompt);
      if (slash) {
        prompt = '';
        switch (slash.kind) {
          case 'open-model-picker':
            queueMicrotask(openModelMenu);
            return;
          case 'set-model':
            selectedModel = slash.model;
            return;
          case 'open-effort-picker':
            queueMicrotask(openEffortMenu);
            return;
          case 'set-effort':
            selectedEffort = slash.effort;
            return;
          case 'open-mode-picker':
            queueMicrotask(() => openModeMenu());
            return;
          case 'set-mode':
            interactionMode = slash.mode;
            if (slash.prompt) await launch(true, slash.prompt);
            return;
          case 'send-prompt':
            await launch(true, slash.prompt);
            return;
          default:
            break;
        }
      }
    }
    const rawTrimmed = rawPrompt.trim();
    if (withPrompt && !rawTrimmed && files.list.length === 0 && stageContext.length === 0) {
      composerEl?.focus();
      return;
    }
    if (withPrompt && councilOn && !councilReady) {
      ownError = 'Pick at least two agents to ask, or turn Ask several off.';
      return;
    }
    const message = withPrompt ? assembleComposerMessage(rawTrimmed, stageContext) : '';
    await run({ withPrompt, message, titleSeed: rawTrimmed || stageContext[0]?.label || '', raw: false, background });
  }

  let cmdIdx = $state(0);
  let cmdDismissed = $state(false);
  const slashPopId = $props.id();
  const cmdPrefix = $derived.by(() => {
    const m = /^\/([\w:-]*)$/.exec(prompt);
    return m ? m[1].toLowerCase() : null;
  });
  const slashSuggestions = $derived.by(() => {
    if (cmdPrefix == null || cmdDismissed) return [] as ComposerSlashCommand[];
    return STAGE_SLASH_COMMANDS.filter((c) => c.name.toLowerCase().includes(cmdPrefix));
  });
  const cmdOpen = $derived(slashSuggestions.length > 0);
  $effect(() => {
    void slashSuggestions.length;
    cmdIdx = 0;
  });
  $effect(() => {
    void prompt;
    cmdDismissed = false;
  });

  function acceptStageCmd(c: ComposerSlashCommand): void {
    if (isImmediateSlashCommand(c.name)) {
      prompt = `/${c.name}`;
      cmdDismissed = true;
      void launch(true);
      return;
    }
    prompt = `/${c.name} `;
    cmdDismissed = true;
    focus(true);
  }

  function handleMentionFile(picked: PickedFileMention): void {
    prompt = picked.nextText;
    stageContext = addComposerContext(stageContext, fileMentionContext({ path: picked.path, label: picked.display, isDir: picked.isDir }));
    queueMicrotask(() => composerEl && autoGrow(composerEl));
  }

  function handleMentionAgent(picked: PickedAgentMention): void {
    prompt = picked.nextText;
    const o = picked.option;
    if (councilOn) {
      council.pick({ provider: o.provider, model: o.model ?? null, effort: null });
    } else if (harnesses.includes(o.provider)) {
      provider = o.provider;
      selectedModel = o.model || null;
      if (!supportsEffortSelection(o.provider, auth.meta)) selectedEffort = null;
    }
  }

  function onComposerKeydown(e: KeyboardEvent): void {
    if (e.isComposing || mentionEl?.onKey(e) || emojiEl?.onKey(e)) return;
    if (e.key === 'Tab' && e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && !cmdOpen) {
      if (modeSupported) {
        e.preventDefault();
        interactionMode = nextInteractionModeToggle(interactionMode);
        return;
      }
      if (planSupported) {
        e.preventDefault();
        interactionMode = interactionMode === 'plan' ? 'full' : 'plan';
        return;
      }
    }
    if (cmdOpen && handleSlashMenuKey(e, slashSuggestions, cmdIdx, (i) => (cmdIdx = i), () => (cmdDismissed = true), acceptStageCmd)) {
      return;
    }
    if (e.key === 'Escape' && !prompt && onescape?.()) {
      e.preventDefault();
      return;
    }
    if (enterSends(e)) {
      e.preventDefault();
      void launch(!((e.metaKey || e.ctrlKey) && !prompt.trim() && stageContext.length === 0 && files.list.length === 0), undefined, e.metaKey || e.ctrlKey);
    }
  }

  let picker = $state<{ open(): void } | null>(null);
  /** A file or folder: `@` at the end, and the mention typeahead takes over. */
  function insertMention(): void {
    prompt = prompt && !/\s$/.test(prompt) ? `${prompt} @` : `${prompt}@`;
    MentionMenuView.prefetch();
    focus(true);
  }
  function openAttach(e: MouseEvent): void {
    const can = files.can;
    if (!can.images) return insertMention();
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: 'File or folder', icon: 'file', hint: '@', action: insertMention },
      { label: can.files ? 'Image or PDF…' : 'Image…', icon: 'image', action: () => picker?.open() },
    ]);
  }

  const shownError = $derived(ownError || error);
  // A council's seats are read-only by design, and a terminal agent takes the
  // meta row's mode menu instead.
  const accessShown = $derived(!!accessMode && runtime && !councilOn);
</script>

<div class="nt-composer" class:compact class:plan={planShown} bind:this={composerBoxEl} data-testid="start-composer">
  {#if cmdOpen && StageSlashView.component}
    <StageSlashView.component
      id={slashPopId}
      items={slashSuggestions}
      active={cmdIdx}
      onaccept={acceptStageCmd}
      onhover={(i) => (cmdIdx = i)}
    />
  {/if}
  {#if prompt.includes('@') && MentionMenuView.component}
    <MentionMenuView.component
      bind:this={mentionEl}
      text={prompt}
      cwd={cwd || null}
      textareaEl={composerEl}
      anchorEl={composerBoxEl}
      allowAgents={true}
      agentSectionLabel={councilOn ? 'Add a model to compare' : 'Switch agent'}
      onpickfile={handleMentionFile}
      onpickagent={handleMentionAgent}
      ondrilldir={(nextText) => { prompt = nextText; }}
    />
  {/if}
  {#if EmojiAssistView.component}<EmojiAssistView.component bind:this={emojiEl} textareaEl={composerEl} />{/if}
  <RuntimeContextChips
    context={stageContext}
    onremove={(id) => (stageContext = stageContext.filter((c) => c.id !== id))}
    onfocus={() => focus()}
  />
  <textarea
    bind:this={composerEl}
    bind:value={prompt}
    use:composerViewState={viewStateArg}
    dir="auto"
    rows={2}
    spellcheck="false"
    {...{ autocorrect: 'off' }}
    autocapitalize="off"
    autocomplete="off"
    placeholder={planShown
      ? 'Describe what to explore and plan. @ files, / commands, ⇧⇥ leaves Plan'
      : (placeholder ?? 'Ask anything. @ files or agents, / commands')}
    aria-label={inputLabel}
    aria-autocomplete="list"
    aria-controls={cmdOpen ? `${slashPopId}-list` : undefined}
    aria-activedescendant={cmdOpen ? `${slashPopId}-opt-${cmdIdx}` : undefined}
    data-testid="new-thread-input"
    disabled={busy}
    onfocus={() => { MentionMenuView.prefetch(); StageSlashView.prefetch(); }}
    oninput={(e) => {
      if (e.currentTarget.value.includes('@')) MentionMenuView.prefetch();
      if (e.currentTarget.value.startsWith('/')) StageSlashView.prefetch();
      autoGrow(e.currentTarget);
    }}
    onkeydown={onComposerKeydown}
    onpaste={(e) => files.paste(e)}
  ></textarea>
  <StageAttach part="list" {files} />

  {#if shownError}
    <p class="nt-error" use:announceShown={{ text: shownError, assertive: true }}>{shownError}</p>
  {/if}

  {#if allowCouncil}
    <CouncilStagePicker part="seats" {provider} model={selectedModel} effort={selectedEffort} />
  {/if}
  <div class="nt-foot">
    <!-- One attach control: a file or folder goes in as an @ mention (the
         typeahead browses folders too); images / PDFs where the agent takes them. -->
    <button
      type="button"
      class="icon-btn"
      aria-haspopup={files.can.images ? 'menu' : undefined}
      aria-label="Attach"
      title={files.can.images ? 'Attach a file (@), an image or a PDF' : 'Attach a file (@)'}
      data-testid="new-thread-attach"
      disabled={busy}
      onclick={openAttach}
    >
      <Icon name="plus" size={16} />
    </button>
    <StageAttach bind:this={picker} part="picker" {files} />
    <span class="nt-spacer"></span>
    {#if allowCouncil}
      <CouncilStagePicker part="toggle" {provider} model={selectedModel} effort={selectedEffort} disabled={busy} />
    {/if}
    <button
      bind:this={agentEl}
      type="button"
      class="nt-agent"
      aria-haspopup="menu"
      aria-label="Agent: {harnessLabel(provider)}, {agentLabel}"
      title="Harness, model and effort (/model · /effort)"
      data-testid="picker-agent"
      onpointerenter={() => ModelPicker.prefetch()}
      disabled={busy}
      aria-expanded={pickerOpen}
      onclick={openModelMenu}
    >
      {#if hasProviderIcon(provider)}
        <ProviderIcon {provider} size={14} />
      {:else}
        <Icon name="terminal" size={14} />
      {/if}
      <span class="ellipsis">{agentLabel}</span>
    </button>
    {#if accessShown}
      <span class="nt-access">
        <RuntimeAccessChip mode={accessMode!} agent={harnessLabel(provider)} cwd={cwd || pickerStart} disabled={busy} onpick={(m) => onaccess?.(m)} />
      </span>
    {/if}
    <button
      type="button"
      class="nt-send"
      class:ask={councilReady}
      aria-label={busy ? 'Starting thread' : councilReady ? `Ask ${council.seats.length} agents` : 'Start thread'}
      title={councilReady ? 'Ask every agent (↵) — ⌘↵ starts them in the background' : 'Start thread (↵) — ⌘↵ with no text opens an empty one'}
      data-testid="new-thread-send-btn"
      disabled={busy || (!prompt.trim() && stageContext.length === 0 && files.list.length === 0)}
      onclick={() => void launch(true)}
    >
      {#if busy}
        <span class="spinner" aria-hidden="true"></span>
      {:else if councilReady}
        Ask {council.seats.length}
      {:else}
        <Icon name="arrowUp" size={16} />
      {/if}
    </button>
  </div>
</div>

{@render below?.({
  modeSupported,
  accessShown,
  planSupported,
  interactionMode,
  setMode: (m) => (interactionMode = m),
  openModeMenu,
})}

{#if pickerOpen && agentEl && ModelPicker.component}
  <ModelPicker.component
    anchor={agentEl}
    providers={harnesses}
    {provider}
    model={selectedModel}
    effort={selectedEffort}
    mode={interactionMode}
    onprovider={selectProvider}
    onmodel={(m) => { if (allowCouncil) council.pick({ provider, model: m, effort: selectedEffort }); selectedModel = m; }}
    oneffort={takesEffort ? (e) => (selectedEffort = e) : undefined}
    effortLevels={runtime ? (stageCaps.efforts ?? []) : undefined}
    onmode={modeSupported ? (m) => (interactionMode = m) : undefined}
    onclose={() => (pickerOpen = false)}
  />
{/if}

<style>
  .nt-composer {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 14px 10px;
    border-radius: var(--radius-l);
    border: 1px solid var(--border);
    background: var(--surface);
    box-shadow: var(--shadow-card);
  }
  .nt-composer.compact {
    container-type: inline-size;
    gap: 6px;
    padding: 10px 10px 8px;
    border-radius: var(--radius-m);
  }
  .nt-composer:focus-within {
    border-color: var(--border-strong);
  }
  .nt-composer.plan {
    border-color: color-mix(in srgb, var(--warning) 42%, var(--border));
  }
  .nt-composer textarea {
    inline-size: 100%;
    min-block-size: 56px;
    max-block-size: 240px;
    resize: none;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-l);
    line-height: 1.6;
    unicode-bidi: plaintext;
    text-align: start;
    outline: none;
    field-sizing: content;
  }
  .nt-composer.compact textarea {
    min-block-size: 40px;
    max-block-size: 180px;
  }
  .nt-error {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  .nt-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    min-inline-size: 0;
  }
  .compact .nt-foot {
    gap: 4px;
  }
  .nt-spacer {
    flex: 1;
  }
  .nt-agent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    max-inline-size: 260px;
    block-size: 30px;
    padding-inline: 8px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .compact .nt-agent {
    flex: 0 1 auto;
    padding-inline: 6px;
    font-size: var(--fs-s);
  }
  .nt-agent:hover:not(:disabled) {
    background: var(--hover);
  }
  .nt-access {
    display: inline-flex;
    flex: 0 1 auto;
    min-inline-size: 0;
  }
  /* A dock at its narrowest (280 px): the access chip keeps its icon (its
     name stays in the aria-label and tooltip), the agent chip its model. */
  @container (max-width: 340px) {
    .nt-access :global(.ellipsis) {
      display: none;
    }
  }
  .nt-send {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    inline-size: 32px;
    block-size: 32px;
    border: 0;
    border-radius: 50%;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    cursor: pointer;
  }
  .nt-send:disabled {
    background: var(--surface-3);
    color: var(--text-dim);
    cursor: default;
  }
  .nt-send.ask {
    inline-size: auto;
    padding-inline: 12px;
    border-radius: 999px;
    font-weight: 600;
  }
  .nt-send .spinner {
    --spinner-size: 14px;
  }
  @media (max-width: 1024px) {
    .nt-composer.compact textarea {
      font-size: 16px; /* ui-guards: allow — 16px stops iOS zoom-on-focus */
    }
  }
</style>
