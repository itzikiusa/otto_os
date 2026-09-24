<script lang="ts">
  // The HTTP method as a small mono word, toned by what it does (read /
  // create / change / destroy — see methodTone). The word itself is the
  // signal; colour only reinforces it.
  import { methodTone } from '../../lib/api/apiVars';

  interface Props {
    method: string;
    /** Fixed-width column in lists so names line up. */
    fixed?: boolean;
  }
  let { method, fixed = false }: Props = $props();
  const m = $derived((method || 'GET').toUpperCase());
</script>

<span class="mtag {methodTone(m)}" class:fixed>{m === 'DELETE' ? 'DEL' : m === 'OPTIONS' ? 'OPT' : m}</span>

<style>
  .mtag {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .fixed {
    width: 36px;
  }
  .get { color: var(--success); }
  .post { color: var(--info); }
  .put { color: var(--warning); }
  .delete { color: var(--danger); }
</style>
