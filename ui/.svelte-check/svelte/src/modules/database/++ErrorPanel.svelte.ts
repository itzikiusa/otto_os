///<reference types="svelte" />
;// Structured query-error panel. `normalizeDbError` (error-normalize.ts) turns
// the engine's error into a one-line headline (what failed), the likely cause
// and a fix hint, clickable "did you mean" chips, and a statement position
// for the caret excerpt. The full raw error stays one click away under "Show
// full error" (with Copy), and "Ask AI to fix" — which opens the DB Assistant
// seeded with the statement + the RAW error — is unchanged.

import Icon from '../../lib/components/Icon.svelte';
import { database } from '../../lib/stores/database.svelte';
import { toasts } from '../../lib/toast.svelte';
import { copyText } from '../../lib/clipboard';
import { applySuggestion, normalizeDbError } from '../../lib/database/error-normalize';
;

  interface Props {
    error: string;
    /** Active engine (mysql/postgres/clickhouse/mongodb/redis), or null outside
     *  the explorer (the Athena view). Picks the normaliser's rule table, and —
     *  being set only for an explorer tab — gates the chips that edit the
     *  explorer's statement. */
    engine: string | null;
    statement: string;
    /** Omitted where there is no DB Assistant to open (e.g. the Athena view). */
    onAskAi?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { error, engine, statement, onAskAi }: Props = $props();

  const n = $derived(normalizeDbError(engine, error, statement));

  // Policy refusals from the daemon read `forbidden: <code>: <why>`. They are
  // not query mistakes: say so, show the code as the chip, and don't offer an
  // AI "fix" for something only an administrator can change.
  const policy = $derived.by(() => {
    const m = error.match(/^\s*forbidden:\s*(?:([a-z0-9_]+):\s*)?([\s\S]*)$/i);
    if (!m) return null;
    const why = m[2].trim();
    return { code: m[1] ?? null, message: why ? why.charAt(0).toUpperCase() + why.slice(1) : error };
  });

  // The offending statement line + a caret under the reported column (or the
  // line's first non-space char when only a line is known). A long line is
  // windowed around the column so the caret stays on screen.
  const WINDOW = 60;
  const excerpt = $derived.by(() => {
    const pos = n.position;
    if (!pos || !statement.trim()) return null;
    const lines = statement.split('\n');
    if (pos.line < 1 || pos.line > lines.length) return null;
    const line = lines[pos.line - 1].replace(/\t/g, ' ');
    const chars = Array.from(line);
    let caretAt = pos.col ? Math.min(pos.col - 1, chars.length) : chars.length - line.trimStart().length;
    let shown = line;
    if (chars.length > WINDOW * 2) {
      const start = Math.max(0, caretAt - WINDOW);
      const end = Math.min(chars.length, caretAt + WINDOW);
      shown = `${start > 0 ? '…' : ''}${chars.slice(start, end).join('')}${end < chars.length ? '…' : ''}`;
      caretAt = caretAt - start + (start > 0 ? 1 : 0);
    }
    return {
      label: pos.col ? `line ${pos.line}, col ${pos.col}` : `line ${pos.line}`,
      text: `${shown}\n${' '.repeat(Math.max(0, caretAt))}^`,
    };
  });

  const label = $derived(engine ? engine.toUpperCase() : 'SQL');
  // The chips edit the explorer's live statement — only inside the explorer.
  const canApply = $derived(!!engine && !!database.tab);

  function applyFix(suggestion: string) {
    const tab = database.tab;
    if (!tab || !n.token) return;
    const next = applySuggestion(tab.statement, n.token, suggestion);
    if (next === null) {
      toasts.info('Nothing to replace', `\`${n.token}\` isn’t in the editor any more.`);
      return;
    }
    // One statement change — the editor applies it as an undoable edit (⌘Z).
    database.setStatement(next);
    toasts.success(`Replaced \`${n.token}\` with \`${suggestion}\``, 'Run the query again to check. ⌘Z undoes it.');
  }

  async function copyRaw() {
    if (await copyText(error)) toasts.success('Error copied');
    else toasts.error('Couldn’t copy', 'Select the text and copy it manually.');
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`err-panel`,});
   { svelteHTML.createElement("div", {   "class":`err-head`,"role":`alert`,});
     { svelteHTML.createElement("span", { "class":`err-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});} }
     { svelteHTML.createElement("span", { "class":`err-title`,});policy ? 'Not allowed on this connection' : n.title; }
    if(policy?.code){ { svelteHTML.createElement("span", { "class":`err-code mono`,});policy.code; }} else if (!policy && n.code){ { svelteHTML.createElement("span", { "class":`err-code mono`,});n.code; }}
     { svelteHTML.createElement("span", { "class":`err-engine mono`,});label; }
     { svelteHTML.createElement("span", { "class":`err-grow`,}); }
    if(onAskAi && !policy){
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":onAskAi,"title":`Open the DB Assistant to investigate and fix this error`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}    
       }
    }
   }
  if(policy){
     { svelteHTML.createElement("p", { "class":`err-policy`,});policy.message; }
     { svelteHTML.createElement("p", { "class":`err-hint`,});                   }
  }else{
    if(n.cause){ { svelteHTML.createElement("p", { "class":`err-cause`,});n.cause; }}
    if(n.hint){
       { svelteHTML.createElement("p", { "class":`err-hint`,}); { svelteHTML.createElement("span", { "class":`err-hint-icon`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`bulb`,"size":12,}});} }n.hint; }
    }
    if(n.suggestions.length > 0){
       { svelteHTML.createElement("div", { "class":`err-suggest`,});
         { svelteHTML.createElement("span", { "class":`err-suggest-label`,});   }
           for(let s of __sveltets_2_ensureArray(n.suggestions)){s;
          if(canApply){
             { svelteHTML.createElement("button", {         "class":`err-chip mono`,"onclick":() => applyFix(s),"title":`Replace \`${n.token}\` with \`${s}\` in the editor`,"aria-label":`Replace ${n.token} with ${s} in the editor`,});s; }
          }else{
             { svelteHTML.createElement("span", { "class":`err-chip static mono`,});s; }
          }
        }
       }
    }
    if(excerpt){
       { svelteHTML.createElement("div", { "class":`err-excerpt`,});
         { svelteHTML.createElement("div", { "class":`err-excerpt-label mono`,});excerpt.label; }
         { svelteHTML.createElement("pre", { "class":`err-excerpt-code mono`,});excerpt.text; }
       }
    }
     { svelteHTML.createElement("details", { "class":`err-raw`,});
       { svelteHTML.createElement("summary", {});   }
       { svelteHTML.createElement("div", { "class":`err-raw-body`,});
         { svelteHTML.createElement("pre", { "class":`err-msg mono`,});error; }
         { svelteHTML.createElement("button", {       "class":`btn small err-copy`,"onclick":copyRaw,"title":`Copy the full error text`,"aria-label":`Copy the full error text`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} 
         }
       }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ErrorPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ErrorPanel__SvelteComponent_ = ReturnType<typeof ErrorPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default ErrorPanel__SvelteComponent_;