<script lang="ts">
  import type { Session } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import PhoneHandoffButton from '../../lib/components/PhoneHandoffButton.svelte';
  import { canSwitchSessionSurface, openSessionSurface } from '../../lib/sessionSurface';
  import { viewport } from '../../lib/stores/viewport.svelte';
  let { session, sessionId, readOnly }: { session: Session | null; sessionId: string; readOnly: boolean } = $props();
</script>

{#if !viewport.isDesktop && !readOnly && canSwitchSessionSurface(session)}
  <button class="surface-switch" title="Same agent and conversation in a chat view" onmousedown={(e) => e.stopPropagation()} onclick={() => void openSessionSurface(session!, 'threads')}>Threads <Icon name="comment" size={12} /></button>
{:else}
  <PhoneHandoffButton route={`agents/${sessionId}`} what="this session" name={session?.title ?? sessionId} testid="pane-continue-phone" />
{/if}

<style>
  .surface-switch { display: inline-flex; align-items: center; gap: 4px; flex-shrink: 0; border: 0; background: transparent; padding-inline: 8px; border-radius: var(--radius-s); cursor: pointer; min-block-size: max(var(--hit-min), var(--mobile-topbar-h)); color: var(--accent-text); font-size: var(--fs-s); }
  .surface-switch:hover { background: var(--accent-soft); }
</style>
