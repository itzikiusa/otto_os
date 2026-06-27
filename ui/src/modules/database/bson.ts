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

function esc(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/**
 * Render a value as syntax-highlighted, indented JSON **HTML** (for `{@html}` in
 * a `<pre>`): object keys, strings, numbers, booleans, null, and BSON sentinels
 * each get a `.json-*` span. All text is HTML-escaped. BSON sentinels render as
 * `ObjectId("…")` / `ISODate("…")` (unquoted, mongosh-style) rather than their
 * raw `{"$oid": …}` shape.
 */
export function highlightJsonHtml(value: unknown, indent = 0): string {
  const b = bsonScalar(value);
  if (b !== null) return `<span class="json-bson">${esc(b)}</span>`;

  if (value === null || value === undefined) return `<span class="json-null">null</span>`;
  const t = typeof value;
  if (t === 'string') return `<span class="json-str">${esc(JSON.stringify(value))}</span>`;
  if (t === 'number' || t === 'bigint') return `<span class="json-num">${esc(String(value))}</span>`;
  if (t === 'boolean') return `<span class="json-bool">${value}</span>`;

  const pad = '  '.repeat(indent);
  const padIn = '  '.repeat(indent + 1);
  if (Array.isArray(value)) {
    if (value.length === 0) return '[]';
    const items = value.map((v) => padIn + highlightJsonHtml(v, indent + 1));
    return `[\n${items.join(',\n')}\n${pad}]`;
  }
  if (t === 'object') {
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length === 0) return '{}';
    const items = entries.map(
      ([k, v]) =>
        `${padIn}<span class="json-key">${esc(JSON.stringify(k))}</span>: ${highlightJsonHtml(v, indent + 1)}`,
    );
    return `{\n${items.join(',\n')}\n${pad}}`;
  }
  return esc(String(value));
}
