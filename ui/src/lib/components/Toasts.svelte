<script lang="ts">
  import { toasts, type Toast } from '../toast.svelte';
  import Icon, { type IconName } from './Icon.svelte';

  // The stripe carries the tone, and the glyph repeats it (colour is never
  // the only signal).
  const ICON: Record<Toast['level'], IconName> = {
    info: 'info',
    success: 'check',
    warn: 'warning',
    error: 'warning',
  };
</script>

<div class="toasts" aria-live="polite">
  {#each toasts.toasts as t (t.id)}
    <!-- An error interrupts (assertive); everything else waits its turn. The
         timer holds while the pointer or focus is on the toast. -->
    <div
      class="toast {t.level}"
      role={t.level === 'error' ? 'alert' : 'status'}
      onmouseenter={() => toasts.pause(t.id)}
      onmouseleave={() => toasts.resume(t.id)}
      onfocusin={() => toasts.pause(t.id)}
      onfocusout={() => toasts.resume(t.id)}
    >
      <div class="toast-stripe"></div>
      <span class="toast-icon" aria-hidden="true"><Icon name={ICON[t.level]} size={14} /></span>
      <div class="toast-content">
        <div class="toast-title">
          {t.title}
          {#if t.count > 1}<span class="toast-count" title={`Happened ${t.count} times`}>×{t.count}</span>{/if}
        </div>
        {#if t.body}<div class="toast-body">{t.body}</div>{/if}
        {#if t.action}
          <button class="btn small toast-action" onclick={() => void toasts.runAction(t.id)}>{t.action.label}</button>
        {/if}
      </div>
      <button class="icon-btn" onclick={() => toasts.dismiss(t.id)} aria-label="Dismiss" title="Dismiss">
        <Icon name="x" size={12} />
      </button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    inset-inline-end: 16px;
    /* --toast-lift: the floating command bar raises the stack while its pill
       floats over the content column, so a toast never covers it. */
    bottom: calc(38px + var(--toast-lift));
    z-index: var(--z-toast);
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 320px;
    /* A burst of toasts must never stack past the top edge: cap the column and
       let it scroll (newest stay visible at the bottom anchor). A % of this
       fixed box's containing block is the WINDOW height; 100vh in the
       WKWebView is the screen's and let the stack run off the top. */
    max-height: calc(100% - 54px - var(--toast-lift));
    overflow-y: auto;
    overscroll-behavior: contain;
    transition: bottom 160ms ease-out;
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 10px 10px 0;
    /* Opaque, but the same edge + elevation as every floating layer. */
    background: var(--surface);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-m);
    box-shadow: var(--glass-shadow);
    animation: toast-in 160ms ease-out;
    overflow: hidden;
    flex-shrink: 0;
  }
  .toast-stripe {
    align-self: stretch;
    width: 3px;
    border-radius: 2px;
    margin-inline-start: 0;
    background: var(--accent);
  }
  .toast.success .toast-stripe {
    background: var(--status-working);
  }
  .toast.warn .toast-stripe {
    background: var(--status-warn);
  }
  .toast.error .toast-stripe {
    background: var(--status-exited);
  }
  .toast-icon {
    display: grid;
    place-items: center;
    height: 18px;
    flex-shrink: 0;
    color: var(--accent-text);
  }
  .toast.success .toast-icon {
    color: var(--success);
  }
  .toast.warn .toast-icon {
    color: var(--warning);
  }
  .toast.error .toast-icon {
    color: var(--danger);
  }
  .toast-content {
    flex: 1;
    min-width: 0;
  }
  .toast-title {
    font-size: var(--fs-m);
    font-weight: 600;
    line-height: 18px;
    overflow-wrap: anywhere;
  }
  .toast-count {
    margin-inline-start: 6px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .toast-body {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin-top: 2px;
    /* Multi-line causes (git's "your changes remain stashed — …") keep their
       line breaks; long paths still wrap. */
    white-space: pre-line;
    overflow-wrap: anywhere;
  }
  .toast-action {
    margin-top: 8px;
  }
  /* Phone: the bottom nav owns the window's bottom edge (the status bar is
     usually gone there), so the stack sits above it and spans the width. */
  @media (max-width: 640px) {
    .toasts {
      bottom: calc(var(--mobile-bottomnav-h) + env(safe-area-inset-bottom, 0px) + 8px);
      inset-inline: 12px;
      width: auto;
    }
  }
  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .toast {
      animation: none;
    }
    .toasts {
      transition: none;
    }
  }
</style>
