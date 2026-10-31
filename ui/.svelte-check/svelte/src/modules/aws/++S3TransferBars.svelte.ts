///<reference types="svelte" />
;// The S3 browser's sticky transfer bars: the upload in progress and the
// download in progress (with Cancel). Split out of S3Browser.svelte; the
// bars are plain text, and each transfer is announced once when it starts
// (lib/announce — a status inserted with its text is silent in VoiceOver),
// never per progress tick.

import { announce } from '../../lib/announce.svelte';
import { fmtBytes } from './util';
;

  interface Props {
    uploading: { done: number; total: number; name: string } | null;
    download: { key: string; received: number; total: number | null } | null;
    oncanceldownload: () => void;
  };function $$render() {

  
  
  
  
  
  
  

  let { uploading, download, oncanceldownload }: Props = $props();

  function leaf(key: string): string {
    const trimmed = key.endsWith('/') ? key.slice(0, -1) : key;
    return trimmed.slice(trimmed.lastIndexOf('/') + 1) || key;
  }
  // Keyed by the file / object, not the progress object, so a tick is silent.
  const uploadingName = $derived(uploading?.name ?? '');
  const downloadKey = $derived(download?.key ?? '');
  $effect(() => { if (uploadingName) announce(`Uploading ${uploadingName}`); });
  $effect(() => { if (downloadKey) announce(`Downloading ${leaf(downloadKey)}`); });
;
async () => {

if(uploading){
   { svelteHTML.createElement("div", { "class":`dl-bar`,});
     { svelteHTML.createElement("span", { "class":`mono`,}); uploading.name; }
     { svelteHTML.createElement("progress", {   "max":uploading.total,"value":uploading.done,}); }
     { svelteHTML.createElement("span", { "class":`dim`,});uploading.done + 1;  uploading.total; }
   }
}

if(download){
   { svelteHTML.createElement("div", { "class":`dl-bar`,});
     { svelteHTML.createElement("span", { "class":`mono`,});leaf(download.key); }
     { svelteHTML.createElement("progress", {   "max":download.total ?? undefined,"value":download.total ? download.received : undefined,}); }
     { svelteHTML.createElement("span", { "class":`dim`,});fmtBytes(download.received);download.total ? ` / ${fmtBytes(download.total)}` : ''; }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":oncanceldownload,});  }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const S3TransferBars__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type S3TransferBars__SvelteComponent_ = ReturnType<typeof S3TransferBars__SvelteComponent_>;
/*Ωignore_endΩ*/export default S3TransferBars__SvelteComponent_;