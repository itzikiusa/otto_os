<script lang="ts">
  // `#/workbench` — the old Scratches page. Scratches are now a project in the
  // IDE (the same folder in every workspace), so the route forwards there.
  import { router } from '../../lib/router.svelte';

  router.replace('ide?scratches=1');
</script>
