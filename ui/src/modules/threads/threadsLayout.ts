// Threads page widths (ThreadInspector's DetailPane). Unit-tested in
// unit/threadsStage.test.ts with the shared detailFit rule.

/** The conversation column's floor beside the inspector or a surface (Changes,
 *  Files, PR…): below it the pane becomes a sheet. 1440 with the list open
 *  gives a surface 480 px; 1280 keeps the inspector inline at 320. */
export const MIN_CONVERSATION_W = 600;
/** A surface opens this wide (remembered per device once resized). */
export const SURFACE_W = 520;
