// WebKit's worker stack can overflow while Go compiles D2's initialization
// regexps. The same runtime succeeds on the main thread. Keep its globals in
// a private about:blank realm and its protocol on a private MessageChannel.
// Only trusted, pinned package code is evaluated; diagram source is data.
import type { D2Api } from './d2';

export async function createD2Frame(source: string, wasm: ArrayBuffer): Promise<D2Api> {
  const frame = document.createElement('iframe');
  frame.hidden = true;
  frame.tabIndex = -1;
  frame.setAttribute('aria-hidden', 'true');
  frame.title = 'Diagram renderer';
  document.body.append(frame);
  const ports = new MessageChannel();
  let resolveRequest: ((value: unknown) => void) | null = null;
  let rejectRequest: ((reason: Error) => void) | null = null;
  let disposed = false;
  const transportError = () => Object.assign(new Error('The diagram renderer timed out. Try again.'), { name: 'D2TransportError' });
  const dispose = () => {
    disposed = true;
    ports.port1.close(); ports.port2.close(); frame.remove();
  };
  const request = <T>(type: string, data: unknown): Promise<T> => new Promise((resolve, reject) => {
    if (disposed) { reject(transportError()); return; }
    // renderD2 serializes compile/render pairs, matching the worker protocol.
    const timeout = setTimeout(() => {
      resolveRequest = null; rejectRequest = null; dispose();
      reject(transportError());
    }, 30_000);
    resolveRequest = value => { clearTimeout(timeout); resolve(value as T); };
    rejectRequest = error => { clearTimeout(timeout); reject(error); };
    ports.port1.postMessage({ type, data });
  });
  ports.port1.onmessage = event => {
    const { type, data, error } = event.data;
    if (type === 'ready' || type === 'result') {
      const resolve = resolveRequest; resolveRequest = null; rejectRequest = null;
      resolve?.(data);
    } else if (type === 'error') {
      const reject = rejectRequest; resolveRequest = null; rejectRequest = null;
      reject?.(new Error(error));
    }
  };
  try {
    const realm = frame.contentWindow as (Window & { Function: FunctionConstructor }) | null;
    if (!realm) throw new Error('Could not create the diagram renderer.');
    // The package already uses Function for ELK. This requires only the same
    // unsafe-eval policy; no inline/blob scripts or broader frame permissions.
    const scoped = source.replace('export let BrotliDecode', 'let BrotliDecode')
      .replace('export function setupMessageHandler', 'function setupMessageHandler')
      .replace('setupMessageHandler(false, self, initWasmBrowser);', 'setupMessageHandler(false, port, initWasmBrowser);');
    realm.Function('port', scoped)(ports.port2);
    await request('init', { wasm, wasmExecContent: null, wasmExecUrl: null });
  } catch (error) {
    dispose(); throw error;
  }
  return {
    get disposed() { return disposed; },
    compile: (src, options = {}) => request('compile', { fs: { index: src }, options }),
    render: (diagram, options = {}) => request('render', { diagram, options }),
    dispose,
  };
}
