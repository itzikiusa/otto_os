<script lang="ts">
  // One conflict segment: ours/theirs shown side-by-side (reusing the diff
  // split-row look), with an action bar to pick the resolution — Use ours /
  // Use theirs / Both / Edit. The chosen resolution is reported back up as the
  // array of resolved lines (or null while undecided) via `onresolve`.
  import Icon from '../../lib/components/Icon.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';

  type Choice = 'ours' | 'theirs' | 'both' | 'edit' | null;

  interface Props {
    ours: string[];
    theirs: string[];
    base: string[];
    /** 1-based index of this conflict within the file (for the label). */
    index: number;
    /** Repo path of the conflicted file — drives CodeEditor language detection. */
    path?: string;
    /** Work-tree root for the CodeEditor (LSP/context); optional. */
    root?: string;
    /** Fired whenever the resolution changes. `lines` is null while undecided. */
    onresolve: (lines: string[] | null) => void;
  }
  let { ours, theirs, base, index, path = '', root = '', onresolve }: Props = $props();

  let choice = $state<Choice>(null);
  // Edit-mode buffer (joined text the user can hand-edit).
  let editText = $state('');
  // Whether the diff3 merge base is currently expanded.
  let showBase = $state(false);

  // ≤1024 (phone + tablet): the ours/theirs side-by-side columns are too narrow
  // to read, so the table stacks vertically (ours block over theirs block) with
  // its own per-side label. matchMedia drives a class toggle on the table.
  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const sync = () => (isMobile = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });
  // True when the parser populated a merge base (diff3 conflict style).
  const hasBase = $derived(base.length > 0);

  // The resolved lines for the current choice.
  const resolved = $derived.by((): string[] | null => {
    switch (choice) {
      case 'ours':
        return ours;
      case 'theirs':
        return theirs;
      case 'both':
        return [...ours, ...theirs];
      case 'edit':
        // An empty editor still counts as a (deliberate) empty resolution.
        return editText.length === 0 ? [] : editText.split('\n');
      default:
        return null;
    }
  });

  // Propagate every resolution change to the parent.
  $effect(() => {
    onresolve(resolved);
  });

  function pick(c: Exclude<Choice, 'edit' | null>): void {
    choice = c;
  }

  function startEdit(): void {
    // Seed the editor with the "Both" merge so users start from something
    // sensible (ours then theirs), unless they had a prior choice.
    if (choice !== 'edit') {
      const seed =
        choice === 'ours'
          ? ours
          : choice === 'theirs'
            ? theirs
            : [...ours, ...theirs];
      editText = seed.join('\n');
    }
    choice = 'edit';
  }

  // Pad the two sides to equal height so the side-by-side rows line up.
  const splitRows = $derived.by(() => {
    const n = Math.max(ours.length, theirs.length);
    const rows: { left: string | null; right: string | null }[] = [];
    for (let i = 0; i < n; i++) {
      rows.push({ left: ours[i] ?? null, right: theirs[i] ?? null });
    }
    return rows;
  });
</script>

<div class="hunk" class:resolved={choice !== null}>
  <div class="hunk-bar">
    <span class="hunk-label">
      <Icon name="merge" size={12} />
      Conflict {index}
    </span>
    {#if choice !== null}
      <span class="resolved-badge"><Icon name="check" size={11} /> resolved</span>
    {/if}
    <span class="grow"></span>
    <div class="seg">
      <button class:active={choice === 'ours'} onclick={() => pick('ours')} title="Keep our version">
        Use ours
      </button>
      <button class:active={choice === 'theirs'} onclick={() => pick('theirs')} title="Keep their version">
        Use theirs
      </button>
      <button class:active={choice === 'both'} onclick={() => pick('both')} title="Keep both (ours then theirs)">
        Both
      </button>
      <button class:active={choice === 'edit'} onclick={startEdit} title="Edit the resolution by hand">
        Edit
      </button>
    </div>
  </div>

  {#if choice === 'edit'}
    <div class="edit-wrap">
      <div
        class="edit-editor"
        style="height: {Math.min(Math.max(editText.split('\n').length, 4), 24) * 18 + 14}px"
      >
        <CodeEditor
          {path}
          {root}
          content={editText}
          readOnly={false}
          onchange={(v) => (editText = v)}
        />
      </div>
    </div>
  {:else}
    {#if hasBase}
      <div class="base-block">
        <button
          class="base-toggle"
          aria-expanded={showBase}
          onclick={() => (showBase = !showBase)}
          title="What both sides diverged from (merge base)"
        >
          <Icon name={showBase ? 'chevronDown' : 'chevronRight'} size={11} />
          <span class="base-label">BASE</span>
          <span class="base-hint dim">original — {base.length} line{base.length === 1 ? '' : 's'}</span>
        </button>
        {#if showBase}
          <pre class="base-code mono">{base.join('\n')}</pre>
        {/if}
      </div>
    {/if}
    {#if isMobile}
      <!-- Stacked ours/theirs blocks: the side-by-side columns are too narrow on
           a phone, so each side is its own full-width, labelled, scrolling block. -->
      <div class="stack-sides">
        <div class="stack-side ours" class:dim-side={choice === 'theirs'}>
          <div class="stack-label ours-label">OURS</div>
          {#if ours.length === 0}
            <pre class="stack-code mono empty-side">(empty)</pre>
          {:else}
            <pre class="stack-code mono">{ours.join('\n')}</pre>
          {/if}
        </div>
        <div class="stack-side theirs" class:dim-side={choice === 'ours'}>
          <div class="stack-label theirs-label">THEIRS</div>
          {#if theirs.length === 0}
            <pre class="stack-code mono empty-side">(empty)</pre>
          {:else}
            <pre class="stack-code mono">{theirs.join('\n')}</pre>
          {/if}
        </div>
      </div>
    {:else}
      <table class="split-table">
        <thead>
          <tr>
            <th class="side-head ours">OURS</th>
            <th class="side-head theirs">THEIRS</th>
          </tr>
        </thead>
        <tbody>
          {#each splitRows as row, ri (ri)}
            <tr>
              <td
                class="code mono half ours"
                class:void={row.left === null}
                class:dim-side={choice === 'theirs'}
              >{row.left ?? ''}</td>
              <td
                class="code mono half theirs"
                class:void={row.right === null}
                class:dim-side={choice === 'ours'}
              >{row.right ?? ''}</td>
            </tr>
          {/each}
          {#if splitRows.length === 0}
            <tr><td class="code dim" colspan="2">(empty on both sides)</td></tr>
          {/if}
        </tbody>
      </table>
    {/if}
  {/if}
</div>

<style>
  .hunk {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--surface);
  }
  .hunk.resolved {
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }
  .hunk-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    font-size: 11px;
  }
  .hunk-label {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-weight: 600;
    color: var(--text);
  }
  .resolved-badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10px;
    font-weight: 600;
    color: var(--accent);
  }
  .grow {
    flex: 1;
  }
  .seg {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    flex-shrink: 0;
  }
  .seg button {
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    padding: 3px 9px;
    cursor: pointer;
    border-inline-end: 1px solid var(--border);
    transition: background 100ms, color 100ms;
  }
  .seg button:last-child {
    border-inline-end: none;
  }
  .seg button:hover {
    background: var(--surface);
    color: var(--text);
  }
  .seg button.active {
    background: var(--accent);
    color: var(--accent-contrast);
    font-weight: 600;
  }

  .split-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 11.5px;
    line-height: 1.55;
    table-layout: fixed;
  }
  .side-head {
    text-align: start;
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.06em;
    padding: 2px 10px;
    color: var(--text-dim);
    width: 50%;
    border-bottom: 1px solid var(--border);
  }
  .side-head.ours {
    border-inline-end: 1px solid var(--border);
  }
  .code {
    padding: 0 10px;
    white-space: pre-wrap;
    word-break: break-all;
    user-select: text;
    vertical-align: top;
  }
  .half {
    width: 50%;
  }
  .half.ours {
    border-inline-end: 1px solid var(--border);
    background: color-mix(in srgb, var(--status-working) 9%, transparent);
  }
  .half.theirs {
    background: color-mix(in srgb, var(--accent) 11%, transparent);
  }
  .half.void {
    background: color-mix(in srgb, var(--text-dim) 6%, transparent);
  }
  .half.dim-side {
    opacity: 0.4;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }

  .edit-wrap {
    padding: 8px 10px;
  }
  .edit-editor {
    width: 100%;
    box-sizing: border-box;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    min-height: 80px;
    resize: vertical;
  }

  .base-block {
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--text-dim) 4%, transparent);
  }
  .base-toggle {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    padding: 3px 10px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 9.5px;
    cursor: pointer;
    text-align: start;
  }
  .base-toggle:hover {
    color: var(--text);
  }
  .base-label {
    font-weight: 700;
    letter-spacing: 0.06em;
  }
  .base-hint {
    font-weight: 500;
    letter-spacing: 0;
    text-transform: none;
  }
  .base-code {
    margin: 0;
    padding: 4px 10px 6px;
    /* Logical indent so the base block stays indented from the reading edge
       under the BASE toggle in RTL too. */
    padding-inline-start: 24px;
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--text-dim);
    white-space: pre-wrap;
    word-break: break-all;
    overflow-x: auto;
  }

  /* ── Stacked ours/theirs sides (≤1024px) ── */
  .stack-sides {
    display: flex;
    flex-direction: column;
  }
  .stack-side.ours {
    background: color-mix(in srgb, var(--status-working) 9%, transparent);
  }
  .stack-side.theirs {
    background: color-mix(in srgb, var(--accent) 11%, transparent);
    border-top: 1px solid var(--border);
  }
  .stack-side.dim-side {
    opacity: 0.4;
  }
  .stack-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    padding: 4px 10px 2px;
    color: var(--text-dim);
  }
  .stack-label.ours-label {
    color: color-mix(in srgb, var(--status-working) 75%, var(--text));
  }
  .stack-label.theirs-label {
    color: color-mix(in srgb, var(--accent) 80%, var(--text));
  }
  .stack-code {
    margin: 0;
    padding: 0 10px 6px;
    font-size: 12.5px;
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-word;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .empty-side {
    color: var(--text-dim);
    font-style: italic;
  }

  /* ── Mobile + tablet (≤1024px): the action bar wraps so all four resolution
     buttons stay reachable with real touch targets, and the bar/label don't
     clip off-screen on a narrow phone. ── */
  @media (max-width: 1024px) {
    .hunk-bar {
      flex-wrap: wrap;
      gap: 6px 8px;
      padding: 7px 10px;
    }
    .hunk-label {
      font-size: 12px;
    }
    /* Let the segmented control take a full row of its own and grow each button
       so they're tappable; they distribute evenly across the width. */
    .seg {
      flex-basis: 100%;
      flex-shrink: 1;
    }
    .seg button {
      flex: 1;
      font-size: 12px;
      padding: 8px 6px;
      min-height: 40px;
      white-space: nowrap;
    }
    /* Merge-base toggle: real touch target on a phone. */
    .base-toggle {
      min-height: 36px;
      font-size: 11px;
    }
  }

  /* Smallest phones (≤360px): let the segmented labels wrap rather than clip so
     "Use ours/Use theirs" stay readable when four buttons share the row. */
  @media (max-width: 360px) {
    .seg button {
      white-space: normal;
      line-height: 1.15;
      padding: 6px 4px;
    }
  }
</style>
