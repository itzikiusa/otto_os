import { test } from 'node:test';
import assert from 'node:assert/strict';
import { toolImageSources } from '../src/modules/threads/runtime/toolImages.ts';

const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5V8AAAAASUVORK5CYII=';
test('genuine PNG becomes a bounded data URL, with other images retained', () => {
  assert.deepEqual(toolImageSources([{ mime: 'image/png', data_base64: png }, { mime: 'image/png', data_base64: png }]), [
    `data:image/png;base64,${png}`, `data:image/png;base64,${png}`,
  ]);
});
test('rejects active content, URLs, invalid base64, oversize and malformed records', () => {
  assert.deepEqual(toolImageSources([
    { mime: 'image/svg+xml', data_base64: png }, { mime: 'image/png', data_base64: 'https://evil.example/x' },
    { mime: 'image/png', data_base64: '!!!!' }, { mime: 'image/png', data_base64: 'A'.repeat(1_398_105) },
    null, {}, { mime: 'image/png', data_base64: btoa('<html>bad</html>') },
  ]), []);
});
test('missing historical image field is empty; count is bounded', () => {
  assert.deepEqual(toolImageSources(undefined), []);
  assert.equal(toolImageSources(Array.from({ length: 8 }, () => ({ mime: 'image/png', data_base64: png }))).length, 4);
});
test('JPEG, GIF and WebP raster signatures are accepted and aggregate decoded bytes are capped', () => {
  const images = [
    { mime: 'image/jpeg', data_base64: btoa('\xff\xd8\xff\xe0') },
    { mime: 'image/gif', data_base64: btoa('GIF89a') },
    { mime: 'image/webp', data_base64: btoa('RIFFxxxxWEBP') },
  ];
  assert.equal(toolImageSources(images).length, 3);
  const largePng = btoa('\x89PNG\r\n\x1a\n' + '\0'.repeat(1_048_576 - 8));
  assert.equal(toolImageSources(Array.from({ length: 3 }, () => ({ mime: 'image/png', data_base64: largePng }))).length, 2);
});
