// Pure nbformat v4.5 Jupyter Notebook (.ipynb) parser, serializer, cell mutations,
// and safe rich-output normalizer (pure TypeScript, unit-tested directly in Node).

export type NotebookCellType = 'code' | 'markdown' | 'raw';

export interface NotebookOutput {
  output_type: 'stream' | 'execute_result' | 'display_data' | 'error' | string;
  name?: string;
  text?: string | string[];
  execution_count?: number | null;
  data?: Record<string, string | string[] | unknown>;
  metadata?: Record<string, unknown>;
  ename?: string;
  evalue?: string;
  traceback?: string[];
}

export interface NotebookCell {
  id: string;
  cell_type: NotebookCellType;
  source: string;
  execution_count: number | null;
  outputs: NotebookOutput[];
  metadata: Record<string, unknown>;
}

export interface NotebookDoc {
  nbformat: number;
  nbformat_minor: number;
  metadata: Record<string, unknown>;
  cells: NotebookCell[];
}

export interface NormalizedNotebookOutput {
  kind: 'stream' | 'image' | 'svg' | 'html' | 'text' | 'error';
  streamName?: 'stdout' | 'stderr';
  mime?: string;
  dataUri?: string;
  text: string;
  ename?: string;
  evalue?: string;
}

function joinLines(val: unknown): string {
  if (typeof val === 'string') return val;
  if (Array.isArray(val)) return val.map((x) => (typeof x === 'string' ? x : String(x ?? ''))).join('');
  return '';
}

function splitToNbLines(text: string): string[] {
  if (!text) return [];
  const raw = text.split('\n');
  const lines = raw.map((line, i) => (i < raw.length - 1 ? `${line}\n` : line));
  if (lines.length > 1 && lines[lines.length - 1] === '') lines.pop();
  return lines;
}

let cellSeq = 1;
function nextCellId(): string {
  cellSeq += 1;
  return `cell-${Date.now().toString(36)}-${cellSeq}`;
}

export function stripAnsiCodes(raw: string): string {
  // eslint-disable-next-line no-control-regex
  return raw.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, '');
}

export function createEmptyNotebook(): NotebookDoc {
  return {
    nbformat: 4,
    nbformat_minor: 5,
    metadata: {
      kernelspec: { display_name: 'Python 3', language: 'python', name: 'python3' },
      language_info: { name: 'python' },
    },
    cells: [
      {
        id: nextCellId(),
        cell_type: 'code',
        source: '',
        execution_count: null,
        outputs: [],
        metadata: {},
      },
    ],
  };
}

export function parseIpynb(raw: string): { doc: NotebookDoc; parseError: string | null } {
  const trimmed = (raw ?? '').trim();
  if (!trimmed) return { doc: createEmptyNotebook(), parseError: null };
  try {
    const parsed = JSON.parse(trimmed) as Record<string, unknown>;
    if (!parsed || typeof parsed !== 'object' || !Array.isArray(parsed.cells)) {
      return { doc: createEmptyNotebook(), parseError: 'File is valid JSON but missing a "cells" array.' };
    }
    const cells: NotebookCell[] = parsed.cells.map((c: unknown, idx: number) => {
      const obj = (c && typeof c === 'object' ? c : {}) as Record<string, unknown>;
      const rawType = typeof obj.cell_type === 'string' ? obj.cell_type : 'code';
      const cell_type: NotebookCellType =
        rawType === 'markdown' || rawType === 'raw' ? rawType : 'code';
      return {
        id: typeof obj.id === 'string' && obj.id.trim() ? obj.id : `cell-${idx + 1}`,
        cell_type,
        source: joinLines(obj.source),
        execution_count:
          typeof obj.execution_count === 'number' ? obj.execution_count : null,
        outputs: Array.isArray(obj.outputs) ? (obj.outputs as NotebookOutput[]) : [],
        metadata:
          obj.metadata && typeof obj.metadata === 'object' && !Array.isArray(obj.metadata)
            ? (obj.metadata as Record<string, unknown>)
            : {},
      };
    });
    return {
      doc: {
        nbformat: typeof parsed.nbformat === 'number' ? parsed.nbformat : 4,
        nbformat_minor: typeof parsed.nbformat_minor === 'number' ? parsed.nbformat_minor : 5,
        metadata:
          parsed.metadata && typeof parsed.metadata === 'object' && !Array.isArray(parsed.metadata)
            ? (parsed.metadata as Record<string, unknown>)
            : {},
        cells,
      },
      parseError: null,
    };
  } catch (e) {
    return {
      doc: createEmptyNotebook(),
      parseError: e instanceof Error ? e.message : 'Invalid JSON in .ipynb file',
    };
  }
}

export function serializeIpynb(doc: NotebookDoc): string {
  const payload = {
    cells: doc.cells.map((c) => {
      const base: Record<string, unknown> = {
        cell_type: c.cell_type,
        id: c.id,
        metadata: c.metadata ?? {},
        source: splitToNbLines(c.source),
      };
      if (c.cell_type === 'code') {
        base.execution_count = c.execution_count;
        base.outputs = c.outputs ?? [];
      }
      return base;
    }),
    metadata: doc.metadata ?? {},
    nbformat: doc.nbformat || 4,
    nbformat_minor: doc.nbformat_minor ?? 5,
  };
  return `${JSON.stringify(payload, null, 1)}\n`;
}

export function addNotebookCell(
  doc: NotebookDoc,
  afterIndex: number,
  cellType: NotebookCellType = 'code',
): NotebookDoc {
  const nextCell: NotebookCell = {
    id: nextCellId(),
    cell_type: cellType,
    source: '',
    execution_count: null,
    outputs: [],
    metadata: {},
  };
  const at = Math.max(0, Math.min(doc.cells.length, afterIndex + 1));
  const cells = [...doc.cells.slice(0, at), nextCell, ...doc.cells.slice(at)];
  return { ...doc, cells };
}

export function deleteNotebookCell(doc: NotebookDoc, index: number): NotebookDoc {
  if (index < 0 || index >= doc.cells.length) return doc;
  const cells = doc.cells.filter((_, i) => i !== index);
  return { ...doc, cells };
}

export function moveNotebookCell(doc: NotebookDoc, index: number, delta: -1 | 1): NotebookDoc {
  const target = index + delta;
  if (index < 0 || index >= doc.cells.length || target < 0 || target >= doc.cells.length) return doc;
  const cells = [...doc.cells];
  const [moved] = cells.splice(index, 1);
  cells.splice(target, 0, moved);
  return { ...doc, cells };
}

export function updateNotebookCellSource(doc: NotebookDoc, index: number, source: string): NotebookDoc {
  if (index < 0 || index >= doc.cells.length) return doc;
  const cells = doc.cells.map((c, i) => (i === index ? { ...c, source } : c));
  return { ...doc, cells };
}

export function changeNotebookCellType(
  doc: NotebookDoc,
  index: number,
  cellType: NotebookCellType,
): NotebookDoc {
  if (index < 0 || index >= doc.cells.length) return doc;
  const cells = doc.cells.map((c, i) =>
    i === index
      ? {
          ...c,
          cell_type: cellType,
          execution_count: cellType === 'code' ? c.execution_count : null,
          outputs: cellType === 'code' ? c.outputs : [],
        }
      : c,
  );
  return { ...doc, cells };
}

export function applyNotebookExecution(
  doc: NotebookDoc,
  index: number,
  executionCount: number,
  outputs: NotebookOutput[],
): NotebookDoc {
  if (index < 0 || index >= doc.cells.length) return doc;
  const cells = doc.cells.map((c, i) =>
    i === index ? { ...c, execution_count: executionCount, outputs } : c,
  );
  return { ...doc, cells };
}

export function clearNotebookOutputs(doc: NotebookDoc): NotebookDoc {
  return {
    ...doc,
    cells: doc.cells.map((c) => ({ ...c, execution_count: null, outputs: [] })),
  };
}

/** Normalize raw nbformat v4 output objects into safe renderable blocks. */
export function normalizeNotebookOutputs(outputs: readonly NotebookOutput[]): NormalizedNotebookOutput[] {
  const items: NormalizedNotebookOutput[] = [];
  for (const out of outputs ?? []) {
    if (!out || typeof out !== 'object') continue;
    if (out.output_type === 'stream') {
      const text = stripAnsiCodes(joinLines(out.text));
      const streamName = out.name === 'stderr' ? 'stderr' : 'stdout';
      items.push({ kind: 'stream', streamName, text });
      continue;
    }
    if (out.output_type === 'error') {
      const tb = Array.isArray(out.traceback)
        ? out.traceback.map((l) => stripAnsiCodes(String(l))).join('\n')
        : `${out.ename ?? 'Error'}: ${out.evalue ?? ''}`;
      items.push({
        kind: 'error',
        ename: out.ename ?? 'Error',
        evalue: out.evalue ?? '',
        text: tb,
      });
      continue;
    }
    if (out.output_type === 'execute_result' || out.output_type === 'display_data') {
      const data = out.data ?? {};
      const png = joinLines(data['image/png']).trim();
      if (png) {
        items.push({ kind: 'image', mime: 'image/png', dataUri: `data:image/png;base64,${png}`, text: '' });
        continue;
      }
      const jpg = joinLines(data['image/jpeg']).trim();
      if (jpg) {
        items.push({ kind: 'image', mime: 'image/jpeg', dataUri: `data:image/jpeg;base64,${jpg}`, text: '' });
        continue;
      }
      const svg = joinLines(data['image/svg+xml']).trim();
      if (svg) {
        items.push({
          kind: 'svg',
          mime: 'image/svg+xml',
          dataUri: `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`,
          text: svg,
        });
        continue;
      }
      const html = joinLines(data['text/html']).trim();
      if (html) {
        items.push({ kind: 'html', mime: 'text/html', text: html });
        continue;
      }
      const jsonVal = data['application/json'];
      if (jsonVal !== undefined) {
        items.push({ kind: 'text', mime: 'application/json', text: JSON.stringify(jsonVal, null, 2) });
        continue;
      }
      const plain = joinLines(data['text/plain']);
      if (plain) {
        items.push({ kind: 'text', mime: 'text/plain', text: stripAnsiCodes(plain) });
      }
    }
  }
  return items;
}
