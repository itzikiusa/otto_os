// Paste / drop into the note editor → save as a vault attachment → insert
// the embed at the cursor. Pure helpers (unit-tested in
// ui/unit/vaultAttachmentUpload.test.ts); the stateful queue lives in
// AttachmentBar.svelte.

/** Mirrors the daemon cap (`otto-vault` `MAX_ATTACHMENT_BYTES`). */
export const MAX_ATTACHMENT_BYTES = 20 * 1024 * 1024;

/** Minimal shape of a DataTransfer we read (clipboard or drop). */
export interface TransferLike {
  files?: ArrayLike<File> | null;
  items?: ArrayLike<{ kind: string; getAsFile(): File | null }> | null;
  types?: ArrayLike<string> | readonly string[] | null;
}

/** The files carried by a paste/drop. `files` first; `items` covers mobile
 *  Safari, which exposes a pasted screenshot only as a file item. */
export function transferFiles(dt: TransferLike | null | undefined): File[] {
  if (!dt) return [];
  const files = Array.from(dt.files ?? []);
  if (files.length) return files;
  const out: File[] = [];
  for (const it of Array.from(dt.items ?? [])) {
    if (it.kind !== 'file') continue;
    const f = it.getAsFile();
    if (f) out.push(f);
  }
  return out;
}

/** Does a dragover carry files (so the editor should accept the drop)? */
export function carriesFiles(dt: TransferLike | null | undefined): boolean {
  return !!dt && Array.from(dt.types ?? []).includes('Files');
}

/** The name sent to the daemon. A clipboard image arrives as a generic
 *  `image.png`: send '' so the vault names it "Pasted image <time>". */
export function uploadName(file: { name?: string }, source: 'paste' | 'drop'): string {
  const name = file.name ?? '';
  if (source === 'paste' && /^image\.(png|jpe?g|gif|webp|bmp|tiff?|heic)$/i.test(name)) return '';
  return name;
}

// Rounded UP: a file 1 byte over the cap must not read "20.0 MB of 20 MB".
const mb = (n: number) => `${(Math.ceil((n / (1024 * 1024)) * 10) / 10).toFixed(1)} MB`;

/** A client-side refusal (no request sent), or null when the file may go. */
export function preflight(file: { name?: string; size: number }): string | null {
  if (file.size === 0) return `${file.name || 'The file'} is empty.`;
  if (file.size > MAX_ATTACHMENT_BYTES) {
    return `${file.name || 'The file'} is ${mb(file.size)}; attachments can be at most ${MAX_ATTACHMENT_BYTES / (1024 * 1024)} MB.`;
  }
  return null;
}

/** Text to insert for the `index`-th embed of one paste/drop. */
export function insertionFor(embed: string, index: number): string {
  return index > 0 ? `\n${embed}` : embed;
}

/** Svelte action: hand files pasted or dropped onto `node` to `take` in the
 *  capture phase (before CodeMirror sees them); text stays CodeMirror's. */
export function fileTarget(node: HTMLElement, take: (files: File[], source: 'paste' | 'drop') => void) {
  let current = take;
  const grab = (e: Event, dt: TransferLike | null | undefined, source: 'paste' | 'drop') => {
    const files = transferFiles(dt);
    if (!files.length) return;
    e.preventDefault();
    e.stopPropagation();
    current(files, source);
  };
  const onPaste = (e: ClipboardEvent) => grab(e, e.clipboardData, 'paste');
  const onDrop = (e: DragEvent) => grab(e, e.dataTransfer, 'drop');
  const onOver = (e: DragEvent) => {
    if (!carriesFiles(e.dataTransfer)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
  };
  node.addEventListener('paste', onPaste, true);
  node.addEventListener('drop', onDrop, true);
  node.addEventListener('dragover', onOver, true);
  return {
    update(next: typeof take) {
      current = next;
    },
    destroy() {
      node.removeEventListener('paste', onPaste, true);
      node.removeEventListener('drop', onDrop, true);
      node.removeEventListener('dragover', onOver, true);
    },
  };
}
