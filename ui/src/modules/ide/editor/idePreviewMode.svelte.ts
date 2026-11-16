// The IDE editor's Editor | Split | Preview mode (the trio at the end of the tab
// strip, IdePreviewModeToggle). Remembered per file KIND (all Markdown files,
// all HTML files…) in localStorage; the rules live in the pure
// lib/components/filePreviewKind.ts:
//   • code files have no control; images/PDF always preview (no text to edit);
//   • the stacked phone layout offers Editor | Preview (a remembered Split
//     shows the editor there);
//   • CSV/TSV/JSONL keep their editable table (IdeCsvView) as Preview;
//   • build manifests keep their own Text | Dependency Analyzer switch.
// The commands ("Open Preview", "Open Preview to the Side") call `toggle` /
// `openSide`. No keyboard shortcut: ⇧⌘V is clipboard history in the IDE and
// ⌘K is the app's command bar, so a ⌘K V chord can't exist.

import {
  availablePreviewModes,
  effectivePreviewMode,
  filePreviewKind,
  isTablePreviewKind,
  parsePreviewModes,
  type FilePreviewKind,
  type FilePreviewMode,
} from '../../../lib/components/filePreviewKind';
import { isIdeManifestPath } from '../../../lib/components/cm-ide-manifest';
import { ideLayout } from '../shell/ideLayout.svelte';

export const IDE_PREVIEW_MODE_KEY = 'otto.ide.previewMode.v1';

/** What the editor pane shows: the editor alone, editor + preview, the
 *  preview in place of the editor, or the editable table in its place. */
export type IdePreviewLayout = 'none' | 'split' | 'only' | 'table';

function readModes(): Partial<Record<FilePreviewKind, FilePreviewMode>> {
  try {
    return parsePreviewModes(localStorage.getItem(IDE_PREVIEW_MODE_KEY));
  } catch {
    return {};
  }
}

class IdePreviewModeStore {
  modes = $state<Partial<Record<FilePreviewKind, FilePreviewMode>>>(readModes());

  /** The preview kind of an IDE file; null for code, untitled buffers and manifests. */
  kindOf(path: string | null | undefined): FilePreviewKind | null {
    if (!path || path.startsWith('untitled:') || path.includes('://') || isIdeManifestPath(path)) return null;
    return filePreviewKind(path);
  }

  modeFor(path: string | null | undefined): FilePreviewMode {
    const k = this.kindOf(path);
    return effectivePreviewMode(k, k ? this.modes[k] : undefined, ideLayout.stack);
  }

  /** The buttons the control shows for this file (none = no control). */
  available(path: string | null | undefined): FilePreviewMode[] {
    return availablePreviewModes(this.kindOf(path), ideLayout.stack);
  }

  layoutFor(path: string | null | undefined): IdePreviewLayout {
    const k = this.kindOf(path);
    const m = this.modeFor(path);
    if (!k || m === 'source') return 'none';
    if (m === 'split') return 'split';
    return isTablePreviewKind(k) ? 'table' : 'only';
  }

  /** The class the editor pane wears for its loaded tab (null while the editor
   *  for it is still staging): `pv-split` lays it out as a row with the preview
   *  beside the editor; `pv-only` hides the (still mounted) editor. */
  paneClass(tab: { path: string; loading: boolean; error: string | null } | null): string {
    if (!tab || tab.loading || tab.error) return '';
    const l = this.layoutFor(tab.path);
    return l === 'split' ? 'pv-split' : l === 'only' ? 'pv-only' : '';
  }

  set(path: string | null | undefined, mode: FilePreviewMode): void {
    const k = this.kindOf(path);
    if (!k || !this.available(path).includes(mode)) return;
    this.modes = { ...this.modes, [k]: mode };
    try {
      localStorage.setItem(IDE_PREVIEW_MODE_KEY, JSON.stringify(this.modes));
    } catch {
      /* private mode: the choice holds for this window only */
    }
  }

  /** "Open Preview": Preview, or back to the editor when it already shows. */
  toggle(path: string | null | undefined): void {
    this.set(path, this.modeFor(path) === 'preview' ? 'source' : 'preview');
  }

  /** "Open Preview to the Side": Split (Preview when stacked — there is no side). */
  openSide(path: string | null | undefined): void {
    this.set(path, ideLayout.stack ? 'preview' : 'split');
  }
}

export const idePreviewMode = new IdePreviewModeStore();
