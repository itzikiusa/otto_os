import type { IdeNotebookExecuteResp, IdeNotebookVariable } from '../../../lib/api/types';
import {
  applyNotebookExecutionById,
  type NotebookDoc,
  type NotebookOutput,
} from './ideNotebookModel.ts';

export interface NotebookCellRunHooks {
  getDoc: () => NotebookDoc;
  getPath: () => string;
  isDisposed: () => boolean;
  getViewToken: () => number;
  getRunningCellId: () => string | null;
  setRunningCellId: (id: string | null) => void;
  setSessionAndVars: (sessionId: string, vars: IdeNotebookVariable[]) => void;
  commitDoc: (next: NotebookDoc, targetPath: string) => void;
  onError: (err: unknown) => void;
  executeRequest: (origPath: string, code: string) => Promise<IdeNotebookExecuteResp>;
}

/** Execute one notebook cell with full concurrency/lifecycle guards (CONC-4):
 *  rejects late completions if the user switched tabs, switched to Raw Source view
 *  (disposing the notebook view), or reloaded the document while execution was in flight. */
export async function executeNotebookCellGuarded(
  cellId: string,
  hooks: NotebookCellRunHooks,
): Promise<boolean> {
  const doc = hooks.getDoc();
  const cell = doc.cells.find((c) => c.id === cellId);
  if (!cell || cell.cell_type !== 'code' || hooks.getRunningCellId() !== null) return false;
  const origPath = hooks.getPath();
  const origToken = hooks.getViewToken();
  const stillValid = (): boolean =>
    !hooks.isDisposed() && hooks.getPath() === origPath && hooks.getViewToken() === origToken;

  hooks.setRunningCellId(cellId);
  try {
    const resp = await hooks.executeRequest(origPath, cell.source);
    if (!stillValid()) return false;
    hooks.setSessionAndVars(resp.session_id, resp.variables ?? []);
    const { doc: next, applied } = applyNotebookExecutionById(
      hooks.getDoc(),
      cellId,
      resp.execution_count,
      (resp.outputs ?? []) as unknown as NotebookOutput[],
    );
    if (applied && stillValid()) {
      hooks.commitDoc(next, origPath);
    }
    return resp.status === 'ok';
  } catch (e) {
    if (stillValid()) hooks.onError(e);
    return false;
  } finally {
    if (hooks.getRunningCellId() === cellId) hooks.setRunningCellId(null);
  }
}
