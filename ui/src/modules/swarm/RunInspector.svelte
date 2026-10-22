<script lang="ts">
  // Run detail drawer: everything one swarm turn produced — the brief that was
  // sent, the cwd/worktree it ran in, the parsed artifacts as clickable rows
  // (open file / open PR), the board posts tagged with this run, tokens + cost,
  // and the raw result JSON. Mirrors workflows/RunSteps.svelte.
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import { runStatus, sentenceCase, type BadgeTone } from '../../lib/status';
  import { loadErrorText } from '../../lib/loadError';
  import { isAbortError } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { openExternal, isExternalUrl } from '../../lib/external';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import type { SwarmRun, SwarmMessage, TurnResult, TurnArtifact } from './types';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { formatCount, formatUsd } from '../../lib/metric-format';

  /** Finding severity → Badge tone (a warning is not a selection). */
  const SEV_TONE: Record<string, BadgeTone> = { error: 'bad', warn: 'warn', info: 'neutral' };

  interface Props {
    run: SwarmRun;
    onclose: () => void;
  }
  let { run, onclose }: Props = $props();

  const agent = $derived(swarm.agentById(run.agent_id));
  // The runs list is `lite` (no per-run result blob); a live event's run does
  // carry it. Otherwise fetch this one run in full, again when its status
  // changes (a finished turn writes the result).
  let fetched = $state.raw<SwarmRun | null>(null);
  let resultError = $state<string | null>(null);
  let resultAttempt = $state(0);
  $effect(() => {
    const id = run.id;
    const status = run.status;
    const has = run.result != null;
    const attempt = resultAttempt;
    void status;
    void attempt;
    if (has) {
      resultError = null;
      return;
    }
    const ctl = new AbortController();
    resultError = null;
    swarm
      .getRun(id, ctl.signal)
      .then((r) => {
        if (!ctl.signal.aborted) fetched = r;
      })
      .catch((e: unknown) => {
        if (ctl.signal.aborted || isAbortError(e)) return;
        resultError = loadErrorText(e);
      });
    return () => ctl.abort();
  });
  const result = $derived(
    (run.result ?? (fetched?.id === run.id ? fetched.result : null) ?? null) as TurnResult | null,
  );
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

  /** Absolute local date-time ("21/10/2026, 14:03") — NOT the shared
   *  relative `rel()`; named apart so the two are never confused. */
  function fmtWhen(ts?: string | null): string {
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
      await copyTextOrThrow(text);
      toasts.success(`Copied ${label}`);
    } catch {
      toasts.error('Couldn’t copy to the clipboard', 'Select the text and copy it manually.');
    }
  }

  const rawJson = $derived(result ? JSON.stringify(result, null, 2) : null);
  const runTitle = $derived.by(() => {
    const kind = sentenceCase(run.kind);
    return agent?.name ? `Run detail — ${agent.name} (${kind})` : `Run detail — ${kind}`;
  });
</script>

<Modal title={runTitle} width={640} {onclose}>
  <div class="insp">
    <!-- Header line: agent · kind · status · timing -->
    <div class="hdr">
      <span class="agent">{agent?.name ?? run.agent_id.slice(0, 8)}</span>
      <span class="dim">· {sentenceCase(run.kind)}</span>
      <!-- Same shared vocabulary as the Runs list (done → Succeeded, error →
           Failed, stopped → Cancelled) instead of the raw status word. -->
      <StatusBadge status={runStatus(run.status)} />
      <span class="grow"></span>
      {#if run.session_id}
        <button class="btn small ghost" aria-label="Open {agent?.name ?? 'this run'}’s session" onclick={() => { swarm.selectedSessionId = run.session_id!; onclose(); }}>
          <Icon name="terminal" size={12} /> Open session
        </button>
      {/if}
    </div>
    <div class="meta">
      <span class="dim">Enqueued {fmtWhen(run.enqueued_at)}</span>
      {#if run.started_at}<span class="dim">· Started {fmtWhen(run.started_at)}</span>{/if}
      {#if run.finished_at}<span class="dim">· Finished {fmtWhen(run.finished_at)}</span>{/if}
    </div>

    {#if run.error}
      <div class="err">{run.error}</div>
    {/if}

    <!-- Tokens + cost -->
    <div class="stats">
      <div class="stat">
        <span class="k">Input</span>
        <span class="v mono" title={run.tokens_input?.toLocaleString()}>{run.tokens_input == null ? '—' : formatCount(run.tokens_input)}</span>
      </div>
      <div class="stat">
        <span class="k">Output</span>
        <span class="v mono" title={run.tokens_output?.toLocaleString()}>{run.tokens_output == null ? '—' : formatCount(run.tokens_output)}</span>
      </div>
      <div class="stat">
        <span class="k">Cost</span>
        <span class="v mono">{formatUsd(run.cost_usd)}</span>
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
              <Badge tone={SEV_TONE[c.severity] ?? 'neutral'} label={sentenceCase(c.severity)} />
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
        <h3>Working folder</h3>
        <div class="path-row">
          <Icon name="folder" size={13} />
          <span class="path mono">{cwd}</span>
          <span class="grow"></span>
          <button class="copy-btn" title="Copy path" onclick={() => copy(cwd!, 'path')}>
            <Icon name="copy" size={12} /> Copy
          </button>
        </div>
      </section>
    {/if}

    <!-- Artifacts -->
    <section>
      <h3>Artifacts {#if artifacts.length}<span class="count">{artifacts.length}</span>{/if}</h3>
      {#if artifacts.length === 0 && result == null}
        <LoadState what="artifacts" variant="compact" loading={!resultError} error={resultError} empty={false} onretry={() => (resultAttempt += 1)} />
      {:else if artifacts.length === 0}
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
              {#if target && isExternalUrl(target)}<Icon name="external" size={12} />{:else if target}<Icon name="link" size={12} />{/if}
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
            <Icon name="copy" size={12} /> Copy
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
                <Badge variant="outline" label={sentenceCase(m.kind)} />
                <span class="who">{author(m)}</span>
                <span class="grow"></span>
                <span class="dim time">{fmtWhen(m.created_at)}</span>
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
            <Icon name="copy" size={12} /> Copy
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-xs);
    margin-top: -8px;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-xs);
  }
  .err {
    color: var(--danger);
    background: var(--danger-soft);
    padding: 6px 8px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
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
    gap: 2px;
  }
  .stat .k {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .stat .v {
    font-size: var(--fs-l);
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
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .sec-h {
    display: flex;
    align-items: center;
  }
  .count {
    color: var(--accent-text);
    margin-inline-start: 4px;
  }
  .summary {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .path-row {
    display: flex;
    align-items: center;
    gap: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 8px;
    background: var(--surface);
  }
  .path {
    font-size: var(--fs-xs);
    word-break: break-all;
  }
  .arts {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .art {
    display: flex;
    align-items: center;
    gap: 8px;
    text-align: start;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 8px;
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
    opacity: var(--disabled-opacity);
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
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .block {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
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
    font-size: var(--fs-xs);
    margin-bottom: 4px;
  }
  .who {
    font-weight: 600;
  }
  .time {
    font-size: var(--fs-xs);
  }
  .post-body {
    font-size: var(--fs-s);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .copy-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
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
    gap: 4px;
  }
  .finding {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 8px;
    background: var(--surface);
  }
  .finding.error {
    border-color: color-mix(in srgb, var(--danger) 40%, var(--border));
  }
  .finding.warn {
    border-color: color-mix(in srgb, var(--warning) 40%, var(--border));
  }
  .finding-text {
    font-size: var(--fs-s);
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
