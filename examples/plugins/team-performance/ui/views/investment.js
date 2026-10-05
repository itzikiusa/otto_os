// Investment: where delivered scope went — by ticket type and by epic —
// plus unplanned share and git-only features (repos without Jira stories).
'use strict';
(function () {
  const TP = window.TP;
  const { esc, M, fmtD, fmtPct, isNum, section, notAvailable, table, jiraLink, meter, badge, api, info } = TP;

  const KEY_RE = /^[A-Z][A-Z0-9]+-\d+$/;
  const NO_EPIC = /^no[ _]epic$/i;

  /** by_type / by_epic arrive as an array of rows or a {label: {days, share}} map. */
  function toRows(x) {
    if (!x) return null;
    if (Array.isArray(x)) return x;
    if (typeof x === 'object') return Object.entries(x).map(([k, v]) => ({ key: k, label: k, ...(isNum(v) ? { days: v } : v) }));
    return null;
  }

  /** Epic label: summary from the row, the epics lookup or the open tickets. */
  const epicSummary = (o, inv, key) =>
    (inv && inv.epics && inv.epics[key] && (inv.epics[key].summary || inv.epics[key])) || (o.epic_summaries && o.epic_summaries[key]) || '';

  function mix(o, inv, list, caption, isEpic) {
    if (!list || !list.length) return null;
    const total = list.reduce((a, r) => a + (isNum(r.days) ? r.days : 0), 0) || 1;
    const rows = list
      .slice()
      .sort((a, b) => (b.days || 0) - (a.days || 0))
      .map((r) => {
        const share = isNum(r.share) ? r.share : (r.days || 0) / total;
        const k = r.key || r.epic || r.label;
        let label;
        if (isEpic && NO_EPIC.test(String(k))) {
          label = `<b>No epic</b> ${info({ title: 'No epic', definition: 'Delivered tickets that are not linked to any epic (directly or through their parent story). Usually bugs, chores and support — if a big share lands here, investment by feature is under-reported.', formula: 'Σ dev-days of tickets without an epic link' })}<div class="dim small">Not linked to an epic in Jira — bugs, chores and support usually land here, so feature investment is under-reported by this share.</div>`;
        } else if (isEpic && KEY_RE.test(String(k))) {
          const sum = r.summary || epicSummary(o, inv, k);
          label = `${jiraLink(k)} <span class="dim small">${esc(String(sum || '').slice(0, 70)) || 'summary not scanned'}</span>`;
        } else label = esc(r.label || r.type || k || '—');
        return [label, `${meter(share, 'share of dev time')} ${fmtPct(share)}`, fmtD(r.days), isNum(r.n) ? String(r.n) : '—'];
      });
    return table({
      caption,
      cols: [{ label: isEpic ? 'Epic' : 'Type' }, { label: 'Share' }, { label: 'Dev days', num: true, title: 'Business days of dev time (In Progress → Code Review, plus QA work with commits)' }, { label: 'Tickets', num: true }],
      rows,
    });
  }

  /** Drill-down into the no-epic bucket, when the payload lists its tickets. */
  function noEpicDrill(inv) {
    const list = (inv && (inv.no_epic_tickets || (inv.by_epic_tickets && inv.by_epic_tickets['no epic']))) || null;
    if (!list) return '';
    if (!list.length) return '';
    return `<details><summary>${list.length} tickets without an epic</summary>${table({
      caption: 'Delivered tickets without an epic',
      cols: [{ label: 'Ticket' }, { label: 'Type' }, { label: 'Dev days', num: true }],
      rows: list.slice(0, 200).map((t) => [`${jiraLink(t.key)} <span class="dim small">${esc((t.summary || '').slice(0, 60))}</span>`, esc(t.type || '—'), fmtD(t.dev_days ?? t.days)]),
    })}</details>`;
  }

  TP.views.investment = {
    mix,
    toRows,
    render(host, { o, app }) {
      const inv = o.investment || (o.flow && o.flow.investment);
      const byType = toRows(inv && (inv.by_type || (Array.isArray(inv) ? inv.filter((r) => r.kind !== 'epic') : null)));
      const byEpic = toRows(inv && (inv.by_epic || (Array.isArray(inv) ? inv.filter((r) => r.kind === 'epic') : null)));
      const g = document.createElement('div');
      g.className = 'grid-2';
      host.appendChild(g);
      section(g, {
        title: 'By ticket type',
        infoDef: { title: 'Investment mix', definition: 'Share of delivered estimated days per ticket type. Rework tickets are attributed to rework, not new scope.', formula: 'Σ est-days per type ÷ Σ est-days', quality: inv && inv.quality },
        load: async () => mix(o, inv, byType, 'Delivered scope by ticket type') || notAvailable('Investment by type'),
      });
      section(g, {
        title: 'By epic',
        sub: 'Feature-level view: each epic links to Jira.',
        load: async () => {
          const t = mix(o, inv, byEpic, 'Delivered scope by epic (feature level)', true);
          return t ? t + noEpicDrill(inv) : notAvailable('Investment by epic');
        },
      });
      const un = M((o.flow && (o.flow.unplanned_share || o.flow.unplannedShare)) || (inv && inv.unplanned_share));
      section(host, {
        title: 'Planned vs unplanned',
        load: async () =>
          un && isNum(un.value)
            ? `<p>${badge(un.value > 0.35 ? 'warning' : 'success', fmtPct(un.value > 1 ? un.value / 100 : un.value) + ' unplanned')} <span class="dim">of delivered scope was bugs, hotfixes or work added mid-period.</span></p>`
            : notAvailable('Unplanned share'),
      });
      section(host, {
        title: 'Repo work without Jira stories',
        sub: 'Git-only features from the feature-scan repos chosen in Settings.',
        load: async () => {
          const f = await api(`/features?account=${encodeURIComponent(app.account)}`);
          const feats = (f && f.features) || [];
          if (!feats.length) return '<p class="dim">No git-only features — add feature-scan repos in Settings to track work that has no Jira story.</p>';
          return table({
            caption: `${feats.length} git-only features`,
            cols: [{ label: 'Feature' }, { label: 'Repo' }, { label: 'People' }, { label: 'Commits', num: true }, { label: 'Estimate', num: true }, { label: 'Actual', num: true }, { label: 'Merged', num: true }],
            rows: feats.slice(0, 100).map((x) => [
              esc(x.summary) + (x.routine ? ' ' + badge('', 'routine') : ''),
              esc(x.repo),
              esc((x.people || []).join(', ') || (x.authors || []).map((a) => a.name).join(', ')),
              String(x.commit_count ?? '—'),
              fmtD(x.est_days_ai),
              fmtD(x.actual_days),
              TP.fmtDate(x.merged_at),
            ]),
          });
        },
      });
    },
  };
})();
