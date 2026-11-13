<!--
  "Ask another agent" in a runtime thread's composer. `part="button"` (in the
  composer's foot) opens the agent menu: each model picked becomes a chip, so
  one is "Ask another agent" and 2–4 are "Ask several". `part="chips"` (above
  the text) shows the picked agents, what access they get (this thread's, or
  the narrower mode picked for it — never wider), and the `@` menu: typing
  `@cod` offers Codex and its models as chips, the way T3 Code's model
  mentions work. With chips set, Send asks them (askTargets.svelte.ts)
  instead of this thread's agent; each answer streams in its own thread,
  live as a card in this one.
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import type { RuntimeAccessMode } from '../../../lib/api/types';
  import { formatEffortShort, formatModelLabel, modelsForProvider, type EffortLevel } from '../../../lib/providerModels';
  import { harnessLabel } from '../../../lib/providers';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { ASK_MAX_TARGETS, ASK_PROVIDERS, dropMention, mentionAt, mentionOptions, type AskOption } from './askAgents';
  import { askTargets } from './askTargets.svelte';

  interface Props {
    part: 'button' | 'chips';
    sessionId: string;
    /** The thread's provider: the menu opens on another one. */
    provider: string;
    access: RuntimeAccessMode;
    readOnly?: boolean;
    disabled?: boolean;
    /** The composer's text (chips: the `@` menu reads it). */
    text?: string;
    onreplace?: (text: string) => void;
  }
  let { part, sessionId, provider, access, readOnly = false, disabled = false, text = '', onreplace }: Props = $props();

  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));
  let btn = $state<HTMLButtonElement | null>(null);
  let open = $state(false);
  let pickProvider = $state<string>('');
  /** What the picked agent takes (the daemon's per-provider baseline). */
  const pickEfforts = $derived(auth.meta?.runtime_capabilities?.[pickProvider]?.efforts ?? []);
  let pickEffort = $state<EffortLevel | null>(null);
  const targets = $derived(askTargets.for(sessionId));
  const full = $derived(targets.length >= ASK_MAX_TARGETS);

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
    pickProvider = ASK_PROVIDERS.find((p) => p !== provider) ?? 'claude';
    pickEffort = null;
    open = true;
  }

  // ── the `@` menu ──────────────────────────────────────────────────────
  const mention = $derived(part === 'chips' && !full ? mentionAt(text) : null);
  let dismissed = $state<string | null>(null);
  let active = $state(0);
  const options = $derived.by((): AskOption[] => {
    if (!mention || dismissed === text) return [];
    return mentionOptions(mention.query, (p) => modelsForProvider(p, auth.meta), (p) => harnessLabel(p));
  });
  $effect(() => {
    void options.length;
    active = 0;
  });

  function accept(o: AskOption): void {
    askTargets.add(sessionId, { provider: o.provider, ...(o.model ? { model: o.model } : {}) });
    if (mention) onreplace?.(dropMention(text, mention.start));
  }

  /** The composer's keys while the `@` menu shows; true = handled. */
  export function onKey(e: KeyboardEvent): boolean {
    if (options.length === 0) return false;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      active = (active + (e.key === 'ArrowDown' ? 1 : options.length - 1)) % options.length;
    } else if (e.key === 'Escape') {
      dismissed = text;
    } else if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
      accept(options[active]);
    } else return false;
    e.preventDefault();
    return true;
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
    disabled={disabled || full}
    onpointerenter={() => ModelPicker.prefetch()}
    onclick={openMenu}
  >
    <Icon name="people" size={14} />
  </button>
{:else}
  {#if options.length > 0}
    <div class="aa-pop" role="listbox" aria-label="Agents to ask" data-testid="runtime-ask-mentions">
      {#each options as o, i (`${o.provider}:${o.model}`)}
        <div
          class="aa-opt"
          class:active={i === active}
          role="option"
          tabindex="-1"
          aria-selected={i === active}
          onmousedown={(ev) => {
            ev.preventDefault();
            accept(o);
          }}
          onmouseenter={() => (active = i)}
        >
          {#if hasProviderIcon(o.provider)}<ProviderIcon provider={o.provider} size={12} />{/if}
          <span class="ellipsis">{o.label}</span>
          <span class="aa-detail ellipsis">{o.detail}</span>
        </div>
      {/each}
    </div>
  {/if}
  {#if targets.length > 0}
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
    providers={ASK_PROVIDERS}
    provider={pickProvider}
    model={null}
    effort={pickEffort}
    mode="full"
    onprovider={(p: string) => (pickProvider = p)}
    onmodel={(m: string | null) => {
      askTargets.add(sessionId, { provider: pickProvider as (typeof ASK_PROVIDERS)[number], ...(m ? { model: m } : {}), ...(pickEffort ? { effort: pickEffort } : {}) });
      open = false;
    }}
    oneffort={pickEfforts.length ? (e: EffortLevel | null) => (pickEffort = e) : undefined}
    effortLevels={pickEfforts}
    onclose={() => (open = false)}
  />
{/if}

<style>
  .aa-pop {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-block-size: min(220px, 35vh);
    overflow-y: auto;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .aa-opt {
    display: flex;
    align-items: center;
    gap: 8px;
    min-inline-size: 0;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .aa-opt.active {
    background: var(--accent-soft);
  }
  .aa-detail {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
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
