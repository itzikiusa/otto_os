<script lang="ts">
  import type { SessionStatus } from '../api/types';

  interface Props {
    status: SessionStatus;
    size?: number;
  }
  let { status, size = 7 }: Props = $props();

  const titles: Record<SessionStatus, string> = {
    running: 'running',
    working: 'working',
    idle: 'idle',
    exited: 'exited',
    reconnectable: 'reconnectable',
  };
</script>

<span
  class="dot {status}"
  style="width:{size}px;height:{size}px"
  title={titles[status]}
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
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.45;
    }
  }
</style>
