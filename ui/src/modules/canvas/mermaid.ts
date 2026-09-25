// Lazy Mermaid bridge. Mermaid is heavy (and pulls d3) so it MUST stay out of
// the main bundle — we `await import('mermaid')` on first use only and reuse the
// resolved module afterwards. Everything here is best-effort: a bad diagram
// surfaces as an `error` string, never a thrown exception, so node renderers can
// show parse errors inline without crashing the canvas.

type MermaidApi = {
  initialize: (cfg: Record<string, unknown>) => void;
  render: (id: string, src: string) => Promise<{ svg: string }>;
  parse: (src: string) => Promise<unknown>;
};

let _mermaid: MermaidApi | null = null;
let _loading: Promise<MermaidApi> | null = null;
/** The theme mermaid is currently initialized with (initialize is global). */
let _dark: boolean | null = null;
/** Renders run one at a time: a light export must never interleave with a
 *  dark on-screen render between `initialize` and `render`. */
let _queue: Promise<unknown> = Promise.resolve();

// `neutral` on light surfaces, `dark` on dark ones — `neutral` draws dark
// labels and lines on a transparent background, which vanished on the dark
// pasteboard. `strict` (not `loose`): the SVG is injected with {@html} and its
// source can be agent-written or come from a vault note, so labels are
// sanitized and `click` directives / javascript: links are refused.
// startOnLoad:false — we drive render() manually.
//
// The per-diagram knobs below de-clutter the output (the chief complaint on
// dense sequence diagrams): a UI sans font, generous spacing, NO mirrored
// actors at the bottom, wrapped labels, and useMaxWidth so the SVG fits the
// node's width instead of overflowing.
function config(dark: boolean): Record<string, unknown> {
  return {
    startOnLoad: false,
    theme: dark ? 'dark' : 'neutral',
    securityLevel: 'strict',
    fontFamily: 'ui-sans-serif, -apple-system, system-ui, sans-serif',
    flowchart: { useMaxWidth: true, htmlLabels: true, curve: 'basis', padding: 16 },
    sequence: {
      useMaxWidth: true,
      mirrorActors: false,
      wrap: true,
      boxMargin: 12,
      actorMargin: 64,
      messageMargin: 44,
      noteMargin: 12,
      bottomMarginAdj: 4,
    },
    class: { useMaxWidth: true },
    state: { useMaxWidth: true },
    er: { useMaxWidth: true },
  };
}

/** Resolve the mermaid module (lazy + memoized) initialized for `dark`. */
async function load(dark: boolean): Promise<MermaidApi> {
  _loading ??= import('mermaid').then((m) => (_mermaid = m.default as unknown as MermaidApi));
  const api = _mermaid ?? (await _loading);
  if (_dark !== dark) {
    api.initialize(config(dark));
    _dark = dark;
  }
  return api;
}


/** Render `src` to an SVG string. Returns `{ error }` on any parse/render
 *  failure (mermaid leaves a stray error node in the DOM otherwise — caller
 *  renders our message instead). `id` must be unique & DOM-id-safe per node.
 *  `dark: true` draws light-on-transparent for a dark surface (the Canvas
 *  pasteboard); the default suits a light/white figure (vault notes, Design
 *  Hall frames and thumbnails, exports). */
export function renderMermaid(
  id: string,
  src: string,
  opts: { dark?: boolean } = {},
): Promise<{ svg?: string; error?: string }> {
  const run = _queue.then(() => renderNow(id, src, opts.dark ?? false));
  _queue = run.catch(() => undefined);
  return run;
}

async function renderNow(id: string, src: string, dark: boolean): Promise<{ svg?: string; error?: string }> {
  const text = src.trim();
  if (!text) return { error: 'Empty diagram' };
  try {
    const api = await load(dark);
    // Pre-validate so a syntax error is reported cleanly rather than as a
    // half-rendered "Syntax error in graph" SVG.
    await api.parse(text);
    const { svg } = await api.render(id, text);
    return { svg };
  } catch (e) {
    // Mermaid throws strings or Error objects depending on the failure path.
    const msg = e instanceof Error ? e.message : String(e);
    return { error: msg.replace(/^Error:\s*/, '').trim() || 'Diagram error' };
  } finally {
    // Mermaid injects a temporary <div id="d{id}"> for measurement; clean any
    // orphan it leaves on error so it never piles up in the document body.
    try {
      document.getElementById(`d${id}`)?.remove();
      document.getElementById(id)?.remove();
    } catch {
      /* ignore */
    }
  }
}

/** Best-effort validity check (true = parses). Never throws. */
export async function parseMermaid(src: string): Promise<boolean> {
  const text = src.trim();
  if (!text) return false;
  try {
    const api = await load(_dark ?? false);
    await api.parse(text);
    return true;
  } catch {
    return false;
  }
}
