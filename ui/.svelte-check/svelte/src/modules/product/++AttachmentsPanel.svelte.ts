///<reference types="svelte" />
;// AttachmentsPanel — local story attachments with paste/drag-drop/file-picker,
// thumbnails (image), iframe (PDF), file chip+download (other), "Mark as
// mockup", and delete. Screenshot paste from clipboardData is the headline
// feature: calling `uploadBlob(blob)` is the shared entry-point used by both
// the panel's own paste handler and the body-textarea paste handler in
// OverviewTab (which also splices a markdown ref into draftBody).

import { formatBytes } from '../../lib/metric-format';

import { product } from '../../lib/stores/product.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { toastError } from '../../lib/toastError';
import { toasts } from '../../lib/toast.svelte';
import { kindLabel } from '../../lib/labels';
import { AttachmentUrls } from './attachmentUrls.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { ProductAttachment } from './types';
;

  // ── Props ─────────────────────────────────────────────────────────────────

  interface Props {
    // Called by the parent when a blob is pasted into the body textarea so the
    // panel can show the optimistic thumbnail. Returns the created attachment.
    // `insertIntoBody` is handled by the parent (OverviewTab) itself.
    onAttachmentCreated?: (att: ProductAttachment) => void;
  };function $$render() {

  
  
  
  
  
  
  

  
  
  
  
  
  
  
  
  
  
  


  const { onAttachmentCreated }: Props = $props();

  // ── Local state (names prefixed to avoid collisions with OverviewTab state) ──

  let localAtts = $state<ProductAttachment[]>([]);
  let localAttLoading = $state(false);

  // Authed object-URL cache for previews (attachmentUrls.svelte.ts, shared
  // with OverviewTab's body preview): urls / loading / errors by id.
  const atts = new AttachmentUrls();

  // Upload-in-progress count (for optimistic thumbnails).
  let uploading = $state(false);
  let filePicker = $state<HTMLInputElement | null>(null);

  // Drag-over styling toggle.
  let dragOver = $state(false);

  // Screenshot index for unique filenames within a session.
  let screenshotIdx = $state(0);

  // ── Lifecycle ─────────────────────────────────────────────────────────────

  $effect(() => {
    // Re-load when selectedId changes (product.selectedId as a reactive read).
    product.selectedId;
    // Revoke + forget the previous story's URLs (a fetch still in flight for
    // it frees its blob when it settles).
    atts.reset();
    localAtts = [];
    void loadAttachments();
  });

  $effect(() => {
    // Cleanup on unmount: revoke any URLs accumulated since the last story change.
    return () => atts.dispose();
  });

  // ── Helpers ───────────────────────────────────────────────────────────────

  /** Read a Blob with FileReader → base64 string (no data-URL prefix). */
  function fileToB64(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onerror = () => reject(reader.error);
      reader.onload = () => {
        const result = reader.result as string;
        // Strip the "data:<mime>;base64," prefix.
        const idx = result.indexOf(',');
        resolve(idx >= 0 ? result.slice(idx + 1) : result);
      };
      reader.readAsDataURL(blob);
    });
  }

  /** Load the attachment list from the server. */
  /** A failed load — inline with Retry, never as the "drop files here" hint. */
  let loadError = $state<string | null>(null);
  async function loadAttachments(): Promise<void> {
    localAttLoading = true;
    loadError = null;
    try {
      localAtts = await product.listAttachments();
      // Eagerly fetch preview URLs for images.
      for (const att of localAtts) {
        if (att.mime.startsWith('image/')) void loadAttUrl(att.id);
      }
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      localAttLoading = false;
    }
  }

  /** Fetch an authed blob URL for one attachment (cached, idempotent). */
  async function loadAttUrl(id: string): Promise<void> {
    await atts.load(id);
  }

  /**
   * Upload a single Blob as an attachment. Used by:
   *   - Panel paste handler (image pasted directly here)
   *   - OverviewTab body-textarea paste handler (calls `panelRef.uploadBlob`)
   *   - Drag-drop handler
   *   - File-picker handler
   *
   * Returns the created attachment so callers can splice a markdown ref into
   * the body text.
   */
   async function uploadBlob(
    blob: Blob,
    opts: { filename?: string; kind?: string } = {},
  ): Promise<ProductAttachment> {
    const mime = blob.type || 'application/octet-stream';
    const filename = opts.filename ?? (blob instanceof File ? blob.name : `file-${Date.now()}`);
    const kind = opts.kind ?? (mime.startsWith('image/') ? 'image' : undefined);
    const data_b64 = await fileToB64(blob);
    uploading = true;
    try {
      const att = await product.uploadAttachment({ filename, mime, kind, data_b64 });
      localAtts = [att, ...localAtts];
      // Eagerly load preview for images.
      if (mime.startsWith('image/')) void loadAttUrl(att.id);
      onAttachmentCreated?.(att);
      return att;
    } finally {
      uploading = false;
    }
  }

  /** Upload files from a FileList (drag-drop or file-picker). */
  async function uploadFiles(files: FileList | File[]): Promise<void> {
    for (const file of Array.from(files)) {
      try {
        await uploadBlob(file, { filename: file.name });
      } catch (e) {
        toastError(`Couldn’t upload ${file.name}`, e);
      }
    }
  }

  /** Handle paste event directly on the panel (not in the body textarea). */
  function handlePanelPaste(e: ClipboardEvent): void {
    if (!e.clipboardData) return;
    for (const item of Array.from(e.clipboardData.items)) {
      if (item.kind === 'file' && item.type.startsWith('image/')) {
        e.preventDefault();
        const blob = item.getAsFile();
        if (!blob) continue;
        const idx = ++screenshotIdx;
        void (async () => {
          try {
            await uploadBlob(blob, { filename: `screenshot-${idx}.png`, kind: 'image' });
          } catch (ex) {
            toastError('Couldn’t upload the screenshot', ex);
          }
        })();
        break; // handle first image item only
      }
    }
  }

  /** Drag-over: allow drop. */
  function handleDragOver(e: DragEvent): void {
    e.preventDefault();
    dragOver = true;
  }
  function handleDragLeave(): void {
    dragOver = false;
  }

  /** Drop handler: upload all dropped files. */
  function handleDrop(e: DragEvent): void {
    e.preventDefault();
    dragOver = false;
    if (e.dataTransfer?.files?.length) {
      void uploadFiles(e.dataTransfer.files);
    }
  }

  /** File-picker change handler. */
  function handleFileInput(e: Event): void {
    const input = e.currentTarget as HTMLInputElement;
    if (input.files?.length) {
      void uploadFiles(input.files);
      input.value = ''; // reset so the same file can be re-selected
    }
  }

  /** Mark as mockup. */
  async function markAsMockup(att: ProductAttachment): Promise<void> {
    try {
      const updated = await product.patchAttachment(att.id, { kind: 'mockup' });
      localAtts = localAtts.map((a) => (a.id === att.id ? updated : a));
      toasts.info('Marked as mockup');
    } catch (e) {
      toastError('Couldn’t update attachment', e);
    }
  }

  /** Delete an attachment with confirm. */
  async function deleteAtt(att: ProductAttachment): Promise<void> {
    const ok = await confirmer.ask(`Delete “${att.filename}”?`, {
      title: 'Delete attachment',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await product.deleteAttachment(att.id);
      localAtts = localAtts.filter((a) => a.id !== att.id);
      // Revoke cached blob URL if any.
      atts.forget(att.id);
      toasts.info('Attachment deleted');
    } catch (e) {
      toasts.error('Couldn’t delete', loadErrorText(e));
    }
  }
;
async () => {

  { svelteHTML.createElement("div", {               "class":`att-panel`,"role":`region`,"aria-label":`Story attachments`,"onpaste":handlePanelPaste,"ondragover":handleDragOver,"ondragleave":handleDragLeave,"ondrop":handleDrop,});dragOver;
  
   { svelteHTML.createElement("div", { "class":`att-header-row`,});
     { svelteHTML.createElement("span", { "class":`att-panel-title`,});  }
    
     { svelteHTML.createElement("button", {       "class":`btn small`,"title":`Attach files`,"disabled":uploading,"onclick":() => filePicker?.click(),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} uploading ? 'Uploading…' : 'File';
     }
     { const $$_input2 = svelteHTML.createElement("input", {               "class":`att-file-input`,"type":`file`,"multiple":true,"tabindex":-1,"aria-hidden":`true`,"onchange":handleFileInput,"disabled":uploading,});filePicker = $$_input2;}
   }

  
  if((localAttLoading || loadError) && localAtts.length === 0){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`attachments`,"variant":`compact`,"loading":localAttLoading,"error":loadError,"empty":true,"onretry":() => void loadAttachments(),}});}
  } else if (localAtts.length === 0 && !uploading){
     { svelteHTML.createElement("div", { "class":`att-drop-hint`,});
               
     }
  }

  
  if(uploading){
     { svelteHTML.createElement("div", { "class":`att-uploading`,});  }
  }

  
  if(localAtts.length > 0){
    if(loadError){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`attachments`,"variant":`compact`,"loading":localAttLoading,"error":loadError,"onretry":() => void loadAttachments(),}});}
    }
     { svelteHTML.createElement("div", { "class":`att-list`,});
         for(let att of __sveltets_2_ensureArray(localAtts)){att.id;
         { svelteHTML.createElement("div", { "class":`att-item`,});
          
           { svelteHTML.createElement("div", { "class":`att-meta-row`,});
             { svelteHTML.createElement("span", {   "class":`att-fname`,"title":att.filename,});att.filename; }
             { svelteHTML.createElement("span", { "class":`att-size`,});formatBytes(att.size_bytes); }
            if(att.kind && att.kind !== 'image'){
               { svelteHTML.createElement("span", { "class":`att-kind-badge`,});kindLabel(att.kind); }
            }
           }

          
          if(att.mime.startsWith('image/')){
             { svelteHTML.createElement("div", { "class":`att-preview`,});
              if(atts.urls[att.id]){
                 { svelteHTML.createElement("img", {       "class":`att-img`,"src":atts.urls[att.id],"alt":att.filename,});}
              } else if (atts.loading[att.id]){
                 { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {      "rows":1,"height":64,"label":`the preview`,}});}
              } else if (atts.errors[att.id]){
                 { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {        "what":att.filename,"error":atts.errors[att.id],"empty":true,"onretry":() => loadAttUrl(att.id),}});}
              }else{
                 { svelteHTML.createElement("button", {     "class":`att-load-btn`,"onclick":() => loadAttUrl(att.id),});
                   
                 }
              }
             }
          } else if (att.mime === 'application/pdf'){
             { svelteHTML.createElement("div", { "class":`att-preview att-pdf-wrap`,});
              if(atts.urls[att.id]){
                 { svelteHTML.createElement("iframe", {         "class":`att-pdf`,"src":atts.urls[att.id],"title":att.filename,"sandbox":`allow-scripts allow-same-origin`,}); }
              } else if (atts.errors[att.id]){
                 { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {        "what":att.filename,"error":atts.errors[att.id],"empty":true,"onretry":() => loadAttUrl(att.id),}});}
              }else{
                 { svelteHTML.createElement("button", {       "class":`att-load-btn`,"onclick":() => loadAttUrl(att.id),"disabled":atts.loading[att.id],});
                  atts.loading[att.id] ? 'Loading preview…' : 'Preview PDF';
                 }
              }
             }
          }else{
            
             { svelteHTML.createElement("div", { "class":`att-file-chip`,});
               { svelteHTML.createElement("span", {   "class":`att-chip-icon`,"aria-hidden":`true`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":14,}});} }
               { svelteHTML.createElement("span", { "class":`att-chip-name`,});att.filename; }
              if(atts.urls[att.id]){
                 { svelteHTML.createElement("a", {       "class":`att-dl-link`,"href":atts.urls[att.id],"download":att.filename,});  }
              } else if (!atts.errors[att.id]){
                 { svelteHTML.createElement("button", {       "class":`att-load-btn`,"onclick":() => loadAttUrl(att.id),"disabled":atts.loading[att.id],});
                  atts.loading[att.id] ? 'Preparing…' : 'Download';
                 }
              }
             }
            if(atts.errors[att.id]){
               { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "what":att.filename,"error":atts.errors[att.id],"empty":true,"onretry":() => loadAttUrl(att.id),}});}
            }
          }

          
           { svelteHTML.createElement("div", { "class":`att-action-row`,});
            if(att.kind !== 'mockup'){
               { svelteHTML.createElement("button", {       "class":`att-action-btn`,"onclick":() => markAsMockup(att),"title":`Mark as mockup`,});   }
            }else{
               { svelteHTML.createElement("span", { "class":`att-kind-badge mockup-badge`,});  }
            }
             { svelteHTML.createElement("button", {         "class":`att-action-btn att-delete-btn`,"onclick":() => deleteAtt(att),"title":`Delete attachment`,"aria-label":`Delete ${att.filename}`,});  }
           }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {} as any as { uploadBlob: typeof uploadBlob }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AttachmentsPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AttachmentsPanel__SvelteComponent_ = ReturnType<typeof AttachmentsPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default AttachmentsPanel__SvelteComponent_;