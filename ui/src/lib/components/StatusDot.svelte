<script lang="ts">
  import type { SessionStatus } from '../api/types';

  interface Props {
    status: SessionStatus;
    size?: number;
    /** When true, renders an amber pulsing dot regardless of `status` — the
     *  session is blocked on operator input (distinct from plain idle). */
    needsYou?: boolean;
  }
  let { status, size = 7, needsYou = false }: Props = $props();

  const titles: Record<SessionStatus, string> = {
    running: 'running',
    working: 'working',
    idle: 'idle',
    exited: 'exited',
    reconnectable: 'reconnectable',
  };

  const title = $derived(needsYou ? 'needs you — waiting on operator input' : titles[status]);
</script>

<span
  class="dot {needsYou ? 'needs-you' : status}"
  style="width:{size}px;height:{size}px"
  title={title}
></span>

<style>
  .dot {
    display: inline-block;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--status-idle);
    transition: background 150ms ease-out;
  }
  .dot.working {
    background: var(--status-working);
    animation: pulse 1.6s ease-in-out infinite;
  }
  .dot.running {
    background: var(--accent);
  }
  .dot.exited {
    background: var(--status-exited);
  }
  .dot.reconnectable {
    background: #febc2e;
  }
  /* "Needs you" — blocked on operator input. Amber pulse distinct from
     the green "working" pulse so the two states read differently at a glance. */
  .dot.needs-you {
    background: #febc2e;
    animation: needs-you-pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.45;
    }
  }
  @keyframes needs-you-pulse {
    0%,
    100% {
      opacity: 1;
      transform: scale(1.15);
    }
    50% {
      opacity: 0.5;
      transform: scale(0.85);
    }
  }
</style>
