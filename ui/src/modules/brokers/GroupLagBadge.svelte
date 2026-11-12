<script lang="ts">
  import type { GroupSummary } from '../../lib/api/types';
  import { lagBadge } from './brokerFigures';

  interface Props {
    group: GroupSummary;
  }
  let { group }: Props = $props();

  // Unknown lag reads "—" (pending "…"), never a 0 that looks caught up.
  const badge = $derived(lagBadge(group.total_lag, group.lag_pending, group.protocol_type));
</script>

<span class="lag" class:has-lag={(group.total_lag ?? 0) > 0} title={badge.title}>{badge.text}</span>

<style>
  .lag {
    color: var(--text-dim);
    white-space: nowrap;
  }
  .lag.has-lag {
    color: var(--warning);
    font-weight: 600;
  }
</style>
