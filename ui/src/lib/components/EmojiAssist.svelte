<!--
  Composer emoji for any chat textarea (lib/emojiInput.ts): Slack-style
  emoticons (`:) ` → 🙂, Backspace on it restores `:)`) and the `:shortcode`
  completion list. Mount it inside the composer's positioned box and call
  `onKey(e)` first from the textarea's keydown; `true` means it took the key.
  Edits go through `insertText`, so the textarea's own undo and `bind:value`
  see them as typing.
-->
<script lang="ts">
  import { EmoticonConverter, shortcodeQuery, type EmojiEdit } from '../emojiInput';
  import { emojiTableLoaded, loadEmojiTable, searchEmojiShortcodes, type EmojiSuggestion } from '../emojiShortcodes';

  interface Props {
    textareaEl: HTMLTextAreaElement | null | undefined;
  }
  let { textareaEl }: Props = $props();

  const id = $props.id();
  const converter = new EmoticonConverter();
  let query = $state<{ start: number; query: string } | null>(null);
  let dismissedAt = $state(-1);
  let active = $state(0);
  let tableReady = $state(emojiTableLoaded());
  const items = $derived.by((): EmojiSuggestion[] => {
    if (!query || query.start === dismissedAt || !tableReady) return [];
    return searchEmojiShortcodes(query.query, 8);
  });

  function sync(): void {
    const el = textareaEl;
    const q = el && el.selectionStart === el.selectionEnd && document.activeElement === el ? shortcodeQuery(el.value, el.selectionStart) : null;
    if (q && !tableReady) void loadEmojiTable().then(() => (tableReady = true));
    if (q?.start !== query?.start || q?.query !== query?.query) active = 0;
    query = q;
  }

  $effect(() => {
    const el = textareaEl;
    if (!el) return;
    // Deferred: Chromium fires blur while the focused composer is being
    // removed, inside the block effect's teardown, where a state write throws
    // (state_unsafe_mutation).
    const close = (): void => queueMicrotask(() => (query = null));
    for (const t of ['input', 'keyup', 'click'] as const) el.addEventListener(t, sync);
    el.addEventListener('blur', close);
    return () => {
      for (const t of ['input', 'keyup', 'click'] as const) el.removeEventListener(t, sync);
      el.removeEventListener('blur', close);
    };
  });

  function apply(el: HTMLTextAreaElement, e: EmojiEdit): void {
    el.setSelectionRange(e.from, e.to);
    if (!document.execCommand('insertText', false, e.insert)) {
      el.setRangeText(e.insert, e.from, e.to, 'end');
      el.dispatchEvent(new Event('input', { bubbles: true }));
    }
  }

  function accept(s: EmojiSuggestion): void {
    const el = textareaEl;
    if (!el || !query) return;
    const spaced = /\s/.test(el.value[el.selectionStart] ?? '');
    el.focus();
    apply(el, { from: query.start, to: el.selectionStart, insert: spaced ? s.emoji : `${s.emoji} ` });
    query = null;
  }

  export function onKey(e: KeyboardEvent): boolean {
    const el = textareaEl;
    if (!el || e.isComposing) return false;
    if (items.length > 0) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        active = (active + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length;
        return true;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        dismissedAt = query?.start ?? -1;
        return true;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey && !e.metaKey && !e.ctrlKey)) {
        e.preventDefault();
        accept(items[active]);
        return true;
      }
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return false;
    const edit = converter.key(e.key, el.value, el.selectionStart, el.selectionEnd);
    if (!edit) return false;
    e.preventDefault();
    apply(el, edit);
    return true;
  }
</script>

{#if items.length > 0}
  <div class="ea-pop" id="{id}-list" role="listbox" aria-label="Emoji" data-testid="emoji-pop">
    {#each items as s, i (s.shortcode)}
      <div
        id="{id}-opt-{i}"
        class="ea-row"
        class:active={i === active}
        role="option"
        tabindex="-1"
        aria-selected={i === active}
        onmousedown={(ev) => {
          ev.preventDefault();
          accept(s);
        }}
        onmouseenter={() => (active = i)}
      >
        <span class="ea-emoji" aria-hidden="true">{s.emoji}</span>
        <span class="ea-name mono" dir="ltr">:{s.shortcode}:</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .ea-pop {
    position: absolute;
    inset-inline: 0;
    bottom: calc(100% + 4px);
    z-index: 5;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: min(220px, 35vh);
    overflow-y: auto;
    padding: 4px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface-2);
  }
  .ea-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .ea-row.active {
    background: var(--accent-soft);
  }
  .ea-emoji {
    font-size: var(--fs-l);
  }
  .ea-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-dim);
  }
</style>
