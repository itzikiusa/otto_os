/** A `require` for modules transpiled into a `node:vm` context: each import
 * path must be mapped EXPLICITLY (matched by suffix, first match wins). An
 * unmapped path throws instead of silently resolving to `{}` — a `{}` stub
 * turns every missing export into `undefined`, so a store could call an
 * unstubbed boundary and the test would pass on a code path that never ran. */
export function strictRequire(map: ReadonlyArray<readonly [suffix: string, exports: unknown]>): (p: string) => unknown {
  return (p: string) => {
    for (const [suffix, exports] of map) if (p.endsWith(suffix)) return exports;
    throw new Error(`unit harness: unmapped require('${p}') — add it to the strictRequire map`);
  };
}

/** Exports for an import the code under test must NOT reach in this test:
 * any property read (i.e. any use of an imported binding) throws. Map a path
 * to this instead of `{}` so "unused here" is a checked claim. */
export function unused(path: string): unknown {
  return new Proxy({}, {
    get(_t, k) {
      if (k === '__esModule' || typeof k === 'symbol') return undefined;
      throw new Error(`unit harness: '${path}' is mapped as unused, but the code read .${String(k)}`);
    },
  });
}
