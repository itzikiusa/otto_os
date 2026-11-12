<!--
  The daemon's notice about threads opened from this one (runtime/children.rs):
  "You asked Codex: …" becomes their live cards, "Codex finished: <one line>"
  a compact line with Open (T3 Code's SubagentNotificationLink).
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import type { ThreadNotice } from './childThreads';
  import { openChildThread } from './childNav';
  import RuntimeChildGroup from './RuntimeChildGroup.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';

  let { notice }: { notice: ThreadNotice } = $props();

  const child = $derived(runtimeThreads.get(notice.ids[0]));
</script>

{#if notice.event === 'started'}
  <RuntimeChildGroup kids={notice.ids.map((id) => ({ id }))} />
{:else}
  <div class="cn" data-event={notice.event} data-testid="runtime-child-notice-{notice.ids[0]}">
    {#if child && hasProviderIcon(child.provider)}<ProviderIcon provider={child.provider} size={12} />{:else}<Icon name="assistant" size={12} />{/if}
    <span class="cn-text ellipsis" dir="auto" title={notice.text}>{notice.text}</span>
    <button type="button" class="btn small ghost" title="Open this thread (⇧-click: beside)" onclick={(e) => openChildThread(notice.ids[0], e.shiftKey)}>Open</button>
  </div>
{/if}

<style>
  .cn {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    min-block-size: 28px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .cn-text {
    flex: 1;
    min-inline-size: 0;
  }
  .cn[data-event='failed'] .cn-text {
    color: var(--danger);
  }
</style>
