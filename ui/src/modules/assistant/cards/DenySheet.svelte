<script lang="ts">
  // Deny with an optional reason (patterns §5). The reason goes back to the
  // agent so it can adjust; leaving it empty is fine.
  import Modal from '../../../lib/components/Modal.svelte';

  interface Props {
    action: string;
    onclose: () => void;
    ondeny: (reason: string | null) => void;
  }
  let { action, onclose, ondeny }: Props = $props();
  let reason = $state('');

  function submit(): void {
    const r = reason.trim();
    ondeny(r ? r : null);
  }
</script>

<Modal title="Deny request" width={440} {onclose}>
  <div class="field">
    <label for="deny-reason">Reason (optional)</label>
    <textarea
      id="deny-reason"
      class="input"
      rows="3"
      bind:value={reason}
      placeholder="Not now — ask me again after lunch"
      onkeydown={(e) => {
        if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          submit();
        }
      }}
    ></textarea>
    <span class="hint">Otto won’t {action.toLowerCase()}. It sees your reason and can suggest something else.</span>
  </div>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={submit}>Deny</button>
  {/snippet}
</Modal>

<style>
  textarea.input {
    height: auto;
    min-height: 72px;
    resize: vertical;
    padding: 6px 8px;
    font: inherit;
  }
</style>
