<script lang="ts">
  // Review status as the shared Badge with a dot (colour + word, never colour
  // alone). With `onclick` it is the artifact header's status menu button —
  // the same `.chip` look, as a real <button>.
  import Badge from '../../lib/components/Badge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { badgeTone, designStatus } from '../../lib/status';
  import type { DesignStatus } from '../../lib/api/types';

  interface Props {
    status: DesignStatus | string;
    onclick?: (e: MouseEvent) => void;
    /** Extra words after the label ("· v8"). */
    suffix?: string;
  }
  let { status, onclick, suffix = '' }: Props = $props();
  const info = $derived(designStatus(status));
  const tone = $derived(badgeTone(info.tone));
</script>

{#if onclick}
  <button class="chip {tone} as-btn" {onclick} aria-haspopup="menu" title="Change status" data-testid="design-status">
    <span class="dot" aria-hidden="true"></span>{info.label}{suffix}
    <Icon name="chevronDown" size={12} />
  </button>
{:else}
  <Badge {tone} label="{info.label}{suffix}" dot />
{/if}

<style>
  .dot {
    inline-size: 6px;
    block-size: 6px;
    border-radius: 50%;
    background: currentColor;
    flex: none;
  }
  .as-btn {
    cursor: pointer;
    font-family: inherit;
    transition: filter var(--dur-fast) ease-out;
  }
  .as-btn:hover {
    filter: brightness(0.97);
  }
</style>
