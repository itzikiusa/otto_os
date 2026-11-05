<script lang="ts">
  // The small "Continue on phone" icon button for a pane / page header.
  // Renders nothing on a paired phone.
  import Icon from './Icon.svelte';
  import { phoneDevice } from '../stores/phoneDevice.svelte';
  import { phoneHandoff } from '../stores/phoneHandoff.svelte';

  interface Props {
    route: string;
    what: string;
    /** Extra classes for the host's header layout rules. */
    class?: string;
    size?: 13 | 14;
    testid?: string;
    /** Accessible name suffix (the pane's title). */
    name?: string;
    /** PageHeader overflow priority (lower collapses into ⋯ first). */
    overflow?: number;
  }
  let { route, what, class: cls = '', size = 13, testid, name, overflow }: Props = $props();
  $effect(() => void phoneDevice.ensure());
  const label = $derived(name ? `Continue on phone — ${name}` : 'Continue on phone');
</script>

{#if !phoneDevice.isPairedPhone}
  <button
    type="button"
    class="icon-btn {cls}"
    data-testid={testid}
    data-overflow={overflow}
    data-icon="phone"
    data-label="Continue on phone"
    onmousedown={(e) => e.stopPropagation()}
    onclick={() => phoneHandoff.open(route, what)}
    title="Continue on phone (QR)"
    aria-label={label}
  ><Icon name="phone" {size} /></button>
{/if}
