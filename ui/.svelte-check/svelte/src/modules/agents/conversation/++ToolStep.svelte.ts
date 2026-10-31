///<reference types="svelte" />
;
import { copyTextOrThrow } from '../../../lib/clipboard';
import { formatBytes } from '../../../lib/metric-format';
// One tool call inside a response's step group, as ONE scannable line —
// kind icon · verb · target (command / file / pattern) · dim detail — with a
// status mark at the end (spinner while running, ✓, ✕, or "no result") and,
// for commands, the output's last line under it (the tail you'd look for
// first: "test result: FAILED …"). Expanding shows the detail: the full
// command + output (scrolled to its end; windowed past 400 lines), an edit's
// inline diff, a written file's content, a read's text, result images.
// Elided live-push previews fetch the stored result on first expand.
import { getContext, tick } from 'svelte';
import { toastError } from '../../../lib/toastError';
import Icon from '../../../lib/components/Icon.svelte';
import { announceShown } from '../../../lib/announce.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../../lib/loadError';
import VirtualList from '../../../lib/components/VirtualList.svelte';
import ImageBlock from './ImageBlock.svelte';
import InlineDiff from './InlineDiff.svelte';
import CodeView from './CodeView.svelte';
import Markdown from './Markdown.svelte';
import { openFile } from '../../../lib/stores/openfile.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { linkifyOutput } from './chatMarkdown';
import { openExternal } from '../../../lib/external';
import { TOOL_CHROME, editInputPatch, relPath, lastLine, patchToDiff, toolLine, toolStatus } from './format';
import { autoLang, ensureHljs, highlightBlock, highlightLine, langFromPath } from '../../../lib/hl';
import type { Block } from '../../../lib/api/types';
import { CONV_CTX, type ConvContext } from './context';
import { effectiveResult, fetchToolResult, isElided } from './toolDetail';
import type { ToolResult } from '../../../lib/api/types';
;

  interface Props {
    block: Extract<Block, { kind: 'tool_call' }>;
    /** The response is still being worked on — a call without a result is
     *  running, not abandoned. */
    live?: boolean;
    /** The agent is stopped on this call waiting for you (permission prompt /
     *  question on its terminal) — shown as waiting, not spinning. */
    waiting?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { block, live = false, waiting = false }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);

  let open = $state(false);
  // An over-cap live delta ships this result as a short preview
  // (`result.elided`); the full stored output is fetched the first time the
  // step is expanded (SA-04 — the view used to re-fetch the whole page).
  let full = $state<ToolResult | null>(null);
  let fullFor = $state<string | null>(null);
  let fullState = $state<'idle' | 'loading' | 'error'>('idle');
  let fullError = $state('');
  const loaded = $derived(fullFor === block.id);
  const result = $derived(effectiveResult(block, loaded ? full : null));
  const previewOnly = $derived(isElided(block) && !loaded);
  function loadFull(): void {
    const sid = ctx.sessionId;
    if (!sid || fullState === 'loading') return;
    const id = block.id;
    fullState = 'loading';
    fetchToolResult(sid, id).then(
      (r) => {
        if (block.id !== id) return;
        fullFor = id;
        full = r;
        fullState = 'idle';
      },
      (e) => {
        if (block.id !== id) return;
        fullError = loadErrorText(e);
        fullState = 'error';
      },
    );
  }
  $effect(() => {
    if (open && previewOnly && fullState === 'idle') loadFull();
  });
  const chrome = $derived(TOOL_CHROME[block.tool] ?? TOOL_CHROME.other);
  const line = $derived(toolLine(block));
  const cwd = $derived(ctx.cwd ?? null);
  /** The folder / scope, relative to where the agent ran. */
  const detail = $derived(line.detail && cwd && line.detail === cwd.replace(/\/$/, '') ? '' : relPath(line.detail, cwd));
  const status = $derived(toolStatus(block, live));
  const isShell = $derived(block.tool === 'shell');
  const STATUS_LABEL = { ok: 'Succeeded', err: 'Failed', running: 'Running…', none: 'No result recorded' } as const;
  const blocked = $derived(waiting && status === 'running');

  // Edit calls carry `structuredPatch` on the result; older records (and a
  // failed edit) do not — synthesize a −old/+new hunk from the input so the
  // change is still shown as a diff, never as two blobs of text.
  const diff = $derived.by(() => {
    if (result?.patch) return patchToDiff(result.patch, result.file_path);
    const synth = editInputPatch(block);
    return synth ? patchToDiff(synth, result?.file_path ?? null) : null;
  });
  const hasDiff = $derived(!!diff && diff.files.some((f) => f.hunks.length));
  const diffStats = $derived.by(() => {
    if (!diff) return null;
    let add = 0;
    let del = 0;
    for (const f of diff.files) {
      add += f.added ?? 0;
      del += f.deleted ?? 0;
    }
    return add || del ? { add, del } : null;
  });
  /** A fresh Write has no patch — its content (from the input) is the change. */
  const writtenContent = $derived.by(() => {
    if (block.tool !== 'write' || hasDiff) return '';
    const c = (block.input as { content?: unknown } | null)?.content;
    return typeof c === 'string' ? c : '';
  });

  const filePath = $derived(
    result?.file_path ??
      (block.tool === 'read' || block.tool === 'edit' || block.tool === 'write'
        ? (line.detail ? `${line.detail}/${line.target}` : line.target) || null
        : null),
  );
  const text = $derived(result?.text ?? '');
  // The collapsed row's second line: a command's last output line (a preview
  // cut from the head would lie about the tail, so not for elided results).
  const tail = $derived(isShell && !previewOnly && status !== 'running' ? lastLine(text) : '');
  const command = $derived.by(() => {
    if (!isShell || block.input == null || typeof block.input !== 'object') return '';
    const c = (block.input as { command?: unknown; cmd?: unknown }).command ?? (block.input as { cmd?: unknown }).cmd;
    return typeof c === 'string' ? c : Array.isArray(c) ? c.map(String).join(' ') : '';
  });

  // Syntax colors: hljs loads lazily (stays out of the main bundle). Command
  // output is never auto-detected — a log coloured as code reads as wrong.
  let hlReady = $state(false);
  $effect(() => {
    if (!open) return;
    void ensureHljs().then(() => (hlReady = true));
  });
  const lang = $derived.by(() => {
    if (!hlReady || isShell) return null;
    const byPath = filePath ? langFromPath(filePath) : null;
    if (byPath) return byPath;
    return autoLang(text);
  });
  const lines = $derived(text ? text.split('\n') : []);
  // Long outputs render through the windowed list (uniform mono rows); short
  // ones as a plain <pre> so selection/copy stays natural.
  const windowed = $derived(lines.length > 400);
  const inputJson = $derived.by(() => {
    if (block.input == null) return '';
    try {
      return typeof block.input === 'string' ? block.input : JSON.stringify(block.input, null, 2);
    } catch {
      return String(block.input);
    }
  });
  let showInput = $state(false);
  // Plain output (commands, unhighlighted text) gets its paths and URLs linked.
  const outHtml = $derived(open && !windowed && text ? (lang ? highlightBlock(text, lang) : linkifyOutput(highlightBlock(text, null))) : '');
  function onOutClick(e: MouseEvent): void {
    const a = e.target instanceof Element ? e.target.closest('a') : null;
    if (!a) return;
    e.preventDefault();
    if (a.classList.contains('file-ref') && a.dataset.path) {
      const line = a.dataset.anchor ? Number(a.dataset.anchor.split(':')[0]) : null;
      ctx?.openPreview?.({ kind: 'file', path: a.dataset.path, line });
    } else if (a.getAttribute('href')) {
      const href = a.getAttribute('href') ?? '';
      if (ctx.openUrl) ctx.openUrl(href, e.altKey);
      else void openExternal(href);
    }
  }
  const inputHtml = $derived(open && showInput && inputJson ? highlightBlock(inputJson, hlReady ? 'json' : null) : '');
  // A Read's result is `cat -n` text ("    80\tpub fn …"): shown as source with
  // the file's own line numbers, like an editor, instead of a numbered blob.
  const readView = $derived.by(() => {
    if (!open || block.tool !== 'read' || !text) return null;
    const m = /^\s*(\d+)\t/.exec(text);
    if (!m) return null;
    const body: string[] = [];
    for (const l of lines) {
      const hit = /^\s*\d+\t/.exec(l);
      if (!hit) {
        if (l.trim() === '') continue;
        return null;
      }
      body.push(l.slice(hit[0].length));
    }
    return { start: Number(m[1]), text: body.join('\n') };
  });

  // A command's output opens at its END — the failure summary / final status
  // is what you look for first.
  let bodyEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    if (!open || !isShell || !text) return;
    void tick().then(() => {
      const el = bodyEl?.querySelector<HTMLElement>('.out, .out-vlist');
      if (el) el.scrollTop = el.scrollHeight;
    });
  });

  const canFiles = $derived(!!ctx.sessionId && !ctx.readonly);
  function openInFiles(): void {
    if (filePath) openFile.open(filePath);
  }
  /** The side panel: a write's content as written, an edit's file with this diff. */
  function openPreview(): void {
    if (!filePath) return;
    ctx?.openPreview?.({ kind: 'file', path: filePath, content: writtenContent || null, diff: hasDiff ? diff : null });
  }
  function openDiff(): void {
    if (!diff) return;
    ctx?.openPreview?.({ kind: 'diff', title: filePath ? (filePath.split('/').pop() ?? filePath) : 'Diff', diff, focus: null });
  }
  async function copyCommand(): Promise<void> {
    try {
      await copyTextOrThrow(command);
      toasts.info('Copied', 'The command');
    } catch (e) {
      toastError('Couldn’t copy to the clipboard', e);
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {     "class":`step`,...__sveltets_2_empty({"data-tool":block.tool}),...__sveltets_2_empty({"data-status":status === 'running' ? 'pending' : status}),});open;
   { svelteHTML.createElement("button", {       "class":`step-row`,"onclick":() => (open = !open),"aria-expanded":open,"title":line.hint || block.title || block.name,});
     { svelteHTML.createElement("span", {   "class":`step-icon`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":chrome.icon,"size":13,}});} }
     { svelteHTML.createElement("span", { "class":`step-text`,});
       { svelteHTML.createElement("span", { "class":`step-main`,});
         { svelteHTML.createElement("span", { "class":`step-verb`,});blocked ? `Wants to ${line.base}` : status === 'running' ? line.present : line.verb; }
        if(line.target){ { svelteHTML.createElement("span", {  "class":`step-target`,});line.mono;line.target; }}
        if(detail){ { svelteHTML.createElement("span", { "class":`step-detail mono`,});detail; }}
       }
      if(tail){
         { svelteHTML.createElement("span", {  "class":`step-tail mono`,});status === 'err';tail; }
      } else if (status === 'none'){
         { svelteHTML.createElement("span", { "class":`step-tail`,});         }
      }
     }
    if(diffStats){
       { svelteHTML.createElement("span", {   "class":`step-stats mono`,"title":`Lines added / removed`,}); { svelteHTML.createElement("span", { "class":`add`,}); diffStats.add; }  { svelteHTML.createElement("span", { "class":`del`,}); diffStats.del; } }
    }
    if(result?.truncated || previewOnly){
       { svelteHTML.createElement("span", {   "class":`chip step-trunc`,"title":previewOnly ? 'Preview — expand to load the full output' : 'Output capped at 64 KB',});formatBytes(result?.bytes ?? 0); }
    }
     { svelteHTML.createElement("span", {       "class":`step-status ${status}`,"role":`img`,"aria-label":blocked ? 'Waiting for you' : STATUS_LABEL[status],"title":blocked ? 'Waiting for you' : STATUS_LABEL[status],});blocked;
      if(blocked){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
      } else if (status === 'running'){ { svelteHTML.createElement("span", {    "class":`spinner`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, " "); }
      } else if (status === 'ok'){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}
      } else if (status === 'err'){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
      }else{ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`minus`,"size":12,}});}}
     }
     { svelteHTML.createElement("span", {   "class":`step-caret`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});} }
   }
  if(open){
     { const $$_div1 = svelteHTML.createElement("div", {  "class":`step-body`,});bodyEl = $$_div1;
      if(isShell && command){
         { svelteHTML.createElement("div", { "class":`cmd`,});
           { svelteHTML.createElement("span", {   "class":`cmd-prompt mono`,"aria-hidden":`true`,});  }
           { svelteHTML.createElement("code", {   "class":`cmd-text mono`,"dir":`ltr`,});command; }
           { svelteHTML.createElement("button", {       "class":`icon-btn cmd-copy`,"onclick":() => void copyCommand(),"aria-label":`Copy command`,"title":`Copy command`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} }
         }
        if(line.hint){ { svelteHTML.createElement("div", { "class":`cmd-hint`,});line.hint; }}
      } else if (filePath){
         { svelteHTML.createElement("div", { "class":`file-line`,});
           { svelteHTML.createElement("button", {     "class":`file-chip mono`,"onclick":openPreview,"title":`Preview — ${filePath}`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});}  { svelteHTML.createElement("span", { "class":`file-chip-path`,});relPath(filePath, cwd); }
           }
          if(hasDiff){
             { svelteHTML.createElement("button", {     "class":`link-btn`,"onclick":openDiff,"title":`Open this change in the side panel`,});  }
          }
          if(canFiles){
             { svelteHTML.createElement("button", {       "class":`icon-btn file-open`,"onclick":openInFiles,"aria-label":`Open in Files`,"title":`Open in Files`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});} }
          }
         }
      }
      if(hasDiff && diff){
         { const $$_ffiDenilnI2C = __sveltets_2_ensureComponent(InlineDiff); new $$_ffiDenilnI2C({ target: __sveltets_2_any(), props: { diff,}});}
      } else if (writtenContent){
         { svelteHTML.createElement("div", { "class":`code-frame`,}); { const $$_weiVedoC3C = __sveltets_2_ensureComponent(CodeView); new $$_weiVedoC3C({ target: __sveltets_2_any(), props: {    "text":writtenContent,"lang":hlReady && filePath ? langFromPath(filePath) : null,}});} }
      } else if (result == null){
         { svelteHTML.createElement("div", { "class":`pending`,});status === 'running' ? 'Waiting for the result…' : 'No result was recorded for this call.'; }
      } else if (block.tool === 'web' || block.tool === 'ask'){
          { const $$_nwodkraM2C = __sveltets_2_ensureComponent(Markdown); new $$_nwodkraM2C({ target: __sveltets_2_any(), props: {  "md":text,"small":true,}});}
      } else if (readView && !windowed){
         { svelteHTML.createElement("div", { "class":`code-frame`,}); { const $$_weiVedoC3C = __sveltets_2_ensureComponent(CodeView); new $$_weiVedoC3C({ target: __sveltets_2_any(), props: {      "text":readView.text,"start":readView.start,"lang":lang,}});} }
      } else if (windowed){
         { const $$_tsiLlautriV2C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV2 = new $$_tsiLlautriV2C({ target: __sveltets_2_any(), props: {       "items":lines,"estimateHeight":18,"class":`out-vlist`,"findText":(l: string) => l,row:(l) => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("div", {   "class":`out-line mono hljs`,"dir":`ltr`,}); highlightLine(l || ' ', lang); }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV2.$$prop_def;/*Ωignore_endΩ*/
          
         VirtualList}
      } else if (text){
        
         { svelteHTML.createElement("pre", {         "class":`out mono hljs`,"dir":`ltr`,...__sveltets_2_empty({"data-lang":lang}),"onclick":onOutClick,});status === 'err';isShell; outHtml; }
      } else if (!result.image_ids.length && !previewOnly){
         { svelteHTML.createElement("div", { "class":`pending`,});  }
      }
      if(previewOnly){
        if(fullState === 'error'){
           { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`the full output`,"error":fullError,"empty":true,"onretry":loadFull,}});}
        } else if (fullState === 'loading' || !ctx.sessionId){
          
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(ctx.sessionId ? 'Loading the full output…' : '')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`trunc-note`,});ctx.sessionId ? 'Loading the full output…' : `Preview of ${formatBytes(result?.bytes ?? 0)}.`; }}
        }
      } else if (result?.truncated){
         { svelteHTML.createElement("div", { "class":`trunc-note`,});     formatBytes(result.bytes);  }
      }
      if(result?.image_ids.length){
         { svelteHTML.createElement("div", { "class":`imgs`,});
          
              for(let id of __sveltets_2_ensureArray(result.image_ids)){let i = 1;i;
              { const $$_kcolBegamI3C = __sveltets_2_ensureComponent(ImageBlock); new $$_kcolBegamI3C({ target: __sveltets_2_any(), props: {   id,"alt":`Tool result image`,"small":true,}});}
          }
         }
      }
      if(inputJson && !isShell){
         { svelteHTML.createElement("button", {     "class":`link-btn`,"onclick":() => (showInput = !showInput),"aria-expanded":showInput,});showInput ? 'Hide raw input' : 'Show raw input'; }
        if(showInput){
           { svelteHTML.createElement("pre", {   "class":`out mono hljs`,"dir":`ltr`,}); inputHtml; }
        }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ToolStep__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ToolStep__SvelteComponent_ = ReturnType<typeof ToolStep__SvelteComponent_>;
/*Ωignore_endΩ*/export default ToolStep__SvelteComponent_;