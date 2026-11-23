<script lang="ts">
  // Inline stale-poll notice shown when fallback polling hits MAX_POLLS.
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';

  interface Props {
    message: string;
    onretry: () => void;
  }
  let { message, onretry }: Props = $props();
</script>

<div class="rp-poll-stale card" use:announceShown={message}>
  <Icon name="warning" size={14} />
  <span class="rp-poll-stale-msg">{message}</span>
  <button class="btn small" onclick={onretry}>Check again</button>
</div>

<style>
  .rp-poll-stale {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    margin-block-end: 8px;
    background: var(--warning-soft);
    color: var(--warning);
    font-size: var(--fs-s);
  }
  .rp-poll-stale-msg {
    flex: 1;
    min-width: 0;
  }
</style>
