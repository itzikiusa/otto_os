<script lang="ts">
  // Session status dot. Every surface routes through `sessionState()` (lib/
  // status.ts) so one state draws one dot everywhere: pass `state` when the
  // caller already derived it (sidebar, tabs, tiles, pane header), or the raw
  // `status` + `needsYou` and the dot derives it. Amber is reserved for
  // "needs you"; a suspended (resumable) session is a hollow idle ring.
  import type { SessionStatus } from '../api/types';
  import { sessionState, type SessionStateInfo } from '../status';

  interface Props {
    status?: SessionStatus;
    size?: number;
    /** When true, renders an amber pulsing dot regardless of `status` — the
     *  session is blocked on operator input (distinct from plain idle). */
    needsYou?: boolean;
    /** A pre-derived state (wins over `status`/`needsYou`). */
    state?: SessionStateInfo;
  }
  let { status = 'idle', size = 7, needsYou = false, state }: Props = $props();

  const info = $derived(state ?? sessionState(null, status, needsYou));
</script>

<span
  class="dot {info.key}"
  class:live={info.live}
  style="width:{size}px;height:{size}px"
  role="img"
  aria-label={info.label}
  title={info.hint ?? info.label}
  data-state={info.key}
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
  }
  .dot.running {
    background: var(--accent);
  }
  .dot.failed {
    background: var(--status-exited);
  }
  /* Ended: stopped for good, nothing wrong — a faded idle dot. */
  .dot.ended {
    opacity: 0.5;
  }
  /* Suspended / resumable: a hollow idle ring — calm, "parked", not an alert. */
  .dot.suspended {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px var(--status-idle);
  }
  /* Stale (events socket down): the last live state, no longer trusted. */
  .dot.stale {
    background: transparent;
    box-shadow: inset 0 0 0 1.5px var(--status-working);
  }
  /* "Needs you" — blocked on operator input. The ONLY amber dot. */
  .dot.needs-you {
    background: var(--status-warn);
  }
  .dot.working.live {
    animation: pulse 1.6s ease-in-out infinite;
  }
  .dot.needs-you.live {
    animation: needs-you-pulse 1.2s ease-in-out infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .dot.live {
      animation: none !important;
    }
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
