<script lang="ts">
  // The agent's in-progress response, read off the terminal screen by the
  // live tail (`transcript_live`, ≤ 1 frame / 700 ms). The provider writes a
  // transcript record only when a block COMPLETES, so this is the only way to
  // see text arrive before the folded turn lands. Shown while the session is
  // working; hidden as soon as the last folded turn already contains the
  // draft's tail (the screen keeps showing finished text).
  import { detectBlockDirection } from '../../../lib/textDirection';

  interface Props {
    text: string;
    /** Markdown of the last folded assistant text (dedupe against it). */
    lastText: string;
  }
  let { text, lastText }: Props = $props();

  function plainFlat(s: string): string {
    return s
      .replace(/^\s*[⏺•]\s?/gm, '')
      .replace(/[*_`~]+/g, '')
      .replace(/\s+/g, ' ')
      .trim();
  }

  /** Strip a leading part already covered by the folded turn: normalize
   *  bullets + inline markdown so plain terminal output matches folded markdown,
   *  and hide completely when the folded turn already covers the draft. */
  const visible = $derived.by(() => {
    const draft = text.replace(/\r/g, '').replace(/^\s*[⏺•]\s?/gm, '');
    const flatDraft = plainFlat(draft);
    if (!flatDraft) return '';
    const flatLast = plainFlat(lastText);
    if (flatLast && (flatLast.endsWith(flatDraft) || flatLast.includes(flatDraft))) return '';
    const probe = flatLast.slice(-48);
    if (probe.length >= 8) {
      const at = flatDraft.lastIndexOf(probe);
      if (at >= 0) {
        const keepFlat = flatDraft.slice(at + probe.length).trim();
        if (!keepFlat) return '';
        const idx = draft.lastIndexOf(keepFlat.slice(0, 24));
        return idx >= 0 ? draft.slice(idx).trimEnd() : keepFlat;
      }
    }
    return draft.trimEnd();
  });
  const shown = $derived(visible);
</script>

<!-- Not a live region: it re-renders ~every 700 ms while text streams; the
     chat announces the finished response instead (ConversationView). -->
{#if visible}
  <div class="live-draft" data-live-draft>
    <p class="draft" dir={detectBlockDirection(shown)}>{shown}<span class="caret" aria-hidden="true"></span></p>
    <div class="draft-note">Live preview from the terminal</div>
  </div>
{/if}

<style>
  /* The response continuing: the agent's green rail, dashed while it streams. */
  .live-draft {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    border-inline-start: 2px dashed var(--border-strong);
    background: color-mix(in srgb, var(--agent, transparent) 4%, transparent);
    padding-block: 8px;
    padding-inline: 18px 16px;
    margin-inline-start: 10px;
    border-start-end-radius: var(--radius-l);
    border-end-end-radius: var(--radius-l);
  }
  .draft {
    margin: 0;
    max-height: 50vh;
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: var(--fs-m);
    line-height: 1.6;
    color: var(--text);
    unicode-bidi: plaintext;
    text-align: start;
  }
  .caret {
    display: inline-block;
    width: 7px;
    height: 1em;
    margin-inline-start: 2px;
    vertical-align: text-bottom;
    background: var(--text-dim);
    border-radius: 1px;
  }
  @media (prefers-reduced-motion: no-preference) {
    .caret {
      animation: otto-blink 1s steps(2, start) infinite;
    }
  }
  .draft-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
