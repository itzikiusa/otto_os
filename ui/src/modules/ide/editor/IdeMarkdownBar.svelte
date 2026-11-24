<script lang="ts">
  // The Markdown formatting row above the IDE editor (Markdown files only, not in
  // Preview): Heading ▾, Bold, Italic, Code, Link, lists, Checklist, Quote, Table and a ⋯
  // with the rarer blocks and "Format table". The commands live in cm-md-assist.ts (the
  // editor's own keys and "/" menu run the same ones). Buttons never take the editor's
  // focus, so the selection stays. On a phone the row scrolls inside itself with 44 px
  // targets; Editor · Split · Preview stays at the end of the tab strip.
  import { EditorView } from '@codemirror/view';
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import {
    BLOCKS,
    formatTableAtCursor,
    insertBlock,
    insertLink,
    setHeading,
    toggleLinePrefix,
    toggleWrap,
  } from '../../../lib/components/cm-md-assist';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { toasts } from '../../../lib/toast.svelte';

  let rowEl = $state<HTMLDivElement | null>(null);

  /** The primary pane's editor (the row sits above it). */
  function view(): EditorView | null {
    const el = rowEl?.closest('[data-testid="ide-editor-area"]')?.querySelector<HTMLElement>('.ide-panes > .ide-pane:not(.split-secondary) .cm-editor');
    return el ? EditorView.findFromDOM(el) : null;
  }
  function run(cmd: (v: EditorView) => boolean): void {
    const v = view();
    if (!v) return;
    cmd(v);
    v.focus();
  }
  const block = (label: string) => (v: EditorView) => {
    insertBlock(v, BLOCKS.find((b) => b.label === label)!);
    return true;
  };

  interface Btn { id: string; label: string; keys?: string; icon?: IconName; glyph?: string; cmd: (v: EditorView) => boolean }
  const BUTTONS: Btn[] = [
    { id: 'bold', label: 'Bold', keys: '⌘B', glyph: 'B', cmd: toggleWrap('**') },
    { id: 'italic', label: 'Italic', glyph: 'I', cmd: toggleWrap('*') },
    { id: 'code', label: 'Code', icon: 'code', cmd: toggleWrap('`') },
    { id: 'link', label: 'Link', keys: 'paste a URL over text', icon: 'link', cmd: insertLink },
    { id: 'bullet', label: 'Bulleted list', keys: 'Tab / ⇧Tab to indent', icon: 'listBullet', cmd: toggleLinePrefix('bullet') },
    { id: 'number', label: 'Numbered list', icon: 'listNumbered', cmd: toggleLinePrefix('number') },
    { id: 'check', label: 'Checklist', icon: 'listCheck', cmd: toggleLinePrefix('check') },
    { id: 'quote', label: 'Quote', icon: 'quote', cmd: toggleLinePrefix('quote') },
    { id: 'table', label: 'Table', keys: 'Tab moves between cells', icon: 'grid', cmd: block('Table') },
  ];
  const tip = (b: Btn): string => (b.keys ? `${b.label} (${b.keys})` : b.label);

  function headingMenu(e: MouseEvent): void {
    const items: MenuItem[] = [1, 2, 3].map((n) => ({ label: `Heading ${n}`, hint: '#'.repeat(n), action: () => run(setHeading(n)) }));
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { align: 'start' });
  }
  function moreMenu(e: MouseEvent): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      {
        label: 'Format table',
        icon: 'format',
        action: () => run((v) => formatTableAtCursor(v) || (toasts.info('Put the cursor in a table first'), true)),
      },
      { separator: true },
      ...['Code block', 'Divider', 'Callout'].map((label) => ({ label, action: () => run(block(label)) })),
      { separator: true },
      { label: 'Type / on an empty line for every block', disabled: true },
    ], { align: 'end' });
  }
  /** Keep the editor's focus (and selection) when a button is clicked (mousedown: on
   *  WebKit a prevented pointerdown also swallows a tap's click). */
  const keepFocus = (e: Event): void => e.preventDefault();
</script>

<div class="md-row" role="toolbar" aria-label="Markdown formatting" data-testid="ide-md-toolbar" bind:this={rowEl}>
  <button type="button" class="md-btn md-heading" title="Heading" aria-label="Heading" aria-haspopup="menu" onmousedown={keepFocus} onclick={headingMenu} data-testid="ide-md-heading">
    <span class="md-glyph">H</span><Icon name="chevronDown" size={12} />
  </button>
  {#each BUTTONS as b (b.id)}
    <button type="button" class="md-btn" title={tip(b)} aria-label={tip(b)} onmousedown={keepFocus} onclick={() => run(b.cmd)} data-testid={`ide-md-${b.id}`}>
      {#if b.icon}<Icon name={b.icon} size={14} />{:else}<span class="md-glyph" class:i={b.id === 'italic'}>{b.glyph}</span>{/if}
    </button>
  {/each}
  <button type="button" class="md-btn" title="More blocks" aria-label="More blocks" aria-haspopup="menu" onmousedown={keepFocus} onclick={moreMenu} data-testid="ide-md-more">
    <Icon name="more" size={14} />
  </button>
</div>

<style>
  .md-row {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-block: 2px;
    padding-inline: var(--sp-2);
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
    min-inline-size: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .md-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 1px;
    min-inline-size: 28px;
    block-size: 26px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
  }
  .md-btn:hover {
    background: var(--hover);
    color: var(--text);
  }
  .md-btn:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .md-glyph {
    font-weight: 600;
    font-size: var(--fs-m);
    line-height: 1;
  }
  .md-glyph.i {
    font-style: italic;
  }
  @media (pointer: coarse) {
    .md-btn {
      min-inline-size: 44px;
      block-size: 44px;
    }
  }
</style>
