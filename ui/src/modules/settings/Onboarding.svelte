<script lang="ts">
  // First-run wizard: welcome → root password → first workspace → tools → done.
  import { api } from '../../lib/api/client';
  import type { LoginResp, Workspace } from '../../lib/api/types';
  import { auth } from '../../lib/stores/auth.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  let step = $state(0);
  let password = $state('');
  let password2 = $state('');
  let displayName = $state('');
  let wsName = $state('');
  let wsPath = $state('');
  let busy = $state(false);
  let error = $state('');

  const strength = $derived.by(() => {
    let score = 0;
    if (password.length >= 10) score++;
    if (password.length >= 14) score++;
    if (/[A-Z]/.test(password) && /[a-z]/.test(password)) score++;
    if (/\d/.test(password)) score++;
    if (/[^A-Za-z0-9]/.test(password)) score++;
    return score; // 0..5
  });
  const strengthLabel = $derived(
    ['too short', 'weak', 'fair', 'good', 'strong', 'excellent'][strength] ?? 'weak',
  );
  const pwValid = $derived(password.length >= 10 && password === password2);

  // ClickHouse powers usage tracking; surface whether it's already present so
  // the user knows what to expect (install happens from the Usage dashboard
  // after setup, since it needs an authenticated session).
  const clickhouse = $derived(auth.meta?.tools?.find((t) => t.name === 'clickhouse'));

  async function finish(): Promise<void> {
    if (busy) return;
    busy = true;
    error = '';
    try {
      const resp = await api.post<LoginResp>('/onboarding/root', {
        password,
        display_name: displayName.trim() === '' ? null : displayName.trim(),
      });
      auth.acceptLogin(resp);
      if (wsName.trim() !== '' && wsPath.trim() !== '') {
        await api.post<Workspace>('/workspaces', { name: wsName.trim(), root_path: wsPath.trim() });
      }
      if (auth.meta) auth.meta.needs_onboarding = false;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      busy = false;
      return;
    }
    busy = false;
  }
</script>

<div class="ob-wrap">
  <div class="ob-card card">
    <div class="ob-progress">
      {#each Array(5) as _, i (i)}
        <span class="ob-dot" class:done={i <= step}></span>
      {/each}
    </div>

    {#if step === 0}
      <div class="ob-body">
        <div class="ob-mark">Otto</div>
        <h1>Welcome to Otto</h1>
        <p>
          Your agentic development environment: terminal sessions for Claude and Codex,
          managed connections, and full git + pull-request review — backed by a local daemon
          that keeps everything running even when the app is closed.
        </p>
        <button class="btn primary big" onclick={() => (step = 1)}>Get Started</button>
      </div>
    {:else if step === 1}
      <div class="ob-body">
        <h1>Set the root password</h1>
        <p>The root account manages users, workspaces, and daemon settings.</p>

        <div class="field">
          <label for="ob-name">Display name <span class="dim">(optional)</span></label>
          <input id="ob-name" class="input" bind:value={displayName} placeholder="Root" />
        </div>
        <div class="field">
          <label for="ob-pass">Password</label>
          <input id="ob-pass" class="input" type="password" bind:value={password} autocomplete="new-password" />
          <div class="strength">
            <div class="strength-bar">
              <div
                class="strength-fill s{strength}"
                style="width: {Math.min(100, strength * 20)}%"
              ></div>
            </div>
            <span class="hint">
              {password.length < 10 ? `min 10 chars (${password.length}/10)` : strengthLabel}
            </span>
          </div>
        </div>
        <div class="field">
          <label for="ob-pass2">Confirm password</label>
          <input id="ob-pass2" class="input" type="password" bind:value={password2} autocomplete="new-password" />
          {#if password2.length > 0 && password !== password2}
            <span class="hint err">Passwords don't match</span>
          {/if}
        </div>

        <div class="ob-actions">
          <button class="btn" onclick={() => (step = 0)}>Back</button>
          <button class="btn primary" disabled={!pwValid} onclick={() => (step = 2)}>Continue</button>
        </div>
      </div>
    {:else if step === 2}
      <div class="ob-body">
        <h1>Create your first workspace</h1>
        <p>A workspace maps to a project directory. Sessions and repos live inside it.</p>

        <div class="field">
          <label for="ob-wsname">Name</label>
          <input id="ob-wsname" class="input" bind:value={wsName} placeholder="my-project" />
        </div>
        <div class="field">
          <label for="ob-wspath">Directory</label>
          <input id="ob-wspath" class="input mono" bind:value={wsPath} placeholder="/Users/you/code/my-project" spellcheck="false" />
        </div>

        <div class="ob-actions">
          <button class="btn" onclick={() => (step = 1)}>Back</button>
          <button class="btn ghost" onclick={() => (step = 3)}>Skip</button>
          <button
            class="btn primary"
            disabled={wsName.trim() === '' || wsPath.trim() === ''}
            onclick={() => (step = 3)}
          >
            Continue
          </button>
        </div>
      </div>
    {:else if step === 3}
      <div class="ob-body">
        <h1>Usage tracking</h1>
        <p>
          Otto can track token usage, cost, and system metrics in an embedded
          <strong>ClickHouse</strong> engine — a full <em>Usage</em> dashboard, kept locally for
          up to 180 days (configurable). No server or port; it runs as
          <span class="mono">clickhouse local</span>.
        </p>

        <div class="tools">
          <div class="tool-row">
            <span class="tool-status" class:ok={clickhouse?.found}>
              {#if clickhouse?.found}<Icon name="check" size={12} />{:else}<Icon name="x" size={11} />{/if}
            </span>
            <span class="mono">clickhouse</span>
            <span class="grow"></span>
            <span class="dim">{clickhouse?.found ? (clickhouse.version ?? 'found') : 'not installed'}</span>
          </div>
        </div>
        <p class="hint-line">
          {#if clickhouse?.found}
            Detected — usage tracking will turn on automatically.
          {:else}
            Install it later with one click from <strong>Usage → Install ClickHouse</strong>
            (<span class="mono">curl https://clickhouse.com/ | sh</span>).
          {/if}
        </p>

        <div class="ob-actions">
          <button class="btn" onclick={() => (step = 2)}>Back</button>
          <button class="btn primary" onclick={() => (step = 4)}>Continue</button>
        </div>
      </div>
    {:else}
      <div class="ob-body">
        <h1>Tool check</h1>
        <p>Otto spawns these CLIs on your behalf. Missing tools can be installed later.</p>

        <div class="tools">
          {#each auth.meta?.tools ?? [] as t (t.name)}
            <div class="tool-row">
              <span class="tool-status" class:ok={t.found}>
                {#if t.found}<Icon name="check" size={12} />{:else}<Icon name="x" size={11} />{/if}
              </span>
              <span class="mono">{t.name}</span>
              <span class="grow"></span>
              <span class="dim">{t.found ? (t.version ?? 'found') : 'not found'}</span>
            </div>
          {:else}
            <div class="dim">Tool detection unavailable.</div>
          {/each}
        </div>

        {#if error}<div class="hint err">{error}</div>{/if}

        <div class="ob-actions">
          <button class="btn" onclick={() => (step = 3)}>Back</button>
          <button class="btn primary big" disabled={busy} onclick={finish}>
            {busy ? 'Setting up…' : 'Finish Setup'}
          </button>
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .ob-wrap {
    height: 100%;
    display: grid;
    place-items: center;
    background: var(--bg);
  }
  .ob-card {
    width: 440px;
    max-width: calc(100vw - 48px);
    padding: 22px 30px 28px;
  }
  .ob-progress {
    display: flex;
    gap: 6px;
    justify-content: center;
    margin-bottom: 16px;
  }
  .ob-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--surface-2);
    border: 1px solid var(--border);
    transition: background 150ms ease-out;
  }
  .ob-dot.done {
    background: var(--accent);
    border-color: transparent;
  }
  .ob-mark {
    font-size: 30px;
    font-weight: 700;
    letter-spacing: -0.02em;
    text-align: center;
    margin-bottom: 4px;
    background: linear-gradient(120deg, var(--accent), color-mix(in srgb, var(--accent) 50%, var(--text)));
    -webkit-background-clip: text;
    background-clip: text;
    color: transparent;
  }
  h1 {
    font-size: 17px;
    margin: 0 0 4px;
    text-align: center;
  }
  .ob-body > p {
    margin: 0 0 18px;
    font-size: 12.5px;
    color: var(--text-dim);
    text-align: center;
    line-height: 1.55;
  }
  .ob-actions {
    display: flex;
    justify-content: center;
    gap: 8px;
    margin-top: 18px;
  }
  .btn.big {
    height: 30px;
    padding: 0 18px;
  }
  .strength {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .strength-bar {
    flex: 1;
    height: 4px;
    border-radius: 2px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .strength-fill {
    height: 100%;
    border-radius: 2px;
    background: var(--status-exited);
    transition: width 180ms ease-out, background 180ms ease-out;
  }
  .strength-fill.s2 {
    background: #febc2e;
  }
  .strength-fill.s3 {
    background: #febc2e;
  }
  .strength-fill.s4,
  .strength-fill.s5 {
    background: var(--status-working);
  }
  .hint.err {
    color: var(--status-exited);
  }
  .hint-line {
    font-size: 11.5px;
    color: var(--text-dim);
    text-align: center;
    margin: 10px 0 0;
    line-height: 1.5;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }
  .tools {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: 8px;
  }
  .tool-row {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--radius-s);
  }
  .tool-row:nth-child(odd) {
    background: var(--surface-2);
  }
  .tool-status {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--status-exited) 18%, transparent);
    color: var(--status-exited);
  }
  .tool-status.ok {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
</style>
