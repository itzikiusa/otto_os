<script lang="ts">
  interface Props {
    rows?: number;
    height?: number;
    /** Noun for the screen-reader text: "Loading {label}". */
    label?: string;
    /** false when a parent (LoadState) already owns the status announcement. */
    announce?: boolean;
    /** Fade in after ~150 ms (the height is reserved at once) so a fast load never
     *  flashes the placeholder. false when a parent (LoadState) already waits. */
    grace?: boolean;
  }
  let { rows = 3, height = 36, label = '', announce = true, grace = true }: Props = $props();
</script>

<div class="skeleton-list" class:grace aria-busy="true" role={announce ? 'status' : undefined}>
  {#if announce}<span class="sr-only">Loading{label ? ` ${label}` : ''}</span>{/if}
  {#each Array(rows) as _, i (i)}
    <div class="skeleton-row" style="height:{height}px; --stagger:{i * 90}ms"></div>
  {/each}
</div>

<style>
  .skeleton-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 4px 0;
  }
  .skeleton-row {
    border-radius: var(--radius-m);
    background: linear-gradient(
      100deg,
      var(--surface-2) 40%,
      color-mix(in srgb, var(--surface-2) 60%, var(--text-dim) 8%) 50%,
      var(--surface-2) 60%
    );
    background-size: 220% 100%;
    animation: otto-shimmer 1.4s ease-in-out var(--stagger, 0ms) infinite;
  }
  /* Transparent for the first 150 ms, then a quick fade-in (backwards fill holds
     the start frame during the delay; the row's own style is the end state). */
  .grace .skeleton-row {
    animation:
      otto-shimmer 1.4s ease-in-out var(--stagger, 0ms) infinite,
      otto-fade-in var(--dur-enter) var(--ease-out) 150ms backwards;
  }
  @media (prefers-reduced-motion: reduce) {
    .skeleton-row,
    .grace .skeleton-row {
      animation: none;
    }
  }
</style>
