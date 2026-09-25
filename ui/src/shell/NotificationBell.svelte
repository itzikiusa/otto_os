<script lang="ts">
  // Notification center bell: unread badge + an anchored popover of notices.
  //
  // It lives in the Navigator header / collapsed Rail (`side`: the panel opens
  // beside the sidebar edge, a caret pointing back at the bell) and in the
  // phone top bar (`below`: caret on top). The panel is portalled to <body>
  // and positioned `fixed` from the button's rect: the sidebar's
  // `backdrop-filter` makes it the containing block for fixed descendants, so
  // an in-place panel would be clipped to the sidebar.
  //
  // Unread survives OPENING: rows keep their unread dot while the panel is up
  // and are marked read when it closes (you've now seen them). Session notices
  // arrive grouped per session (`notifications.rows`); a row is one button
  // that runs the notice's action.
  import { tick, untrack } from 'svelte';
  import Icon from '../lib/components/Icon.svelte';
  import ProviderIcon from '../lib/components/ProviderIcon.svelte';
  import { notifications, type NoticeRow } from '../lib/stores/notifications.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { ctxMenu } from '../lib/contextmenu.svelte';
  import { confirmer } from '../lib/confirm.svelte';
  import { copyText } from '../lib/clipboard';
  import { toasts } from '../lib/toast.svelte';
  import type { Notice } from '../lib/api/types';

  let { placement = 'side' }: { placement?: 'side' | 'below' } = $props();

  let open = $state(false);
  let btnEl = $state<HTMLButtonElement | null>(null);
  let panelEl = $state<HTMLElement | null>(null);
  let panelH = $state(0);
  /** `anchor` = the bell's centre along the caret edge, in panel coordinates. */
  let pos = $state({ top: 0, left: 0, width: 360, maxHeight: 400, anchor: 20 });
  /** Which panel edge carries the caret (physical, so RTL just flips it). */
  let caretSide = $state<'left' | 'right' | 'top'>('left');
  /** Notice ids whose full detail is expanded. */
  let expanded = $state<Set<string>>(new Set());

  const PANEL_W = 360;
  const EDGE = 8;
  /** Room between the anchor edge and the panel — the caret sits in it. */
  const GAP = 10;
  /** Keep the caret's centre this far along its edge, so the whole caret (a
   *  14px square → ~20px base) stays clear of the 12px rounded corners. */
  const CARET_MIN = 22;
  /** Bodies longer than this get a clamped preview + "Show details". */
  const LONG_BODY = 140;

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
      // Beside the sidebar's edge (not the button's) so the panel never covers
      // the Navigator it was opened from; the caret bridges the short gap back
      // to the bell, which sits at the header's inline-end.
      const edge = btnEl.closest('nav')?.getBoundingClientRect() ?? r;
      left = rtl ? edge.left - GAP - width : edge.right + GAP;
      // Line the panel's head up with the bell (caret ~24px down).
      top = r.top + r.height / 2 - 24;
      caretSide = rtl ? 'right' : 'left';
    } else {
      left = rtl ? r.left : r.right - width;
      top = r.bottom + GAP;
      caretSide = 'top';
    }
    left = Math.min(Math.max(left, EDGE), vw - width - EDGE);
    // A bell near the window top (the Navigator header) may pull the panel a
    // few px above the usual edge margin, so the caret can still meet the
    // bell's centre without landing on the rounded corner.
    const minTop =
      caretSide === 'top' ? EDGE : Math.max(2, Math.min(EDGE, r.top + r.height / 2 - CARET_MIN));
    top = Math.min(Math.max(top, minTop), Math.max(minTop, vh - EDGE - 200));
    const maxHeight = Math.min(vh * 0.72, vh - top - EDGE);
    const anchor =
      caretSide === 'top' ? r.left + r.width / 2 - left : r.top + r.height / 2 - top;
    pos = { top, left, width, maxHeight, anchor };
  }

  // Caret offset along its edge, kept clear of the rounded corners (and of the
  // real rendered height, which is often below the max).
  const caretAt = $derived.by(() => {
    const span = caretSide === 'top' ? pos.width : panelH || pos.maxHeight;
    return Math.min(Math.max(pos.anchor, CARET_MIN), Math.max(CARET_MIN, span - CARET_MIN));
  });

  /** ui.modalCount right after we registered — anything above it is a layer
   *  opened on top of this panel. */
  let baseModals = 0;

  function isTopLayer(): boolean {
    return !ctxMenu.open && !confirmer.open && !ui.paletteOpen && ui.modalCount <= baseModals;
  }

  $effect(() => {
    if (!open) return;
    // untracked: place() reads/writes caretSide + pos; this effect keys on `open`.
    untrack(place);
    // Register as an overlay so the live browser's native webview (which
    // paints above the HTML) hides while the panel is up. untrack: pushModal
    // reads modalCount — see Modal.svelte.
    untrack(() => {
      ui.pushModal();
      baseModals = ui.modalCount;
    });
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === 'Escape' && isTopLayer()) {
        e.preventDefault();
        close();
      } else if (e.key === 'Tab' && isTopLayer()) {
        trapTab(e);
      }
    };
    window.addEventListener('resize', place);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('resize', place);
      window.removeEventListener('keydown', onKey);
      untrack(() => ui.popModal());
    };
  });

  // A sheet opened above us (palette, a Modal) would sit UNDER this panel's
  // layer — step aside instead of covering it.
  $effect(() => {
    if (open && (ui.modalCount > baseModals || ui.paletteOpen)) untrack(() => close(false));
  });

  // Focus the first row (else the first control) once the panel is painted.
  $effect(() => {
    if (!open) return;
    void tick().then(() => {
      if (!panelEl) return;
      const first =
        panelEl.querySelector<HTMLElement>('.nb-row') ??
        panelEl.querySelector<HTMLElement>('button:not([disabled])');
      first?.focus();
    });
  });

  const FOCUSABLE = 'button:not([disabled]), [href], [tabindex]:not([tabindex="-1"])';
  function trapTab(e: KeyboardEvent): void {
    if (!panelEl) return;
    const els = Array.from(panelEl.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
      (el) => el.offsetParent !== null,
    );
    if (els.length === 0) return;
    const idx = els.indexOf(document.activeElement as HTMLElement);
    if (e.shiftKey && idx <= 0) {
      e.preventDefault();
      els[els.length - 1].focus();
    } else if (!e.shiftKey && (idx === -1 || idx === els.length - 1)) {
      e.preventDefault();
      els[0].focus();
    }
  }

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
    if (open) close();
    else {
      now = Date.now();
      expanded = new Set();
      open = true;
    }
  }

  /** Close the panel; what was on screen is now seen → mark it read. Only
   *  the notices this client holds are marked (not the server-wide read-all),
   *  so one the daemon created but hasn't pushed yet stays unread. Focus
   *  returns to the bell unless the close was a row action that moved on. */
  function close(restoreFocus = true): void {
    if (!open) return;
    open = false;
    void notifications.markSeenRead();
    if (restoreFocus) btnEl?.focus();
  }

  function actionLabel(notice: Notice): string | null {
    switch (notice.action?.type) {
      case 'open_url':
        return 'Open link';
      case 'open_session':
        return 'Go to session';
      case 'reauth':
        return 'Re-authenticate';
      default:
        return null;
    }
  }

  function isLong(n: Notice): boolean {
    return n.body.length > LONG_BODY || n.body.includes('\n');
  }

  function toggleDetails(id: string): void {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

  function activate(row: NoticeRow): void {
    const n = row.latest;
    if (n.action) {
      void notifications.markManyRead(row.notices.map((x) => x.id));
      close(false);
      void notifications.runAction(n);
    } else if (row.sessionId === null && isLong(n)) {
      toggleDetails(n.id);
      void notifications.markRead(n.id);
    } else {
      void notifications.markManyRead(row.notices.map((x) => x.id));
    }
  }

  function rowLabel(row: NoticeRow): string {
    const parts = [row.title, row.text];
    // The severity glyph is aria-hidden — say it in words instead.
    if (row.severity === 'error') parts.unshift('Error');
    else if (row.severity === 'warn') parts.unshift('Warning');
    if (row.notices.length > 1) parts.push(`${row.notices.length} notifications`);
    if (row.unread) parts.push('unread');
    const act = actionLabel(row.latest);
    return act ? `${parts.join(', ')}. ${act}` : parts.join(', ');
  }

  async function copyDetail(n: Notice): Promise<void> {
    if (await copyText(n.body)) toasts.success('Copied to clipboard');
    else toasts.warn('Copy failed', 'The clipboard is not available here.');
  }

  function openMore(e: MouseEvent): void {
    // Anchor under the ⋯ button (a keyboard "click" carries no cursor point).
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    ctxMenu.show(new MouseEvent('click', { clientX: r.left, clientY: r.bottom + 4 }), [
      {
        label: 'Clear all notifications…',
        icon: 'trash',
        danger: true,
        disabled: notifications.notices.length === 0,
        action: () => void clearAll(),
      },
    ]);
  }

  async function clearAll(): Promise<void> {
    // The confirm sheet renders under this layer — step aside first.
    close();
    const n = notifications.notices.length;
    const ok = await confirmer.ask(
      `Delete all ${n} notification${n === 1 ? '' : 's'}? This can't be undone.`,
      { title: 'Clear all notifications', confirmLabel: 'Clear all' },
    );
    if (ok) await notifications.clear();
  }

  function relative(iso: string): string {
    const then = new Date(iso).getTime();
    if (Number.isNaN(then)) return '';
    const secs = Math.max(0, Math.round((now - then) / 1000));
    if (secs < 45) return 'now';
    const mins = Math.round(secs / 60);
    if (mins < 60) return `${mins}m`;
    const hrs = Math.round(mins / 60);
    if (hrs < 24) return `${hrs}h`;
    const days = Math.round(hrs / 24);
    if (days < 7) return `${days}d`;
    return new Date(then).toLocaleDateString([], { month: 'short', day: 'numeric' });
  }

  const SECTION: Record<NoticeRow['bucket'], string> = { 0: 'Needs you', 1: 'Alerts', 2: 'Recent' };
  const sections = $derived.by(() => {
    const out: { bucket: NoticeRow['bucket']; rows: NoticeRow[] }[] = [];
    for (const r of notifications.rows) {
      const last = out[out.length - 1];
      if (last && last.bucket === r.bucket) last.rows.push(r);
      else out.push({ bucket: r.bucket, rows: [r] });
    }
    return out;
  });

  const unreadCount = $derived(notifications.unreadRows);
  const badge = $derived(unreadCount > 99 ? '99+' : String(unreadCount));
  const allRead = $derived(notifications.notices.every((n) => n.read));
</script>

<div class="bell-wrap">
  <button
    bind:this={btnEl}
    class="icon-btn bell-btn"
    class:has-unread={unreadCount > 0}
    onclick={toggle}
    aria-label={unreadCount > 0 ? `Notifications, ${unreadCount} unread` : 'Notifications'}
    aria-haspopup="dialog"
    aria-expanded={open}
    title="Notifications"
  >
    <Icon name="bell" size={15} />
    {#if unreadCount > 0}
      <span class="badge sev-{notifications.unreadSeverity ?? 'info'}" aria-hidden="true">{badge}</span>
    {/if}
  </button>

  {#if open}
    <div class="bell-layer" use:portal>
      <!-- Backdrop closes the panel on any outside interaction. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="bell-backdrop"
        onclick={() => close()}
        oncontextmenu={(e) => { e.preventDefault(); close(); }}
        role="presentation"
      ></div>

      <div
        class="nb-pop"
        style="top:{pos.top}px;left:{pos.left}px;width:{pos.width}px;max-height:{pos.maxHeight}px"
      >
        <span
          class="nb-caret at-{caretSide} glass-raised"
          style="--caret:{caretAt}px"
          aria-hidden="true"
        ></span>
        <div
          bind:this={panelEl}
          bind:offsetHeight={panelH}
          class="panel glass-raised"
          role="dialog"
          aria-label="Notifications"
        >
          <header class="panel-head">
            <span class="panel-title">Notifications</span>
            <span class="grow"></span>
            <button
              class="btn ghost small"
              onclick={() => notifications.markAllRead()}
              disabled={allRead}
            >
              Mark all read
            </button>
            <button
              class="btn ghost small nb-more-btn"
              onclick={openMore}
              aria-label="More notification actions"
              title="More"
            >
              <Icon name="more" size={13} />
            </button>
          </header>

          <div class="panel-list">
            {#if notifications.error && notifications.notices.length === 0}
              <div class="panel-empty nb-error" role="alert">
                <Icon name="warning" size={20} />
                <p>Couldn't load notifications</p>
                <button class="btn small" onclick={() => notifications.load()} disabled={notifications.loading}>
                  <Icon name="refresh" size={12} /> Retry
                </button>
              </div>
            {:else if !notifications.loaded}
              <div class="panel-empty" aria-busy="true">
                <span class="nb-spinner" aria-hidden="true"></span>
                <p>Loading notifications…</p>
              </div>
            {:else if notifications.rows.length === 0}
              <div class="panel-empty">
                <Icon name="bell" size={22} />
                <p>You're all caught up</p>
              </div>
            {:else}
              {#each sections as sec (sec.bucket)}
                {#if sections.length > 1}
                  <div class="nb-section">{SECTION[sec.bucket]}</div>
                {/if}
                {#each sec.rows as row (row.key)}
                  {@const n = row.latest}
                  {@const long = row.sessionId === null && isLong(n)}
                  {@const act = actionLabel(n)}
                  <div class="nb-item" class:unread={row.unread}>
                    <button
                      class="nb-row"
                      onclick={() => activate(row)}
                      aria-label={rowLabel(row)}
                      title={act ?? (long ? (expanded.has(n.id) ? 'Hide details' : 'Show details') : undefined)}
                    >
                      <span class="nb-dot" aria-hidden="true"></span>
                      <span class="nb-lead" aria-hidden="true">
                        {#if row.provider}
                          <ProviderIcon provider={row.provider} size={16} />
                        {:else if row.severity === 'error'}
                          <span class="nb-sev err"><Icon name="warning" size={14} /></span>
                        {:else if row.severity === 'warn'}
                          <span class="nb-sev warn"><Icon name="warning" size={14} /></span>
                        {:else if row.sessionId}
                          <Icon name="command" size={14} />
                        {:else}
                          <Icon name={n.kind === 'credential' ? 'key' : 'bell'} size={14} />
                        {/if}
                      </span>
                      <span class="nb-main">
                        <span class="nb-line">
                          <span class="nb-title">{row.title}</span>
                          {#if row.provider && row.severity !== 'info'}
                            <span class="nb-sev {row.severity === 'error' ? 'err' : 'warn'}">
                              <Icon name="warning" size={12} />
                            </span>
                          {/if}
                          {#if row.notices.length > 1}
                            <span class="nb-count">×{row.notices.length}</span>
                          {/if}
                          <time class="nb-time" datetime={n.created_at}>{relative(n.created_at)}</time>
                        </span>
                        {#if row.text}
                          <span class="nb-text" class:one={row.sessionId !== null} title={row.sessionId ? row.text : undefined}>
                            {row.text}
                          </span>
                        {/if}
                      </span>
                    </button>
                    {#if long}
                      <button
                        class="nb-disclose"
                        aria-expanded={expanded.has(n.id)}
                        onclick={() => toggleDetails(n.id)}
                      >
                        <Icon name={expanded.has(n.id) ? 'chevronUp' : 'chevronDown'} size={11} />
                        {expanded.has(n.id) ? 'Hide details' : 'Show details'}
                      </button>
                      {#if expanded.has(n.id)}
                        <div class="nb-detail">
                          <pre>{n.body}</pre>
                          <button class="btn ghost small nb-copy" onclick={() => copyDetail(n)}>
                            <Icon name="copy" size={12} /> Copy
                          </button>
                        </div>
                      {/if}
                    {/if}
                    <button
                      class="nb-dismiss"
                      onclick={() =>
                        row.notices.length > 1
                          ? notifications.dismissMany(row.notices.map((x) => x.id))
                          : notifications.dismiss(n.id)}
                      aria-label={`Dismiss ${row.title}`}
                      title="Dismiss"
                    >
                      <Icon name="x" size={10} />
                    </button>
                  </div>
                {/each}
              {/each}
            {/if}
          </div>
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
  /* Coloured by the most severe UNREAD notice: accent for info, then warning /
     danger. The semantic colours are text-safe on --bg, so --bg on them is too. */
  .badge {
    position: absolute;
    top: -2px;
    inset-inline-end: -3px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 8px;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    font-size: var(--fs-xs);
    font-weight: 700;
    line-height: 16px;
    text-align: center;
    box-shadow: 0 0 0 1.5px var(--bg);
  }
  .badge.sev-warn {
    background: var(--warning);
    color: var(--bg);
  }
  .badge.sev-error {
    background: var(--danger);
    color: var(--bg);
  }

  /* Below the global context menu (9998/9999) so the ⋯ menu opens on top. */
  .bell-backdrop {
    position: fixed;
    inset: 0;
    z-index: calc(var(--z-popover-backdrop) - 2);
  }
  .nb-pop {
    /* top/left/width/max-height come from place() — clamped to the viewport. */
    position: fixed;
    z-index: calc(var(--z-popover-backdrop) - 1);
    display: flex;
    flex-direction: column;
  }
  .panel {
    display: flex;
    flex-direction: column;
    flex: 1 1 auto;
    min-height: 0;
    /* Raised glass (tokens.css .glass-raised). */
    border-radius: var(--radius-l);
    overflow: hidden;
  }

  /* The caret: a rotated glass square clipped to the half outside the panel,
     its diagonal laid 1px over the panel's edge so the hairline breaks under
     it. Only the two outer sides carry the border. */
  .nb-caret {
    position: absolute;
    width: 14px;
    height: 14px;
    z-index: 1;
    box-shadow: none;
    transform: rotate(45deg);
  }
  .nb-caret.at-left {
    left: -6px;
    top: calc(var(--caret) - 7px);
    border-width: 0 0 1px 1px;
    clip-path: polygon(0 0, 0 100%, 100% 100%);
  }
  .nb-caret.at-right {
    right: -6px;
    top: calc(var(--caret) - 7px);
    border-width: 1px 1px 0 0;
    clip-path: polygon(0 0, 100% 0, 100% 100%);
  }
  .nb-caret.at-top {
    top: -6px;
    left: calc(var(--caret) - 7px);
    border-width: 1px 0 0 1px;
    clip-path: polygon(0 0, 100% 0, 0 100%);
  }

  @media (prefers-reduced-motion: no-preference) {
    .nb-pop {
      animation: nb-in 140ms ease-out;
    }
  }
  @keyframes nb-in {
    from {
      opacity: 0;
    }
  }

  .panel-head {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-block: 8px;
    padding-inline: 14px 8px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .panel-title {
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .nb-more-btn {
    padding: 0 6px;
  }

  .panel-list {
    overflow-y: auto;
    flex: 1;
    min-height: 0;
    padding-block: 4px;
  }

  .panel-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 36px 24px;
    color: var(--text-dim);
  }
  .panel-empty p {
    margin: 0;
    font-size: var(--fs-m);
  }
  .nb-error {
    color: var(--danger);
  }
  .nb-error p {
    color: var(--text);
  }
  .nb-spinner {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
  }
  @media (prefers-reduced-motion: no-preference) {
    .nb-spinner {
      animation: nb-spin 0.8s linear infinite;
    }
  }
  @keyframes nb-spin {
    to {
      transform: rotate(360deg);
    }
  }

  .nb-section {
    padding: 8px 14px 3px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }

  .nb-item {
    position: relative;
    margin: 0 4px;
    border-radius: var(--radius-m);
  }
  .nb-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    width: 100%;
    min-height: 48px;
    padding-block: 8px;
    /* The trailing gutter holds the dismiss button. */
    padding-inline: 4px 30px;
    border: none;
    border-radius: inherit;
    background: transparent;
    color: var(--text);
    text-align: start;
    font: inherit;
    cursor: pointer;
  }
  .nb-row:hover {
    background: var(--hover);
  }
  .nb-row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .nb-dot {
    width: 7px;
    height: 7px;
    margin-top: 6px;
    border-radius: 50%;
    flex-shrink: 0;
    background: transparent;
  }
  .nb-item.unread .nb-dot {
    background: var(--accent);
  }
  .nb-lead {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .nb-sev {
    display: inline-grid;
    place-items: center;
    flex-shrink: 0;
  }
  .nb-sev.warn {
    color: var(--warning);
  }
  .nb-sev.err {
    color: var(--danger);
  }
  .nb-main {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
    min-width: 0;
  }
  .nb-line {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .nb-title {
    flex: 0 1 auto;
    min-width: 0;
    font-size: var(--fs-m);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .nb-item.unread .nb-title {
    color: var(--text);
    font-weight: 600;
  }
  .nb-count {
    flex-shrink: 0;
    padding: 0 5px;
    border-radius: 7px;
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    line-height: 16px;
  }
  .nb-time {
    margin-inline-start: auto;
    flex-shrink: 0;
    padding-inline-start: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .nb-text {
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.4;
    word-break: break-word;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
  }
  .nb-text.one {
    -webkit-line-clamp: 1;
    line-clamp: 1;
  }

  .nb-disclose {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin: -4px 0 6px;
    margin-inline-start: 41px;
    padding: 1px 4px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .nb-disclose:hover {
    background: var(--hover);
  }
  .nb-detail {
    position: relative;
    margin: 0 8px 8px;
    margin-inline-start: 41px;
  }
  .nb-detail pre {
    margin: 0;
    max-height: 160px;
    overflow: auto;
    padding: 8px 10px;
    padding-inline-end: 64px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface-2);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .nb-copy {
    position: absolute;
    top: 4px;
    inset-inline-end: 4px;
  }

  .nb-dismiss {
    position: absolute;
    top: 8px;
    inset-inline-end: 6px;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border: none;
    border-radius: 4px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
  }
  @media (prefers-reduced-motion: no-preference) {
    .nb-dismiss {
      transition: opacity 120ms ease-out, background 120ms ease-out;
    }
  }
  .nb-item:hover .nb-dismiss,
  .nb-item:focus-within .nb-dismiss {
    opacity: 1;
  }
  .nb-dismiss:hover {
    background: var(--hover);
    color: var(--text);
  }
  .nb-dismiss:focus-visible {
    opacity: 1;
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }
</style>
