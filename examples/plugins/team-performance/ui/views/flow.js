// Flow: cycle time split by phase (hatched = not tracked), PR pickup →
// review → merge funnel, PR size, review depth, WIP/throughput, open tickets.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, M, fmtD, fmtNum, fmtPct, isNum, section, notAvailable, table, chartBlock, legendHtml, stackedBars, histogram, columns, jiraLink, info, meter } = TP;

  // Canonical phase order; finer splits (review wait vs in review, QA wait vs
  // QA rework) render when the data carries them.
  const SEGS = [
    { key: 'design', label: 'Design', color: '--cat-4', hatchWhenNull: true },
    { key: 'dev', label: 'Dev', color: '--cat-1' },
    { key: 'review_wait', label: 'Review wait', color: '--cat-6' },
    { key: 'review', label: 'Review', color: '--cat-3' },
    { key: 'qa_wait', label: 'QA wait', color: '--cat-6' },
    { key: 'qa_rework', label: 'QA rework', color: '--cat-5' },
    { key: 'deploy', label: 'Deployment', color: '--cat-2' },
    { key: 'rework', label: 'Rework charged back', color: '--cat-5' },
  ];
  const DEFS = {
    design: 'Only where evidence exists: design/spike/POC/research sub-tasks or tickets, pre-dev Jira activity, a design status. Otherwise “not tracked” (hatched) — never a fake zero.',
    dev: 'In Progress → Code Review. QA time counts only when commits landed on enough distinct days during QA; backlog never counts; commit stretches fill gaps where statuses were not moved.',
    review: 'PR opened → merged. Pickup = PR opened → first review or approval.',
    deploy: 'Merge → first deployment tag containing the change.',
    rework: 'Time charged back to the original ticket when later tickets rewrote its recent code (git blame) or Jira links/titles/reopens point to it.',
  };

  const phaseVal = (v) => {
    if (v === null) return null;
    const m = M(v);
    if (!m) return undefined;
    if (m.tracked === false) return null;
    return m.value;
  };
  function phaseRows(p) {
    const rows = [];
    const add = (label, obj, n) => {
      if (!obj) return;
      const src = obj.phases || obj;
      const parts = {};
      for (const s of SEGS) if (s.key in src) parts[s.key] = phaseVal(src[s.key]);
      if (!('design' in parts)) parts.design = null;
      rows.push({ label: n != null ? `${label} (n=${n})` : label, parts });
    };
    add('Team median', p.team || p.median || p, p.n);
    for (const r of p.by_type || []) add(r.label || r.type, r, r.n);
    return rows;
  }

  function phasesHtml(o) {
    const p = o.phases;
    if (!p) return notAvailable('The phase breakdown');
    const rows = phaseRows(p);
    const used = SEGS.filter((s) => s.hatchWhenNull || rows.some((r) => isNum(r.parts[s.key])));
    const svg = stackedBars({
      rows,
      segs: used,
      title: 'Cycle time by phase',
      desc: `Median working days per phase for ${rows.length} groups. Hatched segments mean the phase was not tracked.`,
    });
    const tbl = table({
      caption: 'Median working days per phase (— = not tracked)',
      cols: [{ label: 'Group' }, ...used.map((s) => ({ label: s.label, num: true }))],
      rows: rows.map((r) => [esc(r.label), ...used.map((s) => (r.parts[s.key] === null ? '<span class="dim">not tracked</span>' : fmtD(r.parts[s.key])))]),
    });
    const legend = legendHtml([...used.map((s) => ({ label: s.label, color: s.color })), { label: 'Not tracked', hatch: true }]);
    const cov = M(p.coverage);
    return chartBlock({ svg, legend, tableHtml: tbl }) + (cov && isNum(cov.value) ? `<p class="dim small">Design evidence found on ${fmtPct(cov.value)} of tickets.</p>` : '');
  }

  function prHtml(o) {
    const pr = o.pr_flow;
    if (!pr) return notAvailable('Pull-request flow (fetched through Otto, paced)');
    const stages = [
      ['Pickup', pr.pickup_days || pr.pickup, 'PR opened → first review or approval'],
      ['Review', pr.review_days || pr.review, 'first review → approval'],
      ['Merge', pr.merge_days || pr.merge, 'approval → merged'],
    ].map(([l, v, d]) => [l, M(v), d]);
    const max = Math.max(0.01, ...stages.map(([, m]) => (m && isNum(m.value) ? m.value : 0)));
    const funnel = `<div class="funnel" role="list" aria-label="PR flow median days">${stages
      .map(
        ([l, m, d]) => `<div class="stage" role="listitem"><span>${esc(l)} <span class="dim small">${esc(d)}</span></span>
          <span class="bar-wrap"><span class="bar" style="display:block;inline-size:${m && isNum(m.value) ? Math.max(1, (m.value / max) * 100) : 0}%"></span></span>
          <span class="num">${m && isNum(m.value) ? fmtD(m.value) : '<span class="na">not available yet</span>'}</span></div>`,
      )
      .join('')}</div>`;
    let size = '';
    const sz = pr.size || pr.pr_size;
    if (sz && Array.isArray(sz.bins) && sz.bins.length) {
      size = `<h3>PR size</h3>${chartBlock({
        svg: histogram({ bins: sz.bins, title: 'PR size distribution', desc: 'Number of pull requests per changed-lines bucket.' }),
        tableHtml: table({ caption: 'PRs per size bucket', cols: [{ label: 'Lines changed' }, { label: 'PRs', num: true }], rows: sz.bins.map((b) => [esc(b.label), String(b.n)]) }),
      })}${isNum(sz.p50) ? `<p class="dim small">Median ${fmtNum(sz.p50, 0)} lines changed.</p>` : ''}`;
    }
    const depth = pr.review_depth || {};
    const depthHtml = `<div class="tiles">
      <div class="tile"><div class="label">Comments per PR ${info({ title: 'Review depth', definition: 'Review comments per merged PR. Very low on large PRs can mean rubber-stamping.', formula: 'Σ comments ÷ merged PRs', quality: depth.quality })}</div><div class="value">${fmtNum(M(depth.comments_per_pr)?.value)}</div></div>
      <div class="tile"><div class="label">Reviewers per PR</div><div class="value">${fmtNum(M(depth.reviewers_per_pr)?.value)}</div></div>
      <div class="tile"><div class="label">PRs merged</div><div class="value">${isNum(pr.n) ? pr.n : '—'}</div><div class="context">${pr.unreviewed_share != null ? `${fmtPct(pr.unreviewed_share)} merged without review` : ''}</div></div>
    </div>`;
    return funnel + size + `<h3>Review depth</h3>` + depthHtml;
  }

  function wipHtml(o) {
    const f = o.flow || {};
    const parts = [];
    const tp = f.throughput && (f.throughput.series || f.throughput);
    if (Array.isArray(tp) && tp.length) {
      parts.push(
        chartBlock({
          svg: columns({ points: tp, title: 'Throughput per week', desc: 'Tickets completed each week.' }),
          tableHtml: table({ caption: 'Throughput per week', cols: [{ label: 'Week' }, { label: 'Done', num: true }], rows: tp.map((p) => [esc(p.label), fmtNum(p.value, 1)]) }),
        }),
      );
    }
    const tiles = [
      ['Avg WIP', M(f.wip)?.value, 'Tickets in progress at the same time, averaged per working day.', 'mean(open in-progress tickets per day)'],
      ['Context switching', M(f.focus || f.context_switching)?.value, 'Distinct tickets a person commits to per working day. Higher means more fragmented focus.', 'mean(distinct keys per person-day with commits)'],
      ['Unplanned share', M(f.unplanned_share)?.value, 'Share of delivered scope that entered after the sprint/period started or is bug/hotfix work.', 'unplanned est-days ÷ delivered est-days'],
    ];
    parts.push(
      `<div class="tiles">${tiles
        .map(([t, v, d, fm]) => `<div class="tile"><div class="label">${esc(t)} ${info({ title: t, definition: d, formula: fm })}</div><div class="value">${isNum(v) ? (t === 'Unplanned share' ? TP.fmtPct(v) : fmtNum(v)) : '<span class="na">not available yet</span>'}</div></div>`)
        .join('')}</div>`,
    );
    return parts.join('');
  }

  function openHtml(o) {
    const rows = o.open_tasks || [];
    if (!rows.length) return '<p class="dim">Nothing in flight in this scope.</p>';
    return table({
      caption: `${rows.length} open tickets with predicted timelines`,
      cols: [{ label: 'Ticket' }, { label: 'Assignee' }, { label: 'Status' }, { label: 'Elapsed', num: true }, { label: 'Estimate', num: true }, { label: 'Expected total', num: true }, { label: 'Consumed' }, { label: 'Projected done', num: true }],
      rows: rows.slice(0, 200).map((t) => [
        `${jiraLink(t.key)} <span class="dim small">${esc((t.summary || '').slice(0, 60))}</span>`,
        esc(t.assignee_name || '—'),
        esc(t.status || ''),
        fmtD((t.design_days || 0) + (t.impl_days || 0)),
        fmtD(t.est_days_ai),
        t.prediction && t.prediction.total ? `${fmtD(t.prediction.total.p50)} <span class="dim">[${fmtD(t.prediction.total.p25)}–${fmtD(t.prediction.total.p75)}]</span>` : '<span class="dim">needs ≥3 similar</span>',
        isNum(t.pct_consumed) ? `${meter(t.pct_consumed, 'share of expected time consumed')} ${TP.fmtPct(t.pct_consumed)}` : '—',
        TP.fmtDate(t.projected_done_at),
      ]),
    });
  }

  TP.views.flow = {
    render(host, { o }) {
      section(host, {
        title: 'Cycle time by phase',
        infoDef: { title: 'Phases', definition: Object.entries(DEFS).map(([k, v]) => `${k}: ${v}`).join(' '), formula: 'per-ticket phase durations in business days, median per group', quality: o.phases && o.phases.quality },
        sub: 'Where the time goes between first work and production.',
        load: async () => phasesHtml(o),
      });
      const g = document.createElement('div');
      g.className = 'grid-2';
      host.appendChild(g);
      section(g, {
        title: 'Pull requests',
        infoDef: { title: 'PR flow', definition: 'Bitbucket PR timings fetched through Otto, paced at ≥2s per call with backoff, cached incrementally.', formula: 'median per stage', quality: o.pr_flow && o.pr_flow.quality },
        load: async () => prHtml(o),
      });
      section(g, { title: 'WIP and throughput', load: async () => wipHtml(o) });
      section(host, { title: 'Open tickets', sub: 'Predictions anchor on the estimate × the person’s pace factor.', load: async () => openHtml(o) });
    },
  };
})();
