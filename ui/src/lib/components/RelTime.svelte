<script lang="ts">
  // A timestamp as people read it: relative ("3m ago", "in 14h") with the exact
  // local time on hover and the machine value in `datetime`. Ticks with the
  // shared clock (stores/now.svelte.ts). Never render a raw ISO string.
  import { rel } from '../stores/now.svelte';

  interface Props {
    iso: string | number | null | undefined;
    /** Shown when `iso` is missing/unparseable. */
    fallback?: string;
    class?: string;
  }
  let { iso, fallback = '—', class: klass = '' }: Props = $props();

  const ms = $derived(iso == null || iso === '' ? NaN : typeof iso === 'number' ? iso : Date.parse(iso));
  const valid = $derived(Number.isFinite(ms) && ms > 0);
</script>

{#if valid}
  <time class={klass} datetime={new Date(ms).toISOString()} title={new Date(ms).toLocaleString()}>{rel(ms)}</time>
{:else}
  <span class={klass}>{fallback}</span>
{/if}
