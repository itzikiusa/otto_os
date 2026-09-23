<script lang="ts">
  // Notification center bell: unread badge + dropdown panel of notices.
  // Click toggles the panel; opening it marks everything visible as read.
  //
  // It lives in the Navigator header / collapsed Rail (`side`: the panel opens
  // beside the sidebar) and in the phone top bar (`below`). The panel is
  // portalled to <body> and positioned `fixed` from the button's rect: the
  // sidebar's `backdrop-filter` makes it the containing block for fixed
  // descendants, so an in-place panel would be clipped to the sidebar.
  import Icon from '../lib/components/Icon.svelte';
  import { notifications } from '../lib/stores/notifications.svelte';
  import type { Notice } from '../lib/api/types';

  let { placement = 'side' }: { placement?: 'side' | 'below' } = $props();

  let open = $state(false);
  let btnEl = $state<HTMLButtonElement | null>(null);
  let pos = $state({ top: 0, left: 0, width: 340, maxHeight: 400 });

  const PANEL_W = 340;
  const EDGE = 8;

  // Clamp the panel into the viewport (never flip without a floor) and cap its
  // height to the room left below its top, so the list always scrolls in view.
  function place(): void {
    if (!btnEl) return;
    const r = btnEl.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const width = Math.min(PANEL_W, vw - EDGE * 2);
    const rtl = getComputedStyle(btnEl).direction === 'rtl';
    let left: number;
    let top: number;
    if (placement === 'side') {
      // Open beside the sidebar's edge (not the button's) so the panel never
      // covers the Navigator it was opened from.
      const edge = btnEl.closest('nav')?.getBoundingClientRect() ?? r;
      left = rtl ? edge.left - 8 - width : edge.right + 8;
      top = r.top;
    } else {
      left = rtl ? r.left : r.right - width;
      top = r.bottom + 6;
    }
    left = Math.min(Math.max(left, EDGE), vw - width - EDGE);
    top = Math.min(Math.max(top, EDGE), Math.max(EDGE, vh - EDGE - 160));
    const maxHeight = Math.min(vh * 0.7, vh - top - EDGE);
    pos = { top, left, width, maxHeight };
  }

  $effect(() => {
    if (!open) return;
    place();
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') close();
    };
    window.addEventListener('resize', place);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('keydown', onKey);
    };
  });

  // Move a node to <body> for its lifetime (escapes the sidebar's containing block).
  function portal(node: HTMLElement): { destroy(): void } {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }

  // Tick so relative timestamps refresh while the panel is open.
  let now = $state(Date.now());
  $effect(() => {
    if (!open) return;
    const t = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(t);
  });

  // Load notices once on mount.
  $effect(() => {
    void notifications.load();
  });

  function toggle(): void {
    open = !open;
    if (open) {
      now = Date.now();
      // Opening the center implies you've seen what's there.
      void notifications.markAllRead();
    }
  }

  function close(): void {
    open = false;
  }

  function actionLabel(notice: Notice): string {
    switch (notice.action?.type) {
      case 'open_url':
        return 'Open';
      case 'open_session':
        return 'Go to session';
      case 'reauth':
        return 'Re-authenticate';
      default:
        return 'Open';
    }
  }

  function relative(iso: string): string {
    const then = new Date(iso).getTime();
    if (Number.isNaN(then)) return '';
    const secs = Math.max(0, Math.round((now - then) / 1000));
    if (secs < 45) return 'just now';
    const mins = Math.round(secs / 60);
    if (mins < 60) return `${mins}m ago`;
    const hrs = Math.round(mins / 60);
    if (hrs < 24) return `${hrs}h ago`;
    const days = Math.round(hrs / 24);
    if (days < 7) return `${days}d ago`;
    return new Date(then).toLocaleDateString([], { month: 'short', day: 'numeric' });
  }

  const badge = $derived(notifications.unread > 99 ? '99+' : String(notifications.unread));
</script>

<div class="bell-wrap">
  <button
    bind:this={btnEl}
    class="icon-btn bell-btn"
    class:has-unread={notifications.unread > 0}
    onclick={toggle}
    aria-label="Notifications"
    aria-haspopup="true"
    aria-expanded={open}
    title="Notifications"
  >
    <Icon name="bell" size={15} />
    {#if notifications.unread > 0}
      <span class="badge">{badge}</span>
    {/if}
  </button>

  {#if open}
    <div class="bell-layer" use:portal>
      <!-- Backdrop closes the panel on any outside interaction. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="bell-backdrop"
        onclick={close}
        oncontextmenu={(e) => { e.preventDefault(); close(); }}
        onkeydown={(e) => e.key === 'Escape' && close()}
        role="presentation"
      ></div>

      <div
        class="panel"
        role="dialog"
        aria-label="Notifications"
        style="top:{pos.top}px;left:{pos.left}px;width:{pos.width}px;max-height:{pos.maxHeight}px"
      >
        <header class="panel-head">
          <span class="panel-title">Notifications</span>
          <div class="panel-actions">
            <button
              class="link-btn"
              onclick={() => notifications.markAllRead()}
              disabled={notifications.notices.every((n) => n.read)}
            >
              Mark all read
            </button>
            <button
              class="link-btn"
              onclick={() => notifications.clear()}
              disabled={notifications.notices.length === 0}
            >
              Clear
            </button>
          </div>
        </header>

        <div class="panel-list">
          {#if notifications.notices.length === 0}
            <div class="panel-empty">
              <Icon name="bell" size={22} />
              <p>You're all caught up</p>
            </div>
          {:else}
            {#each notifications.notices as notice (notice.id)}
              <div class="notice sev-{notice.severity}" class:unread={!notice.read}>
                <span class="sev-dot"></span>
                <div class="notice-body">
                  <div class="notice-row">
                    <span class="notice-title">{notice.title}</span>
                    <span class="notice-time">{relative(notice.created_at)}</span>
                  </div>
                  {#if notice.body}<p class="notice-text">{notice.body}</p>{/if}
                  {#if notice.action}
                    <button class="btn small action-btn" onclick={() => notifications.runAction(notice)}>
                      {actionLabel(notice)}
                    </button>
                  {/if}
                </div>
                <button
                  class="dismiss"
                  onclick={() => notifications.dismiss(notice.id)}
                  aria-label="Dismiss notification"
                  title="Dismiss"
                >
                  <Icon name="x" size={10} />
                </button>
              </div>
            {/each}
          {/if}
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .bell-wrap {
    position: relative;
    display: flex;
    align-items: center;
  }
  .bell-btn {
    position: relative;
    width: 28px;
    height: 28px;
  }
  .bell-btn.has-unread {
    color: var(--text);
  }
  .badge {
    position: absolute;
    top: -1px;
    inset-inline-end: -1px;
    min-width: 15px;
    height: 15px;
    padding: 0 3px;
    border-radius: 8px;
    background: var(--status-exited);
    color: #fff;
    font-size: 9.5px;
    font-weight: 700;
    line-height: 15px;
    text-align: center;
    box-shadow: 0 0 0 1.5px var(--bg);
  }

  .bell-backdrop {
    position: fixed;
    inset: 0;
    z-index: 9998;
  }

  .panel {
    /* top/left/width/max-height come from place() — clamped to the viewport. */
    position: fixed;
    z-index: 9999;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
    overflow: hidden;
    animation: panel-in 140ms ease-out;
  }
  @keyframes panel-in {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }

  .panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 9px 12px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .panel-title {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text);
  }
  .panel-actions {
    display: flex;
    gap: 4px;
  }
  .link-btn {
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: 11.5px;
    padding: 2px 5px;
    border-radius: 4px;
    cursor: pointer;
  }
  .link-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .link-btn:disabled {
    color: var(--text-dim);
    opacity: 0.5;
    cursor: default;
  }

  .panel-list {
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }

  .panel-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 40px 24px;
    color: var(--text-dim);
  }
  .panel-empty p {
    margin: 0;
    font-size: 12.5px;
  }

  .notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 10px 10px 12px;
    border-bottom: 1px solid var(--border);
    border-inline-start: 2px solid transparent;
    position: relative;
  }
  .notice:last-child {
    border-bottom: none;
  }
  .notice.unread {
    background: color-mix(in srgb, var(--accent) 7%, transparent);
  }
  .notice.sev-warn.unread {
    background: color-mix(in srgb, var(--warning) 9%, transparent);
  }
  .notice.sev-error.unread {
    background: color-mix(in srgb, var(--status-exited) 9%, transparent);
  }
  .notice.sev-warn {
    border-inline-start-color: var(--warning);
  }
  .notice.sev-error {
    border-inline-start-color: var(--status-exited);
  }

  .sev-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    margin-top: 4px;
    flex-shrink: 0;
    background: var(--accent);
  }
  .sev-warn .sev-dot {
    background: var(--status-warn);
  }
  .sev-error .sev-dot {
    background: var(--status-exited);
  }

  .notice-body {
    flex: 1;
    min-width: 0;
  }
  .notice-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }
  .notice-title {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .notice-time {
    font-size: 10.5px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .notice-text {
    margin: 2px 0 0;
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.45;
    word-break: break-word;
  }
  .action-btn {
    margin-top: 7px;
  }

  .dismiss {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity 120ms ease-out, background 120ms ease-out;
  }
  .notice:hover .dismiss {
    opacity: 1;
  }
  .dismiss:hover {
    background: color-mix(in srgb, var(--text-dim) 20%, transparent);
    color: var(--text);
  }
</style>
