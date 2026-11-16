<script lang="ts">
  // A thread's reading tools over its stream: ⌘F finds in the whole loaded
  // thread (not only its mounted rows), and quoting a passage of an answer
  // puts it in the composer with the caret below it.
  import { tick } from 'svelte';
  import SelectionQuote from '../../../lib/components/SelectionQuote.svelte';
  import { registerFindProvider } from '../../../lib/findProviders';
  import { withQuote } from '../../../lib/quote/quote';
  import { threadFindProvider } from './threadFind';
  import type { TimelineRow } from './timeline';

  interface Props {
    root: HTMLElement | null;
    rows: TimelineRow[];
    rowCap: number;
    text: string;
    composer: HTMLTextAreaElement | null;
  }
  let { root, rows, rowCap = $bindable(), text = $bindable(), composer }: Props = $props();

  $effect(() => registerFindProvider(threadFindProvider({ root: () => root, rows: () => rows, cap: () => rowCap, setCap: (n) => (rowCap = n) })));

  function quote(passage: string): void {
    text = withQuote(text, passage);
    composer?.focus({ preventScroll: true });
    void tick().then(() => composer?.setSelectionRange(text.length, text.length));
  }
</script>

<SelectionQuote {root} within=".rt-assistant" onquote={quote} />
