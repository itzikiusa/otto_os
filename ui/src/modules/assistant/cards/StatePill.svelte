<script lang="ts" module>
  // The Assistant's one tone mapping (the McpPill pattern): a word in a soft
  // tinted pill. Tone comes from the domain helpers in ../model.ts, never
  // picked at the call site.
  export type Tone = 'ok' | 'warn' | 'bad' | 'info' | 'neutral';
</script>

<script lang="ts">
  interface Props {
    tone: Tone;
    label: string;
    /** Adds a pulsing dot — only for live state (running now). */
    live?: boolean;
    title?: string;
  }
  let { tone, label, live = false, title }: Props = $props();
</script>

<span class="pill {tone}" {title} data-tone={tone}>
  {#if live}<span class="live" aria-hidden="true"></span>{/if}{label}
</span>

<style>
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding: 0 8px;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 600;
    white-space: nowrap;
    border: 1px solid transparent;
  }
  .pill.ok {
    color: var(--success);
    background: var(--success-soft);
  }
  .pill.warn {
    color: var(--warning);
    background: var(--warning-soft);
  }
  .pill.bad {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .pill.info {
    color: var(--info);
    background: var(--info-soft);
  }
  .pill.neutral {
    color: var(--text-dim);
    background: var(--surface-2);
    border-color: var(--border);
  }
  .live {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    animation: pill-pulse 1.4s ease-in-out infinite;
  }
  @keyframes pill-pulse {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .live {
      animation: none;
    }
  }
</style>
