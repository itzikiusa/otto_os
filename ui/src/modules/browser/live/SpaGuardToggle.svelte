<script lang="ts">
  // Toolbar toggle for the live session's opt-in SPA Fetch/XHR mutation guard.
  // Sends `{ type: "set_guard", guard_spa_mutations }` over the active live WS.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { BrowserLiveSession } from '../../../lib/api/types';
  import { toasts } from '../../../lib/toast.svelte';
  import { readSpaGuardFlag } from './spaGuard';

  interface Props {
    tabId: string;
    session: BrowserLiveSession | null | undefined;
    disabled?: boolean;
    onsetguard: (enabled: boolean) => boolean;
  }
  let { tabId, session, disabled = false, onsetguard }: Props = $props();

  let localOverride = $state<boolean | null>(null);
  let lastTabId = '';
  const serverFlag = $derived(readSpaGuardFlag(session));

  $effect(() => {
    if (tabId !== lastTabId) {
      lastTabId = tabId;
      localOverride = null;
    }
  });
  $effect(() => {
    if (localOverride !== null && serverFlag === localOverride) {
      localOverride = null;
    }
  });

  const enabled = $derived(localOverride ?? serverFlag ?? false);

  function toggle(): void {
    if (disabled) return;
    const next = !enabled;
    if (!onsetguard(next)) {
      toasts.info('Live session reconnecting', 'Wait for the live browser connection, then try again.');
      return;
    }
    localOverride = next;
    toasts.success(
      next ? 'SPA mutation guard enabled' : 'SPA mutation guard turned off',
      next
        ? 'Agent Fetch/XHR writes (POST, PUT, PATCH, DELETE) will pause for your approval.'
        : 'Only document form submissions are held for approval.',
    );
  }
</script>

<button
  type="button"
  class="icon-btn tool"
  class:active={enabled}
  {disabled}
  onclick={toggle}
  aria-label="Guard SPA mutations"
  aria-pressed={enabled}
  title={enabled
    ? 'SPA mutation guard on — holds agent Fetch/XHR writes for approval'
    : 'Guard SPA mutations — intercept agent Fetch/XHR writes for approval'}
  data-testid="live-spa-guard-toggle"
>
  <Icon name="shield" size={14} />
</button>
