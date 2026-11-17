// The phone's step bar over the editor (IdeTouchDebugBar.svelte): when it shows and
// what it says. Pure, so the unit tests drive it.

export interface TouchDebugSnap {
  state: 'starting' | 'running' | 'paused' | 'terminated' | 'error';
  paused_file?: string | null;
  paused_line?: number | null;
}

export interface TouchDebugBarInput {
  dbg: TouchDebugSnap | null;
  /** The IDE is stacked and shows the Editor pane. */
  editorPane: boolean;
  /** A step was just sent: keep the bar while the program runs to the next stop. */
  stepping: boolean;
  /** Typing with the keyboard up: the bar would sit on the line being typed. */
  typingMode: boolean;
}

export interface TouchDebugBar {
  show: boolean;
  paused: boolean;
  /** `file:line` of the stop ('' while running). */
  where: string;
}

export function touchDebugBar(i: TouchDebugBarInput): TouchDebugBar {
  const d = i.dbg;
  const paused = d?.state === 'paused';
  const live = paused || (i.stepping && (d?.state === 'running' || d?.state === 'starting'));
  const show = Boolean(d) && i.editorPane && !i.typingMode && live;
  const file = d?.paused_file ? d.paused_file.slice(d.paused_file.lastIndexOf('/') + 1) : '';
  const where = paused && file ? `${file}${d?.paused_line ? `:${d.paused_line}` : ''}` : '';
  return { show, paused, where };
}

/** How far to scroll the editor so the paused line clears the bar (0 = it already does).
 *  `gap` keeps a little air between the line and the bar. */
export function pausedLineClearance(lineBottom: number, barTop: number, gap = 8): number {
  const d = Math.ceil(lineBottom + gap - barTop);
  return d > 0 ? d : 0;
}
