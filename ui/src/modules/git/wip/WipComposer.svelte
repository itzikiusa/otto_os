<script lang="ts">
  // The commit card (split out of WipPanel.svelte): the message — drafted
  // from the diff when the panel asks, edited here — and ONE primary that
  // commits and pushes when there is somewhere to push ("Commit only" is the
  // secondary). Amend / Sign live in ⋯ and show as chips only when on.
  // Rendering only — the panel owns the message state, the draft lifecycle
  // (commitComposer.ts) and the commit itself.
  import type { Snippet } from 'svelte';
  import type { CommitConfig } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import LazyTerminal from '../../../lib/components/LazyTerminal.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';

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
    /** Every change in the tree (the "N of M included" line). */
    totalCount: number;
    /** Where "Commit and push" pushes ("origin/feat/x"); null → no push. */
    pushTo: string | null;
    signCfg: CommitConfig | null;
    /** The "couldn't read signing settings" note + its Retry (panel-owned). */
    signError?: Snippet;
    onkey: (e: KeyboardEvent) => void;
    ondraft: () => void;
    onstopdraft?: () => void;
    oncommit: (push: boolean) => void;
  }
  let {
    subject = $bindable(), body = $bindable(), amend = $bindable(), signOn = $bindable(), showDraftTerm = $bindable(),
    amendSubject, amendPublished, drafting, draftElapsed, draftable, draftedAt, liveDraftId, committing, canCommit,
    commitHint, stagedCount, totalCount, pushTo, signCfg, signError, onkey, ondraft, onstopdraft, oncommit,
  }: Props = $props();

  /** git's 50/72 convention: a quiet count from 50, warning past 72. */
  const len = $derived(subject.trim().length);
  const included = $derived(
    stagedCount === totalCount
      ? `${totalCount === 1 ? '1 file' : `${totalCount} files`} included`
      : `${stagedCount} of ${totalCount} files included`,
  );
  const verb = $derived(amend ? 'Amend' : 'Commit');

  function moreMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Amend the last commit', checked: amend, action: () => (amend = !amend) },
      {
        label: 'Sign this commit',
        checked: signOn,
        title: signCfg?.signing_key ? `Sign with ${signCfg.format ?? 'gpg'} key ${signCfg.signing_key}` : undefined,
        action: () => (signOn = !signOn),
      },
      { separator: true },
      {
        label: draftedAt ? 'Draft the message again' : 'Draft the message from the diff',
        icon: 'sparkle',
        disabled: drafting || committing || !draftable,
        title: draftable ? undefined : 'Include a file first — drafting can’t see new files',
        action: ondraft,
      },
      ...(liveDraftId ? [{ label: showDraftTerm ? 'Hide the drafting agent' : 'Watch the drafting agent', icon: 'terminal', action: () => (showDraftTerm = !showDraftTerm) }] : []),
    ]);
  }
</script>

<div class="wp-composer" data-testid="commit-card">
  <div class="wc-row">
    <div class="wc-text">
      <div class="wc-subject">
        <input dir="auto"
          class="input subject-input"
          bind:value={subject}
          placeholder={amend
            ? amendSubject
              ? `Keep “${amendSubject}” — or type a new message`
              : 'Keep the previous message — or type a new one'
            : drafting
              ? 'Drafting the message from the diff…'
              : 'Commit message'}
          aria-label="Commit summary"
          spellcheck="false"
          onkeydown={onkey}
        />
        {#if len > 50}
          <span class="wc-len" class:over={len > 72} title="Summaries read best under 72 characters">{len}</span>
        {/if}
      </div>
      <p class="wc-sub">
        {#if drafting}
          <span class="spinner" style="--spinner-size: 10px" aria-hidden="true"></span>
          <span>Drafting from the diff{draftElapsed > 0 ? ` · ${draftElapsed}s` : ''}</span>
          {#if onstopdraft}<button type="button" class="wc-link" onclick={onstopdraft}>Stop</button>{/if}
          <span aria-hidden="true">·</span>
        {:else if draftedAt && (subject.trim() || body.trim())}
          <span title="Drafted by the agent from the diff — edit it before you commit">Drafted from the diff ·</span>
        {/if}
        <span data-testid="commit-card-included">{included}</span>
        {#if pushTo}<span>· pushes to <span class="mono" dir="ltr">{pushTo}</span></span>{/if}
        {#if amend}<span class="chip wc-chip">Amend</span>{/if}
        {#if signOn}<span class="chip wc-chip">Signed</span>{/if}
        {#if !drafting && !subject.trim() && !body.trim() && draftable}
          <button type="button" class="wc-link draft-btn" onclick={ondraft} disabled={committing}><Icon name="sparkle" size={12} /> Draft from the diff</button>
        {/if}
      </p>
    </div>
    <div class="wc-actions">
      <button type="button" class="icon-btn wc-more" aria-haspopup="menu" aria-label="Commit options" title="Amend, sign, draft the message" onclick={moreMenu}>
        <Icon name="more" size={16} />
      </button>
      {#if pushTo}
        <button class="btn wc-btn wc-commit-only" disabled={!canCommit} onclick={() => oncommit(false)} title={commitHint || `${verb} without pushing`}>
          {committing ? `${verb === 'Amend' ? 'Amending' : 'Committing'}…` : `${verb} only`}
        </button>
      {/if}
      <button
        class="btn primary wc-btn wc-commit"
        disabled={!canCommit}
        onclick={() => oncommit(!!pushTo)}
        title={commitHint || (pushTo ? `${verb}, then push to ${pushTo} (⌘↵)` : `${verb} (⌘↵)`)}
        aria-describedby={commitHint ? 'wp-commit-hint' : undefined}
      >
        <Icon name={pushTo ? 'arrowUp' : 'commit'} size={14} />
        {committing
          ? `${verb === 'Amend' ? 'Amending' : 'Committing'}…`
          : pushTo
            ? `${verb} and push`
            : `${verb}${stagedCount > 0 ? ` (${stagedCount})` : ''}`}
      </button>
    </div>
  </div>
  <textarea dir="auto"
    class="input body-input"
    rows="2"
    bind:value={body}
    placeholder="Description (optional) — why this change"
    aria-label="Commit description (optional)"
    spellcheck="false"
    onkeydown={onkey}
  ></textarea>
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
  {@render signError?.()}
  {#if commitHint}<span id="wp-commit-hint" class="wp-commit-hint">{commitHint}</span>{/if}
</div>

<style>
  /* The card: pinned above the file list, the message reads as the page's
     title, the one primary at its end. */
  .wp-composer {
    container: wc / inline-size;
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 12px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface-2);
  }
  .wc-row {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }
  .wc-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .wc-subject {
    position: relative;
  }
  .subject-input {
    width: 100%;
    height: 34px;
    font-size: var(--fs-l);
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
  .wc-sub {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 6px;
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .wc-chip {
    font-size: var(--fs-xs);
  }
  .wc-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0;
    border: none;
    background: none;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .wc-link:hover:not(:disabled) {
    text-decoration: underline;
  }
  .wc-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .wc-commit {
    gap: 6px;
  }
  .body-input {
    width: 100%;
    min-height: 40px;
    resize: vertical;
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
  .wp-commit-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  /* A narrow card (the graph's detail pane, a phone): the actions take their
     own full-width row under the message. */
  @container wc (max-width: 520px) {
    .wc-row {
      flex-direction: column;
      align-items: stretch;
    }
    .wc-actions {
      justify-content: flex-end;
    }
    .wc-commit {
      flex: 1;
      justify-content: center;
    }
  }

  @media (max-width: 1024px) {
    .subject-input {
      font-size: 16px; /* ui-guards: allow — 16px stops iOS zoom-on-focus */
      height: 44px;
    }
    .body-input {
      font-size: 16px; /* ui-guards: allow — 16px stops iOS zoom-on-focus */
      line-height: 1.45;
    }
    .wc-btn,
    .wc-more {
      min-height: 44px;
    }
    .wc-btn {
      font-size: var(--fs-l);
    }
    .wc-more {
      min-width: 44px;
    }
  }
</style>
