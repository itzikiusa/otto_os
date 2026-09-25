<script lang="ts">
  // Settings → Assistant: how Otto Assistant picks provider + model per turn
  // (plan §2.4). It never switches silently — rules decide, an explicit pin or
  // a leading @claude / @codex always wins, and a usage limit asks first
  // unless you turn on automatic switching. One form, one Save.
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import SettingToggle from './SettingToggle.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { formatCount } from '../../lib/metric-format';
  import { assistant, describeError } from '../../lib/stores/assistant.svelte';
  import { PROVIDER_NAME, ROUTE_ROWS, loadShare, parseKeywords, type Provider } from '../assistant/model';
  import { whenLabel } from '../assistant/format';
  import StatePill from '../assistant/cards/StatePill.svelte';
  import type { AssistantRouteKind, AssistantRouteTarget, AssistantRoutingSettings } from '../../lib/api/types';

  $effect(() => {
    void assistant.loadRouting();
    void assistant.loadLimits();
    void assistant.loadUsageWeek();
    void assistant.loadAccounts();
  });

  const routing = $derived(assistant.routing);
  const PROVIDERS: Provider[] = ['claude', 'codex'];

  // ── the draft form ─────────────────────────────────────────────────────────
  interface Draft {
    targets: Record<AssistantRouteKind, AssistantRouteTarget>;
    code: string;
    hard: string;
    auto_failover: boolean;
    memory_approval: boolean;
  }
  function toDraft(r: AssistantRoutingSettings): Draft {
    return {
      targets: $state.snapshot(r.targets),
      code: r.extra_keywords.code.join(', '),
      hard: r.extra_keywords.hard.join(', '),
      auto_failover: r.auto_failover,
      memory_approval: r.memory_approval,
    };
  }
  let draft = $state<Draft | null>(null);
  let base = $state('');
  // Seed (and re-seed after a save) from the loaded settings.
  $effect(() => {
    const r = routing.data;
    if (!r) return;
    const d = toDraft(r);
    draft = d;
    base = JSON.stringify(d);
  });
  const dirty = $derived(!!draft && JSON.stringify(draft) !== base);
  let saving = $state(false);
  let saveError = $state('');
  let savedNote = $state(false);

  async function save(): Promise<void> {
    if (!draft || !dirty) return;
    saving = true;
    saveError = '';
    try {
      await assistant.saveRouting({
        targets: $state.snapshot(draft.targets),
        extra_keywords: { code: parseKeywords(draft.code), hard: parseKeywords(draft.hard) },
        auto_failover: draft.auto_failover,
        memory_approval: draft.memory_approval,
      });
      savedNote = true;
      // The Save button sits in the header; the inline note is at the top of a
      // long form, so also confirm with a toast when it may be off-screen.
      toasts.success('Routing saved', 'New turns use these rules.');
    } catch (e) {
      saveError = `Couldn’t save routing. ${describeError(e)}`;
      toasts.error('Couldn’t save routing', describeError(e));
    } finally {
      saving = false;
    }
  }
  function setProvider(kind: AssistantRouteKind, p: Provider): void {
    if (!draft) return;
    draft.targets[kind] = { provider: p, model: null, account_id: null };
    savedNote = false;
  }

  // ── subscriptions ──────────────────────────────────────────────────────────
  const share = $derived(loadShare(assistant.usageWeek.data.filter((p) => p.provider === 'claude' || p.provider === 'codex')));
  function accountsFor(p: Provider) {
    return assistant.accounts.data.filter((a) => a.provider === p);
  }
  function limitFor(p: Provider) {
    return assistant.limits.data.find((l) => l.provider === p && l.limited) ?? null;
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('assistant')} subtitle="How each turn routes between Claude and Codex plans">
    {#snippet actions()}
      <button class="btn small ghost" data-icon="assistant" onclick={() => router.go('assistant')}><Icon name="assistant" size={12} /> Open Assistant</button>
      {#if routing.data}
        <button class="btn small primary" onclick={() => void save()} disabled={!dirty || saving}>{saving ? 'Saving…' : 'Save'}</button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody width="readable">
    {#if routing.state === 'loading' && !routing.data}
      <div aria-busy="true" aria-label="Loading routing settings"><Skeleton rows={6} height={40} /></div>
    {:else if routing.state === 'unsupported'}
      <EmptyState variant="page" icon="assistant" title="The assistant isn’t available yet" body="This Otto daemon doesn’t include the assistant. Update Otto to configure routing." />
    {:else if routing.state === 'error' && !routing.data}
      <div class="error" role="alert">
        <Icon name="warning" size={14} />
        <div class="error-t"><strong>Couldn’t load routing settings.</strong><span class="dim">{routing.error}</span></div>
        <button class="btn small" onclick={() => void assistant.loadRouting()}>Retry</button>
      </div>
    {:else if draft}
      <div class="form" data-testid="assistant-routing">
        {#if saveError}<p class="err" role="alert">{saveError}</p>{/if}
        {#if savedNote && !dirty}<p class="ok" role="status">Saved. New turns use these rules.</p>{/if}

        <section>
          <h2 class="section-title">Subscriptions</h2>
          <p class="help">Otto uses the CLIs’ own sign-ins — no API keys, and it never reads your credentials.</p>
          <div class="subs">
            {#each PROVIDERS as p (p)}
              {@const lim = limitFor(p)}
              {@const s = share.find((x) => x.provider === p)}
              <div class="sub card" data-testid={`sub-${p}`}>
                <div class="sub-head">
                  <ProviderIcon provider={p} size={16} />
                  <strong>{PROVIDER_NAME[p]}</strong>
                  <span class="push">
                    {#if lim}<StatePill tone="warn" label={lim.until ? `Limit until ${whenLabel(lim.until)}` : 'Limit reached'} />{:else}<StatePill tone="ok" label="No limit hit" />{/if}
                  </span>
                </div>
                <div class="meter" role="meter" aria-label={`${PROVIDER_NAME[p]} share of this week’s load`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={s?.pct ?? 0}>
                  <span class:warn={!!lim} style:width="{s?.pct ?? 0}%"></span>
                </div>
                <p class="sm">
                  {#if s}{s.pct}% of this week’s load · {formatCount(s.tokens)} tokens{:else if assistant.usageWeek.state === 'unsupported'}Weekly usage needs usage tracking (admin){:else if assistant.usageWeek.state === 'loading'}Loading usage…{:else}No usage this week{/if}
                  {#if lim?.message}<br /><span class="dim">{lim.message}</span>{/if}
                </p>
                <ul class="accts">
                  <li><Icon name="user" size={12} /> Default CLI account</li>
                  {#each accountsFor(p) as a (a.id)}
                    <li>
                      <Icon name="user" size={12} />
                      <span class="grow">{a.label}</span>
                      {#if a.signed_in === true}<span class="dim">Signed in</span>{:else if a.signed_in === false}<span class="warn-t">Not signed in</span>{:else}<span class="dim">Checking…</span>{/if}
                    </li>
                  {/each}
                </ul>
              </div>
            {/each}
          </div>
        </section>

        <section>
          <h2 class="section-title">Rules</h2>
          <p class="help">
            Classified on this Mac with simple keyword rules — no extra model call. The model chip on a thread, or a message
            starting with <span class="mono">@claude</span> / <span class="mono">@codex</span>, always overrides. A thread
            that has already started stays on its CLI unless the new message matches a rule strongly.
          </p>
          <div class="table-wrap">
            <table class="rules">
              <thead>
                <tr><th scope="col">When the request is…</th><th scope="col">Use</th><th scope="col">Model</th></tr>
              </thead>
              <tbody>
                {#each ROUTE_ROWS as row (row.kind)}
                  {@const t = draft.targets[row.kind]}
                  <tr data-testid={`rule-${row.kind}`}>
                    <td>
                      <div class="rule-label">{row.label}</div>
                      <div class="dim sm">{row.hint}</div>
                    </td>
                    <td>
                      <div class="segmented" role="group" aria-label={`Provider for ${row.label}`}>
                        {#each PROVIDERS as p (p)}
                          <button type="button" aria-pressed={t.provider === p} class:active={t.provider === p} onclick={() => setProvider(row.kind, p)}>
                            <ProviderIcon provider={p} size={12} />{PROVIDER_NAME[p]}
                          </button>
                        {/each}
                      </div>
                    </td>
                    <td class="model-cell">
                      {#key t.provider}
                        <ModelPicker
                          provider={t.provider}
                          value={t.model ?? ''}
                          id={`route-model-${row.kind}`}
                          compact
                          onchange={(m) => {
                            if (draft) draft.targets[row.kind] = { ...draft.targets[row.kind], model: m.trim() || null };
                            savedNote = false;
                          }}
                        />
                      {/key}
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          <div class="kw">
            <div class="field">
              <label for="kw-code">Extra words that mean “code”</label>
              <input id="kw-code" class="input" bind:value={draft.code} placeholder="terraform, jq, regex" oninput={() => (savedNote = false)} />
            </div>
            <div class="field">
              <label for="kw-hard">Extra words that mean “think hard”</label>
              <input id="kw-hard" class="input" bind:value={draft.hard} placeholder="deep dive, audit, proof" oninput={() => (savedNote = false)} />
            </div>
          </div>
          <p class="help">Comma-separated. They add to the built-in rules.</p>
        </section>

        <section>
          <h2 class="section-title">When a limit is reached</h2>
          <div class="radios" role="radiogroup" aria-label="When a usage limit is reached">
            <label class="radio">
              <input type="radio" name="failover" checked={!draft.auto_failover} onchange={() => { if (draft) draft.auto_failover = false; savedNote = false; }} />
              <span>
                <span class="opt">Ask before switching provider</span>
                <span class="dim sm">The thread asks “Claude limit reached until 14:00 — continue on Codex?” and remembers your answer for that thread.</span>
              </span>
            </label>
            <label class="radio">
              <input type="radio" name="failover" checked={draft.auto_failover} onchange={() => { if (draft) draft.auto_failover = true; savedNote = false; }} />
              <span>
                <span class="opt">Switch automatically</span>
                <span class="dim sm">Moves the thread to the other subscription and posts a visible note in it.</span>
              </span>
            </label>
          </div>
          <p class="note"><Icon name="info" size={12} /> A switch starts a new session seeded with a thread summary, the last turns and your profile, so the conversation carries on in one thread.</p>
        </section>

        <section>
          <h2 class="section-title">Memory</h2>
          <SettingToggle
            label="Review memories before Otto keeps them"
            hint="Off: Otto saves what matters and shows a chip with Undo. On: suggestions wait on the Memory tab until you accept them."
            checked={draft.memory_approval}
            onchange={(v) => { if (draft) draft.memory_approval = v; savedNote = false; }}
          />
        </section>
      </div>
    {/if}
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. (Not the
     global `.page` class — its padding would inset the PageHeader.) */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 28px;
    max-width: 880px;
  }
  .section-title {
    margin: 0 0 4px;
  }
  .help {
    margin: 0 0 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .dim {
    color: var(--text-dim);
  }
  .sm {
    font-size: var(--fs-s);
  }
  .mono {
    font-family: var(--font-mono);
    color: var(--text);
  }
  .err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .ok {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--success);
  }
  .subs {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: 12px;
  }
  .sub {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .sub-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .push {
    margin-inline-start: auto;
  }
  .meter {
    height: 6px;
    border-radius: 999px;
    background: var(--surface-3);
    overflow: hidden;
  }
  .meter span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .meter span.warn {
    background: var(--status-warn);
  }
  .sub .sm {
    margin: 0;
  }
  .accts {
    list-style: none;
    margin: 0;
    padding: 8px 0 0;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-s);
  }
  .accts li {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .accts :global(svg) {
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .warn-t {
    color: var(--warning);
  }
  .table-wrap {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-m);
  }
  th {
    text-align: start;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text-dim);
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
  }
  td {
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    vertical-align: middle;
  }
  tr:last-child td {
    border-bottom: 0;
  }
  .rule-label {
    font-weight: 500;
  }
  .segmented > button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .model-cell {
    min-width: 200px;
  }
  .kw {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: 12px;
    margin-top: 12px;
  }
  .kw .field {
    margin: 0;
  }
  .radios {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .radio {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    cursor: pointer;
  }
  .radio input {
    margin-top: 3px;
  }
  .radio > span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .opt {
    font-weight: 500;
  }
  .note {
    margin: 10px 0 0;
    display: flex;
    align-items: flex-start;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .note :global(svg) {
    margin-top: 2px;
  }
  .error {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    max-width: 640px;
  }
  .error > :global(svg) {
    color: var(--danger);
    margin-top: 2px;
  }
  .error-t {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--fs-s);
  }
</style>
