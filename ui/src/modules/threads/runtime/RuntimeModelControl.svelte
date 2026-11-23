<script lang="ts">
  // The composer's model chip ("Opus 5.5 · Medium" with the provider glyph)
  // and the model / effort picker it opens (lazy).
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { formatEffortShort, formatModelLabel } from '../../../lib/providerModels';
  import { harnessLabel } from '../../../lib/providers';
  import { auth } from '../../../lib/stores/auth.svelte';
  import type { RuntimeSession } from '../../../lib/api/types';
  import type { EffortLevel } from '../../../lib/providerModels';
  import { resolveEffectiveDraftModel } from './runtimeComposer';
  import RuntimeModelChip from './RuntimeModelChip.svelte';

  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));

  interface Props {
    provider: string;
    session: RuntimeSession | null | undefined;
    /** The thread's draft: its model / effort pick. */
    draft: { model: string | null | undefined; effort: EffortLevel | null };
    efforts: string[];
    disabled: boolean;
    testId: string;
  }
  let { provider, session, draft, efforts, disabled, testId }: Props = $props();
  let open = $state(false);
  let el = $state<HTMLButtonElement | null>(null);
  const chosen = $derived(resolveEffectiveDraftModel(draft.model, session?.model));
  const label = $derived.by(() => {
    const name = chosen ? formatModelLabel(provider, chosen, auth.meta) : harnessLabel(provider);
    const effort = draft.effort ?? session?.effort ?? null; // the pick, else what the thread stored
    return effort ? `${name} · ${formatEffortShort(effort)}` : name;
  });
</script>

<RuntimeModelChip
  bind:el
  {provider}
  {label}
  expanded={open}
  {disabled}
  showIcon={!!session}
  {testId}
  onprefetch={() => ModelPicker.prefetch()}
  onopen={() => {
    if (ModelPicker.error) ModelPicker.retry();
    open = true;
  }}
/>
{#if open && el && ModelPicker.component}
  <ModelPicker.component
    anchor={el}
    providers={[provider]}
    {provider}
    lockProvider
    model={chosen}
    effectiveModel={session?.model ?? null}
    effort={draft.effort}
    mode="full"
    onmodel={(m) => (draft.model = m)}
    oneffort={efforts.length ? (e) => (draft.effort = e) : undefined}
    effortLevels={efforts}
    onclose={() => (open = false)}
  />
{/if}
