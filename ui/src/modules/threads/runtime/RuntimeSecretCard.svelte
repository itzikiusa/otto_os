<script lang="ts">
  // A private secret request (`otto.request_secret`, T3 Code #15907): the
  // thread's agent asks its owner for a token or password. What is typed here
  // goes straight to the daemon and from there only to the agent's tool
  // result — never the composer, a draft, the timeline or the database.
  import Icon from '../../../lib/components/Icon.svelte';
  import { secretRequests, type SecretRequest } from '../../../lib/stores/secretRequests.svelte';
  import { toastError } from '../../../lib/toastError';

  interface Props {
    request: SecretRequest;
    /** In the attention dock, whose headline already says who asks. */
    docked?: boolean;
  }
  let { request, docked = false }: Props = $props();

  let value = $state('');
  let busy = $state(false);

  async function send(v: string | null): Promise<void> {
    busy = true;
    try {
      await secretRequests.answer(request, v);
      value = '';
    } catch (e) {
      toastError(v === null ? 'Couldn’t decline' : 'Couldn’t send the secret', e);
    } finally {
      busy = false;
    }
  }
</script>

<form
  class="sc"
  class:docked
  data-testid="runtime-secret-card"
  onsubmit={(e) => {
    e.preventDefault();
    if (value) void send(value);
  }}
>
  <p class="sc-ask" dir="auto">
    {#if docked}
      <span>{#if request.name}<strong>{request.name}:</strong> {/if}{request.prompt}</span>
    {:else}
      <Icon name="lock" size={13} />
      <span><strong>The agent asks for a secret{request.name ? ` (${request.name})` : ''}:</strong> {request.prompt}</span>
    {/if}
  </p>
  <div class="sc-row">
    <input
      type="password"
      class="input"
      bind:value
      autocomplete="off"
      spellcheck="false"
      placeholder="Only the agent gets it, not the chat"
      aria-label="Secret"
      data-testid="runtime-secret-input"
    />
    <button type="submit" class="btn small primary sc-btn" disabled={busy || !value} data-testid="runtime-secret-send">Give secret</button>
    <button type="button" class="btn small sc-btn" disabled={busy} onclick={() => void send(null)} data-testid="runtime-secret-decline">Decline</button>
  </div>
</form>

<style>
  .sc {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    font-size: var(--fs-s);
  }
  .sc-ask {
    display: flex;
    gap: 6px;
    align-items: baseline;
    margin: 0;
    color: var(--text);
  }
  .sc-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .sc-row {
    flex-wrap: wrap;
  }
  .sc-row input {
    flex: 1 1 160px;
    min-inline-size: 0;
  }
  .sc.docked {
    padding: 0;
    border: 0;
    background: transparent;
  }
  /* Phone: 44 px targets (the field and both answers). */
  @media (pointer: coarse) {
    .sc-row :is(.sc-btn, input) {
      min-block-size: calc(var(--hit-min) + 8px);
    }
    .sc-btn {
      padding-inline: 14px;
    }
  }
</style>
