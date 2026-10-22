// Design Hall loads imported canvas images from durable data: URLs. It does not
// resolve otto-canvas-file: markers; a new import already inlines the bytes.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';

const boardPath = new URL('../src/lib/design/DesignBoard.svelte', import.meta.url);
const scene = JSON.stringify({
  elements: [],
  files: {
    png: { id: 'png', dataURL: 'data:image/png;base64,qq', mimeType: 'image/png', created: 5 },
    marker: { id: 'marker', dataURL: 'otto-canvas-file:abc', mimeType: 'image/png' },
    empty: { id: 'empty', mimeType: 'image/png' },
    text: { id: 'text', dataURL: 'data:text/plain;base64,qq', mimeType: 'text/plain' },
  },
});

test('initial data and a reload pass only data: image files to Excalidraw', () => {
  const added: { dataURL?: string }[] = [];
  const board = componentFunctions(
    boardPath,
    [
      'isRecord',
      'isBinaryMime',
      'finiteNumber',
      'isBinaryFileData',
      'binaryFileFrom',
      'dataUrlFiles',
      'initialData',
      'loadScene',
      'parse',
    ],
    {
      source: scene,
      lastApplied: '',
      suppressChange: false,
      normalizeScene: () => [],
      setTimeout() {},
      excaliApi: {
        updateScene() {},
        scrollToContent() {},
        addFiles(files: { dataURL?: string }[]) { added.push(...files); },
      },
    },
  );
  const files = board.initialData().files as Record<string, { dataURL: string; mimeType: string; created: number }>;
  assert.deepEqual(Object.keys(files), ['png']);
  assert.equal(files.png.dataURL.startsWith('data:'), true);
  assert.equal(files.png.mimeType, 'image/png');
  assert.equal(files.png.created, 5);
  board.loadScene(scene);
  assert.equal(added.length, 1);
  assert.equal(added[0].dataURL, 'data:image/png;base64,qq');

  const guarded = board.dataUrlFiles({
    octets: { id: 'octets', dataURL: 'data:application/octet-stream;base64,aa', mimeType: 'application/octet-stream' },
    header: { dataURL: 'data:image/gif;base64,bb' },
    late: { id: 'late', dataURL: 'data:image/png;base64,cc', mimeType: 'image/png', created: 9, lastRetrieved: 11, version: 2 },
    badCreated: { id: 'bad', dataURL: 'data:image/png;base64,dd', mimeType: 'image/png', created: 'yesterday' },
    list: ['nope'],
  });
  assert.equal(guarded.octets.mimeType, 'application/octet-stream');
  assert.equal(guarded.octets.created, 0);
  assert.equal(guarded.header.id, 'header');
  assert.equal(guarded.header.mimeType, 'image/gif');
  assert.equal(guarded.late.lastRetrieved, 11);
  assert.equal(guarded.late.version, 2);
  assert.equal(guarded.bad.created, 0);
  assert.equal(guarded.badCreated, undefined);
  assert.equal(guarded.list, undefined);
  assert.equal(board.dataUrlFiles([{ dataURL: 'data:image/png;base64,zz', mimeType: 'image/png' }]).png, undefined);
  assert.equal(board.binaryFileFrom('k', { dataURL: 'otto-canvas-file:abc', mimeType: 'image/png' }), null);
});
