<script lang="ts">
  // Thin module re-export of shared BlamePanel (windowed via tw.findRows()).
  import SharedBlamePanel from '../../lib/diff/BlamePanel.svelte';

  interface Props {
    repoId: string;
    path: string;
    rev?: string;
    onclose: () => void;
    onfocuscommit?: (sha: string) => void;
  }
  let props: Props = $props();
</script>

<SharedBlamePanel {...props} />
