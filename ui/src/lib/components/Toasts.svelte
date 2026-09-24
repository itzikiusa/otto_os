<script lang="ts">
  import { toasts } from '../toast.svelte';
</script>

<div class="toasts" aria-live="polite">
  {#each toasts.toasts as t (t.id)}
    <div class="toast {t.level}">
      <div class="toast-stripe"></div>
      <div class="toast-content">
        <div class="toast-title">{t.title}</div>
        {#if t.body}<div class="toast-body">{t.body}</div>{/if}
      </div>
      <button class="icon-btn" onclick={() => toasts.dismiss(t.id)} aria-label="Dismiss">✕</button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    inset-inline-end: 16px;
    bottom: 38px;
    z-index: var(--z-toast);
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 320px;
    /* A burst of toasts must never stack past the top edge: cap the column and
       let it scroll (newest stay visible at the bottom anchor). A % of this
       fixed box's containing block is the WINDOW height; 100vh in the
       WKWebView is the screen's and let the stack run off the top. */
    max-height: calc(100% - 54px);
    overflow-y: auto;
    overscroll-behavior: contain;
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 10px 10px 0;
    /* Opaque, but the same edge + elevation as every floating layer. */
    background: var(--surface);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-m);
    box-shadow: var(--glass-shadow);
    animation: toast-in 160ms ease-out;
    overflow: hidden;
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
  .toast-content {
    flex: 1;
    min-width: 0;
  }
  .toast-title {
    font-size: 12.5px;
    font-weight: 600;
  }
  .toast-body {
    font-size: 12px;
    color: var(--text-dim);
    margin-top: 2px;
    word-break: break-word;
  }
  @keyframes toast-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
</style>
