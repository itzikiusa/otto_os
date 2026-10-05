// Estimates: accuracy histogram, calibration from lead corrections, the
// correct-estimate flow, estimate inflation and the "how long things take" guide.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, M, fmtD, fmtPct, fmtX, isNum, put, section, table, histogram, jiraLink, badge, modal, toast, notAvailable } = TP;
  // Below 600px the data table opens by default (see TP.openTablesOnNarrow).
  const chartBlock = (o) => (TP.openTablesOnNarrow ? TP.openTablesOnNarrow(TP.chartBlock(o)) : TP.chartBlock(o));

  function correct(app, t) {
    const m = modal({
      title: `Correct the estimate for ${t.key}`,
      body: `<p class="dim">${esc(t.summary || '')}</p>
        <label class="field"><span>Estimate (ideal days, person-agnostic)</span><input type="number" id="ce-days" step="0.25" min="0.25" value="${t.est_days ?? t.est_days_ai ?? ''}"></label>
        <label class="field"><span>Why? Fed back into the estimator as calibration.</span><input type="text" id="ce-reason" placeholder="e.g. Included a migration the description didn’t mention"></label>
        <p class="dim small">Measured actual: ${fmtD(t.actual_days)}. Corrections change scope-weighted metrics on the next recompute.</p>`,
      actions: [{ label: 'Cancel', value: null }, { label: 'Save correction', value: 'save', primary: true }],
    });
    let days = null, reason = '';
    m.el.addEventListener('input', () => {
      days = parseFloat(m.el.querySelector('#ce-days').value);
      reason = m.el.querySelector('#ce-reason').value.trim();
    });
    m.done.then(async (act) => {
      if (act !== 'save') return;
      if (!Number.isFinite(days) || days <= 0) {
        toast('Enter a positive number of days.', 'danger');
        return;
      }
      try {
        await put('/override', { account: app.account, project: t.project || t.key.split('-')[0], key: t.key, est_days: days, est_reason: reason || null });
        toast(`Estimate for ${t.key} corrected.`, 'success');
        app.refresh();
      } catch (e) {
        toast(`Couldn’t save: ${e.message}`, 'danger');
      }
    });
  }

  /** Each calibration signal: definition, formula and which direction is better. */
  const CAL = {
    corrections: { title: 'Lead corrections used', definition: 'Estimates a lead corrected with a reason. They calibrate future estimates for similar tickets.', formula: 'count(corrections with a reason)', direction: 'More is better: more ground truth.' },
    factor: { title: 'Calibration factor', definition: 'Multiplier learned from lead corrections and applied to raw estimator output.', formula: 'median(corrected ÷ original estimate)', direction: 'Closer to ×1 is better: the estimator already agrees with leads.' },
    ruler: { title: 'Ruler version', definition: 'Version of the estimation ruler (prompt, reference tickets, scale). Estimates from different rulers are not mixed in one comparison.', formula: 'stable hash of the ruler inputs', direction: 'Compare periods only on the same ruler.' },
    pace: { title: 'Pace', definition: 'Actual dev working days per estimated ideal day, on estimated tickets only.', formula: 'Σ actual dev-days ÷ Σ estimate', direction: 'Closer to ×1 is better; above ×1 = slower than estimated.' },
    inflation: { title: 'Estimate inflation', definition: 'How much bigger estimates got for comparable tickets vs the previous period. Above ×1 makes the raw pace look better than it is.', formula: 'median estimate per size bucket ÷ previous period', direction: 'Closer to ×1 is better.' },
    adjusted: { title: 'Pace, inflation-adjusted', definition: 'Pace with estimate inflation removed, so periods are comparable even when estimates drift.', formula: 'pace × estimate inflation', direction: 'Closer to ×1 is better; trend this when inflation is above ×1.1.' },
  };
  const calCell = (k) => `${esc(CAL[k].title)} ${TP.info(CAL[k])}`;

  /** Calibration table; when inflation > 1.1 the adjusted pace is the headline above it. */
  function calibrationHtml(acc, eb) {
    const c = acc && acc.calibration;
    const rows = [];
    if (c) {
      rows.push([calCell('corrections'), isNum(c.corrections) ? String(c.corrections) : '—']);
      rows.push([calCell('factor'), fmtX(c.factor)]);
      if (c.ruler_version) rows.push([calCell('ruler'), esc(c.ruler_version)]);
    }
    const inflated = isNum(eb.est_inflation) && eb.est_inflation > 1.1 && isNum(eb.pace_adjusted);
    const adjRow = [calCell('adjusted'), `${fmtX(eb.pace_adjusted)} <span class="dim">prev ${fmtX(eb.pace_ref)}</span>`];
    if (inflated) rows.push(adjRow);
    rows.push([calCell('pace'), fmtX(eb.pace)]);
    rows.push([calCell('inflation'), fmtX(eb.est_inflation)]);
    if (!inflated) rows.push(adjRow);
    const head = inflated
      ? `<div class="headline"><span class="value">${fmtX(eb.pace_adjusted)}</span><span>pace, inflation-adjusted — estimates grew ${fmtX(eb.est_inflation)} vs the previous period, so the raw ${fmtX(eb.pace)} flatters the team.</span></div>`
      : '';
    return head + table({ caption: 'How estimates are calibrated', cols: [{ label: 'Signal' }, { label: 'Value', num: true }], rows }) +
      `<p class="dim small">When estimate inflation is above ×1.1, compare periods on the adjusted pace, not the raw one.</p>`;
  }

  TP.views.estimates = {
    calibrationHtml,
    render(host, { o, app }) {
      const acc = o.estimate_accuracy || (o.estimates && o.estimates.accuracy);
      const accGuard = TP.guardFor(o, ['estimateAccuracy'], acc);
      section(host, {
        title: 'Estimate accuracy',
        headerEnd: TP.guardBadge(accGuard),
        infoDef: {
          title: 'Accuracy',
          definition: 'Distribution of actual ÷ estimate per completed, estimated ticket. ×1 = spot on. Consensus median of several estimators, versioned by ruler.',
          formula: 'actual working days ÷ estimate',
          quality: accGuard || (acc && acc.quality),
        },
        load: async () => {
          if (!acc || !Array.isArray(acc.bins)) return notAvailable('The accuracy distribution');
          const within = M(acc.within_25 ?? acc.within);
          return TP.guardReason(accGuard) + (
            chartBlock({
              svg: histogram({ bins: acc.bins, title: 'Actual ÷ estimate distribution', desc: `Completed tickets per accuracy bucket, n=${acc.n ?? '?'}.` }),
              tableHtml: table({ caption: 'Tickets per accuracy bucket', cols: [{ label: 'Actual ÷ estimate' }, { label: 'Tickets', num: true }], rows: acc.bins.map((b) => [esc(b.label), String(b.n)]) }),
            }) + (within && isNum(within.value) ? `<p>${badge(within.value >= 0.5 ? 'success' : 'warning', TP.fmtShare(within.value) + ' within ±25%')}</p>` : '')
          );
        },
      });

      const g = document.createElement('div');
      g.className = 'grid-2';
      host.appendChild(g);
      section(g, {
        title: 'Calibration',
        load: async () => calibrationHtml(acc, o.est_basis || {}),
      });
      section(g, {
        title: 'Biggest misses',
        infoDef: { title: 'Correct-estimate flow', definition: 'Tickets whose actual time was furthest from the estimate. Correcting them with a reason teaches the estimator.', formula: '|log(actual ÷ estimate)| descending' },
        load: async () => {
          const list = (acc && acc.worst) || [];
          if (!list.length) return acc ? '<p class="dim">No large misses this period.</p>' : notAvailable('The miss list');
          return table({
            caption: 'Largest estimate misses',
            cols: [{ label: 'Ticket' }, { label: 'Estimate', num: true }, { label: 'Actual', num: true }, { label: 'Ratio', num: true }, { label: '' }],
            rows: list.slice(0, 30).map((t, i) => [
              `${jiraLink(t.key)} <span class="dim small">${esc(t.assignee_name || '')}</span>`,
              isNum(t.est_days ?? t.est_days_ai) ? fmtD(t.est_days ?? t.est_days_ai) : TP.unestimated(),
              fmtD(t.actual_days),
              isNum(t.ratio) ? fmtX(t.ratio) : '—',
              `<button type="button" class="compact" data-fix="${i}" aria-label="Correct estimate for ${esc(t.key)}" title="Correct estimate for ${esc(t.key)}">Correct</button>`,
            ]),
          });
        },
        after(body) {
          body.querySelectorAll('[data-fix]').forEach((b) => (b.onclick = () => correct(app, acc.worst[+b.dataset.fix])));
        },
      });

      section(host, {
        title: 'How long tickets actually take here',
        sub: 'Median [p25–p75] working days per type and size, from completed tickets.',
        load: async () => {
          const b = o.baseline || [];
          if (!b.length) return '<p class="dim">Appears once a type/size bucket has at least two completed tickets.</p>';
          const r = (s) => (s && s.p50 != null ? `${fmtD(s.p50)} <span class="dim">[${fmtD(s.p25)}–${fmtD(s.p75)}]</span>` : TP.NT);
          return table({
            caption: 'Estimation guide',
            cols: [{ label: 'Type' }, { label: 'Points', num: true }, { label: 'n', num: true }, { label: 'Design', num: true }, { label: 'Implementation', num: true }, { label: 'Total', num: true }],
            rows: b.map((x) => [esc(x.type), x.points != null ? esc(String(x.points)) : TP.unestimated(), String(x.n), r(x.design), r(x.impl), r(x.total)]),
          });
        },
      });
    },
  };
})();
