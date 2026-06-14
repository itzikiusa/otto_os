<script lang="ts">
  interface Props {
    rows?: number;
    height?: number;
  }
  let { rows = 3, height = 36 }: Props = $props();
</script>

<div class="skeleton-list" aria-busy="true">
  {#each Array(rows) as _, i (i)}
    <div class="skeleton-row" style="height:{height}px; animation-delay:{i * 90}ms"></div>
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
    animation: shimmer 1.4s ease-in-out infinite;
  }
  @keyframes shimmer {
    0% {
      background-position: 120% 0;
    }
    100% {
      background-position: -120% 0;
    }
  }
</style>
