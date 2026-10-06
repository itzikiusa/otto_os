// Quality & rework: change failure, rework rate, time charged back to the
// original ticket (git blame + Jira-detected), hygiene flags.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, M, fmtD, fmtPct, isNum, section, notAvailable, table, jiraLink, info, badge } = TP;

  const reasonLabel = (r) =>
    ({ blame: 'rewrote recent code', caused_by: 'linked “caused by”', relates: 'linked “relates”', fixes: 'linked “fixes”', title_ref: 'title references ticket', follow_up: 'follow-up / fix-for title', reopened: 'reopened', bug_after: 'bug soon after delivery' })[r] || r;

  /** Signal badges: git vs Jira source + each reason; signals[] carries several. */
  function signalCell(r) {
    const sigs = Array.isArray(r.signals) && r.signals.length ? r.signals : [{ source: r.source, reason: r.reason }];
    return sigs
      .map((x) => (typeof x === 'string' ? { reason: x } : x || {}))
      .map((x) => {
        const reason = x.reason || x.kind || x.type || '';
        const jira = (x.source || (reason === 'blame' ? 'git' : 'jira')) === 'jira';
        return `${badge(jira ? 'info' : 'accent', jira ? 'Jira' : 'git')} ${esc(reasonLabel(reason))}`;
      })
      .join(' ') + (r.confidence ? ` <span class="dim small">${esc(String(r.confidence))} confidence</span>` : '');
  }

  function tiles(o) {
    const rw = o.rework || {};
    const d = o.dora || {};
    const cfr = M(d.change_failure_rate || d.cfr);
    const rate = M(rw.rate ?? rw.value);
    const items = [
      { t: 'Rework rate', m: rate, share: true, ids: ['rework'], def: 'Share of delivered work that is rework of a recently delivered ticket. A rework ticket’s estimate is not new scope.', fm: 'rework est-days ÷ delivered est-days', dir: 'Lower is better.', drill: rw.charged || rw.tickets || rw.items },
      { t: 'Time charged back', m: M(rw.charged_days), fmt: fmtD, ids: ['rework'], def: 'Working days of later tickets charged back to the tickets whose recent code they rewrote.', fm: 'Σ rework days (git blame + Jira-detected)', dir: 'Lower is better.' },
      { t: 'Change failure rate', m: cfr, share: true, ids: ['changeFailureRate'], def: 'Deployments followed by a hotfix or a bug on the same code.', fm: 'failed deploys ÷ deploys', dir: 'Lower is better.' },
      { t: 'Fix rate', m: M(o.scope && o.scope.fix_rate), share: true, ids: ['rework'], def: 'Done tickets that needed fix commits within the fix window.', fm: 'tickets with fixes ÷ done', dir: 'Lower is better.' },
    ];
    return `<div class="tiles">${items
      .map((x) => {
        const m = x.m;
        const has = m && isNum(m.value);
        const guard = TP.mergeGuard(TP.guardFor(o, x.ids, m), x.share && has ? TP.overGuard(m.value, x.t) : null);
        const list = Array.isArray(x.drill) ? x.drill : null;
        return TP.tile({
          title: x.t,
          valueHtml: has ? (x.share ? TP.shareHtml(m.value) : x.fmt(m.value)) : '<span class="na">not available yet</span>',
          context: m && isNum(m.failed) && isNum(m.total) ? `${m.failed} of ${m.total} deploys` : m && isNum(m.n) ? `n=${m.n}` : '',
          guard,
          def: { definition: x.def, formula: x.fm, direction: x.dir, quality: m && m.quality },
          drill: has && list && list.length
            ? { valueLabel: 'Days', tickets: list.map((r) => ({ key: r.rework_key || r.key, summary: `rework of ${r.original_key || r.of_key || r.rework_of || '—'}`, value: r.days, note: (r.signals || [r.reason]).map((g) => reasonLabel(typeof g === 'string' ? g : (g && (g.reason || g.kind)) || '')).filter(Boolean).join(', ') })) }
            : null,
        });
      })
      .join('')}</div>`;
  }

  /** Which rework detectors ran: o.rework.sources = {git_blame:{ran,repos}, jira:{ran,tickets}}. */
  function reworkSources(rw) {
    const src = (rw && (rw.sources || rw.detectors)) || {};
    const gb = src.git_blame || src.blame || src.git || null;
    const jl = src.jira_links || src.jira || null;
    return [
      { label: 'git blame scanned', ran: gb ? gb.ran !== false : false, n: gb ? gb.repos ?? gb.n : undefined, unit: 'repos' },
      { label: 'Jira links checked', ran: jl ? jl.ran !== false : false, n: jl ? jl.tickets ?? jl.n : undefined, unit: 'tickets' },
    ];
  }

  function chargedHtml(o) {
    const rw = o.rework;
    if (!rw) return TP.sourcesEmpty({ what: 'Rework', sources: reworkSources(null) });
    const list = rw.charged || rw.tickets || rw.items || [];
    if (!list.length) return TP.sourcesEmpty({ what: 'Rework', sources: reworkSources(rw) });
    return table({
      caption: `${list.length} rework links — the rework ticket’s estimate is not counted as new scope`,
      cols: [{ label: 'Original ticket' }, { label: 'Rework ticket' }, { label: 'Signal' }, { label: 'Days charged', num: true }, { label: 'Owner' }],
      rows: list.slice(0, 300).map((r) => [
        `${jiraLink(r.original_key || r.of_key || r.rework_of || r.key)} <span class="dim small">${esc((r.original_summary || '').slice(0, 50))}</span>`,
        r.rework_key ? jiraLink(r.rework_key) : r.rework_of && r.key ? jiraLink(r.key) : '—',
        signalCell(r),
        fmtD(r.days),
        esc(r.assignee_name || '—'),
      ]),
    });
  }

  /** Hygiene flags: human labels, each a button opening the tickets that carry it. */
  function flagsHtml(o) {
    const f = Object.entries(o.flags || {});
    if (!f.length) return '<p class="dim">No hygiene issues on completed tickets.</p>';
    const byFlag = o.flag_tickets || o.flags_tickets || {};
    return `<ul class="flag-list">${f
      .sort((a, b) => b[1] - a[1])
      .map(([k, v]) => {
        const label = TP.hygieneLabel(k);
        const why = (TP.HYGIENE[k] || [])[1] || '';
        const list = (byFlag[k] || []).map((t) => (typeof t === 'string' ? { key: t } : t));
        const ref = TP.drillRef({ title: `${label} — ${v} ticket${v === 1 ? '' : 's'}`, intro: why, valueLabel: 'Days', tickets: list.map((t) => ({ key: t.key, summary: t.summary, value: t.actual_days ?? t.days })), empty: 'This scan did not list the tickets for this flag. Open a person on the People tab to see flags per ticket.' });
        return `<li><button type="button" class="flag" data-drill="${ref}" aria-haspopup="dialog">${badge('warning', `${label} × ${v}`)}</button> <span class="dim small">${esc(why)}</span></li>`;
      })
      .join('')}</ul>`;
  }

  TP.views.quality = {
    tiles,
    chargedHtml,
    flagsHtml,
    render(host, { o, app }) {
      section(host, { title: 'Quality', load: async () => tiles(o) });
      section(host, {
        title: 'Rework charged back',
        infoDef: {
          title: 'Rework attribution',
          definition: 'A later ticket is rework of an earlier one when it rewrote the earlier ticket’s recent code (git blame), or Jira says so: caused-by/relates/fixes links, titles referencing the key or “follow-up / fix for / missing”, reopened tickets, bugs soon after delivery on the same code.',
          formula: 'days of the rework ticket → added to the original ticket’s REWORK phase',
          quality: o.rework && o.rework.quality,
        },
        headerEnd: TP.guardBadge(TP.guardFor(o, ['rework'])),
        load: async () => chargedHtml(o),
        after: (body) => TP.wireEmptyActs(body, app),
      });
      section(host, { title: 'Hygiene flags', sub: 'Data issues on completed tickets that can skew timing.', load: async () => flagsHtml(o) });
    },
  };
})();
