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
  /** Preserved nbformat v4 Markdown/raw cell attachments (`attachment:<name>`). */
  attachments?: Record<string, Record<string, unknown>>;
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
  /** `text` split into ANSI-coloured runs (streams and tracebacks). */
  segments?: AnsiSegment[];
  /** Shown under the output, e.g. why a widget renders as text. */
  note?: string;
  ename?: string;
  evalue?: string;
}

/** One run of text with its ANSI SGR colour (a token-backed class) and weight. */
export interface AnsiSegment {
  text: string;
  color?: 'danger' | 'success' | 'warning' | 'info' | 'accent' | 'dim';
  bold?: boolean;
}

const ANSI_COLORS: Record<number, AnsiSegment['color']> = {
  0: 'dim', 1: 'danger', 2: 'success', 3: 'warning', 4: 'info', 5: 'accent', 6: 'info',
};

/** Split terminal text into coloured runs (SGR codes only; every other escape
 *  sequence is dropped). Colours map onto semantic tokens, never raw hex. */
export function ansiSegments(raw: string): AnsiSegment[] {
  const out: AnsiSegment[] = [];
  let color: AnsiSegment['color'];
  let bold = false;
  // eslint-disable-next-line no-control-regex
  const re = /\x1b\[([0-9;?]*)([a-zA-Z])/g;
  let last = 0;
  const push = (text: string) => {
    if (!text) return;
    const prev = out[out.length - 1];
    if (prev && prev.color === color && Boolean(prev.bold) === bold) prev.text += text;
    else out.push({ text, ...(color ? { color } : {}), ...(bold ? { bold } : {}) });
  };
  for (let mt = re.exec(raw); mt; mt = re.exec(raw)) {
    push(raw.slice(last, mt.index));
    last = re.lastIndex;
    if (mt[2] !== 'm') continue;
    const codes = (mt[1] || '0').split(';').map((c) => Number(c) || 0);
    for (let i = 0; i < codes.length; i++) {
      const c = codes[i];
      if (c === 0) { color = undefined; bold = false; }
      else if (c === 1) bold = true;
      else if (c === 22) bold = false;
      else if (c === 39) color = undefined;
      else if (c >= 30 && c <= 37) color = c === 37 ? undefined : ANSI_COLORS[c - 30];
      else if (c >= 90 && c <= 97) color = c === 97 ? undefined : ANSI_COLORS[c - 90];
      else if (c === 38 || c === 48) {
        // 38;5;n (256 colours: the first 16 map onto the base set) / 38;2;r;g;b.
        if (codes[i + 1] === 5) {
          const n = codes[i + 2] ?? 0;
          if (c === 38) color = n < 16 && n % 8 !== 7 ? ANSI_COLORS[n % 8] : undefined;
          i += 2;
        } else if (codes[i + 1] === 2) i += 4;
      }
    }
  }
  push(raw.slice(last));
  return out;
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
      const attachments =
        obj.attachments && typeof obj.attachments === 'object' && !Array.isArray(obj.attachments)
          ? (obj.attachments as Record<string, Record<string, unknown>>)
          : undefined;
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
        ...(attachments ? { attachments } : {}),
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
      } else if (c.attachments && Object.keys(c.attachments).length > 0) {
        // nbformat 4.5 allows `attachments` only on markdown and raw cells (DATA2-2).
        base.attachments = c.attachments;
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
  const cells = doc.cells.map((c, i) => {
    if (i !== index) return c;
    const nextMeta = { ...(c.metadata ?? {}) };
    let nextAttachments = c.attachments;
    if (cellType === 'code') {
      // Preserve Markdown/Raw attachments inside metadata so converting to Code
      // never serializes an invalid nbformat 4.5 Code cell (DATA2-2).
      if (nextAttachments && Object.keys(nextAttachments).length > 0) {
        nextMeta.otto_preserved_attachments = nextAttachments;
      }
      nextAttachments = undefined;
    } else if (
      (!nextAttachments || Object.keys(nextAttachments).length === 0) &&
      nextMeta.otto_preserved_attachments &&
      typeof nextMeta.otto_preserved_attachments === 'object' &&
      !Array.isArray(nextMeta.otto_preserved_attachments)
    ) {
      nextAttachments = nextMeta.otto_preserved_attachments as Record<string, Record<string, unknown>>;
      delete nextMeta.otto_preserved_attachments;
    }
    return {
      ...c,
      cell_type: cellType,
      metadata: nextMeta,
      execution_count: cellType === 'code' ? c.execution_count : null,
      outputs: cellType === 'code' ? c.outputs : [],
      ...(nextAttachments ? { attachments: nextAttachments } : { attachments: undefined }),
    };
  });
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

/** Apply execution results by stable cell id so moving/deleting cells mid-run
 *  never overwrites a different cell's outputs. */
export function applyNotebookExecutionById(
  doc: NotebookDoc,
  cellId: string,
  executionCount: number,
  outputs: NotebookOutput[],
): { doc: NotebookDoc; applied: boolean } {
  const idx = doc.cells.findIndex((c) => c.id === cellId && c.cell_type === 'code');
  if (idx < 0) return { doc, applied: false };
  return { doc: applyNotebookExecution(doc, idx, executionCount, outputs), applied: true };
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
      const raw = joinLines(out.text);
      const streamName = out.name === 'stderr' ? 'stderr' : 'stdout';
      items.push({ kind: 'stream', streamName, text: stripAnsiCodes(raw), segments: ansiSegments(raw) });
      continue;
    }
    if (out.output_type === 'error') {
      const raw = Array.isArray(out.traceback) && out.traceback.length > 0
        ? out.traceback.map((l) => String(l)).join('\n')
        : `${out.ename ?? 'Error'}: ${out.evalue ?? ''}`;
      items.push({
        kind: 'error',
        ename: out.ename ?? 'Error',
        evalue: out.evalue ?? '',
        text: stripAnsiCodes(raw),
        segments: ansiSegments(raw),
      });
      continue;
    }
    if (out.output_type === 'execute_result' || out.output_type === 'display_data') {
      const data = out.data ?? {};
      // Widgets and JavaScript never run: show the richest other MIME type.
      if (data['application/vnd.jupyter.widget-view+json'] !== undefined) {
        const plain = stripAnsiCodes(joinLines(data['text/plain']));
        items.push({ kind: 'text', mime: 'text/plain', text: plain, note: 'Widgets aren’t supported in Otto notebooks.' });
        continue;
      }
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
      } else if (data['application/javascript'] !== undefined) {
        items.push({ kind: 'text', text: '', note: 'JavaScript outputs don’t run in Otto notebooks.' });
      }
    }
  }
  return items;
}

/** Apply one streamed `/ide/notebooks/stream` output event to a cell's live outputs. */
export function applyNotebookStreamOutput(
  outputs: readonly NotebookOutput[],
  ev: { kind: 'output'; index: number; output: Record<string, unknown> } | { kind: 'clear_output' },
): NotebookOutput[] {
  if (ev.kind === 'clear_output') return [];
  const next = [...outputs];
  next[ev.index] = ev.output as unknown as NotebookOutput;
  return next.filter((o) => o !== undefined);
}

interface KernelChoice { id: string; kind: string; language: string; spec_name?: string | null; active: boolean }

/** The kernel a notebook opens on: its `metadata.kernelspec.name`, else the
 *  first Jupyter kernel of the same language (Jupyter's rule), else the active one. */
export function pickNotebookKernel(kernels: readonly KernelChoice[], metadata: Record<string, unknown>): string {
  const spec = (metadata?.kernelspec ?? {}) as { name?: unknown; language?: unknown };
  const lang = typeof spec.language === 'string' ? spec.language.toLowerCase() : '';
  const byName = typeof spec.name === 'string' ? kernels.find((k) => k.kind === 'jupyter' && k.spec_name === spec.name) : undefined;
  const byLang = lang ? kernels.find((k) => k.kind === 'jupyter' && k.language.toLowerCase() === lang) : undefined;
  return (byName ?? byLang ?? kernels.find((k) => k.active) ?? kernels[0])?.id ?? '';
}

/** Write the chosen Jupyter kernel into the notebook metadata, as Jupyter does. */
export function withKernelspecMetadata(
  doc: NotebookDoc,
  kernel: { kind: string; spec_name?: string | null; display_name: string; language: string },
): NotebookDoc {
  if (kernel.kind !== 'jupyter' || !kernel.spec_name) return doc;
  const kernelspec = { display_name: kernel.display_name, language: kernel.language, name: kernel.spec_name };
  const cur = doc.metadata?.kernelspec as Record<string, unknown> | undefined;
  if (cur && cur.name === kernelspec.name && cur.display_name === kernelspec.display_name && cur.language === kernelspec.language) return doc;
  return { ...doc, metadata: { ...doc.metadata, kernelspec } };
}
