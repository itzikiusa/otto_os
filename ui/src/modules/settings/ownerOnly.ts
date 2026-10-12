// Daemon-wide settings writes need Otto's owner in person (PUT /settings,
// secrets: `require_human`; notifications: root). A share link, a paired
// phone or an agent session authorizes as the owner and can open these pages,
// so a save used to end in a bare 403 toast. Say who can save instead.

export const OWNER_ONLY_HINT =
  'Only Otto’s owner, signed in to Otto itself, can change these settings. A share link, a paired phone or an agent session can view them but not save.';

/** The toast body for a failed settings save: the owner-only hint for a 403,
 *  else the error's own text. */
export function settingsSaveErrorText(e: unknown, fallback: (e: unknown) => string): string {
  const status = (e as { status?: unknown } | null)?.status;
  return status === 403 ? OWNER_ONLY_HINT : fallback(e);
}
