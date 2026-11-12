<script lang="ts">
  // The "Request changes" composer as a sheet, so it opens over whatever tab
  // you are on — reading the diff on a phone, the header button used to jump
  // you to Summary and lose your place. The draft is the parent's (bound), so
  // closing and reopening keeps it, and the leave-guard still sees it.
  import Modal from '../../lib/components/Modal.svelte';
  import { focusFirstField } from '../../lib/inlineEdit';

  interface Props {
    number: number;
    /** "Posts to PR #N on repo · visible to …" */
    hint: string;
    body: string;
    busy: boolean;
    onsubmit: () => void;
    onclose: () => void;
  }
  let { number, hint, body = $bindable(), busy, onsubmit, onclose }: Props = $props();
</script>

<Modal title="Request changes on #{number}" width={520} {onclose} dismissable={!busy}>
  <div class="rc">
    <textarea dir="auto" class="input" rows="5" bind:value={body} disabled={busy} use:focusFirstField
      aria-label="What needs to change" placeholder="The retry loop needs a cap before this can merge."></textarea>
    <p class="hint dim">{hint}</p>
  </div>
  {#snippet footer()}
    <div class="rc-foot">
      <button class="btn ghost" disabled={busy} onclick={onclose}>Cancel</button>
      <button class="btn primary" disabled={busy} onclick={onsubmit} data-testid="prd-request-changes-submit">
        {busy ? 'Requesting…' : 'Request changes'}
      </button>
    </div>
  {/snippet}
</Modal>

<style>
  .rc {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .rc textarea {
    width: 100%;
  }
  .hint {
    margin: 0;
    font-size: var(--fs-xs);
  }
  .rc-foot {
    display: flex;
    justify-content: flex-end;
    flex-wrap: wrap;
    gap: 8px;
    width: 100%;
  }
  @media (max-width: 1024px) {
    .rc textarea {
      font-size: var(--fs-l);
    }
  }
</style>
