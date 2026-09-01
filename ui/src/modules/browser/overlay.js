// @ts-nocheck — not a real app module: raw source text `eval`'d into a
// third-party page's own JS context (see the header below), so it's exempt
// from the app's `checkJs` (this file's `window`/`document` are the TARGET
// page's globals, not this app's — `lib.dom` typings don't apply usefully
// here, and dynamic property bags like `markBoxes[selector]` are load-bearing).
//
// Otto browser-overlay: injected into a live (native webview) tab via
// `browser_eval` to power the element picker. Plain JS, self-contained IIFE —
// shipped as raw source text (see `overlaySrc` in BrowserView.svelte, a
// `?raw` import of this file) and `eval`'d straight into the target page's
// own JS context, which has no way to `import` an ES module from Otto's own
// bundle. Duplicates `buildSelector()` from `./selector.ts` by hand for the
// same reason — a live-tab webview is denied Tauri IPC (see Task 9's report),
// so it can't call back into the app to reuse shared logic either. Keep the
// two in sync if the selector algorithm changes.
//
// Protocol: BrowserView polls `window.__ottoOverlay.tick(highlightJson)` via
// `browser_eval` on an interval while a live tab is active. `tick` applies
// the given highlight list (the existing marks for this URL) to the page,
// then drains and returns the queue of marks made (by clicking, in pick
// mode) since the last poll. There is no push channel — `postMessage` from
// inside a denied-IPC child webview never reaches the host either — so
// pull/poll over `browser_eval` is the only way marks travel back to the app.
(function () {
  if (window.__ottoOverlay) return; // idempotent — this file is re-injected on every nav

  var DATA_ATTRS = ['data-testid', 'data-test', 'data-id', 'data-qa'];
  // Same cap as ui/src/modules/browser/selector.ts's MAX_SELECTOR_LEN. A
  // hostile page's own id/data-* attribute VALUES are the real trust
  // boundary here (unlike reader mode's always-structural selector), and the
  // server (routes/browser.rs's SELECTOR_MAX_CHARS + build_context_block's
  // fencing) is the actual enforcement point — this cap is defense-in-depth,
  // skipping an over-length candidate in favor of the next priority tier
  // rather than truncating it (a truncated attribute-selector string risks
  // being syntactically broken).
  var MAX_SELECTOR_LEN = 300;
  var picking = false;
  var queue = [];
  var hoverBox = null;
  var markBoxes = {}; // selector -> box element

  // Escapes a value for a `[attr="…"]` selector STRING — not identifier
  // position, so CSS.escape() (used below for #id) isn't reused here; it
  // over-escapes a quoted-string value. Backslash-escaping `"`/`\` is correct
  // and sufficient for a double-quoted CSS string.
  function escapeAttrValue(v) {
    return String(v).replace(/(["\\])/g, '\\$1');
  }

  // #id-shorthand for `id`, preferring the platform's own CSS.escape()
  // (identifier-position escaping — handles a leading digit, a colon, etc.,
  // always producing a valid bare selector) with a manual fallback for an
  // older/embedded WebKit that lacks it.
  function idSelector(id) {
    if (typeof CSS !== 'undefined' && typeof CSS.escape === 'function') {
      return '#' + CSS.escape(id);
    }
    return /^[A-Za-z_][A-Za-z0-9_-]*$/.test(id) ? '#' + id : '[id="' + escapeAttrValue(id) + '"]';
  }

  // Same priority as ui/src/modules/browser/selector.ts's buildSelector:
  // #id > data-* test attribute (on el only) > nth-of-type tag-path from
  // document.body (exclusive) down to el. A candidate over MAX_SELECTOR_LEN
  // is skipped in favor of the next tier; the nth-of-type fallback (never
  // attacker text, only tag names/counts) is hard-truncated as a last resort.
  function buildSelector(el) {
    var id = el.getAttribute('id');
    if (id) {
      var idSel = idSelector(id);
      if (idSel.length <= MAX_SELECTOR_LEN) return idSel;
    }
    for (var i = 0; i < DATA_ATTRS.length; i++) {
      var v = el.getAttribute(DATA_ATTRS[i]);
      if (v) {
        var dataSel = '[' + DATA_ATTRS[i] + '="' + escapeAttrValue(v) + '"]';
        if (dataSel.length <= MAX_SELECTOR_LEN) return dataSel;
      }
    }
    var steps = [];
    var cur = el;
    while (cur && cur !== document.body && cur.parentElement) {
      var tag = cur.tagName.toLowerCase();
      var parent = cur.parentElement;
      var currentTag = cur.tagName;
      var siblings = [];
      for (var j = 0; j < parent.children.length; j++) {
        if (parent.children[j].tagName === currentTag) siblings.push(parent.children[j]);
      }
      var idx = siblings.indexOf(cur) + 1;
      steps.unshift(siblings.length > 1 ? tag + ':nth-of-type(' + idx + ')' : tag);
      cur = parent === document.body ? null : parent;
    }
    if (!steps.length) steps.push(el.tagName.toLowerCase());
    var path = steps.join(' > ');
    return path.length > MAX_SELECTOR_LEN ? path.slice(0, MAX_SELECTOR_LEN) : path;
  }

  function ensureStyle() {
    if (document.getElementById('__otto_overlay_style__')) return;
    var style = document.createElement('style');
    style.id = '__otto_overlay_style__';
    style.textContent =
      '.__otto_box__{position:fixed;pointer-events:none;z-index:2147483647;' +
      'box-sizing:border-box;border-radius:2px;}' +
      '.__otto_hover__{border:1px dashed #6a5acd;background:rgba(106,90,205,.08);}' +
      '.__otto_mark__{border:1px solid #e0b400;background:rgba(224,180,0,.14);}';
    document.documentElement.appendChild(style);
  }

  function positionBox(box, rect) {
    box.style.left = rect.left + 'px';
    box.style.top = rect.top + 'px';
    box.style.width = rect.width + 'px';
    box.style.height = rect.height + 'px';
  }

  function makeBox(rect, cls) {
    ensureStyle();
    var box = document.createElement('div');
    box.className = '__otto_box__ ' + cls;
    positionBox(box, rect);
    document.documentElement.appendChild(box);
    return box;
  }

  function clearHover() {
    if (hoverBox) {
      hoverBox.remove();
      hoverBox = null;
    }
  }

  function isOverlayEl(el) {
    return !!el && typeof el.className === 'string' && el.className.indexOf('__otto_box__') !== -1;
  }

  function onMouseOver(e) {
    if (!picking) return;
    var target = e.target;
    if (!target || target === document.body || target === document.documentElement || isOverlayEl(target)) {
      clearHover();
      return;
    }
    var rect = target.getBoundingClientRect();
    if (!hoverBox) hoverBox = makeBox(rect, '__otto_hover__');
    else positionBox(hoverBox, rect);
  }

  function onClick(e) {
    if (!picking) return;
    var target = e.target;
    if (!target || target === document.body || target === document.documentElement || isOverlayEl(target)) return;
    e.preventDefault();
    e.stopPropagation();
    var selector = buildSelector(target);
    var rect = target.getBoundingClientRect();
    queue.push({
      selector: selector,
      outerHtml: (target.outerHTML || '').slice(0, 2000),
      text: (target.textContent || '').trim().slice(0, 500),
      rect: { x: rect.left, y: rect.top, width: rect.width, height: rect.height },
    });
    // Instant feedback — a persistent box right away, keyed by selector. The
    // next highlight tick (once the app round-trips the created annotation)
    // just finds it already drawn and leaves it alone.
    if (!markBoxes[selector]) markBoxes[selector] = makeBox(rect, '__otto_mark__');
  }

  document.addEventListener('mouseover', onMouseOver, true);
  document.addEventListener('click', onClick, true);

  function applyHighlights(marks) {
    var seen = {};
    for (var i = 0; i < marks.length; i++) {
      var selector = marks[i].selector;
      seen[selector] = true;
      if (markBoxes[selector]) continue; // already drawn
      var el;
      try {
        el = document.querySelector(selector);
      } catch (err) {
        el = null; // an invalid/stale selector is skipped, not thrown
      }
      if (el) markBoxes[selector] = makeBox(el.getBoundingClientRect(), '__otto_mark__');
    }
    for (var sel in markBoxes) {
      if (!seen[sel]) {
        markBoxes[sel].remove();
        delete markBoxes[sel];
      }
    }
  }

  function repositionAll() {
    for (var sel in markBoxes) {
      var el;
      try {
        el = document.querySelector(sel);
      } catch (err) {
        el = null;
      }
      if (el) positionBox(markBoxes[sel], el.getBoundingClientRect());
    }
  }
  window.addEventListener('scroll', repositionAll, true);
  window.addEventListener('resize', repositionAll, true);

  window.__ottoOverlay = {
    setPicking: function (v) {
      picking = !!v;
      if (!picking) clearHover();
    },
    // Autofill key-icon detection: does the CURRENT page have a password
    // field right now? The app polls this (see BrowserView.svelte's
    // pollLoginForm) rather than trusting the tab's URL alone — a domain the
    // user has a credential for may have navigated away from its login page
    // to some other route with no form at all, and the key icon should
    // track the actual page, not just the host.
    hasLoginForm: function () {
      return !!document.querySelector('input[type="password"]');
    },
    // Applies `highlightJson` (a JSON string of [{selector,color}, …] — the
    // existing marks for this URL) to the page, then drains and returns the
    // queue of marks captured since the previous tick.
    tick: function (highlightJson) {
      var marks = [];
      try {
        marks = JSON.parse(highlightJson || '[]');
      } catch (err) {
        marks = [];
      }
      applyHighlights(marks);
      var drained = queue;
      queue = [];
      return drained;
    },
  };
})();
