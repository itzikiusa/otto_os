<script lang="ts">
  // Send-to-session: a small icon button that opens the shared, viewport-clamped
  // context menu (`ctxMenu` — same picker BroadcastModal/SchemaTree use for a
  // data-driven list) filtered to live agent sessions, and POSTs the annotation
  // to whichever one is picked via `browser.sendAnnotation`. Only sessions the
  // workspace considers "live" (running/working/idle) are valid targets — a
  // dead/suspended/connection session can't receive a prompt.

  import { ws } from '../../lib/stores/workspace.svelte';
  import { browser } from '../../lib/stores/browser.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  let { annotationId }: { annotationId: string } = $props();

  let sending = $state(false);

  function eligible() {
    return ws.agentSessions.filter((s) => {
      const st = ws.statusMap[s.id] ?? s.status;
      return st === 'running' || st === 'working' || st === 'idle';
    });
  }

  async function sendTo(sessionId: string): Promise<void> {
    sending = true;
    try {
      await browser.sendAnnotation(annotationId, sessionId);
      toasts.success('Sent to session');
    } catch (e) {
      toasts.error('Failed to send', e instanceof Error ? e.message : undefined);
    } finally {
      sending = false;
    }
  }

  function open(e: MouseEvent): void {
    const sessions = eligible();
    if (sessions.length === 0) {
      toasts.warn('No live sessions', 'Open an agent session to send marks to it.');
      return;
    }
    const items: MenuItem[] = sessions.map((s) => ({
      label: `${s.title} (${s.provider})`,
      icon: 'terminal',
      action: () => void sendTo(s.id),
    }));
    ctxMenu.show(e, items, { filter: sessions.length > 8, filterPlaceholder: 'Filter sessions…' });
  }
</script>

<button
  class="icon-btn"
  title="Send to session"
  disabled={sending}
  onclick={open}
>
  <Icon name="terminal" size={12} />
</button>

<style>
  .icon-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .icon-btn:hover:not(:disabled) {
    background: var(--surface-2);
    color: var(--text);
  }
  .icon-btn:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
</style>
