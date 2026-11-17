// What one editor pane shows of a debug session: the "paused here" arrow follows only where
// execution stopped (`paused_file/line`); a selected caller frame gets its own muted mark
// (`selected_frame_file/line`); inline values belong to the selected frame, quieted. Daemons
// without the `selected_frame_*` fields behave as before (everything follows the stop).

import type { IdeInlineValue } from '../../../lib/api/types';
import { matchesPausedDebugPath } from './idePausedPath';

export interface IdeDebugEditorProps {
  pausedLine: number | null;
  selectedFrameLine: number | null;
  inlineValues: IdeInlineValue[];
  /** Hover-to-evaluate, only while paused in the file this pane shows. */
  onDebugHover: ((expression: string) => Promise<string>) | null;
}

export const INLINE_VALUE_MAX = 40;
// Reprs that say nothing at a glance: functions, methods, modules, classes (Python, JS, Go).
const NOISY_REPR = /^(?:<(?:function|bound method|built-in (?:function|method)|module|class|method|cyfunction)\b|\[Function\b|\[class\b|ƒ |function\s*\w*\s*\(|class\s+\w+|func\(|<type ')/;

/** Inline values worth painting: no function/module/class reprs, each capped with "…". */
export function quietInlineValues(values: IdeInlineValue[]): IdeInlineValue[] {
  const out: IdeInlineValue[] = [];
  for (const iv of values) {
    const v = (iv.value ?? '').replace(/\s+/g, ' ').trim();
    if (!v || NOISY_REPR.test(v)) continue;
    out.push(v.length > INLINE_VALUE_MAX ? { ...iv, value: `${v.slice(0, INLINE_VALUE_MAX - 1)}…` } : (v === iv.value ? iv : { ...iv, value: v }));
  }
  return out;
}

/** The snapshot fields read here (both snapshot type copies satisfy it). */
export interface IdeDebugSnapFields {
  state: string; paused_file?: string | null; paused_line?: number | null;
  selected_frame_file?: string | null; selected_frame_line?: number | null; inline_values?: IdeInlineValue[] | null;
}

const NONE: IdeDebugEditorProps = { pausedLine: null, selectedFrameLine: null, inlineValues: [], onDebugHover: null };

export function ideDebugEditorProps(
  snap: IdeDebugSnapFields | null | undefined,
  tabPath: string,
  roots: string[],
  evaluate?: (expression: string) => Promise<string>,
): IdeDebugEditorProps {
  if (!snap || snap.state !== 'paused') return NONE;
  const here = (file: string | null | undefined): boolean => matchesPausedDebugPath(file, tabPath, roots);
  const pausedHere = here(snap.paused_file);
  const hasSelected = snap.selected_frame_file != null && snap.selected_frame_line != null;
  const selectedHere = hasSelected && here(snap.selected_frame_file);
  // Older daemons send no selected frame: the values belong to the stop, as before.
  const valuesHere = hasSelected ? selectedHere : pausedHere;
  const pausedLine = pausedHere ? (snap.paused_line ?? null) : null;
  return {
    pausedLine,
    selectedFrameLine: selectedHere && snap.selected_frame_line !== pausedLine ? (snap.selected_frame_line ?? null) : null,
    inlineValues: valuesHere ? quietInlineValues(snap.inline_values ?? []) : [],
    onDebugHover: evaluate && (pausedHere || selectedHere) ? evaluate : null,
  };
}
