// Flow: cycle time split by phase (hatched = not tracked), PR pickup →
// review → merge funnel, PR size, review depth, WIP/throughput, open tickets.
// A phase with no evidence (or fewer than MIN_N tickets) is "not tracked",
// never a zero; the PR section explains WHY it is empty and what to do next.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, M, fmtD, fmtNum, fmtPct, isNum, api, section, notAvailable, table, legendHtml, stackedBars, histogram, columns, jiraLink, info, meter } = TP;
  const MIN_N = 5;
  const chartBlock = (o) => (TP.openTablesOnNarrow ? TP.openTablesOnNarrow(TP.chartBlock(o)) : TP.chartBlock(o));
  const share01 = (v) => (isNum(v) ? (v > 1 ? v / 100 : v) : null);

  // Canonical phase order; finer splits (review wait vs in review, QA wait vs
  // QA rework) render when the data carries them. `src` = phaseSummary field.
  const SEGS = [
    { key: 'design', src: ['design'], label: 'Design', color: '--cat-4' },
    { key: 'dev', src: ['dev'], label: 'Dev', color: '--cat-1' },
    { key: 'review_wait', src: ['review_pickup', 'review_wait'], label: 'Review wait', color: '--cat-6' },
    { key: 'review', src: ['review_in_review', 'review'], fallback: 'review_total', label: 'Review', color: '--cat-3' },
    { key: 'qa_wait', src: ['qa_wait'], label: 'QA wait', color: '--cat-6' },
    { key: 'qa_rework', src: ['qa_rework'], label: 'QA rework', color: '--cat-5' },
    { key: 'deploy', src: ['deploy'], label: 'Deployment', color: '--cat-2' },
    { key: 'rework', src: ['rework_in', 'rework'], label: 'Rework charged back', color: '--cat-5' },
  ].map((s) => ({ ...s, hatchWhenNull: true }));
  const DEFS = {
    design: 'Only where evidence exists: design/spike/POC/research sub-tasks or tickets, pre-dev Jira activity, a design status. Otherwise “not tracked” (hatched) — never a fake zero.',
    dev: 'In Progress → Code Review. QA time counts only when commits landed on enough distinct days during QA; backlog never counts; commit stretches fill gaps where statuses were not moved.',
    review: 'PR opened → merged. Pickup = PR opened → first review or approval. Without PR data it comes from Jira review statuses.',
    deploy: 'Merge → first deployment tag containing the change.',
    rework: 'Time charged back to the original ticket when later tickets rewrote its recent code (git blame) or Jira links/titles/reopens point to it.',
  };

  const hasPrs = (o) => {
    const pr = o && o.pr_flow;
    if (!pr) return false;
    if (isNum(pr.total)) return pr.total > 0;
    if (isNum(pr.n)) return pr.n > 0;
    const p = M(pr.pickup_days);
    return !!(p && isNum(p.n) && p.n > 0);
  };

  /** One phase value: number, or null = NOT TRACKED (no evidence / n < MIN_N), undefined = absent. */
  function phaseVal(v) {
    if (v === null) return null;
    if (v === undefined) return undefined;
    if (isNum(v)) return v;
    if (typeof v !== 'object') return undefined;
    if (v.tracked === false) return null;
    if (isNum(v.n) && v.n < MIN_N) return null;
    const m = M(v);
    return m && isNum(m.value) ? m.value : null;
  }
  const partOf = (src, seg) => {
    for (const k of seg.src) if (k in src) return phaseVal(src[k]);
    if (seg.fallback && seg.fallback in src) return phaseVal(src[seg.fallback]);
    return undefined;
  };

  /** Tickets behind the phase summary: explicit total, else derived from n ÷ coverage. */
  function totalTickets(p) {
    if (isNum(p.n_tickets)) return p.n_tickets;
    if (isNum(p.total)) return p.total;
    if (isNum(p.n)) return p.n;
    let best = null;
    for (const v of Object.values(p)) {
      if (!v || typeof v !== 'object' || !isNum(v.n)) continue;
      const c = share01(v.coverage);
      if (c && c > 0) best = Math.max(best || 0, Math.round(v.n / c));
    }
    return best;
  }

  function phaseRows(p, o) {
    const rows = [];
    const add = (label, obj, n) => {
      if (!obj) return;
      const src = obj.phases || obj;
      const parts = {};
      for (const s of SEGS) {
        const v = partOf(src, s);
        if (v !== undefined) parts[s.key] = v;
      }
      if (!('design' in parts)) parts.design = null;
      rows.push({ label: n != null ? `${label} (n=${n})` : label, parts });
    };
    add('Team median', p.team || p.median || p, p.team || p.median ? p.n : totalTickets(p));
    for (const r of p.by_type || []) add(r.label || r.type, r, r.n);
    const review = hasPrs(o) ? null : 'Review (from Jira statuses)';
    return { rows, segs: SEGS.map((s) => (review && s.key === 'review' ? { ...s, label: review } : review && s.key === 'review_wait' ? { ...s, label: 'Review wait (from Jira statuses)' } : s)) };
  }

  function coverageNotes(p) {
    const N = totalTickets(p);
    const notes = [];
    const d = p.design;
    if (d && typeof d === 'object' && isNum(d.n) && isNum(N)) notes.push(`Design: tracked on ${d.n} of ${N} tickets${d.n < MIN_N ? ' — too few to show a median, so it reads “not tracked”' : ''}.`);
    else {
      const cov = M(p.coverage);
      if (cov && isNum(cov.value)) notes.push(`Design evidence found on ${fmtPct(share01(cov.value))} of tickets.`);
    }
    const q = p.qa_rework;
    if (q && typeof q === 'object' && isNum(N)) {
      const withRework = isNum(q.tickets_with_rework) ? q.tickets_with_rework : isNum(p.qa_rework_tickets) ? p.qa_rework_tickets : isNum(q.n) ? q.n : null;
      if (withRework != null) notes.push(`Tickets with any QA rework: ${withRework} of ${N}.`);
    }
    return notes;
  }

  function phasesHtml(o) {
    const p = o.phases;
    if (!p) return notAvailable('The phase breakdown');
    const { rows, segs } = phaseRows(p, o);
    const used = segs.filter((s) => s.key === 'design' || rows.some((r) => r.parts[s.key] !== undefined));
    const svg = stackedBars({
      rows,
      segs: used,
      title: 'Cycle time by phase',
      desc: `Median working days per phase for ${rows.length} groups. Hatched segments mean the phase was not tracked.`,
    });
    const tbl = table({
      caption: 'Median working days per phase (not tracked = no evidence or fewer than 5 tickets)',
      cols: [{ label: 'Group' }, ...used.map((s) => ({ label: s.label, num: true }))],
      rows: rows.map((r) => [esc(r.label), ...used.map((s) => (r.parts[s.key] == null ? '<span class="dim">not tracked</span>' : fmtD(r.parts[s.key])))]),
    });
    const legend = legendHtml([...used.map((s) => ({ label: s.label, color: s.color })), { label: 'Not tracked', hatch: true }]);
    const notes = coverageNotes(p);
    return chartBlock({ svg, legend, tableHtml: tbl }) + (notes.length ? `<ul class="dim small">${notes.map((n) => `<li>${esc(n)}</li>`).join('')}</ul>` : '');
  }

  // ---- pull requests --------------------------------------------------------

  function prHtml(o) {
    const pr = o.pr_flow;
    const stages = [
      ['Pickup', pr.pickup_days || pr.pickup, 'PR opened → first review or approval'],
      ['Review', pr.review_days || pr.review, 'first review → approval'],
      ['Merge', pr.merge_lag_days || pr.merge_days || pr.merge, 'approval → merged'],
    ].map(([l, v, d]) => [l, M(v), d]);
    const max = Math.max(0.01, ...stages.map(([, m]) => (m && isNum(m.value) ? m.value : 0)));
    const funnel = `<div class="funnel" role="list" aria-label="PR flow median days">${stages
      .map(
        ([l, m, d]) => `<div class="stage" role="listitem"><span>${esc(l)} <span class="dim small">${esc(d)}</span></span>
          <span class="bar-wrap"><span class="bar" style="display:block;inline-size:${m && isNum(m.value) ? Math.max(1, (m.value / max) * 100) : 0}%"></span></span>
          <span class="num">${m && isNum(m.value) ? fmtD(m.value) + (isNum(m.n) ? ` <span class="dim small">n=${m.n}</span>` : '') : '<span class="na">not tracked</span>'}</span></div>`,
      )
      .join('')}</div>`;
    let size = '';
    const sz = pr.size || pr.pr_size;
    const bins = sz && Array.isArray(sz.bins) ? sz.bins : pr.size_buckets ? Object.entries(pr.size_buckets).map(([label, n]) => ({ label, n })) : [];
    if (bins.some((b) => b.n)) {
      const p50 = sz && (isNum(sz.p50) ? sz.p50 : null);
      size = `<h3>PR size</h3>${chartBlock({
        svg: histogram({ bins, title: 'PR size distribution', desc: 'Number of pull requests per changed-lines bucket.' }),
        tableHtml: table({ caption: 'PRs per size bucket', cols: [{ label: 'Lines changed' }, { label: 'PRs', num: true }], rows: bins.map((b) => [esc(b.label), String(b.n)]) }),
      })}${isNum(p50) ? `<p class="dim small">Median ${fmtNum(p50, 0)} lines changed.</p>` : ''}`;
    }
    const depth = pr.review_depth || {};
    const dm = M(depth.comments_per_pr || depth);
    const un = pr.unreviewed || {};
    const unShare = isNum(pr.unreviewed_share) ? pr.unreviewed_share : un.share;
    const depthHtml = `<div class="tiles">
      <div class="tile"><div class="label">Review depth ${info({ title: 'Review depth', definition: 'Review comments per merged PR (median). Very low on large PRs can mean rubber-stamping.', formula: 'median(comments per merged PR)', quality: depth.quality })}</div><div class="value">${dm && isNum(dm.value) ? fmtNum(dm.value) : '<span class="na">not tracked</span>'}</div><div class="context">comments per PR</div></div>
      <div class="tile"><div class="label">PRs merged</div><div class="value">${isNum(pr.total) ? pr.total : isNum(pr.n) ? pr.n : '—'}</div><div class="context">${isNum(unShare) ? `${fmtPct(unShare)} merged without review` : ''}</div></div>
    </div>`;
    const approx = isNum(pr.approximated_times) && pr.approximated_times ? `<p class="dim small">${pr.approximated_times} PR timings approximated from commits or last update.</p>` : '';
    return funnel + size + '<h3>Review depth</h3>' + depthHtml + approx;
  }

  /**
   * First-class empty state for PR data, driven by GET /prs/status (the paced
   * Otto-daemon ingestion). Always: the reason, the next step, and — while a
   * sync runs — paced progress (≥2 s between calls, backoff on 429).
   */
  function prEmptyHtml(s, app) {
    const scanning = app && app.scan && app.scan.state === 'running';
    const repos = Array.isArray(s.repos) ? s.repos : [];
    const failed = repos.filter((r) => r && r.ok === false);
    const unreg = s.unregistered || [];
    let title, reason, action;
    if (s.state === 'running' || scanning) {
      title = 'Fetching pull requests through Otto';
      const next = isNum(s.next_call_in_ms) && s.next_call_in_ms > 0 ? ` · next call in ${TP.fmtSecs(s.next_call_in_ms)}` : '';
      const back = s.pacer && s.pacer.last_backoff_ms ? ` · backing off ${TP.fmtSecs(s.pacer.last_backoff_ms)} after a rate limit` : '';
      const calls = s.pacer && isNum(s.pacer.calls) ? ` · ${s.pacer.calls} calls so far` : '';
      reason = `Paced at ≥2 s per call so Bitbucket isn’t rate-limited; results are cached so the next run only fetches what changed${calls}${next}${back}.`;
      action = { label: 'Refresh status', act: 'refresh' };
    } else if (s.available === false || s.state === 'unavailable') {
      title = 'Pull-request data is unavailable';
      reason = s.error || 'The plugin can’t reach Otto’s git API.';
      action = { label: 'Open Settings', act: 'settings' };
    } else if (unreg.length) {
      title = 'Repos are not registered in Otto';
      reason = `${unreg.length} scanned repo${unreg.length > 1 ? 's are' : ' is'} not registered in Otto’s Git module, so their PRs can’t be fetched: ${unreg.slice(0, 4).join(', ')}${unreg.length > 4 ? '…' : ''}. Add them in Otto → Git (with a bound git account), then scan again.`;
      action = { label: 'Scan again', act: 'scan' };
    } else if (failed.length) {
      title = 'The last PR sync failed';
      reason = failed.slice(0, 3).map((r) => `${r.id || r.repo || 'repo'}: ${r.error || `HTTP ${r.status}`}`).join(' · ');
      action = { label: 'Scan again', act: 'scan' };
    } else if (!s.at) {
      title = 'No pull-request sync has run yet';
      reason = 'The next scan fetches PRs, reviews and comments through Otto, paced and incremental.';
      action = { label: 'Scan now', act: 'scan' };
    } else {
      title = 'No merged pull requests in this period';
      reason = `The last sync (${TP.fmtAgo(s.at)}) found none that merged in this period. Widen the period, or check the repo list in Settings.`;
      action = { label: 'Open Settings', act: 'settings' };
    }
    const btn = action.act === 'scan' && scanning ? '' : `<button type="button" class="compact" data-pr-act="${action.act}">${esc(action.label)}</button>`;
    return `<div class="empty compact" role="status">
      <h3>${esc(title)}</h3>
      <p class="dim">${esc(reason)}</p>
      <p class="dim small">Until PRs arrive, review time comes from Jira review statuses and pickup / size / depth are not tracked.</p>
      ${btn}
    </div>`;
  }

  async function prSection(o, app) {
    if (hasPrs(o)) return prHtml(o);
    const s = await api('/prs/status');
    return prEmptyHtml(s || {}, app);
  }

  // ---- WIP / throughput -------------------------------------------------------

  const numOf = (x) => {
    const m = M(x);
    if (m && isNum(m.value)) return m.value;
    const v = x && x.value;
    if (v && typeof v === 'object') for (const k of ['team', 'mean', 'avg', 'value', 'share']) if (isNum(v[k])) return v[k];
    return null;
  };

  function wipHtml(o) {
    const f = o.flow || {};
    const parts = [];
    const tpx = f.throughput || f.throughputPerWeek;
    const tp = tpx && (tpx.series || (tpx.value && tpx.value.series) || tpx);
    if (Array.isArray(tp) && tp.length) {
      parts.push(
        chartBlock({
          svg: columns({ points: tp, title: 'Throughput per week', desc: 'Tickets completed each week.' }),
          tableHtml: table({ caption: 'Throughput per week', cols: [{ label: 'Week' }, { label: 'Done', num: true }], rows: tp.map((p) => [esc(p.label), fmtNum(p.value, 1)]) }),
        }),
      );
    }
    const guard = (ids) => (TP.guardFor ? TP.guardFor(o, ids) : null);
    const tiles = [
      ['Avg WIP', numOf(f.wip), 'Tickets in progress at the same time, averaged per working day.', 'mean(open in-progress tickets per day)', ['wip'], false],
      ['Context switching', numOf(f.focus || f.context_switching || f.contextSwitching), 'Distinct tickets a person commits to per working day. Higher means more fragmented focus.', 'mean(distinct keys per person-day with commits)', ['contextSwitching'], false],
      ['Unplanned share', numOf(f.unplanned_share || f.unplannedShare), 'Share of delivered scope that entered after the sprint/period started or is bug/hotfix work.', 'unplanned est-days ÷ delivered est-days', ['unplannedShare'], true],
    ];
    parts.push(
      `<div class="tiles">${tiles
        .map(([t, v, d, fm, ids, pct]) => {
          const g = guard(ids);
          return `<div class="tile"><div class="label">${esc(t)} ${info({ title: t, definition: d, formula: fm, quality: g })}</div><div class="value">${isNum(v) ? (pct ? fmtPct(share01(v)) : fmtNum(v)) : '<span class="na">not available yet</span>'}</div>${g && TP.guardBadge ? `<div class="row">${TP.guardBadge(g)}</div>` : ''}</div>`;
        })
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
        isNum(t.pct_consumed) ? `${meter(t.pct_consumed, 'share of expected time consumed')} ${fmtPct(t.pct_consumed)}` : '—',
        TP.fmtDate(t.projected_done_at),
      ]),
    });
  }

  TP.views.flow = {
    phasesHtml,
    prEmptyHtml,
    prHtml,
    wipHtml,
    hasPrs,
    render(outer, { o, app }) {
      const host = document.createElement('div');
      outer.appendChild(host);
      const phaseGuard = TP.guardFor ? TP.guardFor(o, ['cycleTimeByPhase']) : null;
      section(host, {
        title: 'Cycle time by phase',
        infoDef: { title: 'Phases', definition: Object.entries(DEFS).map(([k, v]) => `${k}: ${v}`).join(' '), formula: 'per-ticket phase durations in business days, median per group (phases with < 5 tickets: not tracked)', quality: phaseGuard || (o.phases && o.phases.quality) },
        headerEnd: phaseGuard && TP.guardBadge ? TP.guardBadge(phaseGuard) : '',
        sub: 'Where the time goes between first work and production.',
        load: async () => phasesHtml(o),
      });
      const g = document.createElement('div');
      g.className = 'grid-2';
      host.appendChild(g);
      section(g, {
        title: 'Pull requests',
        infoDef: { title: 'PR flow', definition: 'Bitbucket PR timings fetched through Otto, paced at ≥2 s per call with backoff on 429, cached incrementally.', formula: 'median per stage', quality: (TP.guardFor && TP.guardFor(o, ['prPickup'])) || (o.pr_flow && o.pr_flow.quality) },
        load: async () => prSection(o, app),
        after(body, rerun) {
          body.querySelectorAll('[data-pr-act]').forEach((b) => {
            b.onclick = () => {
              const act = b.dataset.prAct;
              if (act === 'refresh') return rerun();
              if (act === 'scan' && app.startScan) return app.startScan(false);
              if (act === 'settings') return app.goTab('settings');
            };
          });
        },
      });
      section(g, { title: 'WIP and throughput', load: async () => wipHtml(o) });
      section(host, { title: 'Open tickets', sub: 'Predictions anchor on the estimate × the person’s pace factor.', load: async () => openHtml(o) });
    },
  };
})();
