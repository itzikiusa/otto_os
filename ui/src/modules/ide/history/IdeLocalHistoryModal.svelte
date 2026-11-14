<script lang="ts">
  // Thin wrapper preserving the `IdeLocalHistoryModal` component contract while
  // delegating to the full interactive `IdeComparisonModal` (`local-history` mode).
  import IdeComparisonModal from './IdeComparisonModal.svelte';

  export type { IdeLocalHistoryEntry } from './ideComparisonLoader';

  interface Props {
    root: string;
    targetPath: string;
    isDir: boolean;
    initialFilter?: 'all' | 'deleted';
    onclose: () => void;
  }

  let { root, targetPath, isDir, initialFilter = 'all', onclose }: Props = $props();
</script>

<IdeComparisonModal
  mode="local-history"
  {root}
  {targetPath}
  {isDir}
  {initialFilter}
  {onclose}
/>
