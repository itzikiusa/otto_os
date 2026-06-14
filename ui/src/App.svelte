<script lang="ts">
  // Boot flow: GET /meta → onboarding wizard | login | main shell.
  import Shell from './shell/App.svelte';
  import Onboarding from './modules/settings/Onboarding.svelte';
  import Login from './modules/settings/Login.svelte';
  import Toasts from './lib/components/Toasts.svelte';
  import { auth } from './lib/stores/auth.svelte';
  import { ui } from './lib/stores/ui.svelte';

  ui.applyTheme();

  $effect(() => {
    void auth.boot();
  });

  // First launch installs + starts the daemon in the background; poll until
  // it answers instead of parking on a manual Retry button.
  $effect(() => {
    if (auth.phase !== 'offline') return;
    const timer = setInterval(() => void auth.boot(), 2000);
    return () => clearInterval(timer);
  });
</script>

{#if auth.phase === 'loading'}
  <div class="boot">
    <div class="boot-mark">Otto</div>
    <div class="boot-sub">connecting to daemon…</div>
  </div>
{:else if auth.phase === 'offline'}
  <div class="boot">
    <div class="boot-mark">Otto</div>
    <div class="boot-sub">
      Starting the Otto daemon — first launch can take a few seconds…
    </div>
    <button class="btn primary" onclick={() => auth.boot()}>Retry now</button>
  </div>
{:else if auth.phase === 'onboarding'}
  <Onboarding />
{:else if auth.phase === 'login'}
  <Login />
{:else}
  <Shell />
{/if}

<Toasts />

<style>
  .boot {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    background: var(--bg);
  }
  .boot-mark {
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.02em;
    background: linear-gradient(120deg, var(--accent), color-mix(in srgb, var(--accent) 50%, var(--text)));
    -webkit-background-clip: text;
    background-clip: text;
    color: transparent;
  }
  .boot-sub {
    font-size: 13px;
    color: var(--text-dim);
  }
</style>
