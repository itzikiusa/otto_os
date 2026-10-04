import type { SchemaVersionDetail } from './types';

/** Immutable numeric versions only; bounded across subject/cluster switches. */
export class SchemaVersionCache {
  private entries = new Map<string, { value: SchemaVersionDetail; bytes: number }>();
  bytes = 0;
  private maxBytes: number;
  private maxEntries: number;
  constructor(maxBytes = 8 * 1024 * 1024, maxEntries = 64) { this.maxBytes = maxBytes; this.maxEntries = maxEntries; }
  get(base: string, version: number): SchemaVersionDetail | null {
    const key = `${base}/${version}`;
    const entry = this.entries.get(key);
    if (!entry) return null;
    this.entries.delete(key); this.entries.set(key, entry);
    return entry.value;
  }
  set(base: string, value: SchemaVersionDetail): void {
    const key = `${base}/${value.version}`;
    const bytes = 2 * (key.length + value.schema.length + value.subject.length + value.schema_type.length) + 128;
    if (bytes > this.maxBytes) return;
    const previous = this.entries.get(key);
    if (previous) { this.bytes -= previous.bytes; this.entries.delete(key); }
    while (this.entries.size && (this.entries.size >= this.maxEntries || this.bytes + bytes > this.maxBytes)) {
      const oldest = this.entries.keys().next().value!;
      this.bytes -= this.entries.get(oldest)!.bytes; this.entries.delete(oldest);
    }
    this.entries.set(key, { value, bytes }); this.bytes += bytes;
  }
}
