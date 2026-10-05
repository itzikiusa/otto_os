// People: one row per person, every ratio next to the capacity it was earned
// in (business days minus holidays and entered time off). Default sort is by
// name — this is not a leaderboard. Plus substantive sub-tasks.
'use strict';
(function () {
  const TP = window.TP;
  const { esc, fmtD, fmtPct, fmtNum, fmtX, isNum, section, table, jiraLink, info, badge, icon, store } = TP;

  const capOf = (o, a) => a.capacity || (o.capacity && o.capacity.people && o.capacity.people[a.assignee_id]) || null;

  const COLS = [
    { key: 'name', label: 'Person', sort: true },
    { key: 'role', label: 'Role', sort: true },
    {
      key: 'cap',
      label: 'Available days',
      num: true,
      sort: true,
      info: { title: 'Available days', definition: 'Working days in the period minus holidays and the person’s entered time off (Settings → People). Every ratio on this row is read against it.', formula: 'business days − holidays − time off' },
    },
    { key: 'delivered', label: 'Delivered est-d', num: true, sort: true, info: { title: 'Delivered scope', definition: 'Estimated ideal days delivered, credited by commit share on shared tickets. Rework is not new scope.', formula: 'Σ estimate × commit share' } },
    { key: 'per_day', label: 'Per available day', num: true, sort: true, info: { title: 'Delivered per available day', definition: 'Delivered est-days divided by the person’s available days — the only fair cross-person rate. Small samples and missing time off distort it.', formula: 'delivered est-d ÷ available days' } },
    { key: 'done', label: 'Done', num: true, sort: true },
    { key: 'pace', label: 'Pace vs est', num: true, sort: true, info: { title: 'Pace vs estimate', definition: 'Actual working days per estimated day on this person’s tickets. Above ×1 = slower than estimated.', formula: 'Σ actual ÷ Σ estimate' } },
    { key: 'cycle', label: 'Median cycle', num: true, sort: true },
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
      case 'per_day': return isNum(a.weighted_per_available_day) ? a.weighted_per_available_day : capDays && isNum(a.weighted_done) ? a.weighted_done / capDays : null;
      case 'done': return a.completed;
      case 'pace': return a.pace_vs_est;
      case 'cycle': return a.median_cycle;
      case 'mape': return a.mape;
      case 'wip': return a.avg_wip;
      default: return null;
    }
  }

  function rowsHtml(o, list) {
    return list.map((a) => {
      const cap = capOf(o, a);
      const capDays = valueOf(o, a, 'cap');
      const perDay = valueOf(o, a, 'per_day');
      const lowCap = isNum(capDays) && cap && isNum(cap.business_days) && capDays < cap.business_days * 0.6;
      return {
        attrs: `data-person="${esc(a.assignee_id)}"`,
        cells: [
          `<button type="button" class="link" data-open="${esc(a.assignee_id)}">${esc(a.assignee_name || a.assignee_id)}</button>`,
          esc(a.role || '—'),
          isNum(capDays) ? `${fmtNum(capDays, 0)}${cap && cap.time_off_days ? ` <span class="dim small">(${fmtNum(cap.time_off_days, 0)} off)</span>` : ''}${lowCap ? ' ' + badge('warning', 'low capacity') : ''}` : '<span class="dim">not available yet</span>',
          isNum(a.weighted_done) ? fmtNum(a.weighted_done) : '—',
          isNum(perDay) ? fmtNum(perDay, 2) : '—',
          `${a.completed ?? 0}${a.rolled_up ? ` <span class="dim small">+${a.rolled_up} sub</span>` : ''}`,
          fmtX(a.pace_vs_est),
          fmtD(a.median_cycle),
          fmtPct(a.mape),
          isNum(a.avg_wip) ? fmtNum(a.avg_wip) : '—',
          a.goals_total ? badge(a.goals_met === a.goals_total ? 'success' : a.goals_met ? 'warning' : 'danger', `${a.goals_met}/${a.goals_total}`) : '—',
        ],
      };
    });
  }

  function subtasksHtml(o) {
    const list = o.subtasks || (o.flow && o.flow.substantive_subtasks);
    if (!list) return TP.notAvailable('Sub-task attribution');
    if (!list.length) return '<p class="dim">No sub-tasks carried real work of their own in this period — checklist sub-tasks are rolled into their stories.</p>';
    const why = { commits: 'own commits', other_owner: 'under someone else’s story', parent_untimed: 'story has no own timing', dev_time: 'real dev time' };
    return table({
      caption: `${list.length} sub-tasks with real work, credited to the person who did it`,
      cols: [{ label: 'Sub-task' }, { label: 'Parent story' }, { label: 'Done by' }, { label: 'Story owner' }, { label: 'Why it counts' }, { label: 'Days', num: true }, { label: 'Commits', num: true }],
      rows: list.slice(0, 300).map((s) => [
        `${jiraLink(s.key)} <span class="dim small">${esc((s.summary || '').slice(0, 50))}</span>`,
        s.parent_key ? jiraLink(s.parent_key) : '—',
        s.assignee_id ? `<button type="button" class="link" data-open="${esc(s.assignee_id)}">${esc(s.assignee_name || s.assignee_id)}</button>` : esc(s.assignee_name || '—'),
        esc(s.parent_assignee_name || '—'),
        (Array.isArray(s.reasons) ? s.reasons : [s.reason]).filter(Boolean).map((r) => badge('info', why[r] || r)).join(' '),
        fmtD(s.days ?? s.actual_days),
        isNum(s.commits) ? String(s.commits) : '—',
      ]),
    });
  }

  TP.views.people = {
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
        sub: 'Click a name for tickets, phases, rework and goals. Ratios are per available day — a vacation never reads as a slowdown.',
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
