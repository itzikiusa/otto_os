<script lang="ts">
  // What the NEXT message will do, with its Cancel — "Next message continues
  // on Codex" (an armed provider switch, providerSwitch.ts) or "Next message
  // starts fresh" — one line above the composer, never folded (banners.ts).
  import { announceShown } from '../../../lib/announce.svelte';
  import { armedSwitchLine } from './providerSwitch';

  interface Props {
    kind: 'switch' | 'fresh';
    draft: { provider: string | null; model: string | null | undefined; effort: unknown; freshContext: boolean };
    onfocus: () => void;
  }
  let { kind, draft, onfocus }: Props = $props();
  const text = $derived(kind === 'switch' && draft.provider ? armedSwitchLine(draft.provider) : 'Next message starts fresh');

  function cancel(): void {
    if (kind === 'switch') {
      draft.provider = null;
      draft.model = undefined;
      draft.effort = null;
    } else draft.freshContext = false;
    onfocus();
  }
</script>

<div class="rt-fresh" data-testid={kind === 'switch' ? 'runtime-switch-notice' : 'runtime-fresh-notice'} use:announceShown={text}>
  <span>{text}</span>
  <button type="button" class="btn small ghost" onclick={cancel}>Cancel</button>
</div>

<style>
  .rt-fresh {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
