<script lang="ts">
  import { auth } from '../../lib/stores/auth.svelte';
  import { ApiError } from '../../lib/api/client';

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    if (busy) return;
    busy = true;
    error = '';
    try {
      await auth.login(username, password);
    } catch (err) {
      error =
        err instanceof ApiError && err.status === 401
          ? 'Wrong username or password.'
          : 'Could not sign in — daemon unreachable?';
    } finally {
      busy = false;
    }
  }
</script>

<div class="login-wrap">
  <form class="login-card card" onsubmit={submit}>
    <div class="login-mark">Otto</div>
    <p class="login-sub">Sign in to your development environment</p>

    <div class="field">
      <label for="login-user">Username</label>
      <input id="login-user" class="input" bind:value={username} autocomplete="username" />
    </div>
    <div class="field">
      <label for="login-pass">Password</label>
      <input
        id="login-pass"
        class="input"
        type="password"
        bind:value={password}
        autocomplete="current-password"
      />
    </div>

    {#if error}<div class="login-error">{error}</div>{/if}

    <button class="btn primary login-btn" type="submit" disabled={busy || !username || !password}>
      {busy ? 'Signing in…' : 'Sign In'}
    </button>
  </form>
</div>

<style>
  .login-wrap {
    height: 100%;
    display: grid;
    place-items: center;
    background: var(--bg);
  }
  .login-card {
    width: 320px;
    padding: 28px 26px 24px;
    display: flex;
    flex-direction: column;
  }
  .login-mark {
    font-size: 24px;
    font-weight: 700;
    letter-spacing: -0.02em;
    text-align: center;
  }
  .login-sub {
    margin: 4px 0 20px;
    text-align: center;
    font-size: 12px;
    color: var(--text-dim);
  }
  .login-error {
    margin-bottom: 10px;
    font-size: 12px;
    color: var(--status-exited);
  }
  .login-btn {
    height: 30px;
    margin-top: 4px;
  }
</style>
