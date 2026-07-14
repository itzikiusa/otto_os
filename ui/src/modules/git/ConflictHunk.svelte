<script lang="ts">
  // One conflict segment, GitKraken-style: side A (ours) and side B (theirs)
  // shown side-by-side, each with a HEADER CHECKBOX (take the whole side) and
  // PER-LINE CHECKBOXES — so a resolution can mix parts of both sides. The
  // output order is A's picked lines then B's picked lines; "Edit" opens a
  // hand-edit buffer for anything fancier (reorder, rewrite). The resolution is
  // reported up as the array of resolved lines via `onresolve` (null while the
  // user hasn't touched the conflict yet).
  import Icon from '../../lib/components/Icon.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';

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
    /** Side labels — the checked-out branch vs the merge source. */
    oursLabel?: string;
    theirsLabel?: string;
    /** Fired whenever the resolution changes. `lines` is null while undecided. */
    onresolve: (lines: string[] | null) => void;
  }
  let {
    ours,
    theirs,
    base,
    index,
    path = '',
    root = '',
    oursLabel = 'OURS',
    theirsLabel = 'THEIRS',
    onresolve,
  }: Props = $props();

  // Per-line picks. Undecided until the user touches ANY control — from then on
  // the resolution is exactly the checked lines (possibly none = delete block).
  let oursSel = $state<boolean[]>([]);
  let theirsSel = $state<boolean[]>([]);
  let touched = $state(false);
  let editing = $state(false);
  // (Re)seed the pick arrays whenever the conflict content itself changes —
  // a new file load replaces the segment arrays wholesale.
  $effect(() => {
    oursSel = ours.map(() => false);
    theirsSel = theirs.map(() => false);
    touched = false;
    editing = false;
  });
  // Edit-mode buffer (joined text the user can hand-edit).
  let editText = $state('');
  // Whether the diff3 merge base is currently expanded.
  let showBase = $state(false);

  // ≤1024 (phone + tablet): the A/B side-by-side columns are too narrow to
  // read, so the table stacks vertically (A block over B block).
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

  const oursPicked = $derived(oursSel.filter(Boolean).length);
  const theirsPicked = $derived(theirsSel.filter(Boolean).length);
  const oursAll = $derived(ours.length > 0 && oursPicked === ours.length);
  const theirsAll = $derived(theirs.length > 0 && theirsPicked === theirs.length);

  // The resolved lines: A's picks in order, then B's picks in order.
  const resolved = $derived.by((): string[] | null => {
    if (editing) {
      // An empty editor still counts as a (deliberate) empty resolution.
      return editText.length === 0 ? [] : editText.split('\n');
    }
    if (!touched) return null;
    return [
      ...ours.filter((_, i) => oursSel[i]),
      ...theirs.filter((_, i) => theirsSel[i]),
    ];
  });

  // Propagate every resolution change to the parent.
  $effect(() => {
    onresolve(resolved);
  });

  function toggleLine(side: 'ours' | 'theirs', i: number): void {
    touched = true;
    if (side === 'ours') {
      const next = [...oursSel];
      next[i] = !next[i];
      oursSel = next;
    } else {
      const next = [...theirsSel];
      next[i] = !next[i];
      theirsSel = next;
    }
  }

  /** Header checkbox: take (or drop) a whole side. Works on an EMPTY side too —
   *  taking an empty side is a deliberate "delete this block". */
  function toggleSide(side: 'ours' | 'theirs'): void {
    touched = true;
    if (side === 'ours') oursSel = ours.map(() => !oursAll);
    else theirsSel = theirs.map(() => !theirsAll);
  }

  function startEdit(): void {
    if (!editing) {
      // Seed the editor with the current picks (or the "both" merge when the
      // conflict is still untouched) so users start from something sensible.
      const seed = touched
        ? [...ours.filter((_, i) => oursSel[i]), ...theirs.filter((_, i) => theirsSel[i])]
        : [...ours, ...theirs];
      editText = seed.join('\n');
    }
    editing = true;
  }

  function stopEdit(): void {
    editing = false;
    // Leaving edit keeps whatever picks were there before; if none, the hunk
    // returns to undecided (resolved → null) which is the honest state.
  }

</script>

<div class="hunk" class:resolved={resolved !== null}>
  <div class="hunk-bar">
    <span class="hunk-label">
      <Icon name="merge" size={12} />
      Conflict {index}
    </span>
    {#if resolved !== null}
      <span class="resolved-badge"><Icon name="check" size={11} /> resolved</span>
    {/if}
    <span class="grow"></span>
    {#if editing}
      <button class="edit-done" onclick={stopEdit} title="Back to side-by-side picking">
        <Icon name="check" size={11} /> Done editing
      </button>
    {:else}
      <button class="edit-btn" onclick={startEdit} title="Edit the resolution by hand">
        Edit
      </button>
    {/if}
  </div>

  {#if editing}
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

    <!-- Side headers: A / B with take-the-whole-side checkboxes. -->
    <div class="side-heads" class:stacked={isMobile}>
      <label class="side-head ours" title="Take all of {oursLabel}">
        <input
          type="checkbox"
          checked={oursAll}
          onchange={() => toggleSide('ours')}
          aria-label="Take all of {oursLabel} for conflict {index}"
        />
        <span class="side-tag tag-a">A</span>
        <span class="side-name">{oursLabel}</span>
        {#if oursPicked > 0 && !oursAll}<span class="side-partial">{oursPicked}/{ours.length}</span>{/if}
      </label>
      {#if !isMobile}
        <label class="side-head theirs" title="Take all of {theirsLabel}">
          <input
            type="checkbox"
            checked={theirsAll}
            onchange={() => toggleSide('theirs')}
            aria-label="Take all of {theirsLabel} for conflict {index}"
          />
          <span class="side-tag tag-b">B</span>
          <span class="side-name">{theirsLabel}</span>
          {#if theirsPicked > 0 && !theirsAll}<span class="side-partial">{theirsPicked}/{theirs.length}</span>{/if}
        </label>
      {/if}
    </div>

    {#if isMobile}
      <!-- Stacked A block over B block (each with its own header + line picks). -->
      <div class="stack-sides">
        <div class="stack-side ours">
          {#if ours.length === 0}
            <div class="empty-side dim mono">(empty — taking A deletes the block)</div>
          {:else}
            {#each ours as line, i (i)}
              <button class="pick-line ours" class:picked={oursSel[i]} onclick={() => toggleLine('ours', i)}>
                <span class="pick-box">{#if oursSel[i]}<Icon name="check" size={10} />{/if}</span>
                <span class="mono pick-code">{line}</span>
              </button>
            {/each}
          {/if}
        </div>
        <label class="side-head theirs stacked-b" title="Take all of {theirsLabel}">
          <input
            type="checkbox"
            checked={theirsAll}
            onchange={() => toggleSide('theirs')}
            aria-label="Take all of {theirsLabel} for conflict {index}"
          />
          <span class="side-tag tag-b">B</span>
          <span class="side-name">{theirsLabel}</span>
          {#if theirsPicked > 0 && !theirsAll}<span class="side-partial">{theirsPicked}/{theirs.length}</span>{/if}
        </label>
        <div class="stack-side theirs">
          {#if theirs.length === 0}
            <div class="empty-side dim mono">(empty — taking B deletes the block)</div>
          {:else}
            {#each theirs as line, i (i)}
              <button class="pick-line theirs" class:picked={theirsSel[i]} onclick={() => toggleLine('theirs', i)}>
                <span class="pick-box">{#if theirsSel[i]}<Icon name="check" size={10} />{/if}</span>
                <span class="mono pick-code">{line}</span>
              </button>
            {/each}
          {/if}
        </div>
      </div>
    {:else}
      <div class="split-grid">
        <div class="split-col ours">
          {#if ours.length === 0}
            <div class="empty-side dim mono">(empty — taking A deletes the block)</div>
          {:else}
            {#each ours as line, i (i)}
              <button class="pick-line ours" class:picked={oursSel[i]} onclick={() => toggleLine('ours', i)}>
                <span class="pick-box">{#if oursSel[i]}<Icon name="check" size={10} />{/if}</span>
                <span class="mono pick-code">{line}</span>
              </button>
            {/each}
          {/if}
        </div>
        <div class="split-col theirs">
          {#if theirs.length === 0}
            <div class="empty-side dim mono">(empty — taking B deletes the block)</div>
          {:else}
            {#each theirs as line, i (i)}
              <button class="pick-line theirs" class:picked={theirsSel[i]} onclick={() => toggleLine('theirs', i)}>
                <span class="pick-box">{#if theirsSel[i]}<Icon name="check" size={10} />{/if}</span>
                <span class="mono pick-code">{line}</span>
              </button>
            {/each}
          {/if}
        </div>
      </div>
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
  .edit-btn,
  .edit-done {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    padding: 3px 9px;
    cursor: pointer;
    transition: background 100ms, color 100ms;
  }
  .edit-btn:hover,
  .edit-done:hover {
    background: var(--surface);
    color: var(--text);
  }
  .edit-done {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }

  /* ── Side headers (A / B) ── */
  .side-heads {
    display: grid;
    grid-template-columns: 1fr 1fr;
    border-bottom: 1px solid var(--border);
  }
  .side-heads.stacked {
    grid-template-columns: 1fr;
  }
  .side-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    cursor: pointer;
    user-select: none;
  }
  .side-head input[type='checkbox'] {
    margin: 0;
  }
  .side-head.ours {
    border-inline-end: 1px solid var(--border);
    background: color-mix(in srgb, var(--status-working) 7%, transparent);
  }
  .side-heads.stacked .side-head.ours {
    border-inline-end: none;
  }
  .side-head.theirs {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .side-tag {
    display: inline-grid;
    place-items: center;
    width: 14px;
    height: 14px;
    border-radius: 3px;
    font-size: 9px;
    font-weight: 800;
  }
  .tag-a {
    background: color-mix(in srgb, var(--status-working) 28%, transparent);
    color: var(--status-working);
  }
  .tag-b {
    background: color-mix(in srgb, var(--accent) 26%, transparent);
    color: var(--accent);
  }
  .side-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .side-partial {
    font-weight: 600;
    letter-spacing: 0;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-radius: 999px;
    padding: 0 6px;
    flex-shrink: 0;
  }
  .stacked-b {
    border-top: 1px solid var(--border);
  }

  /* ── Pickable lines ── */
  .split-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
  }
  .split-col {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .split-col.ours {
    border-inline-end: 1px solid var(--border);
    background: color-mix(in srgb, var(--status-working) 5%, transparent);
  }
  .split-col.theirs {
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }
  .pick-line {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    width: 100%;
    padding: 1px 8px;
    border: none;
    background: transparent;
    cursor: pointer;
    text-align: start;
    color: var(--text);
  }
  .pick-line:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .pick-line.picked.ours {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
  }
  .pick-line.picked.theirs {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .pick-box {
    display: inline-grid;
    place-items: center;
    width: 13px;
    height: 13px;
    margin-top: 2px;
    border: 1px solid var(--border);
    border-radius: 3px;
    background: var(--surface);
    color: var(--accent);
    flex-shrink: 0;
  }
  .pick-line.picked .pick-box {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, var(--surface));
  }
  .pick-code {
    font-size: 11.5px;
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-all;
    min-width: 0;
  }
  .empty-side {
    padding: 4px 10px;
    font-size: 11px;
    font-style: italic;
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
    padding-inline-start: 24px;
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--text-dim);
    white-space: pre-wrap;
    word-break: break-all;
    overflow-x: auto;
  }

  /* ── Stacked sides (≤1024px) ── */
  .stack-sides {
    display: flex;
    flex-direction: column;
  }
  .stack-side.ours {
    background: color-mix(in srgb, var(--status-working) 5%, transparent);
  }
  .stack-side.theirs {
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }

  /* ── Mobile + tablet (≤1024px): real touch targets. ── */
  @media (max-width: 1024px) {
    .hunk-bar {
      flex-wrap: wrap;
      gap: 6px 8px;
      padding: 7px 10px;
    }
    .hunk-label {
      font-size: 12px;
    }
    .side-head {
      min-height: 38px;
      font-size: 11px;
    }
    .side-head input[type='checkbox'] {
      width: 17px;
      height: 17px;
    }
    .pick-line {
      padding: 6px 10px;
    }
    .pick-box {
      width: 17px;
      height: 17px;
      margin-top: 1px;
    }
    .pick-code {
      font-size: 12.5px;
      word-break: break-word;
      overflow-wrap: anywhere;
    }
    .base-toggle {
      min-height: 36px;
      font-size: 11px;
    }
    .edit-btn,
    .edit-done {
      min-height: 36px;
      padding: 6px 12px;
      font-size: 12px;
    }
  }
</style>
