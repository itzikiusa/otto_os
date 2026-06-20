<script lang="ts">
  // Run detail drawer: everything one swarm turn produced — the brief that was
  // sent, the cwd/worktree it ran in, the parsed artifacts as clickable rows
  // (open file / open PR), the board posts tagged with this run, tokens + cost,
  // and the raw result JSON. Mirrors workflows/RunSteps.svelte.
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { openExternal, isExternalUrl } from '../../lib/external';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import type { SwarmRun, SwarmMessage, TurnResult, TurnArtifact } from './types';

  interface Props {
    run: SwarmRun;
    onclose: () => void;
  }
  let { run, onclose }: Props = $props();

  const agent = $derived(swarm.agentById(run.agent_id));
  const result = $derived((run.result ?? null) as TurnResult | null);
  const brief = $derived(typeof result?.brief === 'string' ? result.brief : null);
  const cwd = $derived(typeof result?.cwd === 'string' ? result.cwd : null);
  const artifacts = $derived<TurnArtifact[]>(
    Array.isArray(result?.artifacts) ? (result!.artifacts as TurnArtifact[]) : [],
  );
  // Board posts tagged with this run (the store keeps the open swarm's board).
  const posts = $derived(swarm.board.filter((m) => m.run_id === run.id));

  const hasTokens = $derived(
    run.tokens_input != null || run.tokens_output != null || run.cost_usd != null,
  );

  function fmtTokens(n?: number | null): string {
    if (n == null) return '—';
    return n.toLocaleString();
  }
  function fmtCost(n?: number | null): string {
    if (n == null) return '—';
    return `$${n < 0.01 && n > 0 ? n.toFixed(4) : n.toFixed(2)}`;
  }
  function rel(ts?: string | null): string {
    if (!ts) return '—';
    return new Date(ts).toLocaleString();
  }
  function author(m: SwarmMessage): string {
    if (m.author_agent_id) return swarm.agentById(m.author_agent_id)?.name ?? 'agent';
    if (m.author_user_id) return 'you';
    return 'system';
  }

  function artifactKind(a: TurnArtifact): 'pr' | 'url' | 'file' {
    if (a.type === 'pr') return 'pr';
    if (a.path) return 'file';
    if (a.url) return isExternalUrl(a.url) ? 'url' : 'file';
    return 'file';
  }
  function artifactTarget(a: TurnArtifact): string | null {
    return a.url || a.path || null;
  }
  async function openArtifact(a: TurnArtifact): Promise<void> {
    const target = artifactTarget(a);
    if (!target) return;
    if (isExternalUrl(target)) {
      await openExternal(target);
      return;
    }
    // A local file path — copy it so the user can paste/open it (the daemon has
    // no generic "reveal in Finder" route; copying is the safe, faithful action).
    await copy(target, 'path');
  }

  async function copy(text: string, label = 'output'): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      toasts.success(`Copied ${label}`);
    } catch {
      toasts.error('Copy failed');
    }
  }

  const rawJson = $derived(run.result ? JSON.stringify(run.result, null, 2) : null);
</script>

<Modal title="Run detail" width={640} {onclose}>
  <div class="insp">
    <!-- Header line: agent · kind · status · timing -->
    <div class="hdr">
      <span class="agent">{agent?.name ?? run.agent_id.slice(0, 8)}</span>
      <span class="dim">· {run.kind}</span>
      <span class="badge {run.status}">{run.status}</span>
      <span class="grow"></span>
      {#if run.session_id}
        <button class="btn small ghost" onclick={() => { swarm.selectedSessionId = run.session_id!; onclose(); }}>
          <Icon name="terminal" size={12} /> Open session
        </button>
      {/if}
    </div>
    <div class="meta">
      <span class="dim">Enqueued {rel(run.enqueued_at)}</span>
      {#if run.started_at}<span class="dim">· Started {rel(run.started_at)}</span>{/if}
      {#if run.finished_at}<span class="dim">· Finished {rel(run.finished_at)}</span>{/if}
    </div>

    {#if run.error}
      <div class="err">{run.error}</div>
    {/if}

    <!-- Tokens + cost -->
    <div class="stats">
      <div class="stat">
        <span class="k">Input</span>
        <span class="v mono">{fmtTokens(run.tokens_input)}</span>
      </div>
      <div class="stat">
        <span class="k">Output</span>
        <span class="v mono">{fmtTokens(run.tokens_output)}</span>
      </div>
      <div class="stat">
        <span class="k">Cost</span>
        <span class="v mono">{fmtCost(run.cost_usd)}</span>
      </div>
    </div>
    {#if !hasTokens}
      <div class="muted small">No usage recorded for this run (usage tracking off, or not flushed yet).</div>
    {/if}

    <!-- Concerns / findings chips -->
    {#if result?.concerns?.length}
      <section>
        <h3>Findings <span class="count">{result.concerns.length}</span></h3>
        <div class="findings">
          {#each (result.concerns ?? []) as c, i (i)}
            <div class="finding {c.severity}">
              <span class="sev-chip {c.severity}">{c.severity}</span>
              <span class="finding-text">{c.text}</span>
            </div>
          {/each}
        </div>
      </section>
    {/if}

    <!-- Summary -->
    {#if result?.summary}
      <section>
        <h3>Summary</h3>
        <p class="summary">{result.summary}</p>
      </section>
    {/if}

    <!-- cwd / worktree -->
    {#if cwd}
      <section>
        <h3>Working directory</h3>
        <div class="path-row">
          <Icon name="folder" size={13} />
          <span class="path mono">{cwd}</span>
          <span class="grow"></span>
          <button class="copy-btn" title="Copy path" onclick={() => copy(cwd!, 'path')}>
            <Icon name="file" size={11} /> Copy
          </button>
        </div>
      </section>
    {/if}

    <!-- Artifacts -->
    <section>
      <h3>Artifacts {#if artifacts.length}<span class="count">{artifacts.length}</span>{/if}</h3>
      {#if artifacts.length === 0}
        <div class="muted small">No artifacts reported.</div>
      {:else}
        <div class="arts">
          {#each artifacts as a, i (i)}
            {@const kind = artifactKind(a)}
            {@const target = artifactTarget(a)}
            <button class="art" class:clickable={!!target} disabled={!target} onclick={() => openArtifact(a)}>
              <Icon name={kind === 'pr' ? 'pr' : kind === 'url' ? 'external' : 'file'} size={13} />
              <span class="art-label">{a.label || target || a.type}</span>
              {#if target}<span class="art-target mono dim">{target}</span>{/if}
              {#if target && isExternalUrl(target)}<Icon name="external" size={11} />{:else if target}<Icon name="link" size={11} />{/if}
            </button>
          {/each}
        </div>
      {/if}
    </section>

    <!-- Brief / prompt -->
    {#if brief}
      <section>
        <div class="sec-h">
          <h3>Brief sent</h3>
          <span class="grow"></span>
          <button class="copy-btn" title="Copy brief" onclick={() => copy(brief!, 'brief')}>
            <Icon name="file" size={11} /> Copy
          </button>
        </div>
        <pre class="block scrolly">{brief}</pre>
      </section>
    {/if}

    <!-- Board posts for this run -->
    <section>
      <h3>Board posts {#if posts.length}<span class="count">{posts.length}</span>{/if}</h3>
      {#if posts.length === 0}
        <EmptyState icon="comment" title="No posts" body="This run did not post to the team board." />
      {:else}
        <div class="posts">
          {#each posts as m (m.id)}
            <div class="post">
              <div class="post-h">
                <span class="chip">{m.kind}</span>
                <span class="who">{author(m)}</span>
                <span class="grow"></span>
                <span class="dim time">{rel(m.created_at)}</span>
              </div>
              <div class="post-body">{m.body}</div>
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <!-- Raw result JSON -->
    {#if rawJson}
      <section>
        <div class="sec-h">
          <h3>Raw result</h3>
          <span class="grow"></span>
          <button class="copy-btn" title="Copy JSON" onclick={() => copy(rawJson!, 'JSON')}>
            <Icon name="file" size={11} /> Copy
          </button>
        </div>
        <pre class="block json scrolly">{rawJson}</pre>
      </section>
    {/if}
  </div>
</Modal>

<style>
  .insp {
    display: flex;
    flex-direction: column;
    gap: 14px;
    font-size: 12.5px;
  }
  .hdr {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .agent {
    font-weight: 600;
  }
  .grow {
    flex: 1;
  }
  .meta {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    font-size: 11px;
    margin-top: -8px;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: 11px;
  }
  .err {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
    padding: 7px 9px;
    border-radius: var(--radius-s);
    font-size: 11.5px;
  }
  .stats {
    display: flex;
    gap: 8px;
  }
  .stat {
    flex: 1;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    background: var(--surface-2);
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .stat .k {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .stat .v {
    font-size: 14px;
    font-weight: 600;
  }
  .mono {
    font-family: var(--font-mono);
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  h3 {
    margin: 0;
    font-size: 10.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .sec-h {
    display: flex;
    align-items: center;
  }
  .count {
    color: var(--accent);
    margin-left: 4px;
  }
  .summary {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .path-row {
    display: flex;
    align-items: center;
    gap: 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 7px 9px;
    background: var(--surface);
  }
  .path {
    font-size: 11.5px;
    word-break: break-all;
  }
  .arts {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .art {
    display: flex;
    align-items: center;
    gap: 8px;
    text-align: left;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 7px 9px;
    background: var(--surface);
    color: var(--text);
    font: inherit;
  }
  .art.clickable {
    cursor: pointer;
  }
  .art.clickable:hover {
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
    background: color-mix(in srgb, var(--accent) 6%, var(--surface));
  }
  .art:disabled {
    opacity: 0.7;
  }
  .art-label {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 40%;
  }
  .art-target {
    flex: 1;
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .block {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-dim);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px;
    margin: 0;
    overflow-x: auto;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .scrolly {
    max-height: 300px;
    overflow: auto;
  }
  .posts {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .post {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    background: var(--surface);
  }
  .post-h {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    margin-bottom: 4px;
  }
  .who {
    font-weight: 600;
  }
  .time {
    font-size: 10.5px;
  }
  .post-body {
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .chip {
    border: 1px solid var(--border);
    background: transparent;
    font-size: 10.5px;
    padding: 1px 7px;
    border-radius: 999px;
  }
  .copy-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    font-size: 10px;
    padding: 2px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .copy-btn:hover {
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 50%, var(--border));
  }
  .muted {
    color: var(--text-dim);
  }
  .findings {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .finding {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 9px;
    background: var(--surface);
  }
  .finding.error {
    border-color: color-mix(in srgb, var(--status-exited) 40%, var(--border));
  }
  .finding.warn {
    border-color: color-mix(in srgb, var(--accent) 30%, var(--border));
  }
  .sev-chip {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: 999px;
    white-space: nowrap;
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
    color: var(--text-dim);
    flex: none;
  }
  .sev-chip.error {
    background: color-mix(in srgb, var(--status-exited) 20%, transparent);
    color: var(--status-exited);
  }
  .sev-chip.warn {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
  }
  .sev-chip.info {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
  }
  .finding-text {
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .badge {
    font-size: 10.5px;
    padding: 1px 7px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    color: var(--text-dim);
  }
  .badge.running,
  .badge.waiting {
    background: color-mix(in srgb, var(--status-working) 22%, transparent);
    color: var(--status-working);
  }
  .badge.done {
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
  }
  .badge.error,
  .badge.stopped {
    background: color-mix(in srgb, var(--status-exited) 22%, transparent);
    color: var(--status-exited);
  }
</style>
