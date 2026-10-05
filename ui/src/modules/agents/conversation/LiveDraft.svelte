<script lang="ts">
  // The agent's in-progress response, read off the terminal screen by the
  // live tail (`transcript_live`, ≤ 1 frame / 700 ms). The provider writes a
  // transcript record only when a block COMPLETES, so this is the only way to
  // see text arrive before the folded turn lands. Shown while the session is
  // working; hidden as soon as the last folded turn already contains the
  // draft's tail (the screen keeps showing finished text).
  interface Props {
    text: string;
    /** Markdown of the last folded assistant text (dedupe against it). */
    lastText: string;
  }
  let { text, lastText }: Props = $props();

  /** Strip a leading part already covered by the folded turn: find the last
   *  ~48 chars of the folded text inside the draft and keep what follows. */
  const visible = $derived.by(() => {
    const draft = text.replace(/\r/g, '');
    if (!draft.trim()) return '';
    const probe = lastText.replace(/\s+/g, ' ').trim().slice(-48);
    if (probe.length >= 16) {
      const flat = draft.replace(/\s+/g, ' ');
      const at = flat.lastIndexOf(probe);
      if (at >= 0) {
        // Map the flattened index back approximately: drop the same share of
        // characters from the raw draft (whitespace runs are the only delta).
        const keepFlat = flat.slice(at + probe.length).trim();
        if (!keepFlat) return '';
        const idx = draft.lastIndexOf(keepFlat.slice(0, 24));
        return idx >= 0 ? draft.slice(idx).trimEnd() : keepFlat;
      }
    }
    return draft.trimEnd();
  });
  // Claude Code prefixes each streamed block with a "⏺" bullet — terminal
  // chrome, not content.
  const shown = $derived(visible.replace(/^\s*⏺\s?/gm, ''));
</script>

<!-- Not a live region: it re-renders ~every 700 ms while text streams; the
     chat announces the finished response instead (ConversationView). -->
{#if visible}
  <div class="live-draft" data-live-draft>
    <p class="draft" dir="auto">{shown}<span class="caret" aria-hidden="true"></span></p>
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
      animation: blink 1s steps(2, start) infinite;
    }
  }
  @keyframes blink {
    to {
      visibility: hidden;
    }
  }
  .draft-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
