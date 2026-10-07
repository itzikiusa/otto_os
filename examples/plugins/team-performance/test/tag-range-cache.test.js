'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { TagRangeCache, MAX_BYTES, MAX_ENTRIES } = require('../lib/tag-range-cache.js');
const key = (i) => i.toString(16).padStart(64, '0');

test('tag cache bounds retained entry count and UTF-8 serialized bytes across reload', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-range-bounds-'));
  try {
    const file = path.join(dir, 'cache.json');
    const cache = new TagRangeCache(file);
    for (let i = 0; i <= MAX_ENTRIES; i++) cache.set(key(i), '');
    assert.equal(cache.get(key(0)), undefined, 'oldest entry evicted');
    assert.equal(cache.get(key(MAX_ENTRIES)), '');
    for (let i = 0; i < 300; i++) cache.set(key(i), '\"שלום\\'.repeat(10000));
    cache.save();
    assert.ok(fs.statSync(file).size <= MAX_BYTES);
    assert.ok(cache.entries.size <= MAX_ENTRIES);
    const loaded = new TagRangeCache(file);
    assert.equal(loaded.get(key(299)), cache.get(key(299)));
    assert.equal(loaded.entries.size, cache.entries.size);
    cache.set(key(99999), 'x'.repeat(MAX_BYTES));
    assert.equal(cache.get(key(99999)), undefined, 'single oversized range is uncached');
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});

test('corrupt and future cache versions cold-start without failing indexing', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'tp-range-corrupt-'));
  try {
    const file = path.join(dir, 'cache.json');
    for (const contents of ['{', JSON.stringify({ version: 2, entries: [[key(1), 'wrong']] }), JSON.stringify({ version: 1, entries: [null] })]) {
      fs.writeFileSync(file, contents);
      const cache = new TagRangeCache(file);
      assert.equal(cache.get(key(1)), undefined);
      cache.set(key(2), 'fresh');
      cache.save();
      assert.equal(new TagRangeCache(file).get(key(2)), 'fresh');
    }
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});
