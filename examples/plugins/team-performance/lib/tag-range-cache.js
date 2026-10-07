// Bounded, disposable acceleration for immutable git ranges. A missing/corrupt
// cache simply makes the next worker cold; it never becomes delivery evidence.
'use strict';
const fs = require('node:fs');
const { writeJsonAtomic } = require('./store.js');
const MAX_BYTES = 16 * 1024 * 1024;
const MAX_ENTRIES = 5000;
class TagRangeCache {
  constructor(file) {
    this.file = file;
    this.entries = new Map();
    this.bytes = 32;
    if (!file) return;
    try {
      if (fs.statSync(file).size > MAX_BYTES) return;
      const data = JSON.parse(fs.readFileSync(file, 'utf8'));
      if (data.version !== 1 || !Array.isArray(data.entries)) return;
      for (const [key, value] of data.entries) {
        if (/^[a-f0-9]{64}$/.test(key) && typeof value === 'string') this.set(key, value);
      }
    } catch { /* disposable cache */ }
  }
  get(key) {
    const entry = this.entries.get(key);
    if (!entry) return undefined;
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry.value;
  }
  set(key, value) {
    const bytes = Buffer.byteLength(JSON.stringify([key, value])) + 1;
    if (bytes > MAX_BYTES - 128) return;
    const old = this.entries.get(key);
    if (old) { this.bytes -= old.bytes; this.entries.delete(key); }
    while (this.entries.size && (this.bytes + bytes > MAX_BYTES - 128 || this.entries.size >= MAX_ENTRIES)) {
      const oldest = this.entries.keys().next().value;
      this.bytes -= this.entries.get(oldest).bytes;
      this.entries.delete(oldest);
    }
    this.entries.set(key, { value, bytes });
    this.bytes += bytes;
  }
  save() {
    if (!this.file) return;
    try { writeJsonAtomic(this.file, { version: 1, entries: [...this.entries].map(([k, e]) => [k, e.value]) }); }
    catch { /* cache I/O must not fail a scan */ }
  }
}
module.exports = { TagRangeCache, MAX_BYTES, MAX_ENTRIES };
