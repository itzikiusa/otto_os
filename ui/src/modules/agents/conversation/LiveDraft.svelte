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
</script>

{#if visible}
  <article class="turn assistant live-draft" data-live-draft aria-live="polite">
    <div class="draft-head"><span class="pulse"></span> Streaming from the terminal</div>
    <pre class="draft mono" dir="ltr">{visible}</pre>
  </article>
{/if}

<style>
  .live-draft {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px 16px 8px;
    min-width: 0;
  }
  .draft-head {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .pulse {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--status-working, #3fb950);
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .draft {
    margin: 0;
    max-width: 920px;
    min-height: 3.5em;
    max-height: 50vh;
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text);
    background: color-mix(in srgb, var(--status-working, #3fb950) 6%, var(--surface));
    border: 1px dashed color-mix(in srgb, var(--status-working, #3fb950) 45%, var(--border));
    border-radius: var(--radius-m);
    padding: 8px 12px;
    text-align: start;
  }
</style>
