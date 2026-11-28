<script lang="ts">
  // ListDetail renders its `list` snippet inside this: the ListPane there —
  // and only there — is that ListDetail's list (its name, "Hide <name> ⌘B",
  // the phone list-page header). A ListPane anywhere else on the page (a
  // second-level list in the content or a Detail pane: the IDE's Git log,
  // usages) is on its own; ListDetail clears LIST_HOST for its subtree.
  import { setContext, type Snippet } from 'svelte';
  import { LIST_HOST, type ListDetailCtx } from './context';

  let { ctx, children }: { ctx: ListDetailCtx; children: Snippet } = $props();
  // svelte-ignore state_referenced_locally
  setContext(LIST_HOST, ctx);
</script>

{@render children()}
