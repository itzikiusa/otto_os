<!--
  "Ask another agent" + unified `@` file/folder & agent mention menu in a
  runtime thread's composer. `part="button"` (in the composer's foot) opens
  the agent menu: each model picked becomes a chip, so one is "Ask another
  agent" and 2–4 are "Ask several". `part="chips"` mounts `RuntimeMentionMenu`
  (searching workspace files/folders AND `@agent` models) and renders picked
  agent chips (`askTargets.svelte.ts`).
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import type { RuntimeAccessMode } from '../../../lib/api/types';
  import {
    formatEffortShort,
    formatModelLabel,
    supportsEffortSelection,
    type EffortLevel,
  } from '../../../lib/providerModels';
  import { agentProviders, harnessLabel } from '../../../lib/providers';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { ASK_MAX_TARGETS, askableProviders } from './askAgents';
  import { askTargets } from './askTargets.svelte';
  import { fileMentionContext, type ComposerContext } from './composerContext';
  import RuntimeMentionMenu, { type PickedAgentMention, type PickedFileMention } from './RuntimeMentionMenu.svelte';

  interface Props {
    part: 'button' | 'chips';
    sessionId: string;
    /** The thread's provider: the menu opens on another one. */
    provider: string;
    access: RuntimeAccessMode;
    cwd?: string | null;
    textareaEl?: HTMLTextAreaElement | null;
    anchorEl?: HTMLElement | null;
    allowAgents?: boolean;
    readOnly?: boolean;
    disabled?: boolean;
    /** The composer's text (chips: the `@` menu reads it). */
    text?: string;
    onreplace?: (text: string) => void;
    onaddcontext?: (ctx: ComposerContext) => void;
  }
  let {
    part,
    sessionId,
    provider,
    access,
    cwd = null,
    textareaEl = null,
    anchorEl = null,
    allowAgents = true,
    readOnly = false,
    disabled = false,
    text = '',
    onreplace,
    onaddcontext,
  }: Props = $props();

  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));
  let btn = $state<HTMLButtonElement | null>(null);
  let mentionMenuEl = $state<{ onKey: (e: KeyboardEvent) => boolean } | null>(null);
  let open = $state(false);
  let pickProvider = $state<string>('');
  let pickEffort = $state<EffortLevel | null>(null);
  const targets = $derived(askTargets.for(sessionId));
  const full = $derived(targets.length >= ASK_MAX_TARGETS);
  const allowedProviders = $derived(askableProviders(agentProviders(), auth.meta));

  const label = (t: { provider: string; model?: string | null; effort?: string | null }): string =>
    [t.model ? formatModelLabel(t.provider, t.model, auth.meta) : harnessLabel(t.provider), t.effort ? formatEffortShort(t.effort) : null]
      .filter(Boolean)
      .join(' · ');

  const ACCESS: Record<RuntimeAccessMode, string> = {
    'approval-required': 'asks before it edits',
    'auto-accept-edits': 'can edit files',
    'full-access': 'full access',
  };
  const accessText = $derived(readOnly ? 'read-only, like this thread' : `${ACCESS[access]}, like this thread`);

  function openMenu(): void {
    pickProvider = allowedProviders.find((p) => p !== provider) ?? allowedProviders[0] ?? 'claude';
    pickEffort = null;
    open = true;
  }

  function handlePickFile(picked: PickedFileMention): void {
    onreplace?.(picked.nextText);
    onaddcontext?.(
      fileMentionContext({
        path: picked.path,
        label: picked.display,
        isDir: picked.isDir,
      }),
    );
  }

  function handlePickAgent(picked: PickedAgentMention): void {
    const o = picked.option;
    askTargets.add(sessionId, { provider: o.provider, ...(o.model ? { model: o.model } : {}) });
    onreplace?.(picked.nextText);
  }

  /** The composer's keys while the `@` menu shows; true = handled. */
  export function onKey(e: KeyboardEvent): boolean {
    return mentionMenuEl?.onKey(e) ?? false;
  }
</script>

{#if part === 'button'}
  <button
    bind:this={btn}
    type="button"
    class="icon-btn"
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-label={targets.length ? 'Ask one more agent' : 'Ask another agent'}
    title={full ? `At most ${ASK_MAX_TARGETS} agents at once` : 'Ask another agent: send this message to a new thread on another model (pick more to ask several; or type @)'}
    data-testid="runtime-ask-agent"
    disabled={disabled || full || allowedProviders.length === 0}
    onpointerenter={() => ModelPicker.prefetch()}
    onclick={openMenu}
  >
    <Icon name="people" size={14} />
  </button>
{:else}
  <RuntimeMentionMenu
    bind:this={mentionMenuEl}
    {text}
    {cwd}
    {textareaEl}
    {anchorEl}
    allowAgents={allowAgents && !full}
    onpickfile={handlePickFile}
    onpickagent={handlePickAgent}
    ondrilldir={(nextText) => onreplace?.(nextText)}
  />
  {#if targets.length > 0 && allowAgents}
    <div class="aa-chips" role="group" aria-label="Agents this message goes to" data-testid="runtime-ask-targets">
      <span class="aa-lead">{targets.length > 1 ? `Ask ${targets.length} agents` : 'Ask'}</span>
      {#each targets as t, i (`${t.provider}:${t.model}:${t.effort}`)}
        <span class="aa-chip">
          {#if hasProviderIcon(t.provider)}<ProviderIcon provider={t.provider} size={12} />{/if}
          <span class="ellipsis">{label(t)}</span>
          <button type="button" class="icon-btn aa-x" aria-label="Don’t ask {label(t)}" title="Remove" onclick={() => askTargets.remove(sessionId, i)}>
            <Icon name="x" size={12} />
          </button>
        </span>
      {/each}
      <span class="aa-access" title="A thread you ask here works in this thread’s folder with this thread’s access (the access chip below narrows it)">· {accessText}</span>
      <button type="button" class="btn small ghost" onclick={() => askTargets.clear(sessionId)}>Cancel</button>
    </div>
  {/if}
{/if}

{#if open && btn && ModelPicker.component}
  <ModelPicker.component
    anchor={btn}
    providers={allowedProviders}
    provider={pickProvider}
    model={null}
    effort={pickEffort}
    mode="full"
    onprovider={(p: string) => (pickProvider = p)}
    onmodel={(m: string | null) => {
      askTargets.add(sessionId, { provider: pickProvider, ...(m ? { model: m } : {}), ...(pickEffort ? { effort: pickEffort } : {}) });
      open = false;
    }}
    oneffort={supportsEffortSelection(pickProvider, auth.meta) ? (e: EffortLevel | null) => (pickEffort = e) : undefined}
    onclose={() => (open = false)}
  />
{/if}

<style>
  .aa-chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 6px;
    min-inline-size: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .aa-lead {
    color: var(--text);
    font-weight: 500;
  }
  .aa-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-inline-size: 100%;
    min-inline-size: 0;
    padding-inline: 8px 2px;
    border: 1px solid var(--accent-line);
    border-radius: 999px;
    color: var(--accent-text);
  }
  .aa-x {
    inline-size: 20px;
    block-size: 20px;
  }
  .aa-access {
    font-size: var(--fs-xs);
  }
</style>
