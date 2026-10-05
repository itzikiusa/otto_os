// Team Performance — shared UI primitives (zero-dep, classic script).
// Everything hangs off window.TP so views/*.js can use it without a bundler.
// Rules (docs/design/guidelines): tokens only, real buttons, every state
// designed (loading skeleton / empty checklist / inline error + Retry), no
// native alert/confirm/prompt — the in-page modal confirmer replaces them.
'use strict';
(function () {
  const TP = (window.TP = window.TP || {});
  TP.views = TP.views || {};
  TP.state = TP.state || { apiBase: '', token: '', hpd: 8, jiraBase: '' };

  // ---- formatting ---------------------------------------------------------
  const esc = (s) =>
    String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
  const isNum = (v) => typeof v === 'number' && Number.isFinite(v);
  const round = (v, d = 1) => (isNum(v) ? Math.round(v * 10 ** d) / 10 ** d : null);
  const fmtD = (v) => {
    if (!isNum(v)) return '—';
    if (v > 0 && v < 1) return round(v * TP.state.hpd, 1) + 'h';
    return round(v, 1) + 'd';
  };
  const fmtPct = (v) => (isNum(v) ? Math.round(v * 100) + '%' : '—');
  const fmtNum = (v, d = 1) => (isNum(v) ? String(round(v, d)) : '—');
  const fmtX = (v) => (isNum(v) ? '×' + v.toFixed(2) : '—');
  const fmtDate = (ms) => (ms ? new Date(ms).toISOString().slice(0, 10) : '—');
  const fmtAgo = (ms) => {
    if (!ms) return 'never';
    const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (s < 60) return `${s}s ago`;
    if (s < 3600) return `${Math.round(s / 60)}m ago`;
    if (s < 86400) return `${Math.round(s / 3600)}h ago`;
    return `${Math.round(s / 86400)}d ago`;
  };
  const fmtSecs = (ms) => {
    const s = Math.max(0, Math.ceil(ms / 1000));
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
  };

  /** Normalise a metric that may be a bare number or {value, unit, n, quality, band, series, …}. */
  function M(x) {
    if (x == null) return null;
    if (isNum(x)) return { value: x };
    if (typeof x === 'object') {
      const value = isNum(x.value) ? x.value : isNum(x.p50) ? x.p50 : isNum(x.median) ? x.median : isNum(x.rate) ? x.rate : null;
      return { ...x, value };
    }
    return null;
  }

  // ---- api ----------------------------------------------------------------
  async function api(path, opts = {}) {
    const r = await fetch(TP.state.apiBase + path, {
      ...opts,
      headers: { Authorization: 'Bearer ' + TP.state.token, 'Content-Type': 'application/json', ...(opts.headers || {}) },
    });
    if (!r.ok) {
      let msg = '';
      try {
        msg = (await r.json()).error || '';
      } catch {
        /* not json */
      }
      const e = new Error(msg || `HTTP ${r.status}`);
      e.status = r.status;
      throw e;
    }
    return r.json();
  }
  const put = (path, body) => api(path, { method: 'PUT', body: JSON.stringify(body) });
  const post = (path, body) => api(path, { method: 'POST', body: JSON.stringify(body) });

  // ---- icons (stroke, currentColor) --------------------------------------
  const ICONS = {
    info: '<circle cx="8" cy="8" r="6.2"/><path d="M8 7.2v3.6M8 5.1v.1"/>',
    warn: '<path d="M8 2.2 14.3 13H1.7z"/><path d="M8 6.4v3M8 11.2v.1"/>',
    check: '<path d="m3.2 8.4 3 3 6.6-6.8"/>',
    x: '<path d="m4 4 8 8M12 4l-8 8"/>',
    more: '<circle cx="3.5" cy="8" r=".6"/><circle cx="8" cy="8" r=".6"/><circle cx="12.5" cy="8" r=".6"/>',
    back: '<path d="M9.8 3.5 5.3 8l4.5 4.5"/>',
    chevron: '<path d="m4.5 6.2 3.5 3.5 3.5-3.5"/>',
    refresh: '<path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.6v2.8h-2.8"/>',
    scan: '<path d="M2.5 5V2.5H5M11 2.5h2.5V5M13.5 11v2.5H11M5 13.5H2.5V11M4.5 8h7"/>',
    up: '<path d="M8 13V3M4 7l4-4 4 4"/>',
    down: '<path d="M8 3v10M4 9l4 4 4-4"/>',
    flat: '<path d="M3 8h10M9.5 4.5 13 8l-3.5 3.5"/>',
    dot: '<circle cx="8" cy="8" r="2.4"/>',
    download: '<path d="M8 2.5v8M4.5 7.5 8 11l3.5-3.5M3 13.5h10"/>',
    ext: '<path d="M9.5 2.5h4v4M13.5 2.5 7.5 8.5M12 9.5v4H2.5V4H6.5"/>',
    plus: '<path d="M8 3v10M3 8h10"/>',
    report: '<path d="M4 1.8h5.5L12.5 5v9.2H4z"/><path d="M9.5 1.8V5h3M6 8.5h4.5M6 11h4.5"/>',
    gear: '<circle cx="8" cy="8" r="2.2"/><path d="M8 1.8v1.6M8 12.6v1.6M1.8 8h1.6M12.6 8h1.6M3.6 3.6l1.1 1.1M11.3 11.3l1.1 1.1M3.6 12.4l1.1-1.1M11.3 4.7l1.1-1.1"/>',
  };
  const icon = (name, label) =>
    `<svg class="svg-icon" viewBox="0 0 16 16" ${label ? `role="img" aria-label="${esc(label)}"` : 'aria-hidden="true"'}>${ICONS[name] || ICONS.dot}</svg>`;

  /** Badge = icon + text, never colour alone. tone: success|warning|danger|info|accent|'' */
  const TONE_ICON = { success: 'check', warning: 'warn', danger: 'x', info: 'info', accent: 'dot' };
  const badge = (tone, text, title) =>
    `<span class="badge ${esc(tone || '')}"${title ? ` title="${esc(title)}"` : ''}>${icon(TONE_ICON[tone] || 'dot')}${esc(text)}</span>`;

  /** DORA-style band → badge. Accepts elite/high/medium/low or good/warn/bad. */
  function bandBadge(band) {
    if (!band) return '';
    const b = String(band).toLowerCase();
    const tone = b === 'elite' || b === 'high' || b === 'good' || b === 'ok' ? 'success' : b === 'medium' || b === 'warn' ? 'warning' : 'danger';
    return badge(tone, b);
  }

  const jiraLink = (key) => {
    if (!key) return '';
    const base = TP.state.jiraBase;
    if (!base) return `<span class="mono">${esc(key)}</span>`;
    return `<a class="mono" href="${esc(base.replace(/\/+$/, ''))}/browse/${encodeURIComponent(key)}" target="_blank" rel="noopener noreferrer" title="Open ${esc(key)} in Jira">${esc(key)}</a>`;
  };

  // ---- info popover registry ---------------------------------------------
  // Every ⓘ shows: definition, formula and input quality — so a number is
  // never read without knowing what it is and how trustworthy its inputs are.
  const INFO = new Map();
  let infoSeq = 0;
  /** def = {title, definition, formula, quality} (quality may be a string or {level,msg}). */
  function info(def) {
    const id = 'i' + ++infoSeq;
    INFO.set(id, def);
    const label = `About ${def.title || 'this metric'}`;
    return `<button type="button" class="info-btn" data-info="${id}" aria-label="${esc(label)}" title="${esc(label)}" aria-haspopup="dialog" aria-expanded="false">${icon('info')}</button>`;
  }
  function qualityText(q) {
    if (!q) return 'Not reported — treat as indicative.';
    if (typeof q === 'string') return q;
    const n = isNum(q.n) ? ` (n=${q.n})` : '';
    return `${q.level ? q.level.toUpperCase() + ': ' : ''}${q.msg || q.note || ''}${n}`;
  }

  // ---- popovers (clamped into the viewport, Esc closes, focus restore) ---
  let openPop = null;
  function closePopover() {
    if (!openPop) return;
    const { el, anchor, onClose } = openPop;
    openPop = null;
    el.remove();
    if (anchor) {
      anchor.setAttribute('aria-expanded', 'false');
      anchor.focus();
    }
    if (onClose) onClose();
  }
  function placeClamped(el, anchor) {
    const r = anchor.getBoundingClientRect();
    const vw = document.documentElement.clientWidth;
    const vh = document.documentElement.clientHeight;
    const pad = 8;
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    let left = r.left;
    if (left + w > vw - pad) left = vw - pad - w;
    left = Math.max(pad, left);
    let top = r.bottom + 4;
    if (top + h > vh - pad) top = Math.max(pad, Math.min(r.top - 4 - h, vh - pad - h));
    top = Math.max(pad, top);
    el.style.left = left + 'px';
    el.style.top = top + 'px';
  }
  /** Open a popover under `anchor`. content: html string. role: dialog|menu. */
  function popover(anchor, content, { role = 'dialog', label = '', className = '', onClose, onOpen } = {}) {
    if (openPop && openPop.anchor === anchor) return closePopover();
    closePopover();
    const el = document.createElement('div');
    el.className = `popover ${className}`;
    el.setAttribute('role', role);
    if (label) el.setAttribute('aria-label', label);
    el.innerHTML = content;
    document.body.appendChild(el);
    placeClamped(el, anchor);
    anchor.setAttribute('aria-expanded', 'true');
    openPop = { el, anchor, onClose };
    if (onOpen) onOpen(el);
    const first = el.querySelector('button, input, select, a[href], [tabindex]');
    if (first) first.focus();
    return el;
  }
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && openPop) {
      e.stopPropagation();
      closePopover();
    }
  }, true);
  document.addEventListener('mousedown', (e) => {
    if (openPop && !openPop.el.contains(e.target) && !openPop.anchor.contains(e.target)) closePopover();
  });
  window.addEventListener('resize', () => openPop && placeClamped(openPop.el, openPop.anchor));
  document.addEventListener('click', (e) => {
    const b = e.target.closest('[data-info]');
    if (!b) return;
    const def = INFO.get(b.dataset.info);
    if (!def) return;
    popover(
      b,
      `<dl><dt>${esc(def.title || 'Metric')}</dt><dd>${esc(def.definition || '')}</dd>
        <dt>Formula</dt><dd class="mono small">${esc(def.formula || '—')}</dd>
        <dt>Input quality</dt><dd>${esc(qualityText(def.quality))}</dd></dl>`,
      { label: def.title },
    );
  });

  // ---- modal + confirmer --------------------------------------------------
  /**
   * Open a modal dialog. Returns {el, close(value), done: Promise<value>}.
   * opts: {title, body (html), actions:[{label, value, primary, danger}], wide, onOpen(el)}
   */
  function modal({ title, body, actions = [], wide = false, onOpen, labelledBy } = {}) {
    const prev = document.activeElement;
    const back = document.createElement('div');
    back.className = 'modal-backdrop';
    const id = 'm' + Date.now().toString(36);
    back.innerHTML = `<div class="modal${wide ? ' wide' : ''}" role="dialog" aria-modal="true" aria-labelledby="${labelledBy || id}">
      ${title ? `<h2 id="${id}">${esc(title)}</h2>` : ''}
      <div class="modal-body">${body || ''}</div>
      ${actions.length ? `<footer>${actions.map((a, i) => `<button type="button" data-act="${i}" class="${a.primary ? 'primary' : a.danger ? 'danger' : ''}">${esc(a.label)}</button>`).join('')}</footer>` : ''}
    </div>`;
    document.body.appendChild(back);
    const dlg = back.firstElementChild;
    let resolve;
    const done = new Promise((r) => (resolve = r));
    const close = (value) => {
      document.removeEventListener('keydown', onKey, true);
      back.remove();
      if (prev && prev.focus) prev.focus();
      resolve(value);
    };
    const onKey = (e) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        close(undefined);
      } else if (e.key === 'Tab') {
        const f = [...dlg.querySelectorAll('button, input, select, textarea, a[href], iframe, [tabindex]:not([tabindex="-1"])')].filter((x) => !x.disabled);
        if (!f.length) return;
        if (e.shiftKey && document.activeElement === f[0]) {
          e.preventDefault();
          f[f.length - 1].focus();
        } else if (!e.shiftKey && document.activeElement === f[f.length - 1]) {
          e.preventDefault();
          f[0].focus();
        }
      }
    };
    document.addEventListener('keydown', onKey, true);
    back.addEventListener('mousedown', (e) => e.target === back && close(undefined));
    dlg.querySelectorAll('[data-act]').forEach((b) => (b.onclick = () => close(actions[+b.dataset.act].value)));
    if (onOpen) onOpen(dlg, close);
    const first = dlg.querySelector('input, select, textarea') || dlg.querySelector('footer .primary, footer button, button');
    if (first) first.focus();
    return { el: dlg, close, done };
  }
  const confirmer = {
    /** Resolves true only on explicit confirmation. */
    ask({ title = 'Are you sure?', message = '', confirmLabel = 'Continue', danger = false } = {}) {
      return modal({
        title,
        body: `<p>${esc(message)}</p>`,
        actions: [
          { label: 'Cancel', value: false },
          { label: confirmLabel, value: true, primary: !danger, danger },
        ],
      }).done.then((v) => v === true);
    },
  };

  // ---- toasts -------------------------------------------------------------
  function toast(msg, tone = '') {
    let host = document.querySelector('.toasts');
    if (!host) {
      host = document.createElement('div');
      host.className = 'toasts';
      host.setAttribute('role', 'status');
      host.setAttribute('aria-live', 'polite');
      document.body.appendChild(host);
    }
    const t = document.createElement('div');
    t.className = `toast ${tone}`;
    t.textContent = msg;
    host.appendChild(t);
    setTimeout(() => t.remove(), 4500);
  }

  // ---- states -------------------------------------------------------------
  const skeleton = (lines = 4, block = true) =>
    `<div class="skeleton" aria-hidden="true">${block ? '<span class="block"></span>' : ''}${'<span></span>'.repeat(lines)}</div><span class="sr-only">Loading…</span>`;
  /** EmptyState with an optional first-run checklist [{label, done}]. */
  const emptyState = ({ title, body = '', steps = [], actionLabel = '', actionId = '' }) => `
    <div class="empty">
      <h2>${esc(title)}</h2>
      ${body ? `<p class="dim">${body}</p>` : ''}
      ${steps.length ? `<ol class="checklist">${steps.map((s) => `<li data-done="${s.done ? 'true' : 'false'}">${icon(s.done ? 'check' : 'dot')}<span>${esc(s.label)}${s.done ? '<span class="sr-only"> (done)</span>' : ''}</span></li>`).join('')}</ol>` : ''}
      ${actionLabel ? `<button type="button" class="primary" id="${esc(actionId)}">${esc(actionLabel)}</button>` : ''}
    </div>`;
  const notAvailable = (what = 'This metric') => `<p class="na">${esc(what)} is not available yet — it appears after the next scan computes it.</p>`;

  /**
   * A self-contained section: header (title, ⓘ, sub), then load() → render.
   * Shows its own skeleton while loading and an inline error + Retry on failure.
   * opts: {title, infoDef, sub, load: async () => html|Node|null, after(el), empty: html}
   */
  function section(host, opts) {
    const el = document.createElement('section');
    el.className = 'section';
    const hid = 'h' + Math.random().toString(36).slice(2, 8);
    el.setAttribute('aria-labelledby', hid);
    el.innerHTML = `<header><h2 id="${hid}">${esc(opts.title)}</h2>${opts.infoDef ? info(opts.infoDef) : ''}${opts.headerEnd ? `<div class="end">${opts.headerEnd}</div>` : ''}${opts.sub ? `<p class="sub">${opts.sub}</p>` : ''}</header><div class="body"></div>`;
    host.appendChild(el);
    const body = el.querySelector('.body');
    const run = async () => {
      body.innerHTML = skeleton(opts.skeletonLines || 3, opts.skeletonBlock !== false);
      body.setAttribute('aria-busy', 'true');
      try {
        const out = await opts.load();
        body.removeAttribute('aria-busy');
        if (out == null || out === '') body.innerHTML = opts.empty || notAvailable(opts.title);
        else if (typeof out === 'string') body.innerHTML = out;
        else {
          body.innerHTML = '';
          body.appendChild(out);
        }
        if (opts.after) opts.after(body, run);
      } catch (e) {
        body.removeAttribute('aria-busy');
        body.innerHTML = `<div class="inline-error" role="alert">${icon('warn')}<span>Couldn’t load ${esc(opts.title.toLowerCase())}: ${esc(e.message)}</span><button type="button" class="compact retry">Retry</button></div>`;
        body.querySelector('.retry').onclick = run;
      }
    };
    run();
    return el;
  }

  // ---- tables -------------------------------------------------------------
  /**
   * cols: [{key, label, num, sort:boolean, scope}]; rows: array of html-cell arrays
   * (or {cells, attrs}). Returns html with a caption and th scope.
   */
  function table({ caption, cols, rows, stickyFirst = true, id = '', sortKey = '', sortDir = 'asc' }) {
    const head = cols
      .map((c) => {
        const cls = c.num ? ' class="num"' : '';
        const aria = c.sort ? ` aria-sort="${sortKey === c.key ? (sortDir === 'asc' ? 'ascending' : 'descending') : 'none'}"` : '';
        const inner = c.sort ? `<button type="button" class="sort" data-sort="${esc(c.key)}">${esc(c.label)}${sortKey === c.key ? (sortDir === 'asc' ? ' ▲' : ' ▼') : ''}</button>` : esc(c.label);
        return `<th scope="col"${cls}${aria}${c.title ? ` title="${esc(c.title)}"` : ''}>${inner}${c.info ? ' ' + info(c.info) : ''}</th>`;
      })
      .join('');
    const body = rows
      .map((r) => {
        const cells = Array.isArray(r) ? r : r.cells;
        const attrs = Array.isArray(r) ? '' : r.attrs || '';
        if (!Array.isArray(r) && r.full) return `<tr ${attrs}><td colspan="${cols.length}">${r.full}</td></tr>`;
        return `<tr ${attrs}>${cells.map((c, i) => (i === 0 ? `<th scope="row">${c}</th>` : `<td${cols[i] && cols[i].num ? ' class="num"' : ''}>${c}</td>`)).join('')}</tr>`;
      })
      .join('');
    return `<div class="table-wrap"><table${id ? ` id="${esc(id)}"` : ''} class="${stickyFirst ? 'sticky-first' : ''}"><caption>${esc(caption)}</caption><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>`;
  }

  // ---- charts (inline SVG, title+desc, always paired with a table) -------
  const SERIES = ['--cat-1', '--cat-3', '--cat-2', '--cat-4', '--cat-5', '--cat-6'];
  let svgSeq = 0;
  function svgOpen(w, h, title, desc) {
    const id = 's' + ++svgSeq;
    return {
      id,
      open: `<svg viewBox="0 0 ${w} ${h}" role="img" aria-labelledby="${id}t ${id}d" preserveAspectRatio="xMinYMin meet"><title id="${id}t">${esc(title)}</title><desc id="${id}d">${esc(desc)}</desc>`,
    };
  }
  /** Wrap an SVG + its data table. */
  const chartBlock = ({ svg, legend = '', tableHtml = '' }) =>
    `<div class="chart">${legend}${svg}${tableHtml ? `<details><summary>Show as table</summary>${tableHtml}</details>` : ''}</div>`;
  const legendHtml = (items) =>
    `<div class="legend">${items.map((i) => `<span class="key"><span class="swatch${i.hatch ? ' hatch' : ''}" ${i.hatch ? '' : `style="background:var(${i.color})"`}></span>${esc(i.label)}</span>`).join('')}</div>`;

  /** Sparkline; values may contain nulls. */
  function sparkline(values, label = 'trend') {
    const vs = (values || []).filter(isNum);
    if (vs.length < 2) return '';
    const w = 96, h = 24;
    const lo = Math.min(...vs), hi = Math.max(...vs);
    const arr = values.slice(-16);
    const pts = arr
      .map((v, i) => (isNum(v) ? `${((i / Math.max(1, arr.length - 1)) * (w - 4) + 2).toFixed(1)},${(hi === lo ? h / 2 : h - 2 - ((v - lo) / (hi - lo)) * (h - 4)).toFixed(1)}` : null))
      .filter(Boolean);
    return `<svg class="spark" viewBox="0 0 ${w} ${h}" role="img" aria-label="${esc(label)}: ${vs.length} points, from ${round(vs[0], 2)} to ${round(vs[vs.length - 1], 2)}"><polyline points="${pts.join(' ')}" fill="none" stroke="var(--cat-1)" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/></svg>`;
  }

  /**
   * Horizontal stacked bars. rows: [{label, parts:{segKey: number|null}}];
   * segs: [{key, label, color}] — a null part = NOT TRACKED, drawn hatched.
   */
  function stackedBars({ rows, segs, title, desc, unit = 'd' }) {
    const rowH = 26, labelW = 150, endW = 70, w = 760;
    const totals = rows.map((r) => segs.reduce((a, s) => a + (isNum(r.parts[s.key]) ? r.parts[s.key] : 0), 0));
    const max = Math.max(1, ...totals);
    const h = rows.length * rowH + 4;
    const plotW = w - labelW - endW;
    const { id, open } = svgOpen(w, h, title, desc);
    let out = open + `<defs><pattern id="${id}hatch" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><rect width="5" height="5" fill="var(--surface-2)"/><line x1="0" y1="0" x2="0" y2="5" stroke="var(--text-dim)" stroke-width="1.6"/></pattern></defs>`;
    rows.forEach((r, i) => {
      const y = i * rowH + 6;
      const lab = r.label.length > 22 ? r.label.slice(0, 21) + '…' : r.label;
      out += `<text x="${labelW - 8}" y="${y + 11}" text-anchor="end">${esc(lab)}</text>`;
      let x = labelW;
      segs.forEach((s) => {
        const v = r.parts[s.key];
        if (v === null && s.hatchWhenNull) {
          out += `<rect x="${x}" y="${y}" width="18" height="14" rx="2" fill="url(#${id}hatch)"><title>${esc(s.label)}: not tracked</title></rect>`;
          x += 20;
          return;
        }
        if (!isNum(v) || v <= 0) return;
        const bw = Math.max(1.5, (v / max) * plotW);
        out += `<rect x="${x.toFixed(1)}" y="${y}" width="${Math.max(1, bw - 1.5).toFixed(1)}" height="14" rx="2" fill="var(${s.color})"><title>${esc(s.label)}: ${round(v, 1)}${unit}</title></rect>`;
        x += bw;
      });
      out += `<text x="${(x + 6).toFixed(1)}" y="${y + 11}">${round(totals[i], 1)}${unit}</text>`;
    });
    out += '</svg>';
    return out;
  }

  /** Vertical histogram. bins: [{label, n}]. */
  function histogram({ bins, title, desc, color = '--cat-1' }) {
    const w = 640, h = 170, pad = 22;
    const max = Math.max(1, ...bins.map((b) => b.n || 0));
    const bw = (w - pad) / Math.max(1, bins.length);
    const { open } = svgOpen(w, h, title, desc);
    let out = open;
    bins.forEach((b, i) => {
      const bh = ((b.n || 0) / max) * (h - pad - 16);
      const x = pad / 2 + i * bw;
      out += `<rect x="${(x + 2).toFixed(1)}" y="${(h - pad - bh).toFixed(1)}" width="${Math.max(1, bw - 4).toFixed(1)}" height="${bh.toFixed(1)}" rx="2" fill="var(${b.color || color})"><title>${esc(b.label)}: ${b.n}</title></rect>`;
      out += `<text x="${(x + bw / 2).toFixed(1)}" y="${h - 6}" text-anchor="middle">${esc(b.label)}</text>`;
      if (b.n) out += `<text x="${(x + bw / 2).toFixed(1)}" y="${(h - pad - bh - 4).toFixed(1)}" text-anchor="middle">${b.n}</text>`;
    });
    return out + '</svg>';
  }

  /** Column chart for a time series [{label, value}]. */
  function columns({ points, title, desc, color = '--cat-1', unit = '' }) {
    const w = 640, h = 150, pad = 20;
    const max = Math.max(1, ...points.map((p) => (isNum(p.value) ? p.value : 0)));
    const bw = (w - pad) / Math.max(1, points.length);
    const { open } = svgOpen(w, h, title, desc);
    let out = open;
    points.forEach((p, i) => {
      const v = isNum(p.value) ? p.value : 0;
      const bh = (v / max) * (h - pad - 14);
      const x = pad / 2 + i * bw;
      out += `<rect x="${(x + 2).toFixed(1)}" y="${(h - pad - bh).toFixed(1)}" width="${Math.max(1, bw - 4).toFixed(1)}" height="${Math.max(0, bh).toFixed(1)}" rx="2" fill="var(${color})"><title>${esc(p.label)}: ${round(v, 2)}${unit}</title></rect>`;
      if (points.length <= 14 || i % 2 === 0) out += `<text x="${(x + bw / 2).toFixed(1)}" y="${h - 5}" text-anchor="middle">${esc(String(p.label).slice(-5))}</text>`;
    });
    return out + '</svg>';
  }

  const meter = (ratio, label) => {
    const p = Math.max(0, Math.min(1, isNum(ratio) ? ratio : 0));
    return `<span class="meter" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(p * 100)}" aria-label="${esc(label || 'share')}"><span style="inline-size:${(p * 100).toFixed(1)}%"></span></span>`;
  };

  /** localStorage wrappers — private mode / blocked storage must never break the page. */
  const store = {
    get(k, d = null) {
      try {
        const v = localStorage.getItem('tp:' + k);
        return v === null ? d : v;
      } catch {
        return d;
      }
    },
    set(k, v) {
      try {
        localStorage.setItem('tp:' + k, v);
      } catch {
        /* storage unavailable */
      }
    },
  };

  Object.assign(TP, {
    esc, isNum, round, fmtD, fmtPct, fmtNum, fmtX, fmtDate, fmtAgo, fmtSecs, M,
    api, put, post, icon, badge, bandBadge, jiraLink, info, qualityText,
    popover, closePopover, modal, confirmer, toast,
    skeleton, emptyState, notAvailable, section, table,
    SERIES, chartBlock, legendHtml, sparkline, stackedBars, histogram, columns, meter, store,
  });
})();
