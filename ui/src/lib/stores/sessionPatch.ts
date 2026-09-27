// Session-list patching — the PURE half (no runes) so `node --test` can
// exercise it; used by the workspace store's session events.

/** `list` with the item `id` replaced by `patch(item)` — or the SAME array when
 *  the id isn't in it or the patch returns the item unchanged. Session events
 *  used to `.map()` both session lists unconditionally; every new array re-ran
 *  ~15 deriveds (sorted channel / other-workspace groups…), the palette
 *  registry and the notification rows, even for a session in neither list. */
export function patchSessionIn<T extends { id: string }>(list: T[], id: string, patch: (s: T) => T): T[] {
  const i = list.findIndex((s) => s.id === id);
  if (i < 0) return list;
  const cur = list[i];
  const next = patch(cur);
  if (next === cur) return list;
  const out = list.slice();
  out[i] = next;
  return out;
}
