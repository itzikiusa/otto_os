// People: one row per person, every ratio next to the capacity it was earned
// in (business days minus holidays and entered time off). Default sort is by
// name — this is not a leaderboard. Plus substantive sub-tasks.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, fmtD, fmtPct, fmtNum, fmtX, isNum, section, table, jiraLink, info, badge, icon, store } = TP;

  /** {business_days, time_off_days, capacity_days} for a row — row fields first, then the team capacity map. */
  const capOf = (o, a) => {
    if (a.capacity) return a.capacity;
    const team = o.capacity && o.capacity.people && o.capacity.people[a.assignee_id];
    if (team) return team;
    if (isNum(a.capacity_days)) return { capacity_days: a.capacity_days, time_off_days: a.time_off_days, business_days: isNum(a.business_days) ? a.business_days : isNum(a.time_off_days) ? a.capacity_days + a.time_off_days : null };
    return null;
  };

  /** Row guardrail: person-level entries, then low capacity / small sample. → {level, msg} or null. */
  function rowGuard(o, a) {
    const msgs = [];
    let level = null;
    const bump = (l, m) => {
      msgs.push(m);
      level = level === 'bad' || l === 'bad' ? 'bad' : 'warn';
    };
    for (const g of a.guardrails || []) if (g && g.level !== 'ok') bump(g.level === 'bad' || g.severity === 'danger' ? 'bad' : 'warn', g.msg || g.reason || '');
    const own = TP.guardFor ? TP.guardFor(o, [`person:${a.assignee_id}`]) : null;
    if (own) bump(own.level, own.msg);
    const cap = capOf(o, a);
    if (cap && isNum(cap.capacity_days) && isNum(cap.business_days) && cap.business_days > 0 && cap.capacity_days / cap.business_days < 0.6) {
      bump(cap.capacity_days === 0 ? 'bad' : 'warn', `Available ${fmtNum(cap.capacity_days, 0)} of ${fmtNum(cap.business_days, 0)} working days — totals are naturally lower.`);
    }
    if (!cap || !isNum(cap.capacity_days)) bump('warn', 'No capacity figure — ratios on this row cannot be compared.');
    if (isNum(a.completed) && a.completed < 5) bump('warn', `Only ${a.completed} completed ticket${a.completed === 1 ? '' : 's'} — one outlier moves every rate.`);
    return level ? { level, msg: msgs.filter(Boolean).join(' ') } : null;
  }
  const capCell = (cap) =>
    cap && isNum(cap.capacity_days)
      ? `${fmtNum(cap.capacity_days, 0)}${isNum(cap.business_days) ? ` <span class="dim small">= ${fmtNum(cap.business_days, 0)} − ${fmtNum(cap.time_off_days || 0, 0)} off</span>` : ''}`
      : '<span class="dim">not available yet</span>';

  const COLS = [
    { key: 'name', label: 'Person', sort: true },
    { key: 'role', label: 'Role', sort: true },
    {
      key: 'cap',
      label: 'Capacity days',
      num: true,
      sort: true,
      info: { title: 'Capacity days', definition: 'Working days in the period minus holidays and the person’s entered time off (Settings → People). Every rate on this row is divided by it.', formula: 'business days − time off = capacity' },
    },
    { key: 'delivered', label: 'Delivered est-d', num: true, sort: true, info: { title: 'Delivered scope', definition: 'Estimated ideal days delivered, credited by commit share on shared tickets. Rework is not new scope.', formula: 'Σ estimate × commit share' } },
    { key: 'per_day', label: 'Est-d per capacity day', num: true, sort: true, info: { title: 'Delivered per capacity day', definition: 'Delivered est-days divided by the person’s capacity days. A planning signal, not a productivity score: it ignores reviews, support, mentoring and design. Small samples and missing time off distort it.', formula: 'delivered est-d ÷ capacity days' } },
    { key: 'done', label: 'Done', num: true, sort: true },
    { key: 'subs', label: 'Sub-tasks with real work', num: true, sort: true, info: { title: 'Sub-tasks with real work', definition: 'Sub-tasks credited to this person because they carried real work: own commits or dev time, under someone else’s story, or under a story with no timing of its own. Checklist sub-tasks are not counted.', formula: 'count(credited substantive sub-tasks)' } },
    { key: 'pace', label: 'Dev-days per est-day', num: true, sort: true, info: { title: 'Pace vs estimate', definition: 'Actual dev working days per estimated day on this person’s tickets. Above ×1 = slower than estimated. Not divided by capacity: time off does not change it.', formula: 'Σ actual dev-days ÷ Σ estimated days' } },
    { key: 'cycle', label: 'Median cycle (business d)', num: true, sort: true },
    { key: 'mape', label: 'Est. error', num: true, sort: true },
    { key: 'wip', label: 'Avg WIP', num: true, sort: true },
    { key: 'goals', label: 'Goals' },
  ];

  function valueOf(o, a, k) {
    const cap = capOf(o, a);
    const capDays = cap && isNum(cap.capacity_days) ? cap.capacity_days : null;
    switch (k) {
      case 'name': return (a.assignee_name || a.assignee_id || '').toLowerCase();
      case 'role': return (a.role || '').toLowerCase();
      case 'cap': return capDays;
      case 'delivered': return a.weighted_done;
      case 'per_day':
        if (isNum(a.weighted_per_capacity_day)) return a.weighted_per_capacity_day;
        if (isNum(a.weighted_per_available_day)) return a.weighted_per_available_day;
        return capDays && isNum(a.weighted_done) ? a.weighted_done / capDays : null;
      case 'done': return a.completed;
      case 'subs': return subCount(o, a.assignee_id);
      case 'pace': return a.pace_vs_est;
      case 'cycle': return a.median_cycle;
      case 'mape': return a.mape;
      case 'wip': return a.avg_wip;
      default: return null;
    }
  }

  /** Substantive sub-tasks credited to a person (o.subtasks), or null when not reported. */
  function subCount(o, id) {
    const list = o.subtasks || (o.flow && o.flow.substantive_subtasks);
    if (!Array.isArray(list)) return null;
    return list.filter((x) => (x.credited_to || x.assignee_id) === id).length;
  }

  function rowsHtml(o, list) {
    return list.map((a) => {
      const cap = capOf(o, a);
      const capDays = valueOf(o, a, 'cap');
      const perDay = valueOf(o, a, 'per_day');
      const guard = rowGuard(o, a);
      return {
        attrs: `data-person="${esc(a.assignee_id)}"`,
        cells: [
          `<button type="button" class="link" data-open="${esc(a.assignee_id)}">${esc(a.assignee_name || a.assignee_id)}</button>${guard && TP.guardBadge ? ' ' + TP.guardBadge(guard) : ''}`,
          esc(a.role || '—'),
          capCell(cap),
          isNum(a.weighted_done) ? fmtNum(a.weighted_done) : '—',
          isNum(perDay) ? `${fmtNum(perDay, 2)}${isNum(capDays) ? ` <span class="dim small">over ${fmtNum(capDays, 0)} d</span>` : ''}` : '<span class="dim">needs capacity</span>',
          `${a.completed ?? 0}${a.rolled_up ? ` <span class="dim small">+${a.rolled_up} sub</span>` : ''}`,
          (() => {
            const n = subCount(o, a.assignee_id);
            return n == null ? '<span class="dim">—</span>' : n ? `<button type="button" class="link" data-open="${esc(a.assignee_id)}" data-focus="subtasks" aria-label="${n} sub-tasks with real work — open ${esc(a.assignee_name || a.assignee_id)}">${n}</button>` : '0';
          })(),
          isNum(a.pace_vs_est) ? `${fmtX(a.pace_vs_est)} <span class="dim small">independent of capacity</span>` : '—',
          isNum(a.median_cycle) ? fmtD(a.median_cycle) : TP.NT,
          fmtPct(a.mape),
          isNum(a.avg_wip) ? fmtNum(a.avg_wip) : '—',
          a.goals_total ? badge(a.goals_met === a.goals_total ? 'success' : a.goals_met ? 'warning' : 'danger', `${a.goals_met}/${a.goals_total}`) : '—',
        ],
      };
    });
  }

  const WHY = { commits: 'own commits', other_owner: 'under someone else’s story', parent_untimed: 'story has no own timing', dev_time: 'real dev time' };
  const nameOf = (o, id) => ((o.assignees || []).find((a) => a.assignee_id === id) || {}).assignee_name || (o.capacity && o.capacity.people && o.capacity.people[id] && o.capacity.people[id].name) || id;

  /** Credited sub-tasks table — shared by People and the person drill-down (TP.creditedSubtasks). */
  function creditedTable(o, list, { caption, openable = true } = {}) {
    return table({
      caption,
      cols: [{ label: 'Sub-task' }, { label: 'Parent story' }, { label: 'Credited to' }, { label: 'Story owner' }, { label: 'Why it counts' }, { label: 'Dev days', num: true, title: 'Business days of dev time on the sub-task itself' }, { label: 'Commits', num: true }],
      rows: list.slice(0, 300).map((s) => {
        const who = s.credited_to || s.assignee_id;
        const whoName = s.credited_name || (who === s.assignee_id ? s.assignee_name : null) || nameOf(o, who);
        const reasons = (Array.isArray(s.reasons) ? s.reasons : [s.reason]).filter(Boolean);
        if (!reasons.length) reasons.push(s.rollup === false ? 'other_owner' : 'dev_time');
        return [
          `${jiraLink(s.key)} <span class="dim small">${esc((s.summary || '').slice(0, 50))}</span>`,
          s.parent_key ? jiraLink(s.parent_key) : '—',
          who && openable ? `<button type="button" class="link" data-open="${esc(who)}">${esc(whoName || who)}</button>` : esc(whoName || '—'),
          esc(s.parent_assignee_name || '—'),
          `${reasons.map((r) => badge('info', WHY[r] || r)).join(' ')}${s.rollup === false ? ' <span class="dim small">counted separately</span>' : ' <span class="dim small">time rolls into the story</span>'}`,
          fmtD(s.dev_days ?? s.days ?? s.actual_days),
          isNum(s.commits) ? String(s.commits) : '—',
        ];
      }),
    });
  }
  TP.creditedSubtasks = creditedTable;

  function subtasksHtml(o) {
    const list = o.subtasks || (o.flow && o.flow.substantive_subtasks);
    if (!list) return TP.notAvailable('Sub-task attribution');
    if (!list.length) return '<p class="dim">No sub-tasks carried real work of their own in this period — checklist sub-tasks are rolled into their stories.</p>';
    return creditedTable(o, list, { caption: `${list.length} sub-tasks with real work, credited to the person who did it` });
  }

  TP.views.people = {
    rowsHtml,
    rowGuard,
    subCount,
    COLS,
    subtasksHtml,
    render(outer, { o, app }) {
      // Own wrapper per render so delegated listeners never pile up on #view.
      const host = document.createElement('div');
      outer.appendChild(host);
      let sortKey = store.get('peopleSort', 'name');
      let sortDir = store.get('peopleDir', 'asc');
      let role = '';
      const roles = [...new Set((o.assignees || []).map((a) => a.role).filter(Boolean))].sort();
      const roleSel = roles.length
        ? `<label class="sr-only" for="pp-role">Filter by role</label><select id="pp-role"><option value="">All roles</option>${roles.map((r) => `<option>${esc(r)}</option>`).join('')}</select>`
        : '';
      section(host, {
        title: 'People',
        headerEnd: roleSel,
        sub: 'Click a name for tickets, phases, rework and goals. Rates are per capacity day (working days − time off), so a vacation never reads as a slowdown. <b>Not a productivity score</b> — it ignores reviews, support, mentoring and design work; never rank people on it.',
        load: async () => {
          if (!(o.assignees || []).length) return TP.emptyState({ title: 'No people in this scope', body: 'People appear once their tickets are scanned. Check Settings → People for excluded or merged accounts.' });
          return '<div class="pp-table"></div>';
        },
        after(body) {
          const paint = () => {
            const box = body.querySelector('.pp-table');
            if (!box) return;
            const list = (o.assignees || []).filter((a) => !role || a.role === role).slice();
            list.sort((x, y) => {
              const a = valueOf(o, x, sortKey), b = valueOf(o, y, sortKey);
              if (a == null && b == null) return 0;
              if (a == null) return 1;
              if (b == null) return -1;
              const c = typeof a === 'string' ? a.localeCompare(b) : a - b;
              return sortDir === 'asc' ? c : -c;
            });
            box.innerHTML = table({ caption: `${list.length} people · ${app.periodLabel()}`, cols: COLS, rows: rowsHtml(o, list), sortKey, sortDir });
            box.querySelectorAll('button.sort').forEach((b) => {
              b.onclick = () => {
                const k = b.dataset.sort;
                sortDir = sortKey === k && sortDir === 'asc' ? 'desc' : 'asc';
                sortKey = k;
                store.set('peopleSort', sortKey);
                store.set('peopleDir', sortDir);
                paint();
                box.querySelector(`button.sort[data-sort="${k}"]`).focus();
              };
            });
          };
          paint();
          const sel = host.querySelector('#pp-role');
          if (sel) sel.onchange = (e) => { role = e.target.value; paint(); };
        },
      });
      section(host, {
        title: 'Sub-tasks with real work',
        infoDef: { title: 'Substantive sub-tasks', definition: 'Most sub-tasks are checklists and roll into their story. A sub-task is surfaced when it carries real dev time or commits, sits under someone else’s story, or its story has no timing of its own.', formula: 'commits > 0 OR dev time > 0 OR owner ≠ story owner OR story untimed' },
        load: async () => subtasksHtml(o),
      });
      if ((o.unmatched_authors || []).length) {
        host.insertAdjacentHTML('beforeend', `<div class="banner info" role="note">${icon('info')}<span>${o.unmatched_authors.length} git authors are not matched to a person, so their commits are not credited. Add them as aliases in Settings → People.</span></div>`);
      }
      host.addEventListener('click', (e) => {
        const b = e.target.closest('[data-open]');
        if (b) app.openPerson(b.dataset.open);
      });
    },
  };
})();
