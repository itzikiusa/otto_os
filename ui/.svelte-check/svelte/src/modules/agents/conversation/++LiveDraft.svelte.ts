///<reference types="svelte" />
;;
  // The agent's in-progress response, read off the terminal screen by the
  // live tail (`transcript_live`, ≤ 1 frame / 700 ms). The provider writes a
  // transcript record only when a block COMPLETES, so this is the only way to
  // see text arrive before the folded turn lands. Shown while the session is
  // working; hidden as soon as the last folded turn already contains the
  // draft's tail (the screen keeps showing finished text).
  interface Props {
    text: string;
    /** Markdown of the last folded assistant text (dedupe against it). */
    lastText: string;
  };function $$render() {


  let { text, lastText }: Props = $props();

  /** Strip a leading part already covered by the folded turn: find the last
   *  ~48 chars of the folded text inside the draft and keep what follows. */
  const visible = $derived.by(() => {
    const draft = text.replace(/\r/g, '');
    if (!draft.trim()) return '';
    const probe = lastText.replace(/\s+/g, ' ').trim().slice(-48);
    if (probe.length >= 16) {
      const flat = draft.replace(/\s+/g, ' ');
      const at = flat.lastIndexOf(probe);
      if (at >= 0) {
        // Map the flattened index back approximately: drop the same share of
        // characters from the raw draft (whitespace runs are the only delta).
        const keepFlat = flat.slice(at + probe.length).trim();
        if (!keepFlat) return '';
        const idx = draft.lastIndexOf(keepFlat.slice(0, 24));
        return idx >= 0 ? draft.slice(idx).trimEnd() : keepFlat;
      }
    }
    return draft.trimEnd();
  });
  // Claude Code prefixes each streamed block with a "⏺" bullet — terminal
  // chrome, not content.
  const shown = $derived(visible.replace(/^\s*⏺\s?/gm, ''));
;
async () => {


if(visible){
   { svelteHTML.createElement("div", {  "class":`live-draft`,...__sveltets_2_empty({"data-live-draft":true}),});
     { svelteHTML.createElement("p", {   "class":`draft`,"dir":`auto`,});shown; { svelteHTML.createElement("span", {   "class":`caret`,"aria-hidden":`true`,}); } }
     { svelteHTML.createElement("div", { "class":`draft-note`,});     }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LiveDraft__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LiveDraft__SvelteComponent_ = ReturnType<typeof LiveDraft__SvelteComponent_>;
/*Ωignore_endΩ*/export default LiveDraft__SvelteComponent_;