<script lang="ts">
  // Shared Mobile SDK readiness diagnostic cards with properly gated 1-click
  // built-in installer CTAs and copyable setup steps.
  import Badge from '../components/Badge.svelte';
  import Icon from '../components/Icon.svelte';
  import { copyWithToast } from '../clipboard';
  import type { SimInstallTarget, SimPlatformReadiness } from '../api/types';
  import { canOfferAndroidInstall, canOfferIosRuntimeDownload } from './simInstaller.svelte';

  interface Props {
    items: readonly SimPlatformReadiness[];
    installDisabled?: boolean;
    startingTarget?: SimInstallTarget | null;
    maxSteps?: number;
    oninstallandroid: () => void;
    oninstallios: () => void;
  }

  let {
    items,
    installDisabled = false,
    startingTarget = null,
    maxSteps = 4,
    oninstallandroid,
    oninstallios,
  }: Props = $props();
</script>

{#if items.length > 0}
  <div class="sim-diag-list" role="region" aria-label="Mobile SDK setup diagnostics">
    {#each items as plat (plat.platform)}
      <div class="sim-diag-card">
        <div class="sim-diag-head">
          <Badge
            tone={plat.state === 'degraded' ? 'warn' : 'neutral'}
            label={plat.platform === 'ios' ? 'iOS Simulator' : 'Android Emulator'}
          />
          <span class="sim-diag-summary">{plat.summary}</span>
          <div class="sim-diag-cta">
            {#if plat.platform === 'android' && canOfferAndroidInstall(plat)}
              <button
                type="button"
                class="btn small primary"
                disabled={installDisabled}
                onclick={oninstallandroid}
                data-testid="sim-install-android"
              >
                <Icon name="download" size={12} />
                {startingTarget === 'android_emulator' ? 'Starting…' : 'Install Android Emulator'}
              </button>
            {:else if plat.platform === 'ios' && canOfferIosRuntimeDownload(plat)}
              <button
                type="button"
                class="btn small primary"
                disabled={installDisabled}
                onclick={oninstallios}
                data-testid="sim-install-ios"
              >
                <Icon name="download" size={12} />
                {startingTarget === 'ios_runtime' ? 'Starting…' : 'Download iOS Runtime'}
              </button>
            {/if}
          </div>
        </div>
        {#if plat.setup_steps.length > 0}
          <div class="sim-diag-steps">
            {#each plat.setup_steps.slice(0, maxSteps) as step (step.title)}
              <div class="sim-step-row">
                <div class="sim-step-head">
                  <span class="sim-step-title">{step.title}</span>
                  {#if step.command}
                    <button
                      type="button"
                      class="icon-btn sim-step-copy"
                      aria-label={`Copy ${step.title} command`}
                      title="Copy command"
                      onclick={() => step.command && void copyWithToast(step.command, step.title)}
                    >
                      <Icon name="copy" size={12} />
                    </button>
                  {/if}
                </div>
                {#if step.detail}
                  <p class="sim-step-detail">{step.detail}</p>
                {/if}
                {#if step.command}
                  <code class="sim-step-cmd" dir="ltr">{step.command}</code>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .sim-diag-list {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    min-width: 0;
  }
  .sim-diag-card {
    flex: 1 1 260px;
    min-inline-size: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .sim-diag-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .sim-diag-summary {
    font-size: var(--fs-xs);
    color: var(--text);
    flex: 1;
    min-inline-size: 140px;
  }
  .sim-diag-cta {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .sim-diag-steps {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--bg);
    border: 1px solid var(--border);
  }
  .sim-step-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .sim-step-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .sim-step-title {
    font-size: var(--fs-s);
    font-weight: 500;
  }
  .sim-step-detail {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .sim-step-cmd {
    display: block;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    background: var(--surface);
    border: 1px solid var(--border);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    overflow-x: auto;
    white-space: nowrap;
  }
  .sim-step-copy {
    width: 24px;
    height: 24px;
    flex-shrink: 0;
  }
</style>
