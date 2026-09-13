import { runPreRequest, runPostResponse } from './scripts';
import { assertScriptPayload, type ScriptInput, type ScriptOutput } from './scriptProtocol';

// This module is loaded only as a dedicated Worker, never in the webview.
self.onmessage = (event: MessageEvent<ScriptInput>) => {
  try {
    const input = event.data;
    assertScriptPayload(input);
    const run = input.kind === 'pre'
      ? runPreRequest(input.code, input.request, input.vars)
      : runPostResponse(input.code, input.response, input.vars);
    const output: ScriptOutput = { run, vars: input.vars, ...(input.kind === 'pre' ? { request: input.request } : {}) };
    assertScriptPayload(output); // Bound the result BEFORE structured cloning back.
    self.postMessage(output);
  } catch (e) {
    self.postMessage({ error: e instanceof Error ? e.message.slice(0, 4096) : 'Script worker failed.' });
  }
};
