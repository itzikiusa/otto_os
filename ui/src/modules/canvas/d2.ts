// Lazy D2 bridge. @terrastruct/d2 is a 7.8MB WASM bundle so it MUST stay out of
// the main chunk — dynamic import on first use, one shared instance after.
//
// VERIFIED against node_modules/@terrastruct/d2/index.d.ts + a runtime smoke test
// (not just the README): `D2` is a named export; `compile()`'s options are FLAT
// `CompileOptions` (`{ sketch, themeID, … }`) — the `.d.ts`'s `Omit<CompileRequest,
// "fs">` overload nests them under `options:`, but that nesting is silently
// IGNORED at runtime (renderOptions came back at defaults). `render()` resolves a
// plain SVG string, not `{svg}`. A compile error throws with `.message` set to a
// JSON array of `{range, errmsg}` objects, not a plain "Error: …" string — parsed
// below into one readable line. Dark themes start at 200 ("Dark Mauve").

type D2CompileOptions = { sketch?: boolean; themeID?: number };
export type D2Api = {
  dispose?: () => void;
  readonly disposed?: boolean;
  ready?: Promise<unknown>;
  worker?: { terminate: () => void };
  compile: (
    src: string,
    opts?: D2CompileOptions,
  ) => Promise<{ diagram: unknown; renderOptions: Record<string, unknown> }>;
  render: (diagram: unknown, opts?: Record<string, unknown>) => Promise<string>;
};

let _d2: D2Api | null = null;
let _loading: Promise<D2Api> | null = null;

/** Resolve (and one-time-construct) the D2 module. Lazy + memoized. A FAILED
 *  load (flaky network / chunk-load error on the 7.8MB WASM import) must not
 *  stay cached — clear the memo so the next render retries instead of leaving
 *  D2 broken for the whole session. */
async function load(): Promise<D2Api> {
  if (_d2?.disposed) { _d2 = null; _loading = null; }
  if (_d2) return _d2;
  _loading ??= import('@terrastruct/d2')
    .then(async (m) => {
      let api = new m.D2() as unknown as D2Api;
      try {
        await api.ready;
      } catch (error) {
        api.worker?.terminate();
        if (!(error instanceof Error) || !/maximum call stack size exceeded|out of stack space/i.test(error.message)) throw error;
        const assets = (m as unknown as { ottoD2Assets: () => Promise<{ source: string; wasm: ArrayBuffer }> }).ottoD2Assets;
        const [{ createD2Frame }, runtime] = await Promise.all([import('./d2-frame'), assets()]);
        api = await createD2Frame(runtime.source, runtime.wasm);
      }
      _d2 = api;
      return api;
    })
    .catch((e: unknown) => {
      _loading = null;
      throw e;
    });
  return _loading;
}

/** The D2 worker bridge holds a SINGLE currentResolve/currentReject slot, so
 *  concurrent compile/render calls clobber each other: caller A receives
 *  caller B's compile result (an object — rendered as "[object Object]") and
 *  caller B's promise never settles (its diagram stays stuck as raw source).
 *  Notes with 2+ d2 fences hit this every time. Serialize every worker
 *  round-trip pair through one queue; failures must not wedge the chain. */
let _queue: Promise<unknown> = Promise.resolve();
function enqueue<T>(job: () => Promise<T>): Promise<T> {
  const next = _queue.then(job, job);
  _queue = next.catch(() => undefined);
  return next;
}

function invalidateTransport(api: D2Api, error: unknown): void {
  if (!(error instanceof Error) || error.name !== 'D2TransportError') return;
  api.dispose?.();
  if (_d2 === api) { _d2 = null; _loading = null; }
}

/** A compile error's `.message` is a JSON array of `{errmsg}` — flatten it into
 *  one readable line; fall back to the raw message for anything else. */
function friendlyError(raw: string): string {
  try {
    const parsed = JSON.parse(raw) as Array<{ errmsg?: string }>;
    if (Array.isArray(parsed) && parsed.length) {
      const msgs = parsed.map((e) => e.errmsg).filter((m): m is string => !!m);
      if (msgs.length) return msgs.join('; ');
    }
  } catch {
    /* not JSON — fall through */
  }
  return raw.replace(/^Error:\s*/, '').trim() || 'Diagram error';
}

/** Render `src` (D2 source) to an SVG string. Returns `{ error }` on any
 *  compile/render failure — never throws — so the caller can show it inline. */
export async function renderD2(
  _id: string,
  src: string,
  opts: { sketch?: boolean; dark?: boolean } = {},
): Promise<{ svg?: string; error?: string }> {
  const text = src.trim();
  if (!text) return { error: 'Empty diagram' };
  try {
    const svg = await enqueue(async () => {
      const api = await load();
      try {
        const compiled = await api.compile(text, {
          sketch: opts.sketch ?? false,
          themeID: opts.dark ? 200 : 0,
        });
        return await api.render(compiled.diagram, compiled.renderOptions);
      } catch (error) {
        invalidateTransport(api, error);
        throw error;
      }
    });
    // Belt-and-braces: never hand a non-SVG payload to innerHTML.
    if (typeof svg !== 'string' || !svg.includes('<svg')) {
      return { error: 'D2 returned an unexpected render payload' };
    }
    return { svg };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    return { error: friendlyError(msg) };
  }
}

/** Best-effort validity check (true = compiles). Never throws. */
export async function parseD2(src: string): Promise<boolean> {
  const text = src.trim();
  if (!text) return false;
  try {
    await enqueue(async () => {
      const api = await load();
      try { await api.compile(text, {}); }
      catch (error) { invalidateTransport(api, error); throw error; }
    });
    return true;
  } catch {
    return false;
  }
}

if (typeof window !== 'undefined') {
  window.addEventListener('pagehide', event => {
    // The browser suspends and resumes bfcache pages, including queued renders.
    // Terminating their worker here would leave that queue waiting forever.
    if (event.persisted) return;
    _d2?.dispose?.(); _d2?.worker?.terminate(); _d2 = null; _loading = null;
  });
}
