///<reference types="svelte" />
;;
  // HTML / SVG document preview in a fully sandboxed frame: an EMPTY `sandbox`
  // attribute means no scripts, no forms, no same-origin access, no top-level
  // navigation — the page is rendered, never executed. A strict CSP meta is
  // prepended as a second fence (no network fetches beyond inline/data images).
  interface Props {
    content: string;
    name: string;
    /** `svg` wraps the markup in a minimal centred page. */
    svg?: boolean;
  };function $$render() {


  let { content, name, svg = false }: Props = $props();

  const CSP =
    "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data: blob: https:; style-src 'unsafe-inline' https:; font-src data: https:\">";

  const doc = $derived(
    svg
      ? `<!doctype html><html><head>${CSP}<meta name="color-scheme" content="light"><style>html,body{margin:0;height:100%;background:Canvas}body{display:flex;align-items:center;justify-content:center}svg{max-width:100%;max-height:100%}</style></head><body>${content}</body></html>`
      : `${CSP}${content}`,
  );
;
async () => {

  { svelteHTML.createElement("iframe", {         "class":`frame`,"sandbox":"","referrerpolicy":`no-referrer`,"title":`Preview of ${name}`,"srcdoc":doc,}); }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const HtmlPreview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type HtmlPreview__SvelteComponent_ = ReturnType<typeof HtmlPreview__SvelteComponent_>;
/*Ωignore_endΩ*/export default HtmlPreview__SvelteComponent_;