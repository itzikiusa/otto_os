// Incremental streaming splitter for `Markdown.svelte`: cuts the completed
// top-level blocks (`blocks`, each rendered once while the answer streams)
// from the active trailing paragraph or open code fence (`tail`, re-rendered
// on every delta). `blocks.join('') + tail === md`.

const REF_DEF_OR_FOOTNOTE_RE = /(?:^\s*\[[^\]^]+\]:|\[\^[^\]]+\])/m;

export interface StreamingMdSplit {
  blocks: string[];
  tail: string;
}

export function splitStreamingMarkdown(md: string): StreamingMdSplit {
  // Short blocks or documents using reference-style links/footnotes render in
  // one pass so cross-paragraph link definitions always resolve.
  if (md.length < 320 || REF_DEF_OR_FOOTNOTE_RE.test(md)) {
    return { blocks: [], tail: md };
  }

  const blocks: string[] = [];
  let openFence: string | null = null;
  let lastCut = 0;
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
      // Blank line outside any open fence: no cut when the next line continues
      // a list (an indented line, or the next item of a loose list) or a table.
      const nextStart = nl === -1 ? md.length : nl + 1;
      const nextNl = md.indexOf('\n', nextStart);
      const nextLine = md.slice(nextStart, nextNl === -1 ? md.length : nextNl);
      if (nextLine.length > 0 && !/^(?: {2,}|\t|\||(?:[-*+]|\d+[.)])\s)/.test(nextLine)) {
        blocks.push(md.slice(lastCut, nextStart));
        lastCut = nextStart;
      }
    }

    if (nl === -1) break;
    pos = nl + 1;
  }

  return { blocks, tail: md.slice(lastCut) };
}

/** Per-block html for one streaming answer: a settled block renders once (and
 *  keeps the same html string while its source is unchanged), the open tail
 *  on every call. */
export function streamingBlocksRenderer(render: (md: string) => string): (md: string) => string[] {
  let memo: { md: string; html: string }[] = [];
  return (md) => {
    const { blocks, tail } = splitStreamingMarkdown(md);
    const out = blocks.map((b, i) => {
      if (memo[i]?.md !== b) memo[i] = { md: b, html: render(b) };
      return memo[i].html;
    });
    memo = memo.slice(0, blocks.length);
    if (tail) out.push(render(tail));
    return out;
  };
}
