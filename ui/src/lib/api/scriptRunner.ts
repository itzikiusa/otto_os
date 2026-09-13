import { createScriptWorker } from './scriptWorkerFactory';
import { assertScriptPayload, type ScriptInput, type ScriptOutput } from './scriptProtocol';

export const SCRIPT_DEADLINE_MS = 5000;

/** One invocation, one Worker, one terminal cleanup; never falls back to UI JS. */
export function runScript(input: ScriptInput, signal?: AbortSignal, deadlineMs = SCRIPT_DEADLINE_MS): Promise<ScriptOutput> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) { reject(new DOMException('Script canceled', 'AbortError')); return; }
    try { assertScriptPayload(input); } catch (e) { reject(e); return; }
    let worker: Worker;
    try { worker = createScriptWorker(); } catch (e) { reject(e); return; }
    let settled = false;
    let timer: ReturnType<typeof setTimeout>;
    const finish = (error?: unknown, result?: ScriptOutput) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      signal?.removeEventListener('abort', abort);
      worker.onmessage = null; worker.onerror = null; worker.onmessageerror = null;
      worker.terminate();
      if (error !== undefined) reject(error); else resolve(result!);
    };
    const abort = () => finish(new DOMException('Script canceled', 'AbortError'));
    worker.onmessage = (event: MessageEvent<ScriptOutput & { error?: string }>) => {
      try {
        const result = event.data;
        if (result?.error) throw new Error(result.error);
        if (!result?.run || !Array.isArray(result.run.logs) || !Array.isArray(result.run.tests) || !result.vars) throw new Error('Invalid script worker result.');
        assertScriptPayload(result);
        finish(undefined, result);
      } catch (e) { finish(e); }
    };
    worker.onerror = (event) => { event.preventDefault(); finish(new Error(event.message || 'Script worker failed.')); };
    worker.onmessageerror = () => finish(new Error('Could not decode script worker message.'));
    signal?.addEventListener('abort', abort, { once: true });
    timer = setTimeout(() => finish(new Error('Script timed out after 5 seconds.')), deadlineMs);
    if (signal?.aborted) { abort(); return; }
    try { worker.postMessage(input); } catch (e) { finish(e); }
  });
}
