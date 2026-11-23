import test from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

interface FakeSocket {
  readyState: number;
  sent: string[];
  closedWith: Array<{ code?: number; reason?: string }>;
  listeners: Map<string, Array<{ fn: (ev: { data?: string }) => void; signal?: AbortSignal }>>;
  emit: (type: string, ev?: { data?: string }) => void;
  listenerCount: (type: string) => number;
}

function setupDirectLsp() {
  const sockets: FakeSocket[] = [];
  const clearedTimers: unknown[] = [];
  const createdTimers: Array<{ id: number; fn: () => void; ms: number }> = [];
  let nextTimerId = 1;

  class MockWebSocket {
    static OPEN = 1;
    static CLOSED = 3;
    readyState = MockWebSocket.OPEN;
    sent: string[] = [];
    closedWith: Array<{ code?: number; reason?: string }> = [];
    listeners = new Map<string, Array<{ fn: (ev: { data?: string }) => void; signal?: AbortSignal }>>();

    constructor() {
      sockets.push(this as unknown as FakeSocket);
    }

    addEventListener(
      type: string,
      fn: (ev: { data?: string }) => void,
      opts?: { signal?: AbortSignal },
    ) {
      const list = this.listeners.get(type) ?? [];
      const entry = { fn, signal: opts?.signal };
      list.push(entry);
      this.listeners.set(type, list);
      if (opts?.signal) {
        opts.signal.addEventListener('abort', () => {
          const cur = this.listeners.get(type);
          if (!cur) return;
          const idx = cur.indexOf(entry);
          if (idx >= 0) cur.splice(idx, 1);
        });
      }
    }

    send(data: string) {
      this.sent.push(data);
    }

    close(code?: number, reason?: string) {
      this.readyState = MockWebSocket.CLOSED;
      this.closedWith.push({ code, reason });
    }

    emit(type: string, ev: { data?: string } = {}) {
      for (const entry of [...(this.listeners.get(type) ?? [])]) {
        entry.fn(ev);
      }
    }

    listenerCount(type: string) {
      return (this.listeners.get(type) ?? []).length;
    }
  }

  let PluginClass: (new (view: unknown) => { destroy: () => void }) | null = null;
  let completionHooks: {
    isReady: () => boolean;
    requestCompletion: (params: unknown) => Promise<unknown>;
  } | null = null;

  const mod = loadSource(
    new URL('../src/lib/components/cm-ide-lsp-direct.ts', import.meta.url),
    {
      '@codemirror/view': {
        EditorView: class {},
        ViewPlugin: {
          fromClass(cls: new (view: unknown) => { destroy: () => void }) {
            PluginClass = cls;
            return { kind: 'plugin' };
          },
        },
      },
      '@codemirror/lint': {
        setDiagnostics: () => ({}),
      },
      './cm-ide-drafts.ts': {
        markLiveLspDiagnostics: () => {},
      },
      './cm-ide-lsp-completion.ts': {
        buildLspInitializationOptions: () => ({}),
        createIdeLspCompletionExtension: (hooks: {
          isReady: () => boolean;
          requestCompletion: (params: unknown) => Promise<unknown>;
        }) => {
          completionHooks = hooks;
          return { kind: 'completion' };
        },
        createLspSemanticTokensExtension: () => ({ kind: 'semantic' }),
        dedupeLspDiagnostics: (d: unknown[]) => d,
        DEFAULT_LSP_TOKEN_MODIFIERS: [],
        DEFAULT_LSP_TOKEN_TYPES: [],
        lspToPos: () => 0,
        rewriteGoBuildTagDiagnostic: (message: string) => ({
          isBuildTagDiag: false,
          message,
          tags: [],
        }),
        shouldAttachIdeLsp: () => true,
      },
    },
    {
      WebSocket: MockWebSocket,
      setTimeout: (fn: () => void, ms: number) => {
        const id = nextTimerId++;
        createdTimers.push({ id, fn, ms });
        return id;
      },
      clearTimeout: (id: unknown) => {
        clearedTimers.push(id);
      },
    },
  );

  mod.createDirectLspExtension({
    wsUri: 'ws://127.0.0.1:7700/ws/lsp',
    rootUri: 'file:///workspace',
    documentUri: 'file:///workspace/main.ts',
    filePath: '/workspace/main.ts',
    languageId: 'typescript',
  });

  assert.ok(PluginClass, 'expected ViewPlugin.fromClass to be called');
  assert.ok(completionHooks, 'expected completion extension hooks to be captured');

  const fakeView = {
    dom: { isConnected: true, dispatchEvent: () => {} },
    state: { doc: { toString: () => 'const x = 1;', length: 12, sliceString: () => '' } },
    dispatch: () => {},
  };
  const instance = new (PluginClass as new (v: unknown) => { destroy: () => void })(fakeView);
  const socket = sockets[0];
  assert.ok(socket, 'expected WebSocket to be constructed');

  // Complete initialize handshake so ready === true
  socket.emit('open');
  const initMsg = JSON.parse(socket.sent[0]) as { id: number; method: string };
  assert.equal(initMsg.method, 'initialize');
  socket.emit('message', {
    data: JSON.stringify({ jsonrpc: '2.0', id: initMsg.id, result: { capabilities: {} } }),
  });

  return {
    instance,
    socket,
    hooks: completionHooks as {
      isReady: () => boolean;
      requestCompletion: (params: unknown) => Promise<unknown>;
    },
    createdTimers,
    clearedTimers,
  };
}

test('destroy() aborts all socket listeners, clears pending timers, and rejects pending requests', async () => {
  const { instance, socket, hooks, createdTimers, clearedTimers } = setupDirectLsp();
  assert.equal(hooks.isReady(), true);
  assert.equal(socket.listenerCount('open'), 1);
  assert.equal(socket.listenerCount('message'), 1);
  assert.equal(socket.listenerCount('close'), 1);
  assert.equal(socket.listenerCount('error'), 1);

  const reqPromise = hooks.requestCompletion({ textDocument: { uri: 'file:///workspace/main.ts' } });
  assert.equal(createdTimers.length, 1);

  instance.destroy();

  await assert.rejects(reqPromise, /LSP closed/);
  assert.equal(hooks.isReady(), false);
  assert.ok(clearedTimers.includes(createdTimers[0].id), 'expected pending request timer to be cleared on destroy');
  assert.equal(socket.listenerCount('open'), 0);
  assert.equal(socket.listenerCount('message'), 0);
  assert.equal(socket.listenerCount('close'), 0);
  assert.equal(socket.listenerCount('error'), 0);
});

test('socket close flips ready=false, clears pending timers, and rejects in-flight requests', async () => {
  const { socket, hooks, createdTimers, clearedTimers } = setupDirectLsp();
  assert.equal(hooks.isReady(), true);

  const reqPromise = hooks.requestCompletion({ textDocument: { uri: 'file:///workspace/main.ts' } });
  assert.equal(createdTimers.length, 1);

  socket.emit('close');

  await assert.rejects(reqPromise, /LSP disconnected/);
  assert.equal(hooks.isReady(), false);
  assert.ok(clearedTimers.includes(createdTimers[0].id), 'expected pending request timer to be cleared on close');

  await assert.rejects(
    hooks.requestCompletion({ textDocument: { uri: 'file:///workspace/main.ts' } }),
    /LSP unavailable/,
  );
});

test('socket error flips ready=false and rejects in-flight requests', async () => {
  const { socket, hooks } = setupDirectLsp();
  const reqPromise = hooks.requestCompletion({ textDocument: { uri: 'file:///workspace/main.ts' } });

  socket.emit('error');

  await assert.rejects(reqPromise, /LSP disconnected/);
  assert.equal(hooks.isReady(), false);
});
