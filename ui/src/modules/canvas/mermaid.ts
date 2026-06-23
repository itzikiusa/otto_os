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

/** Resolve (and one-time-initialize) the mermaid module. Lazy + memoized. */
async function load(): Promise<MermaidApi> {
  if (_mermaid) return _mermaid;
  _loading ??= import('mermaid').then((m) => {
    const api = m.default as unknown as MermaidApi;
    // `neutral` reads well on both light/dark surfaces; `loose` lets us render
    // the wider diagram set (sequence/flow/class/state/er) without HTML escaping
    // tripping over labels. startOnLoad:false — we drive render() manually.
    api.initialize({ startOnLoad: false, theme: 'neutral', securityLevel: 'loose' });
    _mermaid = api;
    return api;
  });
  return _loading;
}

/** Render `src` to an SVG string. Returns `{ error }` on any parse/render
 *  failure (mermaid leaves a stray error node in the DOM otherwise — caller
 *  renders our message instead). `id` must be unique & DOM-id-safe per node. */
export async function renderMermaid(
  id: string,
  src: string,
): Promise<{ svg?: string; error?: string }> {
  const text = src.trim();
  if (!text) return { error: 'Empty diagram' };
  try {
    const api = await load();
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
    const api = await load();
    await api.parse(text);
    return true;
  } catch {
    return false;
  }
}
