<script module lang="ts">
  /** The word the tag shows (DELETE/OPTIONS shortened to fit the column) —
   *  also what a list's ⌘F `findText` should use for this cell. */
  export function methodWord(method: string): string {
    const m = (method || 'GET').toUpperCase();
    return m === 'DELETE' ? 'DEL' : m === 'OPTIONS' ? 'OPT' : m;
  }
</script>

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

<span class="mtag {methodTone(m)}" class:fixed>{methodWord(m)}</span>

<style>
  .mtag {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
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
