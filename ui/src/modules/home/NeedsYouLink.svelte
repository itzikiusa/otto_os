<script lang="ts">
  // The ONE "Needs you" inbox (S20-07). Home → Today → Needs you is canonical:
  // it is the only surface that joins every source — assistant tasks, MCP
  // approvals, sessions waiting on input, work items awaiting approval and
  // unread warnings. The narrower surfaces (Assistant rail, Agents → Work
  // Queue, Mission Control, MCP approvals) keep their own lists but show the
  // SAME total from the same store (`today.needs`) and link here, so a user
  // learns there is one place that is never missing anything.
  import { onDestroy, onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { router } from '../../lib/router.svelte';
  import { today } from './today.svelte';
  import { needsYouLinkLabel } from './needsYou';

  interface Props {
    /** Extra class for placement tweaks by the host surface. */
    class?: string;
  }
  let { class: cls = '' }: Props = $props();

  // Ref-counted: the Today store polls (event-fed) only while something reads it.
  onMount(() => today.start());
  onDestroy(() => today.stop());

  const count = $derived(today.needs.length);
  const label = $derived(needsYouLinkLabel(count));
</script>

<button
  type="button"
  class="nyl {cls}"
  class:has={count > 0}
  data-testid="needs-you-inbox-link"
  title="Everything waiting on you, from every part of Otto, is on Home → Needs you"
  onclick={() => router.go('home')}
>
  <Icon name="bell" size={12} />
  <span>{label}</span>
  <Icon name="arrow" size={12} />
</button>

<style>
  .nyl {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
    white-space: nowrap;
  }
  .nyl:hover { color: var(--text); }
  .nyl.has {
    color: var(--warning);
    border-color: var(--warning-soft);
    background: var(--warning-soft);
  }
  .nyl:focus-visible { outline: 2px solid var(--accent-solid); outline-offset: 2px; }
</style>
