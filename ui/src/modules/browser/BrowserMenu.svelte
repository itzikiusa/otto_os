<script lang="ts">
  // The Browser page's header actions: site credentials, and a ⋯ menu with
  // "Clear browsing data" (desktop app only — it clears this Mac's web view,
  // whose tabs keep their own data store apart from Otto's window; see
  // `browser_clear_data` in apps/desktop/src-tauri/src/browser.rs).
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { nativeBrowserAvailable } from '../../lib/nativeBrowser';
  import { CLEAR_DATA_TEXT, clearBrowsingData } from './clearBrowsingData';

  let { oncredentials }: { oncredentials: () => void } = $props();

  async function clearData(): Promise<void> {
    const ok = await confirmer.ask(CLEAR_DATA_TEXT, {
      title: 'Clear browsing data?',
      confirmLabel: 'Clear browsing data',
      danger: true,
    });
    if (!ok) return;
    try {
      await clearBrowsingData();
      toasts.success('Browsing data cleared', 'Pages already open keep what they loaded until they reload.');
    } catch (e) {
      toastError('Couldn’t clear browsing data', e);
    }
  }

  function more(e: MouseEvent): void {
    ctxMenu.show(e, [{ label: 'Clear browsing data…', icon: 'trash', action: () => void clearData() }]);
  }
</script>

<button class="icon-btn" aria-label="Site credentials" title="Site credentials" onclick={oncredentials}><Icon name="key" size={14} /></button>
{#if nativeBrowserAvailable}
  <button class="icon-btn" data-label="Clear browsing data…" data-icon="trash" aria-label="More browser actions" title="More browser actions" aria-haspopup="menu" onclick={more}><Icon name="more" size={14} /></button>
{/if}
