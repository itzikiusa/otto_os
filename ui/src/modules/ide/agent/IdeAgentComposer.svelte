<script lang="ts">
  // New-thread stage composer for `IdeAgentPanel.svelte` when `!ideAgent.sessionId`
  // (active runtime threads run directly inside `EmbeddedRuntimeThread.svelte`).
  // Provides 100% parity with `NewThreadStage.svelte` — unified `@` file/folder +
  // agent mentions (`RuntimeMentionMenu`), `/` slash commands, multi-agent Council,
  // image/PDF paste & attach, all 7+ harnesses via `ModelPickerPopover`,
  // `RuntimeAccessChip` + Plan mode — plus smart IDE context pills (`IdeContextBar`).
  import { untrack } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { enterSends } from '../../../lib/enterSends';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import {
    formatEffortShort,
    formatModelLabel,
    supportsEffortSelection,
    supportsInteractionMode,
  } from '../../../lib/providerModels';
  import {
    attachmentSupport,
    harnessLabel,
    isRuntimeProvider,
    takesFile,
  } from '../../../lib/providers';
  import {
    HOST_ONLY_SLASH_COMMANDS,
    INTERACTIVE_SLASH_COMMANDS,
    isImmediateSlashCommand,
    parseInteractiveSlashInput,
    type ComposerSlashCommand,
  } from '../../../lib/slashActions';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { ide } from '../ideStore.svelte';
  import { ideAgentEngine as ideAgent } from './ideAgentEngine.svelte';
  import { ideAgentState } from './ideAgentState.svelte';
  import IdeContextBar, { showIdeContextMenu } from './IdeContextBar.svelte';

  let { onsent }: { onsent?: () => void } = $props();

  void ideAgentState.ensureBridge();
  const bridge = $derived(ideAgentState.runtimeBridge);
  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));

  let composerBoxEl = $state<HTMLDivElement | null>(null);
  let addContextBtnEl = $state<HTMLButtonElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let agentBtnEl = $state<HTMLButtonElement | null>(null);
  let fileInputEl = $state<HTMLInputElement | null>(null);
  let mentionEl = $state<{ onKey: (e: KeyboardEvent) => boolean } | null>(null);
  let pickerOpen = $state(false);
  let cmdIdx = $state(0);
  let cmdDismissed = $state(false);
  const slashPopId = $props.id();

  $effect(() =>
    pickerOpen && bridge
      ? untrack(() =>
          bridge.council.watchModifiers({
            provider: ideAgent.provider,
            model: ideAgent.model,
            effort: ideAgent.effort,
          }),
        )
      : undefined,
  );

  const activeProvider = $derived(ideAgent.provider);
  const agentChipLabel = $derived.by(() => {
    if (bridge?.council.on) return bridge.councilAgentLabel(auth.meta);
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
      case 'open-mode-picker':
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
      case 'fresh-context':
        ideAgent.freshContext = true;
        ideAgent.composerText = '';
        return true;
      case 'new-thread':
        ideAgent.startNewChat();
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
    if (e.isComposing || mentionEl?.onKey(e)) return;
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
    if (e.key === 'Tab' && e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      ideAgent.planMode = !ideAgent.planMode;
      return;
    }
    if (enterSends(e)) {
      e.preventDefault();
      if (handleSlashSubmit()) return;
      if (canSend) triggerSend();
    }
  }

  function onPaste(e: ClipboardEvent): void {
    if (!attachmentSupport(activeProvider).images) return;
    const files = [...(e.clipboardData?.files ?? [])].filter((f) => takesFile(activeProvider, f));
    if (files.length === 0) return;
    e.preventDefault();
    void ideAgent.addAttachments(files);
  }
</script>

<footer class="ia-foot">
  {#if ideAgent.freshContext}
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

  <IdeContextBar onfocuscomposer={() => composerEl?.focus()} />

  <div class="ia-composer-box" bind:this={composerBoxEl}>
    {#if slashSuggestions.length > 0 && bridge}
      <bridge.StageSlashPopup
        id={slashPopId}
        items={slashSuggestions}
        active={cmdIdx}
        onaccept={acceptSlashCmd}
        onhover={(i) => (cmdIdx = i)}
      />
    {/if}

    {#if bridge}
      <bridge.RuntimeMentionMenu
        bind:this={mentionEl}
        text={ideAgent.composerText}
        textareaEl={composerEl}
        anchorEl={composerBoxEl}
        cwd={ideAgent.resolveProjectCwd()}
        allowAgents={true}
        agentSectionLabel={bridge.council.on ? 'Add to Council' : 'Switch agent'}
        onpickfile={(picked) => {
          ideAgent.composerText = picked.nextText;
          ideAgent.attachMentionedPath({ path: picked.path, display: picked.display, isDir: picked.isDir });
          requestAnimationFrame(() => {
            composerEl?.focus();
            composerEl?.setSelectionRange(picked.nextCaret, picked.nextCaret);
          });
        }}
        onpickagent={(picked) => {
          ideAgent.composerText = picked.nextText;
          if (bridge.council.on) {
            bridge.council.pick({
              provider: picked.option.provider,
              model: picked.option.model ?? null,
              effort: null,
            });
          } else {
            ideAgent.setProvider(picked.option.provider);
            ideAgent.model = picked.option.model ?? null;
          }
          requestAnimationFrame(() => {
            composerEl?.focus();
            composerEl?.setSelectionRange(picked.nextCaret, picked.nextCaret);
          });
        }}
        ondrilldir={(nextText, nextCaret) => {
          ideAgent.composerText = nextText;
          requestAnimationFrame(() => {
            composerEl?.focus();
            composerEl?.setSelectionRange(nextCaret, nextCaret);
          });
        }}
      />

      <bridge.CouncilStagePicker
        part="seats"
        provider={ideAgent.provider}
        model={ideAgent.model}
        effort={ideAgent.effort}
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
      placeholder="Ask anything. @ files or agents, / commands"
      onkeydown={onComposerKeydown}
      onpaste={onPaste}
      data-testid="ide-agent-composer"
    ></textarea>

    {#if ideAgent.attachments.length > 0 && bridge}
      <div class="ia-attach-row">
        <bridge.RuntimeComposerImages
          images={ideAgent.attachments}
          onremove={(i) => ideAgent.removeAttachment(i)}
        />
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
          data-testid="ide-agent-attach-image"
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

      {#if bridge}
        <bridge.CouncilStagePicker
          part="toggle"
          provider={ideAgent.provider}
          model={ideAgent.model}
          effort={ideAgent.effort}
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

      {#if isRuntimeProvider(activeProvider) && bridge}
        <bridge.RuntimeAccessChip
          mode={ideAgent.effectiveRuntimeAccessMode}
          agent={harnessLabel(activeProvider)}
          cwd={ideAgent.resolveProjectCwd() ?? 'this folder'}
          onpick={(m) => ideAgent.setRuntimeAccessMode(m)}
        />
      {/if}

      <button
        type="button"
        class="btn small ghost"
        class:ia-mode-plan={ideAgent.planMode}
        aria-pressed={ideAgent.planMode}
        title={ideAgent.planMode ? 'Plan mode: propose code blocks without modifying files directly' : 'Toggle Plan mode (⇧⇥)'}
        onclick={() => {
          ideAgent.planMode = !ideAgent.planMode;
        }}
        data-testid="ide-agent-mode"
      >
        Plan
      </button>

      <span class="grow"></span>

      <button
        type="button"
        class="btn small ia-cta"
        disabled={!canSend}
        title="Send message (Enter)"
        onclick={() => triggerSend()}
        data-testid="ide-agent-send"
      >
        <Icon name="send" size={12} /> Send
      </button>
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
      bridge?.council.pick({ provider: ideAgent.provider, model: m, effort: ideAgent.effort });
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
