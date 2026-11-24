<script lang="ts">
  // The composer's model chip ("Opus 5.5 · Medium" with the provider glyph)
  // and the model / effort picker it opens (lazy). Below the thread's own
  // provider the picker lists the others ("Continue with another provider"):
  // a model picked there arms a switch for the next message (providerSwitch.ts).
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { whenIdle } from '../../../lib/lazy-component.svelte';
  import { formatEffortShort, formatModelLabel } from '../../../lib/providerModels';
  import { agentProviders, harnessLabel, isRuntimeProvider, orderThreadHarnesses, providerReadiness } from '../../../lib/providers';
  import { auth } from '../../../lib/stores/auth.svelte';
  import type { RuntimeSession } from '../../../lib/api/types';
  import type { EffortLevel } from '../../../lib/providerModels';
  import { resolveEffectiveDraftModel } from './runtimeComposer';
  import { armSwitch, switchTargets } from './providerSwitch';
  import { runtimeViews } from './runtimeView.svelte';
  import RuntimeModelChip from './RuntimeModelChip.svelte';

  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));
  // Loaded once the provider has reported a context window (most turns start without one).
  const ContextMeter = lazyComponent(() => import('./RuntimeContextMeter.svelte'));

  interface Props {
    provider: string;
    session: RuntimeSession | null | undefined;
    /** The thread's draft: its model / effort pick and an armed switch. */
    draft: { model: string | null | undefined; effort: EffortLevel | null; provider: string | null };
    efforts: string[];
    disabled: boolean;
    testId: string;
    /** The context meter's "Compact the conversation": type the agent's command. */
    oncompact?: (command: string) => void;
  }
  let { provider, session, draft, efforts, disabled, testId, oncompact }: Props = $props();
  // How full the context is, beside the chip (RuntimeContextMeter).
  const view = $derived(session ? runtimeViews.projections[session.id] : null);
  let open = $state(false);
  let el = $state<HTMLButtonElement | null>(null);
  const armed = $derived(draft.provider && draft.provider !== provider ? draft.provider : null);
  const shown = $derived(armed ?? provider);
  const chosen = $derived(armed ? (draft.model ?? null) : resolveEffectiveDraftModel(draft.model, session?.model));
  const targets = $derived(
    switchTargets(provider, orderThreadHarnesses(agentProviders()), (p) => isRuntimeProvider(p, auth.meta) && providerReadiness(p).available),
  );
  const label = $derived.by(() => {
    const name = chosen ? formatModelLabel(shown, chosen, auth.meta) : harnessLabel(shown);
    const effort = draft.effort ?? (armed ? null : session?.effort ?? null); // the pick, else what the thread stored
    return effort ? `${name} · ${formatEffortShort(effort)}` : name;
  });
  // The context meter waits for the thread's first paint (whenIdle).
  let meterIdle = $state(false);
  $effect(() => whenIdle(() => (meterIdle = true), 300));
  function pickProvider(p: string): void {
    armSwitch(draft, provider, p);
  }
  // A model of another provider picked: the switch is armed — the picker
  // closes so the "Next message continues on …" line is in sight.
</script>

{#if meterIdle && view?.thread?.context_usage?.max_tokens && ContextMeter.component}<ContextMeter.component usage={view.thread.context_usage} compact={view.commands.some((c) => c.name === 'compact') ? 'compact' : null} {oncompact} />{/if}
<RuntimeModelChip
  bind:el
  provider={shown}
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
    providers={targets}
    provider={shown}
    home={provider}
    model={chosen}
    effectiveModel={armed ? null : (session?.model ?? null)}
    effort={draft.effort}
    mode="full"
    onprovider={pickProvider}
    onmodel={(m) => { draft.model = m; if (armed) open = false; }}
    oneffort={efforts.length || armed ? (e) => (draft.effort = e) : undefined}
    effortLevels={armed ? undefined : efforts}
    onclose={() => (open = false)}
  />
{/if}
