<script lang="ts">
  // Global confirmation / prompt dialog — mounted once in App. Driven by
  // `confirmer`. In prompt mode it shows a text input (native prompt() is a
  // no-op in the Tauri webview).
  import Modal from './Modal.svelte';
  import PathField from './PathField.svelte';
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
      {#snippet promptInput()}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="input cf-input"
        aria-label={confirmer.message || confirmer.title}
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
      {/snippet}
      {#if confirmer.browseFolder}
        <PathField bind:value={confirmer.inputValue}>{@render promptInput()}</PathField>
      {:else}{@render promptInput()}{/if}
    {/if}
    {#if confirmer.choices && confirmer.checkboxLabel}
      <label class="cf-remember">
        <input type="checkbox" bind:checked={confirmer.checkboxChecked} />
        {confirmer.checkboxLabel}
      </label>
    {/if}
    {#snippet footer()}
      <button class="btn" onclick={() => confirmer.dismiss()}>{confirmer.cancelLabel}</button>
      {#if confirmer.choices}
        {#each confirmer.choices as opt (opt.value)}
          <button
            class="btn"
            class:primary={opt.kind === 'primary'}
            class:danger-solid={opt.kind === 'danger'}
            data-autofocus={opt.kind === 'primary' ? '' : undefined}
            onclick={() => confirmer.pick(opt.value)}
          >
            {opt.label}
          </button>
        {/each}
      {:else}
        <!-- The default button (Return) is the primary action — except for a
             destructive one, where focus stays on Cancel (macOS HIG), so a
             reflexive Return never deletes. -->
        <button
          class="btn"
          class:primary={!confirmer.danger}
          class:danger-solid={confirmer.danger}
          data-autofocus={confirmer.danger ? undefined : ''}
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
    font-size: var(--fs-m);
    line-height: 1.5;
    color: var(--text);
    white-space: pre-wrap;
    /* Paths / ids / branch names have no break points. */
    overflow-wrap: anywhere;
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
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
    user-select: none;
  }
</style>
