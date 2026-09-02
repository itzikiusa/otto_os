<script lang="ts">
  // Environment pill — same treatment as the connections hub: dev/staging in
  // the working tint, prod in the red (exited) tint.
  import type { Environment } from '../../lib/api/types';

  interface Props {
    env: Environment | string;
  }
  let { env }: Props = $props();
</script>

<span class="env-pill mono" class:prod={env === 'prod'} data-env={env}>{env}</span>

<style>
  .env-pill {
    flex-shrink: 0;
    font-size: 8.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 1px 6px;
    border-radius: 999px;
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .env-pill.prod {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
  }
</style>
