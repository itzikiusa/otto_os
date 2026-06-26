<script lang="ts">
  // A row of proof badges. Renders nothing when the list is empty.
  import ProofBadge from './ProofBadge.svelte';
  interface Props {
    badges: string[];
  }
  let { badges }: Props = $props();
</script>

{#if badges.length > 0}
  <span class="proof-badges">
    {#each badges as b (b)}
      <ProofBadge badge={b} />
    {/each}
  </span>
{/if}

<style>
  .proof-badges {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
  }
</style>
