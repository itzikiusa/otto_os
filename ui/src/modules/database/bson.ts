// Display helpers for MongoDB result values. The Mongo driver returns typed
// values whose plain JSON form is ambiguous as MongoDB Extended JSON sentinels —
// `{"$oid": …}` (ObjectId), `{"$date": …}` (Date), `{"$numberDecimal": …}` — so
// the grid/JSON view can SHOW the real type (`ObjectId("…")` / `ISODate("…")`),
// which both tells the user what to query and round-trips: a "query by value" on
// such a cell re-emits the sentinel, which the runner's parser decodes back to
// the BSON type. SQL engines never produce these objects, so this is a no-op there.

/**
 * If `v` is a recognised BSON sentinel, return its mongosh-style display string
 * (`ObjectId("…")`, `ISODate("…")`, or the decimal text); otherwise `null`.
 */
export function bsonScalar(v: unknown): string | null {
  if (v === null || typeof v !== 'object' || Array.isArray(v)) return null;
  const o = v as Record<string, unknown>;
  const keys = Object.keys(o);
  if (keys.length !== 1) return null;
  const k = keys[0];
  if (k === '$oid' && typeof o.$oid === 'string') return `ObjectId("${o.$oid}")`;
  if (k === '$date') {
    const d = o.$date;
    let iso: string;
    if (typeof d === 'string') iso = d;
    else if (typeof d === 'number') iso = new Date(d).toISOString();
    else if (d && typeof d === 'object' && '$numberLong' in (d as object)) {
      const ms = Number((d as Record<string, unknown>).$numberLong);
      iso = Number.isFinite(ms) ? new Date(ms).toISOString() : String(d);
    } else iso = String(d);
    return `ISODate("${iso}")`;
  }
  if (k === '$numberDecimal' && typeof o.$numberDecimal === 'string') return o.$numberDecimal;
  return null;
}

// NOTE: the former `highlightJsonHtml` (whole-value → highlighted HTML string for
// `{@html}`) was removed with its last caller. It forced every branch of a document
// to be rendered up front, which is precisely what made fat Mongo results
// unusable. `JsonTree.svelte` replaces it with a lazily-expanded tree — and
// interpolates text instead of emitting markup, so it can't inject from data.
