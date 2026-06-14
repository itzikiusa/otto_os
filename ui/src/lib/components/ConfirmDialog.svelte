<script lang="ts">
  // Global confirmation dialog — mounted once in App. Driven by `confirmer`.
  import Modal from './Modal.svelte';
  import { confirmer } from '../confirm.svelte';
</script>

{#if confirmer.open}
  <Modal title={confirmer.title} onclose={() => confirmer.resolve(false)} width={400}>
    <p class="cf-msg">{confirmer.message}</p>
    {#snippet footer()}
      <button class="btn" onclick={() => confirmer.resolve(false)}>Cancel</button>
      <button
        class="btn"
        class:primary={!confirmer.danger}
        class:danger={confirmer.danger}
        onclick={() => confirmer.resolve(true)}
      >
        {confirmer.confirmLabel}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .cf-msg {
    margin: 2px 0 4px;
    font-size: 13px;
    line-height: 1.5;
    color: var(--text);
  }
  .btn.danger {
    background: var(--danger, #e5534b);
    color: #fff;
    border-color: transparent;
  }
  .btn.danger:hover {
    filter: brightness(1.05);
  }
</style>
