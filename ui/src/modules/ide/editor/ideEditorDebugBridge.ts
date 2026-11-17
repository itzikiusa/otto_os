// The debug-related props one IDE editor pane takes, read from the store: where execution
// stopped, the selected caller frame, quiet inline values, hover-to-evaluate, and the path for
// breakpoints that edits carried to new lines. Spread onto `CodeEditor` by IdeEditorArea.

import type { IdeBreakpointMove } from '../../../lib/components/cm-ide-bp-model';
import { ide } from '../ideStore.svelte';
import { moveIdeBreakpoints } from '../store/ideBreakpointMoves';
import { ideDebugEditorProps, type IdeDebugEditorProps } from './ideDebugEditorProps';

const evaluateOnHover = (expression: string): Promise<string> => ide.evaluateDebug(expression, 'hover');
const onBreakpointsMoved = (path: string, moves: IdeBreakpointMove[]): void => moveIdeBreakpoints(ide, path, moves);

export function ideEditorDebugProps(tabPath: string): IdeDebugEditorProps & { onBreakpointsMoved: typeof onBreakpointsMoved } {
  return {
    ...ideDebugEditorProps(ide.activeDebug, tabPath, [ide.target.root, ...ide.additionalRoots], evaluateOnHover),
    onBreakpointsMoved,
  };
}
