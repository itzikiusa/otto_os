// GraphQL request variables: the editor's JSON text → the `variables` object
// sent next to the query. Invalid JSON FAILS the send with the line it broke
// on — it used to be swapped for `{}` silently, which surfaced as a server
// "variable $id required" far from its cause.

/** Parse the variables editor text. Blank → `{}`; otherwise a JSON object, or
 *  an Error saying what is wrong and where. */
export function parseGraphqlVariables(text: string | null | undefined): Record<string, unknown> {
  const src = text ?? '';
  if (!src.trim()) return {};
  let v: unknown;
  try {
    v = JSON.parse(src);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    const lineCol = /\(line (\d+) column \d+\)/.exec(msg);
    const pos = /position (\d+)/.exec(msg);
    const line = lineCol ? Number(lineCol[1]) : pos ? src.slice(0, Number(pos[1])).split('\n').length : null;
    throw new Error(`GraphQL variables aren’t valid JSON${line ? ` (line ${line})` : ''}: ${msg}`);
  }
  if (v === null || typeof v !== 'object' || Array.isArray(v)) {
    throw new Error('GraphQL variables must be a JSON object, e.g. { "id": 1 }');
  }
  return v as Record<string, unknown>;
}
