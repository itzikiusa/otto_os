// Quality & rework: change failure, rework rate, time charged back to the
// original ticket (git blame + Jira-detected), hygiene flags.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, M, fmtD, fmtPct, isNum, section, notAvailable, table, jiraLink, info, badge } = TP;

  const reasonLabel = (r) =>
    ({ blame: 'rewrote recent code', caused_by: 'linked “caused by”', relates: 'linked “relates”', fixes: 'linked “fixes”', title_ref: 'title references ticket', follow_up: 'follow-up / fix-for title', reopened: 'reopened', bug_after: 'bug soon after delivery' })[r] || r;

  function tiles(o) {
    const rw = o.rework || {};
    const d = o.dora || {};
    const cfr = M(d.change_failure_rate || d.cfr);
    const items = [
      ['Rework rate', M(rw.rate), fmtPct, 'Share of delivered work that is rework of a recently delivered ticket.', 'rework est-days ÷ delivered est-days'],
      ['Time charged back', M(rw.charged_days), fmtD, 'Working days of later tickets charged back to the tickets whose recent code they rewrote.', 'Σ rework days (git blame + Jira-detected)'],
      ['Change failure rate', cfr, fmtPct, 'Deployments followed by a hotfix or a bug on the same code.', 'failed deploys ÷ deploys'],
      ['Fix rate', M(o.scope && o.scope.fix_rate), fmtPct, 'Done tickets that needed fix commits within the fix window.', 'tickets with fixes ÷ done'],
    ];
    return `<div class="tiles">${items
      .map(([t, m, f, def, fm]) => `<div class="tile"><div class="label">${esc(t)} ${info({ title: t, definition: def, formula: fm, quality: m && m.quality })}</div><div class="value">${m && isNum(m.value) ? f(m.value) : '<span class="na">not available yet</span>'}</div>${m && isNum(m.n) ? `<div class="context">n=${m.n}</div>` : ''}</div>`)
      .join('')}</div>`;
  }

  function chargedHtml(o) {
    const rw = o.rework;
    if (!rw) return notAvailable('Rework attribution');
    const list = rw.charged || rw.tickets || [];
    if (!list.length) return '<p class="dim">No rework detected in this period.</p>';
    return table({
      caption: `${list.length} rework links — the rework ticket’s estimate is not counted as new scope`,
      cols: [{ label: 'Original ticket' }, { label: 'Rework ticket' }, { label: 'Signal' }, { label: 'Days charged', num: true }, { label: 'Owner' }],
      rows: list.slice(0, 300).map((r) => [
        `${jiraLink(r.original_key || r.of_key || r.key)} <span class="dim small">${esc((r.original_summary || '').slice(0, 50))}</span>`,
        r.rework_key ? jiraLink(r.rework_key) : '—',
        `${badge(r.source === 'jira' ? 'info' : 'accent', r.source === 'jira' ? 'Jira' : 'git')} ${esc(reasonLabel(r.reason || ''))}`,
        fmtD(r.days),
        esc(r.assignee_name || '—'),
      ]),
    });
  }

  function flagsHtml(o) {
    const f = Object.entries(o.flags || {});
    if (!f.length) return '<p class="dim">No hygiene issues on completed tickets.</p>';
    return `<div class="chips">${f.map(([k, v]) => badge('warning', `${k.replace(/_/g, ' ')} × ${v}`)).join('')}</div>`;
  }

  TP.views.quality = {
    render(host, { o }) {
      section(host, { title: 'Quality', load: async () => tiles(o) });
      section(host, {
        title: 'Rework charged back',
        infoDef: {
          title: 'Rework attribution',
          definition: 'A later ticket is rework of an earlier one when it rewrote the earlier ticket’s recent code (git blame), or Jira says so: caused-by/relates/fixes links, titles referencing the key or “follow-up / fix for / missing”, reopened tickets, bugs soon after delivery on the same code.',
          formula: 'days of the rework ticket → added to the original ticket’s REWORK phase',
          quality: o.rework && o.rework.quality,
        },
        load: async () => chargedHtml(o),
      });
      section(host, { title: 'Hygiene flags', sub: 'Data issues on completed tickets that can skew timing.', load: async () => flagsHtml(o) });
    },
  };
})();
