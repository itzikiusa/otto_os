<script lang="ts">
  // The WIP panel's commit composer (split out of WipPanel.svelte): Summary +
  // Description, the agent Draft (and its watchable session), Amend / Sign and
  // the one primary Commit. Rendering only — the panel owns the message
  // state, the draft lifecycle (commitComposer.ts) and the commit itself.
  import type { Snippet } from 'svelte';
  import type { CommitConfig } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import AgentByline from '../../../lib/components/AgentByline.svelte';
  import LazyTerminal from '../../../lib/components/LazyTerminal.svelte';

  interface Props {
    subject: string;
    body: string;
    amend: boolean;
    signOn: boolean;
    showDraftTerm: boolean;
    amendSubject: string;
    amendPublished: string | null;
    drafting: boolean;
    draftElapsed: number;
    draftable: boolean;
    draftedAt: number | null;
    liveDraftId: string | null;
    committing: boolean;
    canCommit: boolean;
    commitHint: string;
    stagedCount: number;
    signCfg: CommitConfig | null;
    /** The "couldn't read signing settings" note + its Retry (panel-owned). */
    signError?: Snippet;
    onkey: (e: KeyboardEvent) => void;
    ondraft: () => void;
    oncommit: () => void;
  }
  let {
    subject = $bindable(), body = $bindable(), amend = $bindable(), signOn = $bindable(), showDraftTerm = $bindable(),
    amendSubject, amendPublished, drafting, draftElapsed, draftable, draftedAt, liveDraftId, committing, canCommit,
    commitHint, stagedCount, signCfg, signError, onkey, ondraft, oncommit,
  }: Props = $props();

  /** git's 50/72 convention: a quiet count from 50, warning past 72. */
  const len = $derived(subject.trim().length);
</script>

<div class="wp-composer">
  <div class="wc-subject">
    <input dir="auto"
      class="input subject-input"
      bind:value={subject}
      placeholder={amend
        ? amendSubject
          ? `Keep “${amendSubject}” — or type a new message`
          : 'Keep the previous message — or type a new one'
        : 'Summary (required)'}
      aria-label="Commit summary"
      spellcheck="false"
      onkeydown={onkey}
    />
    {#if len > 50}
      <span class="wc-len" class:over={len > 72} title="Summaries read best under 72 characters">{len}</span>
    {/if}
  </div>
  <textarea dir="auto"
    class="input body-input"
    rows="3"
    bind:value={body}
    placeholder="Description — why this change, in a sentence or two"
    aria-label="Commit description (optional)"
    spellcheck="false"
    onkeydown={onkey}
  ></textarea>
  {#if draftedAt && (subject.trim() || body.trim())}
    <div class="wc-by"><AgentByline label="Draft" at={draftedAt} /></div>
  {/if}
  {#if liveDraftId && showDraftTerm}
    <div class="draft-term">
      <LazyTerminal sessionId={liveDraftId} preferDom showToolbar={false} />
    </div>
  {/if}
  {#if amendPublished}
    <div class="amend-warn" role="note">
      <Icon name="warning" size={12} />
      <span>This commit is already on {amendPublished}. Amending rewrites it — the next push will need a force push with lease.</span>
    </div>
  {/if}
  <div class="wc-options">
    <label class="wc-opt">
      <input type="checkbox" bind:checked={amend} />
      Amend
    </label>
    <label
      class="wc-opt"
      title={signCfg?.signing_key ? `Sign with ${signCfg.format ?? 'gpg'} key ${signCfg.signing_key}` : 'Sign this commit'}
    >
      <input type="checkbox" bind:checked={signOn} />
      Sign
    </label>
    {@render signError?.()}
    <span class="wc-grow"></span>
    <button
      class="btn small ghost draft-btn"
      disabled={drafting || committing || !draftable}
      onclick={ondraft}
      title={draftable
        ? 'Draft a summary + description from your staged changes'
        : 'Stage a file first — drafting can’t see untracked files'}
    >
      {#if drafting}
        <span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span> Drafting…{draftElapsed > 0 ? ` ${draftElapsed}s` : ''}
      {:else}
        <Icon name="sparkle" size={12} /> Draft message
      {/if}
    </button>
    {#if liveDraftId}
      <button
        class="btn small ghost draft-btn"
        onclick={() => (showDraftTerm = !showDraftTerm)}
        title={showDraftTerm ? 'Hide the drafting agent' : 'Watch the drafting agent live'}
      >
        <Icon name={showDraftTerm ? 'chevronUp' : 'terminal'} size={12} />
        {showDraftTerm ? 'Hide agent' : 'Watch agent'}
      </button>
    {/if}
  </div>
  <button
    class="btn primary wc-commit"
    disabled={!canCommit}
    onclick={oncommit}
    title={commitHint || 'Commit (⌘↵)'}
    aria-describedby={commitHint ? 'wp-commit-hint' : undefined}
  >
    <Icon name="commit" size={14} />
    {committing
      ? 'Committing…'
      : `${amend ? 'Amend' : 'Commit'}${stagedCount > 0 ? ` (${stagedCount})` : ''}`}
  </button>
  {#if commitHint}<span id="wp-commit-hint" class="wp-commit-hint">{commitHint}</span>{/if}
</div>

<style>
  .wp-composer {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px 12px;
    border-block-start: 1px solid var(--border);
    background: var(--surface);
  }
  .wc-subject {
    position: relative;
  }
  .subject-input {
    width: 100%;
    height: 30px;
    font-weight: 600;
  }
  .wc-subject:has(.wc-len) .subject-input {
    padding-inline-end: 36px;
  }
  .wc-len {
    position: absolute;
    inset-block: 0;
    inset-inline-end: 8px;
    display: grid;
    place-items: center;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    pointer-events: none;
  }
  .wc-len.over {
    color: var(--warning);
  }
  .body-input {
    width: 100%;
    min-height: 56px;
    resize: vertical;
  }
  .wc-by {
    padding: 2px 0;
  }
  .draft-term {
    height: 220px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    background: var(--term-bg);
  }
  .amend-warn {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .amend-warn :global(svg) {
    flex-shrink: 0;
    margin-block-start: 2px;
    color: var(--warning);
  }
  .wc-options {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 12px;
  }
  .wc-opt {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
  }
  .wc-grow {
    flex: 1;
  }
  /* The one primary: full width, so it reads as the end of the flow. */
  .wc-commit {
    width: 100%;
    justify-content: center;
    gap: 6px;
  }
  .wp-commit-hint {
    margin-block-start: -2px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    text-align: center;
  }

  @media (max-width: 1024px) {
    .subject-input {
      font-size: 16px; /* ui-guards: allow — 16px stops iOS zoom-on-focus */
      height: 38px;
    }
    .body-input {
      font-size: 16px; /* ui-guards: allow — 16px stops iOS zoom-on-focus */
      line-height: 1.45;
    }
    .wc-commit {
      height: 36px;
      font-size: var(--fs-l);
    }
  }
</style>
