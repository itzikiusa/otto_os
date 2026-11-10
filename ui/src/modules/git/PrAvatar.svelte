<script lang="ts">
  // A person on a PR (author, reviewer): the provider's avatar when it sent
  // one, else their initial on the accent tint.
  interface Props {
    name: string;
    url?: string | null;
  }
  let { name, url = null }: Props = $props();
  const initial = $derived((name.trim().charAt(0) || '?').toUpperCase());
</script>

{#if url}
  <img class="pav" src={url} alt="" />
{:else}
  <span class="pav" aria-hidden="true">{initial}</span>
{/if}

<style>
  .pav {
    width: 18px;
    height: 18px;
    flex: none;
    border-radius: 50%;
    object-fit: cover;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
</style>
