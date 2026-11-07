<script lang="ts">
  import type { Snippet } from 'svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';

  let { children }: { children: Snippet } = $props();
</script>

<!-- Preserve the full desktop metadata; phone reading starts with the story. -->
<details class="story-details" open={!viewport.isPhone}>
  <summary>Story details</summary>
  {@render children()}
</details>

<style>
  summary { display: none; }
  @media (max-width: 640px) {
    details { border-bottom: 1px solid var(--border); margin-block-end: 12px; }
    summary {
      display: list-item;
      min-height: var(--hit-min);
      padding: 12px 4px;
      cursor: pointer;
      color: var(--text-dim);
      font-size: var(--fs-m);
    }
  }
</style>
