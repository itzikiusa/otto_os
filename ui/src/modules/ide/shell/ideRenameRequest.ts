// The `/ide/lsp/rename` request body (kept free of app imports so it unit-tests in node).
import type { IdeTextEdit } from '../store/ideTypes';

export interface RenameHost {
  renameState: { path: string; line: number; column: number; symbol: string };
  targetPayload: { project_folders?: string[] };
  lspRenameEdits(path: string, line: number, column: number, newName: string): Promise<IdeTextEdit[] | null>;
}

/**
 * The `/ide/lsp/rename` body. When a language server owns the file its own
 * `textDocument/rename` edits go along (`lsp_edits`); otherwise the daemon falls
 * back to a whole-word text match in files of the same language.
 */
export async function renameBody(
  host: RenameHost,
  newName: string,
  content: string,
  apply: boolean,
): Promise<Record<string, unknown>> {
  const { path, line, column, symbol } = host.renameState;
  // An in-place rename already edited the name in the buffer; the server would
  // rename the new name, so that one stays with the daemon's match of `symbol`.
  const lineText = content.split('\n')[line - 1] ?? '';
  const wordAt = /[\w$]*$/.exec(lineText.slice(0, column - 1))![0] + /^[\w$]*/.exec(lineText.slice(column - 1))![0];
  const lspEdits = !symbol || wordAt === symbol
    ? await host.lspRenameEdits(path, line, column, newName).catch(() => null)
    : null;
  return {
    target: host.targetPayload,
    project_folders: host.targetPayload.project_folders,
    path,
    line,
    column,
    symbol: symbol || undefined,
    content: content || undefined,
    new_name: newName,
    lsp_edits: lspEdits?.length ? lspEdits : undefined,
    apply,
  };
}
