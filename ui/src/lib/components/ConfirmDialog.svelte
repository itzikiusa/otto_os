<script lang="ts">
  // Global confirmation / prompt dialog — mounted once in App. Driven by
  // `confirmer`. In prompt mode it shows a text input (native prompt() is a
  // no-op in the Tauri webview).
  import Modal from './Modal.svelte';
  import { confirmer } from '../confirm.svelte';

  function onPrimary(): void {
    if (confirmer.isPrompt) confirmer.submit();
    else confirmer.resolve(true);
  }
</script>

{#if confirmer.open}
  <Modal title={confirmer.title} onclose={() => confirmer.dismiss()} width={400}>
    {#if confirmer.message}<p class="cf-msg">{confirmer.message}</p>{/if}
    {#if confirmer.isPrompt}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="input cf-input"
        bind:value={confirmer.inputValue}
        placeholder={confirmer.placeholder}
        autofocus
        spellcheck="false"
        onkeydown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            confirmer.submit();
          } else if (e.key === 'Escape') {
            e.preventDefault();
            confirmer.dismiss();
          }
        }}
      />
    {/if}
    {#if confirmer.choices && confirmer.checkboxLabel}
      <label class="cf-remember">
        <input type="checkbox" bind:checked={confirmer.checkboxChecked} />
        {confirmer.checkboxLabel}
      </label>
    {/if}
    {#snippet footer()}
      <button class="btn" onclick={() => confirmer.dismiss()}>Cancel</button>
      {#if confirmer.choices}
        {#each confirmer.choices as opt (opt.value)}
          <button
            class="btn"
            class:primary={opt.kind === 'primary'}
            class:danger={opt.kind === 'danger'}
            onclick={() => confirmer.pick(opt.value)}
          >
            {opt.label}
          </button>
        {/each}
      {:else}
        <button
          class="btn"
          class:primary={!confirmer.danger}
          class:danger={confirmer.danger}
          onclick={onPrimary}
        >
          {confirmer.confirmLabel}
        </button>
      {/if}
    {/snippet}
  </Modal>
{/if}

<style>
  .cf-msg {
    margin: 2px 0 4px;
    font-size: 13px;
    line-height: 1.5;
    color: var(--text);
    white-space: pre-wrap;
  }
  .cf-input {
    width: 100%;
    margin-top: 4px;
  }
  .cf-remember {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 8px;
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
    user-select: none;
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
