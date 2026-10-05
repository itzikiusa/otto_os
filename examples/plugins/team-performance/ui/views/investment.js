// Investment: where delivered scope went — by ticket type and by epic —
// plus unplanned share and git-only features (repos without Jira stories).
'use strict';
(function () {
  const TP = window.TP;
  const { esc, M, fmtD, fmtPct, isNum, section, notAvailable, table, jiraLink, meter, badge, api } = TP;

  function mix(list, caption, keyIsTicket) {
    if (!list || !list.length) return null;
    const total = list.reduce((a, r) => a + (isNum(r.days) ? r.days : 0), 0) || 1;
    return table({
      caption,
      cols: [{ label: keyIsTicket ? 'Epic' : 'Type' }, { label: 'Share' }, { label: 'Delivered', num: true }, { label: 'Tickets', num: true }],
      rows: list
        .slice()
        .sort((a, b) => (b.days || 0) - (a.days || 0))
        .map((r) => {
          const share = isNum(r.share) ? r.share : (r.days || 0) / total;
          return [
            keyIsTicket && r.key ? `${jiraLink(r.key)} <span class="dim small">${esc((r.label || r.summary || '').slice(0, 60))}</span>` : esc(r.label || r.type || '—'),
            `${meter(share, 'share of delivered scope')} ${fmtPct(share)}`,
            fmtD(r.days),
            isNum(r.n) ? String(r.n) : '—',
          ];
        }),
    });
  }

  TP.views.investment = {
    render(host, { o, app }) {
      const inv = o.investment || (o.flow && o.flow.investment);
      const byType = inv && (inv.by_type || (Array.isArray(inv) ? inv.filter((r) => r.kind !== 'epic') : null));
      const byEpic = inv && (inv.by_epic || (Array.isArray(inv) ? inv.filter((r) => r.kind === 'epic') : null));
      const g = document.createElement('div');
      g.className = 'grid-2';
      host.appendChild(g);
      section(g, {
        title: 'By ticket type',
        infoDef: { title: 'Investment mix', definition: 'Share of delivered estimated days per ticket type. Rework tickets are attributed to rework, not new scope.', formula: 'Σ est-days per type ÷ Σ est-days', quality: inv && inv.quality },
        load: async () => mix(byType, 'Delivered scope by ticket type') || notAvailable('Investment by type'),
      });
      section(g, {
        title: 'By epic',
        load: async () => mix(byEpic, 'Delivered scope by epic (feature level)', true) || notAvailable('Investment by epic'),
      });
      const un = M((o.flow && o.flow.unplanned_share) || (inv && inv.unplanned_share));
      section(host, {
        title: 'Planned vs unplanned',
        load: async () =>
          un && isNum(un.value)
            ? `<p>${badge(un.value > 0.35 ? 'warning' : 'success', fmtPct(un.value) + ' unplanned')} <span class="dim">of delivered scope was bugs, hotfixes or work added mid-period.</span></p>`
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
