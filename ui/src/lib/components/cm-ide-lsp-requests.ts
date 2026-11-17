// Requests a host makes of the language server attached to an editor (rename, quick
// fixes, references). Split out of
// `cm-ide-lsp.ts` (the pooled client); marimo loads on demand like there.

import type { EditorView } from '@codemirror/view';
import { posToLsp, workspaceEditToTextEdits, type LspFileTextEdit } from './cm-ide-lsp-completion.ts';
import { normalizeLocations } from './cm-ide-lsp-dive.ts';

type RpcRequester = { request: (method: string, params: unknown) => Promise<unknown> };

async function pluginOf(view: EditorView) {
  return (await import('@marimo-team/codemirror-languageserver')).getLanguageServerPlugin(view);
}

function rpcOf(client: object): RpcRequester | null {
  return (client as { client?: RpcRequester }).client ?? null;
}

/**
 * Ask the language server attached to `view` to rename the symbol at 1-based `line`/`column`.
 * `null` when no ready server with rename support owns the file (the caller then
 * falls back to the daemon's text match).
 */
export async function queryPooledLspRename(view: EditorView, line: number, column: number, newName: string): Promise<LspFileTextEdit[] | null> {
  const plugin = await pluginOf(view);
  const doc = view.state.doc;
  if (!plugin?.client.ready || !plugin.client.hasCapability('textDocument/rename') || line < 1 || line > doc.lines) return null;
  const rpc = rpcOf(plugin.client);
  if (!rpc) return null;
  const pos = Math.min(doc.line(line).to, doc.line(line).from + Math.max(0, column - 1));
  try {
    const res = await rpc.request('textDocument/rename', { textDocument: { uri: plugin.documentUri }, position: posToLsp(doc, pos), newName });
    return workspaceEditToTextEdits(res);
  } catch {
    return null;
  }
}

/** One server quick fix at a line, as concrete edits (an action without an edit, even after `codeAction/resolve`, is left out). */
export interface PooledLspQuickFix {
  title: string;
  kind?: string;
  isPreferred: boolean;
  edits: LspFileTextEdit[];
}

/**
 * The server's `quickfix` code actions for 1-based `line`, with the diagnostics shown on
 * that line in the request context. `null` when no ready server with code actions owns
 * the file (the caller then uses Otto's heuristics).
 */
export async function queryPooledLspCodeActions(view: EditorView, line: number): Promise<PooledLspQuickFix[] | null> {
  const plugin = await pluginOf(view);
  const doc = view.state.doc;
  if (!plugin?.client.ready || !plugin.client.hasCapability('textDocument/codeAction') || line < 1 || line > doc.lines) return null;
  const ln = doc.line(line);
  try {
    type RawAction = { title: string; kind?: string; isPreferred?: boolean; edit?: unknown };
    const raw = ((await plugin.requestCodeActions(view, { start: posToLsp(doc, ln.from), end: posToLsp(doc, ln.to) }, ['quickfix'])) ?? []) as RawAction[];
    // `resolveCodeAction` is private in the typings; it resolves only what lacks an edit.
    const resolve = (plugin as unknown as { resolveCodeAction: (a: RawAction) => Promise<RawAction> }).resolveCodeAction.bind(plugin);
    const out: PooledLspQuickFix[] = [];
    for (const item of raw) {
      if (item.kind && !item.kind.startsWith('quickfix')) continue; // a server that ignores `only`
      const action = await resolve(item);
      const edits = workspaceEditToTextEdits(action.edit);
      if (edits.length) out.push({ title: action.title, kind: 'quickfix', isPreferred: Boolean(action.isPreferred), edits });
    }
    return out;
  } catch {
    return null;
  }
}

/** The server's `textDocument/references` at 1-based `line`/`column`; `null` without a ready server that has them. */
export async function queryPooledLspReferences(view: EditorView, line: number, column: number): Promise<Array<{ path: string; line: number; column: number }> | null> {
  const plugin = await pluginOf(view);
  const doc = view.state.doc;
  if (!plugin?.client.ready || !plugin.client.hasCapability('textDocument/references') || line < 1 || line > doc.lines) return null;
  const rpc = rpcOf(plugin.client);
  if (!rpc) return null;
  const pos = Math.min(doc.line(line).to, doc.line(line).from + Math.max(0, column - 1));
  try {
    const res = await rpc.request('textDocument/references', { textDocument: { uri: plugin.documentUri }, position: posToLsp(doc, pos), context: { includeDeclaration: true } });
    return normalizeLocations(res);
  } catch {
    return null;
  }
}
