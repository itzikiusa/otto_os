// A person's label wherever a raw user id would otherwise be primary text
// (content.md §6 "show the label, never the id"): "you" for the signed-in
// user, else the workspace member's display name (or username), else the id
// shortened. Callers keep the full id in a `title` for anyone who needs it.

/** The member fields this label reads (`MemberEntry` satisfies it). */
export interface LabelMember {
  user_id: string;
  display_name?: string | null;
  username?: string | null;
}

/** "01J9…X4QZ": an id shortened for display (full value goes in `title`). */
export function shortUserId(id: string): string {
  return id.length > 12 ? `${id.slice(0, 4)}…${id.slice(-4)}` : id;
}

/** "you", the member's name, or the shortened id; `fallback` when there is no id. */
export function userLabel(
  id: string | null | undefined,
  meId: string | null | undefined,
  members: readonly LabelMember[] = [],
  fallback = 'unknown',
): string {
  if (!id) return fallback;
  if (meId && id === meId) return 'you';
  const m = members.find((x) => x.user_id === id);
  const name = m?.display_name?.trim() || m?.username?.trim();
  return name || shortUserId(id);
}
