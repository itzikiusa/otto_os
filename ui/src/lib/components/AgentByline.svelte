<script lang="ts">
  // Attribution line for agent-authored content (patterns.md §2): who · what
  // model · when, plus the small AGENT chip that marks it as not-human.
  //
  //   [provider mark] claude  AGENT · sonnet · 3m ago
  //
  // Shows only what is known: a provider mark + name when the record carries a
  // provider (otherwise a neutral sparkle + `name`, default "Otto"), the model
  // when known, and the time (RelTime — relative, exact on hover). `label` is
  // an optional lead-in such as "Plan v3". Agent identity is never the accent
  // colour — the chip is a neutral `.chip` (no `--agent` token exists yet).
  import Icon from './Icon.svelte';
  import ProviderIcon from './ProviderIcon.svelte';
  import RelTime from './RelTime.svelte';

  interface Props {
    /** Provider slug (claude / codex / agy / custom). Omit when unknown. */
    provider?: string | null;
    /** Model alias or id, when known. */
    model?: string | null;
    /** When the agent produced it (ISO or epoch ms). */
    at?: string | number | null;
    /** Name shown when the provider is unknown. */
    name?: string;
    /** Optional lead-in, e.g. "Plan v3". */
    label?: string;
    testid?: string;
  }
  let { provider, model, at, name = 'Otto', label, testid }: Props = $props();

  const p = $derived((provider ?? '').trim());
  const m = $derived((model ?? '').trim());
</script>

<span class="agent-byline" data-testid={testid}>
  {#if label}<span class="ab-label">{label}</span><span class="ab-sep" aria-hidden="true">·</span>{/if}
  <span class="ab-who">
    {#if p}
      <ProviderIcon provider={p} size={13} />
      <span class="ab-name">{p}</span>
    {:else}
      <Icon name="sparkle" size={12} />
      <span class="ab-name">{name}</span>
    {/if}
  </span>
  <span class="chip ab-chip" title="Written by an agent — review before you apply it">Agent</span>
  {#if m}<span class="ab-sep" aria-hidden="true">·</span><span class="ab-model mono">{m}</span>{/if}
  {#if at != null && at !== ''}
    <span class="ab-sep" aria-hidden="true">·</span><RelTime iso={at} class="ab-time" />
  {/if}
</span>

<style>
  .agent-byline {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 2px 6px;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.4;
  }
  .ab-label {
    color: var(--text);
    font-weight: 600;
    font-size: var(--fs-s);
  }
  .ab-who {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--text);
    font-weight: 500;
  }
  .ab-name {
    text-transform: capitalize;
  }
  .ab-chip {
    height: 16px;
    padding: 0 6px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
  }
  .ab-model {
    font-family: var(--font-mono, monospace);
  }
  .ab-sep {
    opacity: 0.7;
  }
</style>
