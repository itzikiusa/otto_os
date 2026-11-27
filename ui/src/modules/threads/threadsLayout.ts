// Threads page widths (ThreadInspector's DetailPane). Unit-tested in
// unit/threadsStage.test.ts with the shared detailFit rule.

/** The conversation column's floor: below it the composer's chips truncate. */
export const MIN_CONVERSATION_W = 640;
/** Beside an open surface (Changes, Files, PR…) the conversation gives up a
 *  little more, so a 1440 window with the list open gives the surface ~500 px. */
export const MIN_CONVERSATION_BESIDE_SURFACE_W = 600;
/** A surface opens this wide (remembered per device once resized). */
export const SURFACE_W = 520;
