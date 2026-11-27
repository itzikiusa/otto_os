<script lang="ts">
  import { plural } from '../../lib/plural';
  // One conflict segment, GitKraken-style: side A (ours) and side B (theirs)
  // shown side-by-side, each with a HEADER CHECKBOX (take the whole side) and
  // PER-LINE CHECKBOXES — so a resolution can mix parts of both sides. The
  // output order is A's picked lines then B's picked lines; "Edit" opens a
  // hand-edit buffer for anything fancier (reorder, rewrite). The resolution is
  // reported up as the array of resolved lines via `onresolve` (null while the
  // user hasn't touched the conflict yet).
  import Icon from '../../lib/components/Icon.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import { editLines, editSeed, isCrlfBlock } from './conflictEdit';

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
    /** The whole file's CRLF verdict (`fileIsCrlf`). A hunk with empty or
     *  single LF-less sides can't tell on its own (S15-308). */
    fileCrlf?: boolean;
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
    fileCrlf,
    onresolve,
  }: Props = $props();

  // Per-line picks. Undecided until the user touches ANY control — from then on
  // the resolution is exactly the checked lines (possibly none = delete block).
  // Raw arrays, replaced wholesale on every pick (never mutated in place).
  let oursSel = $state.raw<boolean[]>([]);
  let theirsSel = $state.raw<boolean[]>([]);
  /** Pick rows mounted per side before "Show more": a lockfile / generated-
   *  file conflict is 5–30k lines a side, and every line is a button. The
   *  whole-side checkboxes and Edit still act on ALL lines. */
  const LINE_PAGE = 300;
  let shown = $state(LINE_PAGE);
  let touched = $state(false);
  let editing = $state(false);
  /** The applied hand edit ("Done editing"). While set it IS the resolution —
   *  leaving edit mode used to fall back to the picks and drop the edit. A new
   *  line/side pick replaces it (the user chose to pick again). */
  let manual = $state.raw<string[] | null>(null);
  /** An EMPTY side has no lines to tick, so its "take all" state lives here
   *  (otherwise the native checkbox ticked while the bound value stayed false). */
  let emptyTaken = $state({ ours: false, theirs: false });
  // (Re)seed the pick arrays whenever the conflict content itself changes —
  // a new file load replaces the segment arrays wholesale.
  $effect(() => {
    oursSel = ours.map(() => false);
    theirsSel = theirs.map(() => false);
    touched = false;
    editing = false;
    manual = null;
    emptyTaken = { ours: false, theirs: false };
    shown = LINE_PAGE;
  });
  const hiddenLines = $derived(Math.max(ours.length, theirs.length) - shown);
  // Edit-mode buffer (joined text the user can hand-edit).
  let editText = $state('');
  // Whether the diff3 merge base is currently expanded.
  let showBase = $state(false);

  // True when the parser populated a merge base (diff3 conflict style).
  const hasBase = $derived(base.length > 0);

  const oursPicked = $derived(oursSel.filter(Boolean).length);
  const theirsPicked = $derived(theirsSel.filter(Boolean).length);
  const oursAll = $derived(ours.length > 0 ? oursPicked === ours.length : emptyTaken.ours);
  const theirsAll = $derived(theirs.length > 0 ? theirsPicked === theirs.length : emptyTaken.theirs);
  const sides = $derived([
    { side: 'ours' as const, tag: 'A', tagClass: 'tag-a', label: oursLabel, lines: ours, sel: oursSel, picked: oursPicked, all: oursAll },
    { side: 'theirs' as const, tag: 'B', tagClass: 'tag-b', label: theirsLabel, lines: theirs, sel: theirsSel, picked: theirsPicked, all: theirsAll },
  ]);
  // CodeMirror drops `\r`; a CRLF block gets it re-appended on the way out.
  // The file's verdict wins when given — the block alone is often too small.
  const crlf = $derived(fileCrlf ?? isCrlfBlock(ours, theirs, base));

  // The resolved lines: A's picks in order, then B's picks in order.
  const resolved = $derived.by((): string[] | null => {
    // An empty editor still counts as a (deliberate) empty resolution.
    if (editing) return editLines(editText, crlf);
    if (manual !== null) return manual;
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
    manual = null;
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
    manual = null;
    if ((side === 'ours' ? ours : theirs).length === 0) {
      emptyTaken = { ...emptyTaken, [side]: !emptyTaken[side] };
      return;
    }
    if (side === 'ours') oursSel = ours.map(() => !oursAll);
    else theirsSel = theirs.map(() => !theirsAll);
  }

  function startEdit(): void {
    if (!editing) {
      // Seed the editor with the current picks (or the "both" merge when the
      // conflict is still untouched) so users start from something sensible.
      const seed =
        manual ??
        (touched
          ? [...ours.filter((_, i) => oursSel[i]), ...theirs.filter((_, i) => theirsSel[i])]
          : [...ours, ...theirs]);
      editText = editSeed(seed);
    }
    editing = true;
  }

  /** "Done editing" APPLIES the edit: it becomes the resolution. */
  function stopEdit(): void {
    manual = editLines(editText, crlf);
    touched = true;
    editing = false;
  }

  /** Leave edit mode WITHOUT applying: back to the previous resolution. */
  function cancelEdit(): void {
    editing = false;
  }

</script>

<div class="hunk" class:resolved={resolved !== null}>
  <div class="hunk-bar">
    <span class="hunk-label">
      <Icon name="merge" size={12} />
      Conflict {index}
    </span>
    {#if resolved !== null}
      <span class="resolved-badge"><Icon name="check" size={12} /> resolved</span>
    {/if}
    <span class="grow"></span>
    {#if editing}
      <button class="edit-btn" onclick={cancelEdit} title="Discard this edit and go back to picking">
        Cancel edit
      </button>
      <button class="edit-done" onclick={stopEdit} title="Use the edited text as this conflict’s resolution">
        <Icon name="check" size={12} /> Apply edit
      </button>
    {:else}
      {#if manual !== null}
        <span class="side-partial" title="This conflict is resolved with your hand-edited text; picking a line replaces it">hand-edited</span>
      {/if}
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
          <Icon name={showBase ? 'chevronDown' : 'chevronRight'} size={12} />
          <span class="base-label">BASE</span>
          <span class="base-hint dim">original — {plural(base.length, 'line')}</span>
        </button>
        {#if showBase}
          <pre class="base-code mono">{base.join('\n')}</pre>
        {/if}
      </div>
    {/if}

    <!-- A / B: each side's take-it-all header over its pickable lines. Side
         by side while the card has room; a narrow card (container query)
         stacks them A over B — the DOM order (A head, A lines, B head, B
         lines) is the reading and tab order either way. -->
    <div class="sides">
      {#each sides as s (s.side)}
        {@const side = s.side}
        <label class="side-head {side}" title="Take all of {s.label}">
          <input
            type="checkbox"
            checked={s.all}
            onchange={() => toggleSide(side)}
            aria-label="Take all of {s.label} for conflict {index}"
          />
          <span class="side-tag {s.tagClass}">{s.tag}</span>
          <span class="side-name">{s.label}</span>
          {#if s.picked > 0 && !s.all}<span class="side-partial">{s.picked}/{s.lines.length}</span>{/if}
        </label>
        <div class="split-col {side}">
          {#if s.lines.length === 0}
            <div class="empty-side dim mono">(empty — taking {s.tag} deletes the block)</div>
          {:else}
            {#each s.lines.slice(0, shown) as line, i (i)}
              <button class="pick-line {side}" class:picked={s.sel[i]} aria-pressed={s.sel[i] === true} onclick={() => toggleLine(side, i)}>
                <span class="pick-box">{#if s.sel[i]}<Icon name="check" size={12} />{/if}</span>
                <span class="mono pick-code">{line}</span>
              </button>
            {/each}
          {/if}
        </div>
      {/each}
    </div>
    {#if hiddenLines > 0}
      <button class="more-lines" onclick={() => (shown += LINE_PAGE * 10)}>
        Show {plural(Math.min(hiddenLines, LINE_PAGE * 10), 'more line')} ({hiddenLines} hidden) — or take a whole side / Edit
      </button>
    {/if}
  {/if}
</div>

<style>
  .more-lines {
    display: block;
    width: 100%;
    padding: 4px 10px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--accent-text);
    text-align: start;
    border-top: 1px solid var(--border);
  }
  .more-lines:hover {
    background: var(--accent-soft);
  }
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
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 4px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-xs);
  }
  .hunk-label {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-weight: 600;
    color: var(--text);
  }
  .resolved-badge {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--accent-text);
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
    font-size: var(--fs-xs);
    padding: 2px 8px;
    cursor: pointer;
    transition: background var(--dur-fast), color var(--dur-fast);
  }
  .edit-btn:hover,
  .edit-done:hover {
    background: var(--surface);
    color: var(--text);
  }
  .edit-done {
    color: var(--accent-text);
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
  }

  /* ── A / B sides: two columns (A head, A lines | B head, B lines) while the
     card has room, one column (A over B) when it is narrow. ── */
  .sides {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    grid-template-rows: auto 1fr;
    grid-auto-flow: column;
  }
  .side-head {
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: .06em;
    color: var(--text-dim);
    cursor: pointer;
    user-select: none;
  }
  .side-head input[type='checkbox'] {
    margin: 0;
  }
  .side-head {
    border-bottom: 1px solid var(--border);
  }
  .side-head.ours {
    border-inline-end: 1px solid var(--border);
    background: color-mix(in srgb, var(--success) 7%, transparent);
  }
  .side-head.theirs {
    background: var(--accent-faint);
  }
  .side-tag {
    display: inline-grid;
    place-items: center;
    width: 14px;
    height: 14px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .tag-a {
    background: color-mix(in srgb, var(--success) 28%, transparent);
    color: var(--success);
  }
  .tag-b {
    background: var(--accent-soft-strong);
    color: var(--accent-text);
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
    color: var(--accent-text);
    background: var(--accent-soft);
    border-radius: 999px;
    padding: 0 6px;
    flex-shrink: 0;
  }
  /* ── Pickable lines ── */
  .split-col {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .split-col.ours {
    border-inline-end: 1px solid var(--border);
    background: color-mix(in srgb, var(--success) 5%, transparent);
  }
  .split-col.theirs {
    background: var(--accent-faint);
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
    background: var(--hover);
  }
  .pick-line.picked.ours {
    background: color-mix(in srgb, var(--success) 18%, transparent);
  }
  .pick-line.picked.theirs {
    background: var(--accent-soft-strong);
  }
  .pick-box {
    display: inline-grid;
    place-items: center;
    width: 13px;
    height: 13px;
    margin-top: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .pick-line.picked .pick-box {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, var(--surface));
  }
  .pick-code {
    font-size: var(--fs-xs);
    line-height: 1.55;
    white-space: pre-wrap;
    word-break: break-all;
    min-width: 0;
  }
  .empty-side {
    padding: 4px 10px;
    font-size: var(--fs-xs);
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
    gap: 4px;
    width: 100%;
    padding: 2px 10px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
    text-align: start;
  }
  .base-toggle:hover {
    color: var(--text);
  }
  .base-label {
    font-weight: 600;
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
    font-size: var(--fs-xs);
    line-height: 1.55;
    color: var(--text-dim);
    white-space: pre-wrap;
    word-break: break-all;
    overflow-x: auto;
  }

  .hunk {
    container-type: inline-size;
  }
  @container (max-width: 600px) {
    .sides {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: none;
      grid-auto-flow: row;
    }
    .side-head.ours,
    .split-col.ours {
      border-inline-end: none;
    }
    .side-head.theirs {
      border-top: 1px solid var(--border);
    }
    .pick-code {
      word-break: break-word;
      overflow-wrap: anywhere;
    }
  }

  /* ── Touch: real targets. ── */
  @media (pointer: coarse) {
    .hunk-bar {
      gap: 6px 8px;
      padding: 6px 10px;
    }
    .hunk-label {
      font-size: var(--fs-s);
    }
    .side-head {
      min-height: 38px;
      font-size: var(--fs-xs);
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
      font-size: var(--fs-s);
    }
    .base-toggle {
      min-height: 36px;
      font-size: var(--fs-xs);
    }
    .edit-btn,
    .edit-done {
      min-height: 36px;
      padding: 6px 12px;
      font-size: var(--fs-s);
    }
  }
</style>
