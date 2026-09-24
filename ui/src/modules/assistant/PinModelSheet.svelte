<script lang="ts">
  // Pin provider + model for the whole thread (the model chip), or go back to
  // the routing rules. Explicit always wins over the router (plan §2.4).
  import Modal from '../../lib/components/Modal.svelte';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import { assistant, describeError } from '../../lib/stores/assistant.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { PROVIDER_NAME, type Provider } from './model';
  import type { AssistantThread } from '../../lib/api/types';

  interface Props {
    thread: AssistantThread;
    onclose: () => void;
  }
  let { thread, onclose }: Props = $props();

  const PROVIDERS: Provider[] = ['claude', 'codex'];
  // Seeded once from the thread; the sheet edits a local draft.
  let provider = $state<Provider>((() => (thread.provider === 'codex' ? 'codex' : 'claude'))());
  let model = $state((() => (thread.route_pinned ? (thread.model ?? '') : ''))());
  let busy = $state(false);
  let error = $state('');

  async function pin(): Promise<void> {
    busy = true;
    error = '';
    try {
      await assistant.route(thread.id, provider, model.trim() || null);
      onclose();
    } catch (e) {
      error = `Couldn’t pin the model. ${describeError(e)}`;
    } finally {
      busy = false;
    }
  }
  async function unpin(): Promise<void> {
    busy = true;
    try {
      await assistant.route(thread.id, null);
      onclose();
    } catch (e) {
      toasts.error('Couldn’t switch back to routing rules', describeError(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="Model for this thread" width={460} {onclose}>
  <p class="lead">
    Every turn in “{thread.title}” uses this until you change it. Start a message with <span class="mono">@claude</span> or
    <span class="mono">@codex</span> to route just that one.
  </p>
  <div class="field">
    <span class="lbl" id="pin-provider">Provider</span>
    <div class="segmented" role="group" aria-labelledby="pin-provider">
      {#each PROVIDERS as p (p)}
        <button
          type="button"
          aria-pressed={provider === p}
          class:active={provider === p}
          onclick={() => {
            provider = p;
            model = '';
          }}
        >
          <ProviderIcon provider={p} size={12} />
          {PROVIDER_NAME[p]}
        </button>
      {/each}
    </div>
  </div>
  {#key provider}
    <ModelPicker {provider} value={model} onchange={(m) => (model = m)} hint="Empty uses the provider’s default model." />
  {/key}
  {#if error}<p class="err" role="alert">{error}</p>{/if}
  {#snippet footer()}
    {#if thread.route_pinned}
      <button class="btn ghost reset" onclick={() => void unpin()} disabled={busy}>Use routing rules</button>
    {/if}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={() => void pin()} disabled={busy}>{busy ? 'Pinning…' : 'Pin to thread'}</button>
  {/snippet}
</Modal>

<style>
  .lead {
    margin: 0 0 12px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .mono {
    font-family: var(--font-mono);
    color: var(--text);
  }
  .field {
    margin-bottom: 12px;
  }
  .lbl {
    display: block;
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
    margin-bottom: 4px;
  }
  .segmented > button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .err {
    margin: 8px 0 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .reset {
    margin-inline-end: auto;
  }
</style>
