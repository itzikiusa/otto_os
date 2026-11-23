import type { EditorView } from '@codemirror/view';
import type { LanguageServerClient, getLanguageServerPlugin } from '@marimo-team/codemirror-languageserver';
import { toasts } from '../toast.svelte';
import { normalizeLocations } from './cm-ide-lsp-dive.ts';
import { posToLsp } from './cm-ide-lsp-completion.ts';

// A live server can answer "no definition" while it is still indexing the project.
// Keep its work tokens together so a navigation waits for the last overlapping job.
export class LspIndexProgress {
  private readonly active = new Set<string | number>();
  private readonly finished = new EventTarget();

  get busy(): boolean { return this.active.size > 0; }

  update(method: string | undefined, params: Record<string, unknown> | undefined): void {
    if (!params) return;
    if (method === 'language/status') {
      // Eclipse JDT LS publishes its project import here, including when the
      // initialization response has already arrived.
      if (params.type === 'Starting') this.active.add('jdtls:start');
      if (params.type === 'ServiceReady' || params.type === 'Started') this.end('jdtls:start');
      return;
    }
    if (method !== '$/progress') return;
    const token = params.token;
    if (typeof token !== 'string' && typeof token !== 'number') return;
    const value = params.value as { kind?: string; title?: string } | undefined;
    if (value?.kind === 'begin' && /index|load|project|workspace|import|crate|cargo|analysis|build/i.test(value.title ?? '')) {
      this.active.add(token);
    } else if (value?.kind === 'end') {
      this.end(token);
    }
  }

  private end(token: string | number): void {
    if (this.active.delete(token) && !this.busy) this.finished.dispatchEvent(new Event('idle'));
  }

  close(): void {
    this.active.clear();
    this.finished.dispatchEvent(new Event('close'));
  }

  waitForIdle(signal: AbortSignal): Promise<boolean> {
    if (signal.aborted) return Promise.resolve(false);
    if (!this.busy) return Promise.resolve(true);
    return new Promise((resolve) => {
      const finish = (ready: boolean) => {
        this.finished.removeEventListener('idle', idle);
        this.finished.removeEventListener('close', stopped);
        signal.removeEventListener('abort', stopped);
        resolve(ready);
      };
      const idle = () => finish(true);
      const stopped = () => finish(false);
      this.finished.addEventListener('idle', idle);
      this.finished.addEventListener('close', stopped);
      signal.addEventListener('abort', stopped);
    });
  }
}
type RpcRequester = { request: (method: string, params: unknown) => Promise<unknown> };

export function getPooledRpc(client: LanguageServerClient): RpcRequester | null {
  return (client as unknown as { client?: RpcRequester }).client ?? null;
}


export const indexProgress = new WeakMap<LanguageServerClient, LspIndexProgress>();

export async function queryPluginLocations(
  plugin: ReturnType<typeof getLanguageServerPlugin> | undefined,
  view: EditorView,
  pos: number,
  mode: 'definition' | 'implementation',
): Promise<Array<{ path: string; line: number; column: number }>> {
  if (!plugin || !plugin.client.ready) return [];
  const position = posToLsp(view.state.doc, pos);
  const params = { textDocument: { uri: plugin.documentUri }, position };
  const rpc = getPooledRpc(plugin.client);

  try {
    if (mode === 'implementation') {
      if (rpc && plugin.client.hasCapability('textDocument/implementation')) {
        const locs = normalizeLocations(await rpc.request('textDocument/implementation', params));
        if (locs.length > 0) return locs;
      }
    } else {
      if (plugin.client.hasCapability('textDocument/definition')) {
        const locs = normalizeLocations(await plugin.client.textDocumentDefinition(params));
        if (locs.length > 0) return locs;
      }
      if (rpc && plugin.client.hasCapability('textDocument/typeDefinition')) {
        const locs = normalizeLocations(await rpc.request('textDocument/typeDefinition', params));
        if (locs.length > 0) return locs;
      }
    }
  } catch {
    /* Fall through to backend HTTP / lexical dive-in */
  }
  return [];
}

export async function lookupNavigation(
  plugin: ReturnType<typeof getLanguageServerPlugin> | undefined,
  view: EditorView,
  pos: number,
  mode: 'definition' | 'implementation',
  isCurrent: () => boolean,
  signal: AbortSignal,
) {
  const client = plugin?.client;
  if (client && !client.ready) {
    toasts.info('Waiting for the language server', `Go to ${mode} will continue when it is ready.`);
    await client.initializePromise?.catch(() => {});
    if (!isCurrent()) return [];
    if (!client.ready) return 'unavailable';
  }
  const progress = client && indexProgress.get(client);
  const wasBusy = progress?.busy;
  const locations = await queryPluginLocations(plugin, view, pos, mode);
  if (locations.length || !progress || (!wasBusy && !progress.busy) || !isCurrent()) return locations;
  if (progress.busy) {
    toasts.info('Still indexing the project', `Go to ${mode} will retry when indexing finishes.`);
    if (!await progress.waitForIdle(signal)) return 'unavailable';
  }
  if (!isCurrent()) return [];
  return queryPluginLocations(plugin, view, pos, mode);
}
