///<reference types="svelte" />
;;
  // TermKeysBar — phone-only accessory bar shown above the soft keyboard.
  // Provides keys that a mobile soft keyboard typically cannot produce.
  //
  // Every button calls `sendSeq(bytes)` which is passed in by Terminal.svelte —
  // that keeps the send path identical to what the physical keyboard uses
  // (sendJson({ type:'input', data: textToBase64(…) }), same readOnly guard,
  // same WS socket, respects can_input scoping from the server).
  //
  // The bar is only rendered when `viewport.isPhone`; Terminal.svelte gates the
  // mount so the desktop sees nothing.

  interface Props {
    /** Write a string of bytes to the PTY (same path Terminal.svelte's onData uses). */
    sendSeq: (seq: string) => void;
    /** Mirror Terminal.svelte's readOnly prop so buttons grey out for viewer shares. */
    readOnly?: boolean;
  };;

  type KeyDef =
    | { label: string; seq: string; kind?: 'normal' | 'arrow' | 'sym' }
    | { label: string; action: () => void; kind?: 'normal' | 'ctrl'; active?: boolean };;function $$render() {



  let { sendSeq, readOnly = false }: Props = $props();

  // Ctrl sticky-modifier state: when true, the next letter key press sends ctrl+letter.
  let ctrlActive = $state(false);

  function send(seq: string): void {
    if (readOnly) return;
    sendSeq(seq);
  }

  function toggleCtrl(): void {
    if (readOnly) return;
    ctrlActive = !ctrlActive;
  }

  // Keys that, when ctrl is active, produce their control-character equivalent.
  function sendCtrl(letter: string): void {
    if (readOnly) return;
    ctrlActive = false;
    // Ctrl+A–Z → \x01–\x1a
    const code = letter.toUpperCase().charCodeAt(0) - 64;
    sendSeq(String.fromCharCode(code));
  }

  // Shift+Enter: sends ESC+CR (\x1b\r) — the same sequence the custom key handler
  // in Terminal.svelte emits for a physical Shift+Enter (see Terminal.svelte ~line 404-423).
  function sendShiftEnter(): void {
    send('\x1b\r');
  }


  // Row 1: Esc, Tab, Ctrl (sticky), Ctrl-C, Shift+Enter
  const row1: KeyDef[] = [
    { label: 'Esc',    seq: '\x1b',   kind: 'normal' },
    { label: 'Tab',    seq: '\x09',   kind: 'normal' },
    { label: 'Ctrl',   action: toggleCtrl, kind: 'ctrl', get active() { return ctrlActive; } },
    { label: 'Ctrl-C', seq: '\x03',   kind: 'normal' },
    { label: '⇧↵',   action: sendShiftEnter, kind: 'normal' },
  ];

  // Row 2: arrow keys + common symbols
  const row2: KeyDef[] = [
    { label: '↑', seq: '\x1b[A', kind: 'arrow' },
    { label: '↓', seq: '\x1b[B', kind: 'arrow' },
    { label: '←', seq: '\x1b[D', kind: 'arrow' },
    { label: '→', seq: '\x1b[C', kind: 'arrow' },
    { label: '|',   seq: '|',    kind: 'sym' },
    { label: '/',   seq: '/',    kind: 'sym' },
    { label: '~',   seq: '~',    kind: 'sym' },
  ];

  function handleKey(k: KeyDef): void {
    if ('action' in k) {
      k.action();
    } else if ('seq' in k) {
      send(k.seq);
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {     "class":`keys-bar`,"role":`toolbar`,"aria-label":`Terminal key shortcuts`,});
   { svelteHTML.createElement("div", { "class":`keys-row`,});
       for(let k of __sveltets_2_ensureArray(row1)){k.label;
       { svelteHTML.createElement("button", {             "class":`key-btn ${k.kind ?? 'normal'}`,"onclick":() => handleKey(k),"disabled":readOnly,"aria-label":k.label,"aria-pressed":'active' in k ? (k.active ?? false) : undefined,});'active' in k && k.active;readOnly;k.label; }
    }
   }
   { svelteHTML.createElement("div", { "class":`keys-row`,});
       for(let k of __sveltets_2_ensureArray(row2)){k.label;
       { svelteHTML.createElement("button", {          "class":`key-btn ${k.kind ?? 'normal'}`,"onclick":() => handleKey(k),"disabled":readOnly,"aria-label":k.label,});readOnly;k.label; }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TermKeysBar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TermKeysBar__SvelteComponent_ = ReturnType<typeof TermKeysBar__SvelteComponent_>;
/*Ωignore_endΩ*/export default TermKeysBar__SvelteComponent_;