<script lang="ts">
  // Contextual inline AI edit bar (`⌘I`): anchored at the top of the active
  // code editor for quick selection edits across the full Threads harness &
  // model catalog, with inline unified diff preview, Accept / Discard, or
  // 1-click handoff into the side-by-side Agent panel.
  import { announceShown } from '../../lib/announce.svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import Badge from '../../lib/components/Badge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { focusOnMount } from '../../lib/focusOnMount';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { formatEffortShort, formatModelLabel, normalizeEffort, supportsEffortSelection } from '../../lib/providerModels';
  import { harnessLabel } from '../../lib/providers';
  import { loadRuntimeMentionMenu } from '../../lib/runtimeThreadPort';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ide } from './ideStore.svelte';
  import { ideAgentEngine as ideAgent } from './agent/ideAgentEngine.svelte';

  const ModelPicker = lazyComponent(() => import('../../lib/components/ModelPickerPopover.svelte'));
  const MentionMenu = lazyComponent(loadRuntimeMentionMenu);

  $effect(() => {
    if (ideAgent.inlinePrompt?.open) MentionMenu.prefetch();
  });

  const targetTab = $derived.by(() => {
    const p = ideAgent.inlinePrompt?.path;
    return p ? (ide.tabs.find((t) => t.path === p) ?? ide.activeTab) : ide.activeTab;
  });
  const readOnly = $derived(!ide.canEditCollab || Boolean(targetTab?.readOnly));
  let pickerOpen = $state(false);
  let providerBtnEl = $state<HTMLButtonElement | null>(null);
  let inlineAnchorEl = $state<HTMLDivElement | null>(null);
  let inlineFieldEl = $state<HTMLTextAreaElement | null>(null);
  let mentionEl = $state<{ onKey: (e: KeyboardEvent) => boolean } | null>(null);

  const inlineAgentLabel = $derived.by(() => {
    const st = ideAgent.inlinePrompt;
    if (!st) return '';
    const base = st.model ? formatModelLabel(st.provider, st.model, auth.meta) : harnessLabel(st.provider);
    const eff = normalizeEffort(st.effort);
    return supportsEffortSelection(st.provider) && eff ? `${base} · ${formatEffortShort(eff)}` : base;
  });

  function restoreEditorFocus(targetPath: string | null): void {
    requestAnimationFrame(() => {
      const handle = targetPath ? ide.editorHandleForPath(ide.resolvePath(targetPath)) : null;
      if (handle) {
        handle.focus();
        return;
      }
      document.querySelector<HTMLElement>('.ide-editor-area .cm-content')?.focus();
    });
  }

  function closeInlineBar(): void {
    const targetPath = ideAgent.inlinePrompt?.path ?? null;
    pickerOpen = false;
    ideAgent.closeInlineEdit();
    restoreEditorFocus(targetPath);
  }

  async function acceptInlineChange(): Promise<void> {
    const targetPath = ideAgent.inlinePrompt?.path ?? null;
    const applied = await ideAgent.acceptInlineProposal();
    if (applied) restoreEditorFocus(targetPath);
  }

  function onInputKeydown(e: KeyboardEvent): void {
    const st = ideAgent.inlinePrompt;
    if (!st || e.isComposing || mentionEl?.onKey(e)) return;
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      if (!st.busy && st.query.trim()) void ideAgent.submitInlineEdit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      closeInlineBar();
    }
  }

  function handoffToChat(): void {
    pickerOpen = false;
    ideAgent.handoffInlineToChat();
    let tries = 0;
    const step = (): void => {
      const el = document.querySelector<HTMLElement>('[data-testid="ide-agent-composer"]');
      if (el) el.focus();
      else if (++tries < 15) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }
</script>

{#if ideAgent.inlinePrompt?.open}
  {@const st = ideAgent.inlinePrompt}
  <div class="ip-bar" role="region" aria-label="Inline AI edit" data-testid="ide-inline-prompt-bar">
    <div class="ip-top">
      <span class="ip-spark" aria-hidden="true"><Icon name="sparkle" size={13} /></span>
      <span class="ip-target" dir="ltr">
        <Badge
          tone="accent"
          variant="outline"
          label="@{st.relPath}:L{st.startLine}-L{st.endLine}"
          title="{st.path}:{st.startLine}-{st.endLine}"
        />
      </span>

      {#key `${st.path}:${st.startLine}:${st.endLine}`}
        <div class="ip-field-wrap" bind:this={inlineAnchorEl}>
          {#if MentionMenu.component}
            <MentionMenu.component
              bind:this={mentionEl}
              text={st.query}
              textareaEl={inlineFieldEl}
              anchorEl={inlineAnchorEl}
              cwd={ideAgent.resolveProjectCwd()}
              allowAgents={false}
              onpickfile={(picked) => {
                st.query = picked.nextText;
                requestAnimationFrame(() => {
                  inlineFieldEl?.focus();
                  inlineFieldEl?.setSelectionRange(picked.nextCaret, picked.nextCaret);
                });
              }}
              ondrilldir={(nextText, nextCaret) => {
                st.query = nextText;
                requestAnimationFrame(() => {
                  inlineFieldEl?.focus();
                  inlineFieldEl?.setSelectionRange(nextCaret, nextCaret);
                });
              }}
            />
          {/if}
          <textarea
            bind:this={inlineFieldEl}
            class="input ip-field"
            rows={1}
            dir="auto"
            aria-label="Describe inline code change"
            placeholder="Ask Agent to edit or refactor these lines (@file to reference)…"
            use:focusOnMount={{ select: true }}
            bind:value={st.query}
            disabled={st.busy}
            onkeydown={onInputKeydown}
            data-testid="ide-inline-prompt-input"
          ></textarea>
        </div>
      {/key}

      <div class="ip-actions">
        <button
          bind:this={providerBtnEl}
          type="button"
          class="btn small ghost ip-model-btn"
          aria-haspopup="dialog"
          aria-expanded={pickerOpen}
          aria-label="Inline edit model: {inlineAgentLabel}"
          title="Switch provider, model, and reasoning effort"
          disabled={st.busy}
          onpointerenter={() => ModelPicker.prefetch()}
          onclick={() => { ModelPicker.prefetch(); pickerOpen = true; }}
          data-testid="ide-inline-provider"
        >
          {#if hasProviderIcon(st.provider)}
            <ProviderIcon provider={st.provider} size={12} />
          {:else}
            <Icon name="assistant" size={12} />
          {/if}
          <span class="ellipsis">{inlineAgentLabel}</span>
        </button>

        <button
          type="button"
          class="btn small ip-cta"
          disabled={st.busy || !st.query.trim()}
          onclick={() => void ideAgent.submitInlineEdit()}
          data-testid="ide-inline-submit"
        >
          {#if st.busy}
            <span class="spinner" style:--spinner-size="12px" aria-hidden="true"></span>
            Generating…
          {:else}
            <Icon name="sparkle" size={12} />
            Generate
          {/if}
        </button>

        <button
          type="button"
          class="btn small ghost"
          title="Move selection and prompt to the side-by-side Agent chat"
          onclick={handoffToChat}
          data-testid="ide-inline-handoff"
        >
          In Chat
        </button>

        <button
          type="button"
          class="icon-btn"
          aria-label="Close inline edit"
          title="Close (Esc)"
          onclick={closeInlineBar}
          data-testid="ide-inline-close"
        >
          <Icon name="x" size={12} />
        </button>
      </div>
    </div>

    {#if st.error}
      <p class="ip-err" dir="auto" use:announceShown={{ text: st.error, assertive: true }}>
        <Icon name="warning" size={12} />
        <span>{st.error}</span>
      </p>
    {/if}

    {#if st.proposal}
      {@const prop = st.proposal}
      <div class="ip-proposal">
        <div class="ip-prop-head">
          <Badge tone="ok" label="+{prop.addedCount}" />
          <Badge tone="bad" label="-{prop.removedCount}" />
          <span class="ip-prop-file mono ellipsis" dir="ltr" title={prop.path}>{prop.relPath}</span>

          <div class="ip-prop-actions">
            <button
              type="button"
              class="icon-btn"
              aria-label="Copy proposed inline code"
              title="Copy proposed code"
              onclick={() => void copyWithToast(prop.code, prop.relPath)}
            >
              <Icon name="copy" size={12} />
            </button>

            <button
              type="button"
              class="btn small ghost"
              onclick={closeInlineBar}
              data-testid="ide-inline-discard"
            >
              Discard
            </button>

            <button
              type="button"
              class="btn small ip-cta"
              disabled={readOnly}
              title={readOnly ? 'Current file is read-only' : 'Apply inline change (⌘Z in editor reverts)'}
              onclick={() => void acceptInlineChange()}
              data-testid="ide-inline-accept"
            >
              <Icon name="check" size={12} />
              Accept
            </button>
          </div>
        </div>

        <div class="ip-diff scroll-thin" dir="ltr" data-testid="ide-inline-diff-preview">
          {#each prop.diffLines as dl, idx (`${idx}:${dl.kind}:${dl.oldLine ?? ''}:${dl.newLine ?? ''}`)}
            <div class="ip-diff-row" class:is-add={dl.kind === 'add'} class:is-del={dl.kind === 'del'}>
              <span class="ip-ln">{dl.newLine ?? dl.oldLine ?? ''}</span>
              <span class="ip-sign">{dl.kind === 'add' ? '+' : dl.kind === 'del' ? '-' : ' '}</span>
              <span class="ip-code">{dl.text}</span>
            </div>
          {/each}
        </div>
      </div>
    {/if}
  </div>

  {#if pickerOpen && providerBtnEl && ModelPicker.component}
    <ModelPicker.component
      anchor={providerBtnEl}
      providers={ideAgent.inlineProviders}
      provider={st.provider}
      model={st.model}
      effort={normalizeEffort(st.effort)}
      mode={ideAgent.interactionMode}
      onprovider={(p) => ideAgent.setInlineProvider(p)}
      onmodel={(m) => ideAgent.setInlineModel(m)}
      oneffort={supportsEffortSelection(st.provider) ? (e) => ideAgent.setInlineEffort(e) : undefined}
      onclose={() => (pickerOpen = false)}
    />
  {/if}
{/if}

<style>
  .ip-bar {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-block: 6px;
    padding-inline: 10px;
    background: var(--surface);
    border-block-end: 1px solid var(--accent-line);
    flex-shrink: 0;
  }
  .ip-top {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    min-inline-size: 0;
  }
  .ip-spark {
    display: inline-flex;
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .ip-target {
    display: inline-flex;
    min-inline-size: 0;
    max-inline-size: 220px;
  }
  .ip-field-wrap {
    position: relative;
    flex: 1;
    min-inline-size: 160px;
    display: flex;
  }
  textarea.ip-field {
    flex: 1;
    min-inline-size: 0;
    block-size: 26px;
    padding-block: 4px;
    padding-inline: 8px;
    font: inherit;
    font-size: var(--fs-s);
    line-height: 16px;
    resize: none;
  }
  .ip-actions {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-inline-start: auto;
    flex-shrink: 0;
  }
  .ip-model-btn {
    max-inline-size: 160px;
  }
  .ip-cta {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--accent-text);
  }
  .ip-cta:hover:not(:disabled) {
    background: var(--accent-soft-strong);
  }
  .ip-err {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--danger);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .ip-proposal {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    overflow: hidden;
  }
  .ip-prop-head {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    padding-block: 4px;
    padding-inline: 8px;
    background: var(--surface-2);
    border-block-end: 1px solid var(--border);
  }
  .ip-prop-file {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ip-prop-actions {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-inline-start: auto;
    flex-shrink: 0;
  }
  .ip-diff {
    max-block-size: 180px;
    overflow: auto;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.45;
  }
  .ip-diff-row {
    display: flex;
    min-inline-size: max-content;
    padding-inline: 8px;
  }
  .ip-diff-row.is-add {
    background: var(--success-soft);
  }
  .ip-diff-row.is-del {
    background: var(--danger-soft);
  }
  .ip-ln {
    inline-size: 32px;
    padding-inline-end: 8px;
    text-align: end;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    user-select: none;
    flex-shrink: 0;
  }
  .ip-sign {
    inline-size: 14px;
    flex-shrink: 0;
    user-select: none;
    color: var(--text-dim);
  }
  .ip-diff-row.is-add .ip-sign {
    color: var(--success);
  }
  .ip-diff-row.is-del .ip-sign {
    color: var(--danger);
  }
  .ip-code {
    white-space: pre;
    color: var(--text);
  }
  @media (max-width: 640px) {
    .ip-field-wrap {
      inline-size: 100%;
      flex-basis: 100%;
    }
  }
</style>
