<script lang="ts">
  // Add / edit account wizard (2 steps). Step 1 picks the credential source:
  // an existing `~/.aws` profile (from `/aws/discover`, with SSO / role hints)
  // or typed access keys (+ optional session token / role ARN) and the region.
  // Step 2 names the account, sets environment + color, then "Save & test":
  // the daemon needs a row to run `sts get-caller-identity`, so the test runs
  // right after save and its result (identity / login required) is shown in a
  // final panel with a "Sign in" shortcut. Editing skips discovery.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { aws } from '../../lib/stores/aws.svelte';
  import { awsApi } from '../../lib/api/aws';
  import { toasts } from '../../lib/toast.svelte';
  import EnvPill from './EnvPill.svelte';
  import type {
    AwsAccount,
    AwsAuthMode,
    AwsTestResp,
    DiscoveredProfile,
    Environment,
    UpsertAwsAccountReq,
  } from '../../lib/api/types';

  interface Props {
    /** null = create. */
    account: AwsAccount | null;
    onclose: () => void;
    onsignin: (a: AwsAccount) => void;
  }
  let { account, onclose, onsignin }: Props = $props();

  // Mounted fresh per open (parent {#if}-gates it) — capturing the initial
  // values into form state is intentional.
  // svelte-ignore state_referenced_locally
  const init = account;
  const editing = init !== null;

  const COLORS = ['#ff9900', '#3b82f6', '#22c55e', '#a855f7', '#ef4444', '#14b8a6', '#eab308', '#64748b'];

  let step = $state<1 | 2 | 3>(editing ? 2 : 1);
  let mode = $state<AwsAuthMode>(init?.auth_mode ?? 'profile');
  let profile = $state(init?.profile ?? '');
  let region = $state(init?.region ?? 'us-east-1');
  let accessKeyId = $state(init?.access_key_id ?? '');
  let secret = $state('');
  let sessionToken = $state('');
  let roleArn = $state(init?.role_arn ?? '');
  // Advanced: custom endpoint (LocalStack / VPC endpoints / S3-compatible).
  // Sent as a plain string so an emptied field CLEARS it on PATCH.
  let endpointUrl = $state(init?.endpoint_url ?? '');
  let advancedOpen = $state(Boolean(init?.endpoint_url));
  let name = $state(init?.name ?? '');
  let environment = $state<Environment>(init?.environment ?? 'dev');
  let color = $state(init?.color ?? COLORS[0]);

  let profiles = $state<DiscoveredProfile[]>([]);
  let profilesLoading = $state(false);
  let profileFilter = $state('');
  let busy = $state(false);
  let error = $state('');
  let saved = $state<AwsAccount | null>(null);
  let testResult = $state<AwsTestResp | null>(null);
  let testing = $state(false);

  $effect(() => {
    untrack(() => {
      void aws.loadRegions();
      if (!editing) void loadProfiles();
    });
  });

  async function loadProfiles(): Promise<void> {
    profilesLoading = true;
    try {
      profiles = (await awsApi.discover()).profiles;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      profiles = [];
    } finally {
      profilesLoading = false;
    }
  }

  const filteredProfiles = $derived.by(() => {
    const q = profileFilter.trim().toLowerCase();
    return q ? profiles.filter((p) => p.name.toLowerCase().includes(q)) : profiles;
  });

  function pickProfile(p: DiscoveredProfile): void {
    profile = p.name;
    if (p.region) region = p.region;
    if (!name) name = p.name;
    step = 2;
  }

  const step1Valid = $derived(
    mode === 'profile' ? profile.trim() !== '' : accessKeyId.trim() !== '' && (editing || secret !== ''),
  );
  const step2Valid = $derived(name.trim() !== '' && region.trim() !== '');

  function hint(p: DiscoveredProfile): string {
    const bits: string[] = [];
    if (p.sso_start_url || p.sso_session) bits.push('SSO');
    if (p.role_arn) bits.push(`role ${p.role_arn.split('/').pop()}`);
    if (p.region) bits.push(p.region);
    bits.push(p.source);
    return bits.join(' · ');
  }

  function body(): UpsertAwsAccountReq {
    const b: UpsertAwsAccountReq = {
      name: name.trim(),
      auth_mode: mode,
      region: region.trim(),
      environment,
      color,
      role_arn: roleArn.trim() || null,
      endpoint_url: endpointUrl.trim(),
    };
    if (mode === 'profile') {
      b.profile = profile.trim();
    } else {
      b.access_key_id = accessKeyId.trim();
      if (secret) b.secret_access_key = secret;
      if (sessionToken) b.session_token = sessionToken;
    }
    return b;
  }

  async function saveAndTest(): Promise<void> {
    if (!step2Valid || busy) return;
    busy = true;
    error = '';
    try {
      const a = editing && init ? await aws.updateAccount(init.id, body()) : await aws.createAccount(body());
      saved = a;
      step = 3;
      await runTest(a);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  async function runTest(a: AwsAccount): Promise<void> {
    testing = true;
    try {
      testResult = await awsApi.test(a.id);
      if (testResult.ok) void aws.loadPermissions(a.id, true);
    } catch (e) {
      testResult = {
        ok: false,
        latency_ms: 0,
        message: e instanceof Error ? e.message : String(e),
        login_required: false,
      };
    } finally {
      testing = false;
    }
  }

  function finish(): void {
    if (saved) toasts.success(editing ? 'Account updated' : 'Account added', saved.name);
    onclose();
  }
</script>

<Modal title={editing ? `Edit ${init?.name ?? 'account'}` : 'Add AWS account'} width={620} {onclose}>
  <div class="wiz" data-testid="aws-account-wizard">
    <ol class="steps" aria-label="Steps">
      <li class:on={step === 1} class:done={step > 1}>1 · Credentials</li>
      <li class:on={step === 2} class:done={step > 2}>2 · Details</li>
      <li class:on={step === 3}>3 · Test</li>
    </ol>

    {#if step === 1}
      <div class="modes" role="tablist" aria-label="Credential source">
        <button role="tab" aria-selected={mode === 'profile'} class:on={mode === 'profile'} onclick={() => (mode = 'profile')}>
          Use an existing AWS profile
        </button>
        <button role="tab" aria-selected={mode === 'access_keys'} class:on={mode === 'access_keys'} onclick={() => (mode = 'access_keys')}>
          Enter access keys
        </button>
      </div>

      {#if mode === 'profile'}
        <p class="hint">
          Profiles are read from <code>~/.aws/config</code> and <code>~/.aws/credentials</code>
          (names only — Otto never reads or writes key values). SSO / assume-role profiles work
          as-is; sign in later from the account card.
        </p>
        {#if profiles.length > 6}
          <input class="in" type="search" placeholder="Filter profiles…" bind:value={profileFilter} aria-label="Filter profiles" />
        {/if}
        <ul class="profiles" aria-busy={profilesLoading}>
          {#if profilesLoading}
            <li class="dim">Reading ~/.aws…</li>
          {:else}
            {#each filteredProfiles as p (p.name + p.source)}
              <li>
                <button class="prof" class:on={profile === p.name} onclick={() => pickProfile(p)}>
                  <span class="pname mono">{p.name}</span>
                  <span class="phint">{hint(p)}</span>
                  <Icon name="chevronRight" size={12} />
                </button>
              </li>
            {:else}
              <li class="dim">No profiles found{profileFilter ? ' for that filter' : ' in ~/.aws'}. Type one below or use access keys.</li>
            {/each}
          {/if}
        </ul>
        <label class="field">
          <span>Profile name</span>
          <input class="in mono" bind:value={profile} placeholder="default" autocomplete="off" />
        </label>
      {:else}
        <label class="field">
          <span>Access key ID</span>
          <input class="in mono" bind:value={accessKeyId} placeholder="AKIA…" autocomplete="off" spellcheck="false" />
        </label>
        <label class="field">
          <span>Secret access key{editing ? ' (leave blank to keep)' : ''}</span>
          <input class="in mono" type="password" bind:value={secret} autocomplete="new-password" />
        </label>
        <label class="field">
          <span>Session token <em>(optional, temporary creds)</em></span>
          <input class="in mono" type="password" bind:value={sessionToken} autocomplete="off" />
        </label>
        <p class="hint">Secrets go to the macOS Keychain; the row stores only the key id.</p>
        <details class="adv" bind:open={advancedOpen}>
          <summary>Advanced</summary>
          <label class="field">
            <span>Endpoint URL <em>(optional)</em></span>
            <input
              class="in mono"
              type="url"
              bind:value={endpointUrl}
              placeholder="https://…"
              spellcheck="false"
              autocomplete="off"
              data-testid="aws-wizard-endpoint"
            />
            <span class="hint">e.g. <code>http://localhost:4566</code> for LocalStack. Also for VPC interface endpoints and S3-compatible stores. Plain <code>http</code> is accepted for localhost only.</span>
          </label>
        </details>
      {/if}

      <div class="row2">
        <label class="field">
          <span>Region</span>
          {#if aws.regions.length}
            <select class="in" bind:value={region}>
              {#each aws.regions as r (r.code)}
                <option value={r.code}>{r.code} — {r.name}</option>
              {/each}
            </select>
          {:else}
            <input class="in mono" bind:value={region} placeholder="us-east-1" />
          {/if}
        </label>
        <label class="field">
          <span>Assume role ARN <em>(optional)</em></span>
          <input class="in mono" bind:value={roleArn} placeholder="arn:aws:iam::123456789012:role/Admin" spellcheck="false" />
        </label>
      </div>
    {:else if step === 2}
      <label class="field">
        <span>Name</span>
        <input class="in" bind:value={name} placeholder="prod-eu / sandbox / data-lake" data-testid="aws-wizard-name" />
      </label>
      <div class="row2">
        <div class="field">
          <span>Environment</span>
          <div class="env-row" role="radiogroup" aria-label="Environment">
            {#each ['dev', 'staging', 'prod'] as Environment[] as e (e)}
              <button
                role="radio"
                aria-checked={environment === e}
                class="env-chip"
                class:selected={environment === e}
                class:prod={e === 'prod'}
                onclick={() => (environment = e)}
              >{e}</button>
            {/each}
          </div>
          {#if environment === 'prod'}
            <span class="hint danger">Production — cards and rows get the red treatment; destructive actions ask for a typed confirmation.</span>
          {/if}
        </div>
        <div class="field">
          <span>Color</span>
          <div class="colors" role="radiogroup" aria-label="Color">
            {#each COLORS as c (c)}
              <button
                role="radio"
                aria-checked={color === c}
                class="sw"
                class:on={color === c}
                style="background:{c}"
                onclick={() => (color = c)}
                aria-label={`Color ${c}`}
              ></button>
            {/each}
          </div>
        </div>
      </div>
      {#if editing}
        <details class="adv">
          <summary>Credentials &amp; region</summary>
          <div class="row2">
            <label class="field">
              <span>Region</span>
              {#if aws.regions.length}
                <select class="in" bind:value={region}>
                  {#each aws.regions as r (r.code)}<option value={r.code}>{r.code} — {r.name}</option>{/each}
                </select>
              {:else}
                <input class="in mono" bind:value={region} />
              {/if}
            </label>
            {#if mode === 'profile'}
              <label class="field"><span>Profile</span><input class="in mono" bind:value={profile} /></label>
            {:else}
              <label class="field"><span>Access key ID</span><input class="in mono" bind:value={accessKeyId} /></label>
              <label class="field"><span>New secret (blank = keep)</span><input class="in mono" type="password" bind:value={secret} autocomplete="new-password" /></label>
              <label class="field"><span>New session token</span><input class="in mono" type="password" bind:value={sessionToken} /></label>
            {/if}
            <label class="field"><span>Assume role ARN</span><input class="in mono" bind:value={roleArn} /></label>
            <label class="field"><span>Endpoint URL <em>(blank = AWS default)</em></span><input class="in mono" type="url" bind:value={endpointUrl} placeholder="http://localhost:4566" spellcheck="false" data-testid="aws-wizard-endpoint" /></label>
          </div>
        </details>
      {/if}
      <div class="summary">
        <span class="dot" style="background:{color}"></span>
        <strong>{name || 'Unnamed'}</strong>
        <EnvPill env={environment} />
        <span class="mono dim">{mode === 'profile' ? `profile ${profile}` : `keys ${accessKeyId}`} · {region}</span>
        {#if endpointUrl.trim()}<span class="mono dim">→ {endpointUrl.trim()}</span>{/if}
      </div>
    {:else}
      <div class="test" aria-live="polite">
        {#if testing}
          <p><span class="spinner"></span> Running <code>sts get-caller-identity</code>…</p>
        {:else if testResult?.ok}
          <p class="ok"><Icon name="check" size={14} /> Connected in {testResult.latency_ms} ms</p>
          {#if testResult.identity}
            <dl class="idn">
              <dt>Account</dt><dd class="mono">{testResult.identity.account}</dd>
              <dt>ARN</dt><dd class="mono">{testResult.identity.arn}</dd>
            </dl>
          {/if}
          <p class="dim">Permissions are being probed per service — the card shows the chips in a moment.</p>
        {:else if testResult}
          <p class="bad"><Icon name="x" size={14} /> {testResult.message}</p>
          {#if testResult.login_required && saved}
            {#if saved.auth_mode === 'profile'}
              <p class="dim">This profile needs an interactive sign-in (<code>aws sso login</code>).</p>
              <button class="primary" onclick={() => saved && onsignin(saved)}><Icon name="key" size={12} /> Sign in now</button>
            {:else}
              <p class="dim">The keys were rejected — go back and re-enter them.</p>
            {/if}
          {/if}
        {/if}
      </div>
    {/if}

    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </div>

  {#snippet footer()}
    {#if step === 1}
      <button class="ghost" onclick={onclose}>Cancel</button>
      <button class="primary" disabled={!step1Valid} onclick={() => (step = 2)}>Next</button>
    {:else if step === 2}
      {#if !editing}<button class="ghost" onclick={() => (step = 1)}>Back</button>{/if}
      <button class="ghost" onclick={onclose}>Cancel</button>
      <button class="primary" disabled={!step2Valid || busy} onclick={() => void saveAndTest()} data-testid="aws-wizard-save">
        {busy ? 'Saving…' : 'Save & test'}
      </button>
    {:else}
      {#if saved && !testResult?.ok}
        <button class="ghost" onclick={() => saved && void runTest(saved)} disabled={testing}>Test again</button>
        <button class="ghost" onclick={() => (step = 2)}>Back</button>
      {/if}
      <button class="primary" onclick={finish}>Done</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .wiz {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .steps {
    display: flex;
    gap: 14px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .steps li.on {
    color: var(--accent);
    font-weight: 600;
  }
  .steps li.done {
    color: var(--text);
  }
  .modes {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }
  .modes button {
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
    font-size: 12.5px;
  }
  .modes button.on {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.45;
  }
  .hint.danger {
    color: var(--status-exited);
  }
  code {
    font-family: var(--font-mono);
    font-size: 11.5px;
  }
  .profiles {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 220px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .profiles li.dim {
    padding: 10px;
    font-size: 12px;
  }
  .prof {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 10px;
    border: 0;
    border-bottom: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .prof:hover,
  .prof.on {
    background: var(--surface-2);
  }
  .pname {
    font-weight: 600;
    font-size: 12.5px;
  }
  .phint {
    flex: 1;
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-dim);
    min-width: 0;
  }
  .field em {
    font-style: normal;
    opacity: 0.8;
  }
  .in {
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
    min-width: 0;
  }
  .row2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .env-row {
    display: flex;
    gap: 6px;
  }
  .env-chip {
    padding: 4px 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    font-size: 12px;
    cursor: pointer;
  }
  .env-chip.selected {
    border-color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
    color: var(--status-working);
  }
  .env-chip.prod.selected {
    border-color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
    color: var(--status-exited);
  }
  .colors {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .sw {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 2px solid transparent;
    cursor: pointer;
    padding: 0;
  }
  .sw.on {
    border-color: var(--text);
    box-shadow: 0 0 0 2px var(--surface);
  }
  .adv summary {
    cursor: pointer;
    font-size: 12px;
    color: var(--text-dim);
  }
  .adv .row2,
  .adv .field {
    margin-top: 8px;
  }
  .summary {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-m);
    font-size: 12.5px;
    flex-wrap: wrap;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }
  .dim {
    color: var(--text-dim);
  }
  .test p {
    margin: 0 0 8px;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  .ok {
    color: var(--status-working);
  }
  .bad {
    color: var(--status-exited);
  }
  .idn {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 10px;
    margin: 0 0 8px;
    font-size: 12px;
  }
  .idn dt {
    color: var(--text-dim);
  }
  .idn dd {
    margin: 0;
    word-break: break-all;
  }
  .spinner {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .err {
    margin: 0;
    font-size: 12px;
    color: var(--status-exited);
  }
  .primary {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--accent-contrast);
    font-weight: 600;
    cursor: pointer;
  }
  .primary:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .ghost {
    padding: 6px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  @media (max-width: 640px) {
    .row2,
    .modes {
      grid-template-columns: 1fr;
    }
  }
</style>
