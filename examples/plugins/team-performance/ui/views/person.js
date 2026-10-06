// Person drill-down (breadcrumb + Back/Esc live in app.js): capacity-aware
// tiles, phases, goals, completed tickets with an evidence/correction dialog,
// rework charged back, contributions and open work.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, M, fmtD, fmtPct, fmtNum, fmtX, fmtDate, isNum, api, put, section, table, jiraLink, info, badge, modal, toast, sparkline, notAvailable } = TP;

  const GOAL_LABEL = {
    median_cycle_days: 'Median cycle (days)',
    median_impl_days: 'Median implementation (days)',
    median_design_days: 'Median design (days)',
    estimate_mape: 'Estimate error',
    avg_wip: 'Avg parallel tickets',
    weighted_throughput: 'Delivered scope (est-days)',
    efficiency: 'Efficiency (estimate ÷ actual)',
  };
  const PH = [['design', 'Design'], ['dev', 'Dev'], ['review', 'Review'], ['deploy', 'Deploy'], ['rework', 'Rework']];
  const verdict = (v) => (!v ? '—' : badge(v === 'slow' ? 'warning' : 'success', { fast: 'fast', on_track: 'on track', slow: 'slow' }[v] || v));
  // Per-ticket phases arrive as numbers, {value}, or the lib/phases shape
  // ({days} / review {total} / rework {in_days}); null days = not tracked.
  const phaseNum = (k, p) => {
    if (isNum(p)) return p;
    if (!p || typeof p !== 'object') return undefined;
    if (p.tracked === false) return null;
    for (const f of k === 'review' ? ['total', 'days'] : k === 'rework' ? ['in_days', 'days'] : ['days']) if (f in p) return isNum(p[f]) ? p[f] : null;
    const m = M(p);
    return m && isNum(m.value) ? m.value : undefined;
  };
  const NT = `${TP.NT}<span class="sr-only"> not tracked</span>`;
  const phaseCell = (t, k) => {
    const p = t.phases && t.phases[k === 'deploy' && t.phases && !('deploy' in t.phases) ? 'deployment' : k];
    if (p === null) return NT;
    const v = phaseNum(k, p);
    if (v === null) return NT;
    if (isNum(v)) return fmtD(v);
    if (k === 'design') return fmtD(t.design_days);
    if (k === 'dev') return fmtD(t.impl_days_git ?? t.impl_days);
    if (k === 'deploy') return fmtD(t.deploy_wait_days);
    return '—';
  };

  /** Review work this person gave, per capacity day (lib/prs by_person), never a bare count. */
  function prTile(p, cap) {
    if (!p || typeof p !== 'object' || !isNum(p.reviewed_given)) return '';
    const rate = p.per_capacity_day && isNum(p.per_capacity_day.reviewed_given) ? p.per_capacity_day.reviewed_given : null;
    return TP.tile({
      title: 'PRs reviewed',
      valueHtml: String(p.reviewed_given),
      context: `${isNum(p.authored) ? `${p.authored} authored · ` : ''}${rate != null ? `${fmtNum(rate, 2)} reviews per capacity day` : 'per-day rate needs capacity'}`,
      capCtx: TP.capContext(cap),
      def: { definition: 'Pull requests this person reviewed or commented on (not their own). Review work is real work that ticket metrics miss.', formula: 'count(PRs reviewed); ÷ capacity days for the rate' },
    });
  }

  async function saveOverride(app, t, patch) {
    await put('/override', { account: app.account, project: t.project || t.key.split('-')[0], key: t.key, ...patch });
  }

  function taskDialog(app, t, reload) {
    const iv = (t.intervals || [])
      .map((x) => [esc(x.status), esc(x.phase), `${fmtDate(x.from)} → ${fmtDate(x.to)}`, fmtD((x.to - x.from) / 86400000)]);
    const git = [
      t.first_commit_at ? `first commit ${fmtDate(t.first_commit_at)}` : 'no commits found',
      t.done_git_at ? `merged ${fmtDate(t.done_git_at)}` : null,
      t.fix_count ? `${t.fix_count} fix commits (${fmtD(t.fix_days)})` : null,
      t.deployed_at ? `deployed ${fmtDate(t.deployed_at)}` : 'no deployment tag seen',
    ].filter(Boolean).join(' · ');
    const kids = (t.children || []).map((c) => [jiraLink(c.key), esc(c.type), esc((c.summary || '').slice(0, 50)), esc(c.assignee_name || '—'), fmtD(c.actual_days)]);
    const body = `
      <p>${jiraLink(t.key)} ${esc(t.summary || '')}</p>
      ${t.epic_key ? `<p class="dim small">Epic ${jiraLink(t.epic_key)} ${esc((t.epic_summary || '').slice(0, 80))}</p>` : ''}
      <p class="dim small">git: ${esc(git)}${(t.git_authors || []).length ? ` · authors: ${esc(t.git_authors.map((a) => `${a.name}×${a.commits}`).join(', '))}` : ''}</p>
      <fieldset><legend>Correct the estimate</legend>
        <div class="form-grid">
          <label class="field"><span>Estimate (days, person-agnostic)</span><input type="number" id="td-ai" step="0.25" min="0.25" value="${t.est_days_ai ?? ''}"></label>
          <label class="field"><span>Estimate for this person (days)</span><input type="number" id="td-dev" step="0.25" min="0.25" value="${t.est_days_dev ?? ''}"></label>
        </div>
        <label class="field"><span>Why? (calibrates future estimates)</span><input type="text" id="td-reason" value="${esc(t.est_reason || '')}"></label>
        ${t.est_ai_original != null ? `<p class="dim small">Originally ${fmtD(t.est_ai_original)} from the estimator.</p>` : ''}
      </fieldset>
      <fieldset><legend>Actual time</legend>
        <div class="form-grid">
          <label class="field"><span>Override actual (days, blank = measured ${fmtD(t.actual_days)})</span><input type="number" id="td-manual" step="0.1" min="0.1" value="${t.manual_days ?? ''}"></label>
          ${t.fix_count ? `<label class="field"><span>Fix time</span><select id="td-fix"><option value="auto">Automatic${t.include_fixes ? ' (included)' : ' (separate)'}</option><option value="yes" ${t.include_fixes_override === true ? 'selected' : ''}>Include all</option><option value="no" ${t.include_fixes_override === false ? 'selected' : ''}>Exclude</option></select></label>` : ''}
        </div>
        <label class="check"><input type="checkbox" id="td-outlier" ${t.outlier ? 'checked' : ''}> Outlier — leave it out of every statistic</label>
        <label class="check"><input type="checkbox" id="td-excluded" ${t.excluded_override ? 'checked' : ''}> Exclude this ticket</label>
      </fieldset>
      ${kids.length ? `<details><summary>${kids.length} sub-tasks</summary>${table({ caption: 'Sub-tasks', cols: [{ label: 'Key' }, { label: 'Type' }, { label: 'Summary' }, { label: 'Assignee' }, { label: 'Days', num: true }], rows: kids })}</details>` : ''}
      ${iv.length ? `<details><summary>Status timeline</summary>${table({ caption: 'Jira status intervals', cols: [{ label: 'Status' }, { label: 'Phase' }, { label: 'Span' }, { label: 'Days', num: true }], rows: iv })}</details>` : ''}`;
    const m = modal({
      title: `${t.key} — evidence and corrections`,
      body,
      actions: [
        { label: 'Cancel', value: null },
        { label: 'Restore estimator values', value: 'restore' },
        { label: 'Save', value: 'save', primary: true },
      ],
    });
    const num = (id) => {
      const el = m.el.querySelector('#' + id);
      if (!el || el.value.trim() === '') return null;
      const n = parseFloat(el.value);
      return Number.isFinite(n) && n > 0 ? n : null;
    };
    const vals = () => ({
      est_days: num('td-ai'),
      est_dev_days: num('td-dev'),
      est_reason: m.el.querySelector('#td-reason').value.trim() || null,
      manual_days: num('td-manual'),
      outlier: m.el.querySelector('#td-outlier').checked,
      excluded: m.el.querySelector('#td-excluded').checked,
      fix: m.el.querySelector('#td-fix') ? m.el.querySelector('#td-fix').value : null,
    });
    let snapshot;
    m.el.addEventListener('input', () => (snapshot = vals()));
    m.el.addEventListener('change', () => (snapshot = vals()));
    snapshot = vals();
    m.done.then(async (act) => {
      if (!act) return;
      try {
        if (act === 'restore') await saveOverride(app, t, { est_days: null, est_dev_days: null, est_reason: null });
        else {
          const v = snapshot;
          const patch = { est_days: v.est_days, est_dev_days: v.est_dev_days, est_reason: v.est_reason, manual_days: v.manual_days, outlier: v.outlier, excluded: v.excluded };
          if (v.fix) patch.include_fixes = v.fix === 'yes' ? true : v.fix === 'no' ? false : null;
          await saveOverride(app, t, patch);
        }
        toast(`${t.key} updated — metrics recomputed.`, 'success');
        await app.refresh();
        reload();
      } catch (e) {
        toast(`Couldn’t save ${t.key}: ${e.message}`, 'danger');
      }
    });
  }

  TP.views.person = {
    phaseCell,
    render(outer, { app, o }, id) {
      const host = document.createElement('div');
      outer.appendChild(host);
      const name = ((o && o.assignees) || []).find((a) => a.assignee_id === id)?.assignee_name || app.people[id]?.name || id;
      host.insertAdjacentHTML('beforeend', `<h2 class="sr-only">${esc(name)}</h2>`);
      let v = null;
      const load = async () => {
        if (!v) v = await api(`/assignee?${app.scopeQ()}&assignee=${encodeURIComponent(id)}`);
        return v;
      };
      const reload = () => {
        v = null;
        app.render();
      };

      section(host, {
        title: name,
        headerEnd: `<button type="button" class="compact" id="ps-report">${TP.icon('report')}New report</button>`,
        load: async () => {
          const d = await load();
          const s = d.stats || {};
          const row = ((o && o.assignees) || []).find((a) => a.assignee_id === id) || {};
          const cap = d.capacity || s.capacity || (o && o.capacity && o.capacity.people && o.capacity.people[id]) || (isNum(row.capacity_days) ? { capacity_days: row.capacity_days, time_off_days: row.time_off_days, business_days: isNum(row.time_off_days) ? row.capacity_days + row.time_off_days : null } : null);
          const capDays = cap && isNum(cap.capacity_days) ? cap.capacity_days : null;
          const perDay = isNum(row.weighted_per_capacity_day) ? row.weighted_per_capacity_day : capDays && isNum(s.weighted_done) ? s.weighted_done / capDays : null;
          const guard = TP.views.people && TP.views.people.rowGuard ? TP.views.people.rowGuard(o || {}, { ...row, ...s, assignee_id: id, capacity: cap || undefined }) : null;
          const t = (title, val, ctx, def, ids) => TP.tile({ title, valueHtml: val, context: ctx, def, guard: ids ? TP.guardFor(o || {}, ids) : null });
          return `${guard ? `<div class="banner${guard.level === 'bad' ? ' danger' : ''}" role="note">${TP.icon('warn')}<span><b>Read with care:</b> ${esc(guard.msg)}</span></div>` : ''}<div class="tiles">
            ${t('Capacity days', capDays != null ? fmtNum(capDays, 0) : '<span class="na">not available yet</span>', cap && isNum(cap.business_days) ? `${fmtNum(cap.business_days, 0)} working days − ${fmtNum(cap.time_off_days || 0, 0)} off = ${fmtNum(capDays, 0)}` : 'enter time off in Settings → People', { definition: 'Business days minus holidays and this person’s time off. Every rate here is divided by it.', formula: 'business days − time off = capacity' })}
            ${t('Delivered scope', isNum(s.weighted_done) ? `${fmtNum(s.weighted_done)}<small> est-d</small>` : '—', perDay != null ? `${fmtNum(perDay, 2)} est-d per capacity day (over ${fmtNum(capDays, 0)} d)` : 'per-day rate needs capacity', { definition: 'Estimated days delivered, credited by commit share. Not a productivity score: reviews, support, mentoring and design are not in it.', formula: 'Σ estimate × share; ÷ capacity days for the rate' }, ['throughputPerWeek'])}
            ${t('Pace vs estimate', fmtX(s.pace_vs_est), 'actual dev-days ÷ estimated days', { definition: 'Above ×1 = slower than estimated. Independent of capacity.', formula: 'Σ actual dev-days ÷ Σ estimated days' }, ['estimateAccuracy'])}
            ${t('Median cycle', isNum(s.median_cycle) ? fmtD(s.median_cycle) : TP.NT, 'business days, first work → done', null, ['cycleTimeByPhase'])}
            ${t('Estimate error', fmtPct(s.mape), 'median absolute % error', { definition: 'How far estimates were from actuals on this person’s tickets.', formula: 'median(|actual − est| ÷ est)' })}
            ${t('Avg parallel tickets', isNum(s.avg_wip) ? fmtNum(s.avg_wip) : '—', '', null, ['wip'])}
            ${prTile(d.pr_person || d.prs || (s.pr || null), cap)}
          </div><p class="dim small">Not a productivity score — compare only with the capacity beside each rate, and never rank people on it.</p>`;
        },
        after() {
          const b = host.querySelector('#ps-report');
          if (b) b.onclick = () => TP.views.reports.newReport(app, { scope: 'dev', assignee: id, fromPerson: true });
        },
      });

      section(host, {
        title: 'Goals',
        sub: 'Targets are editable; progress updates every scan.',
        load: async () => {
          const d = await load();
          if (!(d.goals || []).length) return '<p class="dim">Goals appear once this person has enough completed tickets.</p>';
          return `<div class="table-wrap" tabindex="0" role="region" aria-label="Goals for ${esc(name)}"><table class="sticky-first"><caption>Goals for ${esc(name)}</caption><thead><tr><th scope="col">Goal</th><th scope="col" class="num">Now</th><th scope="col">Target</th><th scope="col">Status</th><th scope="col">History</th></tr></thead><tbody>${d.goals
            .map(
              (g, i) => `<tr data-metric="${esc(g.metric)}"><th scope="row">${esc(GOAL_LABEL[g.metric] || g.metric)} <span class="dim">${g.dir === 'up' ? '↑ higher is better' : '↓ lower is better'}</span>${g.suggested ? ' ' + badge('', 'suggested') : ''}</th>
              <td class="num">${g.metric === 'estimate_mape' ? fmtPct(g.current) : fmtNum(g.current)}</td>
              <td><label class="sr-only" for="goal-${i}">Target for ${esc(GOAL_LABEL[g.metric] || g.metric)}</label><input id="goal-${i}" class="goal-target" type="number" step="0.1" min="0.1" value="${esc(g.target)}"></td>
              <td>${g.met ? badge('success', 'met') : badge('warning', 'not met')}</td>
              <td>${sparkline((g.history || []).map((h) => h.value), 'goal history') || '—'}</td></tr>`,
            )
            .join('')}</tbody></table></div><div class="form-actions"><button type="button" class="compact" id="goals-save">Save goals</button></div>`;
        },
        after(body) {
          const b = body.querySelector('#goals-save');
          if (!b) return;
          b.onclick = async () => {
            const goals = [...body.querySelectorAll('tr[data-metric]')]
              .map((tr) => ({ metric: tr.dataset.metric, target: parseFloat(tr.querySelector('.goal-target').value) }))
              .filter((g) => Number.isFinite(g.target) && g.target > 0);
            try {
              await put('/goals', { account: app.account, assignee: id, goals });
              toast('Goals saved.', 'success');
              reload();
            } catch (e) {
              toast(`Couldn’t save goals: ${e.message}`, 'danger');
            }
          };
        },
      });

      section(host, {
        title: 'Completed tickets',
        sub: 'Open a ticket’s details for evidence, estimate corrections and outlier controls.',
        load: async () => {
          const d = await load();
          const list = d.completed || [];
          if (!list.length) return '<p class="dim">No completed tickets in this period.</p>';
          return table({
            caption: `${list.length} completed tickets — estimate → actual, by phase`,
            cols: [{ label: 'Ticket' }, { label: 'Type' }, { label: 'Estimate', num: true }, { label: 'Actual', num: true }, ...PH.map(([, l]) => ({ label: l, num: true })), { label: 'Verdict' }, { label: '' }],
            rows: list.map((t, i) => ({
              attrs: t.excluded ? 'class="excluded"' : '',
              cells: [
                `${jiraLink(t.key)}${t.routine ? ' ' + badge('', 'routine') : ''}${t.outlier ? ' ' + badge('danger', 'outlier') : ''}${t.suspect_outlier ? ' ' + badge('warning', 'suspect') : ''}${t.est_overridden ? ' ' + badge('info', 'corrected') : ''}<div class="dim small">${esc((t.summary || '').slice(0, 60))}</div>`,
                `${esc(t.type || '')}${t.points != null ? ` · ${t.points}pt` : ''} ${badge(t.timing_source === 'git' ? 'accent' : '', t.timing_source || 'jira')}`,
                isNum(t.est_days_ai) ? fmtD(t.est_days_ai) : TP.unestimated(),
                `${fmtD(t.actual_days)}${t.manual_days != null ? ' ' + badge('info', 'manual') : ''}`,
                ...PH.map(([k]) => phaseCell(t, k)),
                verdict(t.verdicts && t.verdicts.total),
                `<button type="button" class="compact" data-task="${i}" aria-label="Details for ${esc(t.key)}" title="Details for ${esc(t.key)}">Details</button>`,
              ],
            })),
          });
        },
        after(body) {
          body.querySelectorAll('[data-task]').forEach((b) => (b.onclick = () => taskDialog(app, v.completed[+b.dataset.task], reload)));
        },
      });

      section(host, {
        title: 'Sub-tasks with real work',
        infoDef: { title: 'Sub-tasks with real work', definition: 'Most sub-tasks are checklists and roll into their story. These carried real work — own commits or dev time, under someone else’s story, or under a story with no timing of its own — so they are credited to this person.', formula: 'commits > 0 OR dev time > 0 OR owner ≠ story owner OR story untimed' },
        sub: 'Each row says why it counts and links to Jira.',
        load: async () => {
          const d = await load();
          const all = d.subtasks || (o && o.subtasks);
          if (!all) return notAvailable('Sub-task attribution');
          const mine = all.filter((x) => (x.credited_to || x.assignee_id) === id);
          if (!mine.length) return `<p class="dim">No sub-tasks with real work credited to this person in this period — ${all.length} substantive sub-task${all.length === 1 ? '' : 's'} checked across the team.</p>`;
          return TP.creditedSubtasks ? TP.creditedSubtasks(o || {}, mine, { caption: `${mine.length} credited sub-tasks`, openable: false }) : '';
        },
      });

      section(host, {
        title: 'Rework',
        sub: 'Time charged back to this person’s tickets, and rework they did on others’.',
        load: async () => {
          const d = await load();
          const r = d.rework;
          if (!r) return notAvailable('Rework for this person');
          const list = r.charged || r.tickets || r.items || [];
          if (!list.length) return '<p class="dim">No rework linked to this person’s tickets.</p>';
          return table({
            caption: 'Rework links',
            cols: [{ label: 'Original' }, { label: 'Rework ticket' }, { label: 'Signal' }, { label: 'Days', num: true }],
            rows: list.map((x) => [jiraLink(x.original_key || x.rework_of || x.key), x.rework_key ? jiraLink(x.rework_key) : x.rework_of ? jiraLink(x.key) : '—', esc(x.reason || x.source || (x.signals || []).map((g) => (typeof g === 'string' ? g : g.reason || g.kind)).join(', ')), fmtD(x.days)]),
          });
        },
      });

      section(host, {
        title: 'Contributions to others’ tickets',
        load: async () => {
          const d = await load();
          const c = d.contributions || [];
          if (!c.length) return '<p class="dim">No commits on other people’s tickets.</p>';
          return table({
            caption: 'Credited by commit share',
            cols: [{ label: 'Ticket' }, { label: 'Owner' }, { label: 'Share', num: true }, { label: 'Actual', num: true }],
            rows: c.map((x) => [`${jiraLink(x.key)} <span class="dim small">${esc((x.summary || '').slice(0, 60))}</span>`, esc(x.assignee_name || '—'), `${Math.round(x.share * 100)}% · ${x.commits} commits`, fmtD(x.actual_days)]),
          });
        },
      });

      section(host, {
        title: 'Open tickets',
        load: async () => {
          const d = await load();
          const list = d.open || [];
          if (!list.length) return '<p class="dim">Nothing in flight.</p>';
          return table({
            caption: 'In progress, with predictions',
            cols: [{ label: 'Ticket' }, { label: 'Status' }, { label: 'Elapsed', num: true }, { label: 'Estimate', num: true }, { label: 'Expected', num: true }, { label: 'Projected', num: true }],
            rows: list.map((t) => [jiraLink(t.key), esc(t.status), fmtD((t.design_days || 0) + (t.impl_days || 0)), fmtD(t.est_days_ai), t.prediction ? fmtD(t.prediction.total.p50) : '—', fmtDate(t.projected_done_at)]),
          });
        },
      });
    },
  };
})();
