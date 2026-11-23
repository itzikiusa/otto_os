// Threads page widths (ThreadsPage.svelte). Pure, unit-tested in
// unit/threadsStage.test.ts.

/** The conversation column's floor: below it the composer's chips truncate. */
export const MIN_CONVERSATION_W = 640;
export const MIN_RIGHT_PANEL_W = 300;
/** Beside an open surface (Changes, Files, PR…) the conversation gives up a
 *  little more, so a 1440 window with the list open gives the surface ~500 px. */
export const MIN_CONVERSATION_BESIDE_SURFACE_W = 600;
/** A surface opens this wide (remembered per device once resized). */
export const SURFACE_W = 520;

/** The right panel's shown width: the person's saved width while it leaves
 *  the conversation `MIN_CONVERSATION_W`, else narrower down to the panel's
 *  minimum. The saved width is untouched, so it returns once there is room. */
export function fitRightPanel(saved: number, stageW: number, leftW: number, floor = MIN_CONVERSATION_W): number {
  if (!(stageW > 0)) return saved;
  const room = stageW - leftW - floor - 8; // the two dividers
  return Math.max(MIN_RIGHT_PANEL_W, Math.min(saved, room));
}
