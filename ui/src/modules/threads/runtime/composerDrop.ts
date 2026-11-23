// Files dropped on a thread pane join its composer through the SAME intake
// as paste and the attach button (runtimeComposer.ts appendDraftAttachments:
// the per-file / per-message limits, and an unsupported type is refused by
// name). The composer box alone lights up while files are over the pane —
// no full-window overlay. The desktop app keeps the webview's own HTML5 drop
// (tauri.conf.json `dragDropEnabled: false`), so this one path serves both.
import { attachmentSupport, takesFile, type TakesCaps } from './attachmentSupport';
import { appendDraftAttachments, type ComposerFile } from './runtimeComposer';

/** Dropped files into a thread's draft; an agent that takes none says so. */
export function dropAttachments(
  ownerId: string,
  draft: { images: ComposerFile[]; attachError?: string | null },
  files: File[],
  caps: TakesCaps,
  agent: string,
): void {
  const can = attachmentSupport(caps);
  if (!can.images) draft.attachError = `${agent} takes no attachments here.`;
  else void appendDraftAttachments(ownerId, draft, files, (f) => takesFile(caps, f), can.files ? 'images and PDFs' : 'images');
}

const hasFiles = (e: DragEvent): boolean => !!e.dataTransfer && [...e.dataTransfer.types].includes('Files');

export function fileDrop(node: HTMLElement, opts: { box: () => HTMLElement | null; onfiles: (files: File[]) => void }) {
  let o = opts;
  let depth = 0;
  const mark = (on: boolean): void => {
    const box = o.box();
    if (on) box?.setAttribute('data-drop', '');
    else box?.removeAttribute('data-drop');
  };
  const enter = (e: DragEvent): void => {
    if (!hasFiles(e)) return;
    depth++;
    mark(true);
  };
  const over = (e: DragEvent): void => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    e.dataTransfer!.dropEffect = 'copy';
  };
  const leave = (e: DragEvent): void => {
    if (!hasFiles(e)) return;
    depth = Math.max(0, depth - 1);
    if (depth === 0) mark(false);
  };
  const drop = (e: DragEvent): void => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth = 0;
    mark(false);
    const files = [...(e.dataTransfer?.files ?? [])];
    if (files.length) o.onfiles(files);
  };
  node.addEventListener('dragenter', enter);
  node.addEventListener('dragover', over);
  node.addEventListener('dragleave', leave);
  node.addEventListener('drop', drop);
  return {
    update(next: typeof opts) {
      o = next;
    },
    destroy() {
      node.removeEventListener('dragenter', enter);
      node.removeEventListener('dragover', over);
      node.removeEventListener('dragleave', leave);
      node.removeEventListener('drop', drop);
    },
  };
}
