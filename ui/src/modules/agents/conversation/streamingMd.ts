// Incremental streaming splitter for `Markdown.svelte`: separates completed
// top-level blocks (`settled`, cached once per paragraph/fence in `mdCache`)
// from the active trailing paragraph or open code fence (`tail`, rendered
// without polluting `mdCache` on every intermediate token delta).

const REF_DEF_OR_FOOTNOTE_RE = /(?:^\s*\[[^\]^]+\]:|\[\^[^\]]+\])/m;

export interface StreamingMdSplit {
  settled: string;
  tail: string;
}

export function splitStreamingMarkdown(md: string): StreamingMdSplit {
  // Short blocks or documents using reference-style links/footnotes render in
  // one pass so cross-paragraph link definitions always resolve.
  if (md.length < 320 || REF_DEF_OR_FOOTNOTE_RE.test(md)) {
    return { settled: '', tail: md };
  }

  let openFence: string | null = null;
  let lastSafeSplit = -1;
  let pos = 0;

  while (pos < md.length) {
    const nl = md.indexOf('\n', pos);
    const lineEnd = nl === -1 ? md.length : nl;
    const rawLine = md.slice(pos, lineEnd);
    const trimmed = rawLine.trimStart();

    const fenceMatch = /^(```+|~~~+)/.exec(trimmed);
    if (fenceMatch) {
      const marker = fenceMatch[1];
      if (!openFence) {
        openFence = marker;
      } else if (
        marker[0] === openFence[0] &&
        marker.length >= openFence.length &&
        trimmed.slice(marker.length).trim() === ''
      ) {
        openFence = null;
      }
    } else if (!openFence && trimmed === '' && pos > 0) {
      // Blank line outside any open fence: check that the next line isn't
      // indented continuation inside a list item (`  ...`) or a table row (`|`).
      const nextStart = nl === -1 ? md.length : nl + 1;
      const nextNl = md.indexOf('\n', nextStart);
      const nextLine = md.slice(nextStart, nextNl === -1 ? md.length : nextNl);
      if (
        nextLine.length > 0 &&
        !/^(?: {2,}|\t|\|)/.test(nextLine)
      ) {
        lastSafeSplit = nextStart;
      }
    }

    if (nl === -1) break;
    pos = nl + 1;
  }

  if (lastSafeSplit <= 0 || lastSafeSplit >= md.length) {
    return { settled: '', tail: md };
  }
  return {
    settled: md.slice(0, lastSafeSplit),
    tail: md.slice(lastSafeSplit),
  };
}
