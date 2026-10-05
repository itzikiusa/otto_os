// Team Performance report — client layer. The body is rendered server-side
// (lib/reportmodel.js) so the report reads fine with JS off / in print; this
// script only adds the theme switch and the comment layer. Comments go to the
// configurable endpoint in the embedded model (meta.comments_endpoint, e.g.
// "/report/comments?id=…"); with no endpoint (or an unreachable one) the layer
// degrades to read-only and Export still saves whatever was loaded.
(function () {
  'use strict';
  var R = window.__TP_REPORT__ || {};
  var meta = R.meta || {};
  var $ = function (s) { return document.querySelector(s); };
  var esc = function (s) { return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]; }); };

  // ---------------------------------------------------------------- theme
  var THEMES = ['auto', 'light', 'dark'];
  var themeKey = 'tp-report-theme';
  var theme = 'auto';
  try { theme = localStorage.getItem(themeKey) || 'auto'; } catch (e) { /* storage blocked */ }
  function applyTheme() {
    if (theme === 'auto') document.documentElement.removeAttribute('data-theme');
    else document.documentElement.setAttribute('data-theme', theme);
    var b = $('#theme-btn'); if (b) b.textContent = 'Theme: ' + theme;
  }
  applyTheme();
  var tb = $('#theme-btn');
  if (tb) tb.addEventListener('click', function () {
    theme = THEMES[(THEMES.indexOf(theme) + 1) % THEMES.length];
    try { localStorage.setItem(themeKey, theme); } catch (e) { /* ignore */ }
    applyTheme();
  });

  // ------------------------------------------------------------- comments
  var endpoint = typeof meta.comments_endpoint === 'string' && meta.comments_endpoint ? meta.comments_endpoint : null;
  var writable = false;
  var comments = [];
  var target = null; // { anchor, label }
  var panel = $('#cpanel');

  function setStatus(msg, err) { var s = $('#c-status'); if (!s) return; s.textContent = msg; s.className = 'c-status' + (err ? ' err' : ''); }
  function normalize(body) {
    var list = Array.isArray(body) ? body : body && Array.isArray(body.comments) ? body.comments : [];
    return list.filter(function (c) { return c && typeof c.text === 'string' && c.text.trim(); })
      .map(function (c) { return { anchor: String(c.anchor || 'report'), label: c.label ? String(c.label) : '', text: String(c.text), author: c.author ? String(c.author) : '', at: c.at ? String(c.at) : '' }; });
  }
  function labelFor(anchor) {
    var el = document.querySelector('[data-anchor="' + (window.CSS && CSS.escape ? CSS.escape(anchor) : anchor) + '"]');
    return (el && el.getAttribute('data-anchor-label')) || anchor;
  }
  function render() {
    var count = $('#c-count'); if (count) count.textContent = String(comments.length);
    var byAnchor = {};
    comments.forEach(function (c) { byAnchor[c.anchor] = (byAnchor[c.anchor] || 0) + 1; });
    document.querySelectorAll('.c-btn[data-for]').forEach(function (b) {
      var n = byAnchor[b.getAttribute('data-for')] || 0;
      b.classList.toggle('has', n > 0);
      b.textContent = n ? 'Comments (' + n + ')' : 'Comment';
    });
    var list = $('#c-list'); if (!list) return;
    var items = comments.slice().sort(function (a, b) { return a.at < b.at ? 1 : a.at > b.at ? -1 : 0; });
    if (target) items = items.filter(function (c) { return c.anchor === target.anchor; }).concat(items.filter(function (c) { return c.anchor !== target.anchor; }));
    list.innerHTML = items.length
      ? items.map(function (c) {
        return '<div class="c-item"><div class="where">' + esc(c.label || labelFor(c.anchor)) + (c.author ? ' · ' + esc(c.author) : '') + (c.at ? ' · ' + esc(c.at.slice(0, 16).replace('T', ' ')) : '') +
          '</div><div class="txt">' + esc(c.text) + '</div><a class="small" href="#' + esc(c.anchor) + '">Go to section</a></div>';
      }).join('')
      : '<p class="muted small">No comments yet.' + (writable ? ' Use the Comment button on any section.' : '') + '</p>';
  }
  function openPanel(t) {
    target = t || null;
    if (!panel) return;
    panel.classList.add('open'); panel.setAttribute('aria-hidden', 'false');
    var form = $('#c-form');
    if (form) form.hidden = !writable;
    var tl = $('#c-target'); if (tl) tl.textContent = target ? target.label : 'this report';
    render();
    var ta = $('#c-text'); if (writable && ta) ta.focus();
  }
  function closePanel() { if (!panel) return; panel.classList.remove('open'); panel.setAttribute('aria-hidden', 'true'); }

  document.querySelectorAll('[data-anchor]').forEach(function (el) {
    var anchor = el.getAttribute('data-anchor');
    var label = el.getAttribute('data-anchor-label') || anchor;
    var host = el.querySelector(':scope > header') || el;
    var b = document.createElement('button');
    b.type = 'button'; b.className = 'c-btn'; b.setAttribute('data-for', anchor);
    b.setAttribute('aria-label', 'Comment on ' + label); b.title = 'Comment on ' + label; b.textContent = 'Comment';
    b.addEventListener('click', function () { openPanel({ anchor: anchor, label: label }); });
    host.appendChild(b);
  });
  var oc = $('#open-comments'); if (oc) oc.addEventListener('click', function () { openPanel(null); });
  var cc = $('#c-close'); if (cc) cc.addEventListener('click', closePanel);
  document.addEventListener('keydown', function (e) { if (e.key === 'Escape') closePanel(); });

  var ex = $('#c-export');
  if (ex) ex.addEventListener('click', function () {
    var blob = new Blob([JSON.stringify({ report: meta.report_id || null, exported_at: new Date().toISOString(), comments: comments }, null, 2)], { type: 'application/json' });
    var a = document.createElement('a');
    a.href = URL.createObjectURL(blob); a.download = (meta.report_id || 'report') + '-comments.json';
    document.body.appendChild(a); a.click(); a.remove();
    setTimeout(function () { URL.revokeObjectURL(a.href); }, 1000);
  });

  var form = $('#c-form');
  if (form) form.addEventListener('submit', function (e) {
    e.preventDefault();
    var ta = $('#c-text'); var text = ta ? ta.value.trim() : '';
    if (!writable || !text) return;
    var body = { anchor: target ? target.anchor : 'report', label: target ? target.label : 'Whole report', text: text };
    var btn = $('#c-save'); if (btn) btn.disabled = true;
    setStatus('Posting…');
    fetch(endpoint, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body), credentials: 'same-origin' })
      .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.json().catch(function () { return null; }); })
      .then(function (saved) {
        var list = normalize(saved);
        if (list.length > 1) comments = list; else comments.push(list[0] || Object.assign({ at: new Date().toISOString() }, body));
        if (ta) ta.value = '';
        setStatus('Comment posted.'); render();
      })
      .catch(function (err) { setStatus('Could not post the comment (' + err.message + '). Your text is still in the box — try again.', true); })
      .then(function () { if (btn) btn.disabled = false; });
  });

  render();
  if (!endpoint || location.protocol === 'file:') {
    setStatus(endpoint ? 'Read-only: opened from a file, comments cannot be posted.' : 'Read-only: no comment endpoint configured.');
    return;
  }
  setStatus('Loading comments…');
  fetch(endpoint, { credentials: 'same-origin' })
    .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.json(); })
    .then(function (body) { comments = normalize(body); writable = true; setStatus('Comments are shared with everyone who can open this report.'); render(); var f = $('#c-form'); if (f && panel && panel.classList.contains('open')) f.hidden = false; })
    .catch(function (err) { setStatus('Read-only: comments could not be loaded (' + err.message + ').', true); });
})();
