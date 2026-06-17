<script lang="ts">
  // Hand the current agent's working context to another agent in the same
  // workspace — a freshly spawned CLI or an existing running one. Otto gathers
  // the source agent's recent work (+ git state), summarizes it, and types a
  // handover brief into the target. Optionally review/edit the brief first.
  import Modal from '../../lib/components/Modal.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { api } from '../../lib/api/client';
  import type {
    Session,
    HandoverReq,
    HandoverTarget,
    HandoverBriefResp,
  } from '../../lib/api/types';

  interface Props {
    sessionId: string;
    onclose: () => void;
  }
  let { sessionId, onclose }: Props = $props();

  const source = $derived(ws.sessions.find((s) => s.id === sessionId) ?? null);
  const providers = $derived(auth.meta?.providers ?? ['claude', 'codex', 'shell']);
  // Other agent sessions in this workspace, eligible as existing targets.
  const otherAgents = $derived(ws.plainAgentSessions.filter((s) => s.id !== sessionId));

  type Mode = 'new' | 'existing';
  let mode = $state<Mode>('new');
  let provider = $state('');
  let existingId = $state('');
  let focus = $state('');

  // Options.
  let includeGit = $state(true);
  let fast = $state(false);
  let reviewBrief = $state(false);
  let archiveSource = $state(false);

  // Review flow.
  let phase = $state<'compose' | 'review'>('compose');
  let brief = $state('');
  let briefNote = $state('');
  let briefLoading = $state(false);
  let busy = $state(false);

  function toolFound(p: string): boolean {
    const t = (auth.meta?.tools ?? []).find((x) => x.name === p);
    // Unknown tools (custom providers, shell) are assumed available.
    return t ? t.found : true;
  }

  function providerDesc(p: string): string {
    return p === 'claude'
      ? 'Claude Code CLI'
      : p === 'codex'
        ? 'Codex CLI'
        : p === 'shell'
          ? 'Plain shell'
          : 'Custom provider';
  }

  // Default to the first available provider that differs from the source.
  $effect(() => {
    if (provider === '' && providers.length > 0) {
      const avail = providers.filter(toolFound);
      provider =
        avail.find((p) => p !== source?.provider) ?? avail[0] ?? providers[0];
    }
  });

  const canSubmit = $derived(
    mode === 'new' ? provider !== '' && toolFound(provider) : existingId !== '',
  );

  function buildTarget(): HandoverTarget {
    return mode === 'new'
      ? { kind: 'new_agent', provider }
      : { kind: 'existing_session', session_id: existingId };
  }

  async function generateBrief(): Promise<void> {
    if (briefLoading || !canSubmit) return;
    briefLoading = true;
    briefNote = '';
    try {
      const resp = await api.post<HandoverBriefResp>(`/sessions/${sessionId}/handover/brief`, {
        focus: focus.trim() === '' ? null : focus.trim(),
        include_git: includeGit,
        fast,
      });
      brief = resp.brief;
      briefNote = !resp.had_context
        ? 'No prior context was found — the agent will receive only your focus note.'
        : resp.fallback
          ? 'Automatic summary was unavailable; showing raw recent context. Edit as needed.'
          : '';
      phase = 'review';
    } catch (e) {
      toasts.error('Could not generate brief', e instanceof Error ? e.message : String(e));
    } finally {
      briefLoading = false;
    }
  }

  async function send(briefText: string | null): Promise<void> {
    if (busy || !canSubmit) return;
    busy = true;
    try {
      const req: HandoverReq = {
        target: buildTarget(),
        focus: focus.trim() === '' ? null : focus.trim(),
        brief: briefText && briefText.trim() !== '' ? briefText.trim() : null,
        include_git: includeGit,
        fast,
        archive_source: archiveSource,
      };
      const session = await api.post<Session>(`/sessions/${sessionId}/handover`, req);
      ws.addSession(session); // adds (if new) + opens the target pane
      onclose();
      router.go('agents');
      const where = mode === 'new' ? `new ${provider} agent` : (session.title ?? 'the agent');
      toasts.success(
        'Handover started',
        briefText
          ? `Delivering your brief to ${where}.`
          : `Preparing the brief for ${where} — it'll arrive shortly.`,
      );
    } catch (e) {
      toasts.error('Handover failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  function primary(): void {
    if (phase === 'review') {
      void send(brief);
    } else if (reviewBrief) {
      void generateBrief();
    } else {
      void send(null);
    }
  }
</script>

<Modal title="Hand over to another agent" width={560} {onclose}>
  {#if phase === 'compose'}
    <p class="lead">
      Pass <strong>{source?.title ?? 'this agent'}</strong>’s context to another agent. Otto
      summarizes what <span class="chip">{source?.provider ?? 'the agent'}</span> has been doing
      (plus the repo’s git state) and hands the brief over.
    </p>

    <!-- Target: new vs existing -->
    <div class="seg">
      <button class="seg-btn" class:active={mode === 'new'} onclick={() => (mode = 'new')}>
        New agent
      </button>
      <button
        class="seg-btn"
        class:active={mode === 'existing'}
        disabled={otherAgents.length === 0}
        title={otherAgents.length === 0 ? 'No other agents in this workspace' : ''}
        onclick={() => (mode = 'existing')}
      >
        Existing agent {otherAgents.length > 0 ? `(${otherAgents.length})` : ''}
      </button>
    </div>

    {#if mode === 'new'}
      <div class="field">
        <div class="provider-grid">
          {#each providers as p (p)}
            {@const avail = toolFound(p)}
            <button
              class="provider-card"
              class:selected={provider === p}
              class:unavailable={!avail}
              disabled={!avail}
              title={avail ? '' : `${p} is not installed`}
              onclick={() => (provider = p)}
            >
              <span class="provider-name">
                {p}
                {#if p === source?.provider}<span class="badge muted">same</span>{/if}
                {#if !avail}<span class="badge muted">not found</span>{/if}
              </span>
              <span class="provider-desc">{providerDesc(p)}</span>
            </button>
          {/each}
        </div>
      </div>
    {:else}
      <div class="field">
        <ul class="agent-list">
          {#each otherAgents as a (a.id)}
            <li>
              <button
                class="agent-row"
                class:selected={existingId === a.id}
                onclick={() => (existingId = a.id)}
              >
                <StatusDot status={ws.statusMap[a.id] ?? a.status} />
                <span class="agent-title">{a.title}</span>
                <span class="chip">{a.provider}</span>
              </button>
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    <div class="field">
      <label for="ho-focus">What should the agent focus on? <span class="dim">(optional)</span></label>
      <textarea
        id="ho-focus"
        class="input"
        bind:value={focus}
        rows="3"
        placeholder="e.g. The auth refactor is done — focus on wiring the new endpoint into the UI; don't touch the DB layer."
      ></textarea>
    </div>

    <div class="opts">
      <label class="toggle-row">
        <input type="checkbox" bind:checked={includeGit} />
        <span class="toggle-text">
          <span class="toggle-title">Include git state</span>
          <span class="hint">Add the branch, changed files, and recent commits to the brief.</span>
        </span>
      </label>
      <label class="toggle-row">
        <input type="checkbox" bind:checked={reviewBrief} />
        <span class="toggle-text">
          <span class="toggle-title">Review brief before sending</span>
          <span class="hint">Generate the summary and let you edit it before it’s delivered.</span>
        </span>
      </label>
      <label class="toggle-row">
        <input type="checkbox" bind:checked={fast} />
        <span class="toggle-text">
          <span class="toggle-title">Fast summary</span>
          <span class="hint">Use a quicker, lighter model (less thorough).</span>
        </span>
      </label>
      <label class="toggle-row">
        <input type="checkbox" bind:checked={archiveSource} />
        <span class="toggle-text">
          <span class="toggle-title">Archive {source?.provider ?? 'source'} after handover</span>
          <span class="hint">Close out the source agent once the brief is delivered.</span>
        </span>
      </label>
    </div>
  {:else}
    <!-- Review phase -->
    <p class="lead">Review the brief below — edit anything, then send it to the agent.</p>
    {#if briefNote}<p class="note">{briefNote}</p>{/if}
    <div class="field">
      <label for="ho-brief">Handover brief</label>
      <textarea
        id="ho-brief"
        class="input mono brief"
        bind:value={brief}
        rows="14"
        placeholder="The brief is empty — type what the agent should know, or just rely on your focus note."
      ></textarea>
    </div>
  {/if}

  {#snippet footer()}
    {#if phase === 'review'}
      <button class="btn" onclick={() => (phase = 'compose')} disabled={busy}>← Back</button>
      <button class="btn primary" disabled={busy || !canSubmit} onclick={primary}>
        {busy ? 'Sending…' : 'Send handover'}
      </button>
    {:else}
      <button class="btn" onclick={onclose}>Cancel</button>
      <button class="btn primary" disabled={busy || briefLoading || !canSubmit} onclick={primary}>
        {#if briefLoading}
          Generating…
        {:else if busy}
          Starting…
        {:else if reviewBrief}
          Generate brief →
        {:else}
          Hand over
        {/if}
      </button>
    {/if}
  {/snippet}
</Modal>

<style>
  .lead {
    margin: 0 0 14px;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--text-dim);
  }
  .lead strong {
    color: var(--text);
    font-weight: 600;
  }
  .note {
    margin: -6px 0 12px;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--status-idle, var(--text-dim));
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    border-radius: var(--radius-s);
    padding: 7px 9px;
  }
  .chip {
    display: inline-block;
    padding: 0 5px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    vertical-align: middle;
  }

  /* Segmented control: new vs existing target */
  .seg {
    display: flex;
    gap: 4px;
    padding: 3px;
    margin-bottom: 12px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .seg-btn {
    flex: 1;
    padding: 6px 10px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
  }
  .seg-btn.active {
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow);
  }
  .seg-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  .provider-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: 8px;
  }
  .provider-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    cursor: pointer;
    text-align: left;
    transition:
      border-color 130ms ease-out,
      background 130ms ease-out;
  }
  .provider-card:hover:not(:disabled) {
    background: color-mix(in srgb, var(--surface-2) 70%, var(--surface));
  }
  .provider-card.selected {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .provider-card.unavailable {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .provider-name {
    font-size: 13px;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .provider-desc {
    font-size: 11px;
    color: var(--text-dim);
  }
  .badge {
    font-size: 9.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 6px;
    border-radius: 99px;
  }
  .badge.muted {
    background: color-mix(in srgb, var(--text-dim) 20%, transparent);
    color: var(--text-dim);
  }

  /* Existing-agent list */
  .agent-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 180px;
    overflow-y: auto;
  }
  .agent-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    cursor: pointer;
    text-align: left;
  }
  .agent-row:hover {
    background: color-mix(in srgb, var(--surface-2) 70%, var(--surface));
  }
  .agent-row.selected {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .agent-title {
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  textarea.input {
    width: 100%;
    resize: vertical;
    font: inherit;
    line-height: 1.45;
  }
  textarea.brief {
    line-height: 1.5;
  }

  .opts {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 4px;
  }
  .toggle-row {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    padding: 2px 0;
    cursor: pointer;
  }
  .toggle-row input {
    margin-top: 2px;
  }
  .toggle-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .toggle-title {
    font-size: 12.5px;
    font-weight: 600;
  }
</style>
