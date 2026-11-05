<script lang="ts">
  // The one place the "Continue on phone" sheet renders (mounted by the
  // shell); pane headers and ⋯ menus only call `phoneHandoff.open(...)`. The
  // sheet (and its qrcode chunk) loads on first use.
  import { lazyComponent } from '../lazy-component.svelte';
  import { phoneHandoff } from '../stores/phoneHandoff.svelte';
  import LazyMount from './LazyMount.svelte';

  const Sheet = lazyComponent(() => import('./PhoneHandoffModal.svelte'));
  const req = $derived(phoneHandoff.request);
</script>

{#if req}
  <LazyMount
    lazy={Sheet}
    what="Continue on phone"
    quiet
    onerror={() => phoneHandoff.close()}
    props={{ route: req.route, what: req.what, onclose: () => phoneHandoff.close() }}
  />
{/if}
