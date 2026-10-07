/* The measured board — renders site/board.csv (synced from
   benchmarks/decision-model/results/board.csv) with filters and disclosed
   derived scores. No dependencies, no external requests. */

(function () {
  'use strict';

  /* ---------- RFC 4180 CSV (quoted fields, embedded commas/quotes) ---------- */
  function parseCsv(text) {
    var rows = [];
    var row = [];
    var field = '';
    var inQuotes = false;
    for (var i = 0; i < text.length; i++) {
      var c = text[i];
      if (inQuotes) {
        if (c === '"') {
          if (text[i + 1] === '"') { field += '"'; i++; }
          else { inQuotes = false; }
        } else { field += c; }
      } else if (c === '"') {
        /* RFC 4180: a DQUOTE opens quoting only at the start of a field;
           elsewhere in an unquoted field it is literal data. */
        if (field === '') { inQuotes = true; } else { field += c; }
      } else if (c === ',') {
        row.push(field); field = '';
      } else if (c === '\n') {
        row.push(field); field = '';
        rows.push(row); row = [];
      } else if (c !== '\r') {
        field += c;
      }
    }
    if (field !== '' || row.length) { row.push(field); rows.push(row); }
    return rows;
  }

  function esc(s) {
    return String(s).replace(/[&<>"']/g, function (ch) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[ch];
    });
  }

  function num(s) {
    if (s === undefined || s === null || s.trim() === '') return null;
    var v = Number(s);
    return Number.isFinite(v) ? v : null;
  }

  /* size column is "N MiB" (measured artifact size) or empty */
  function parseMiB(s) {
    var m = /([\d.]+)\s*MiB/i.exec(s || '');
    return m ? Number(m[1]) : null;
  }

  /* quantization family from the model name: Q3_K_S, Q4_K_M, UD-Q4_K_XL, Q8_0, q4_0 … */
  function quantFamily(name) {
    var m = /(UD-)?Q\s?([2-8])(_K(?:_[MSL])?|_0|_XXL?|_XL)?/i.exec(name || '');
    if (!m) return null;
    return (m[1] ? 'UD-' : '') + ('Q' + m[2] + (m[3] || '')).toUpperCase().replace(' ', '');
  }

  var BOARD_ORDER = [
    { re: /^Board A/i, short: 'A · JevBench', label: 'Board A — JevBench public split' },
    { re: /^Board B/i, short: 'B · suite', label: 'Board B — our locked suite' },
    { re: /^Board C/i, short: 'C · jabr', label: 'Board C — jabr classifier gate' },
    { re: /^Composite/i, short: 'Composite', label: 'Composite deployment score' },
    { re: /^The target bar/i, short: 'Target bar', label: 'The target bar' },
    { re: /^REPORT\.md/i, short: 'REPORT', label: 'REPORT.md — tuning A/B' }
  ];

  function boardMeta(b) {
    for (var i = 0; i < BOARD_ORDER.length; i++) {
      if (BOARD_ORDER[i].re.test(b)) return BOARD_ORDER[i];
    }
    return { re: null, short: b, label: b, idx: BOARD_ORDER.length };
  }
  function boardIdx(b) {
    for (var i = 0; i < BOARD_ORDER.length; i++) {
      if (BOARD_ORDER[i].re.test(b)) return i;
    }
    return BOARD_ORDER.length;
  }

  /* ---------- state ---------- */
  var state = {
    search: '',
    boards: new Set(),      // short labels currently ON
    quant: '',
    tier: '',
    prov: '',
    rankBy: 'trust',
    lambda: 0.5,
    hasEce: false,
    hasSize: false,  // only 6 rows carry a measured artifact size — sparse column, off by default
    sort: { key: null, dir: -1 }  // null = default board order + acc desc
  };

  var METRICS = {
    trust: { label: 'Trust', desc: true },
    acc: { label: 'Acc', desc: true },
    ipg: { label: 'Acc/GiB', desc: true },
    ece: { label: 'ECE', desc: false },
    p50: { label: 'p50', desc: false }
  };

  function derived(row, lambda) {
    row.trust = (row.acc !== null && row.ece !== null) ? row.acc - lambda * row.ece : null;
    row.ipg = (row.acc !== null && row.sizeMiB !== null && row.sizeMiB > 0)
      ? row.acc / (row.sizeMiB / 1024) : null;
  }

  function metricValue(row, key) {
    if (key === 'trust') return row.trust;
    if (key === 'ipg') return row.ipg;
    if (key === 'acc') return row.acc;
    if (key === 'ece') return row.ece;
    if (key === 'p50') return row.p50;
    return null;
  }

  /* ---------- boot ---------- */
  fetch('board.csv').then(function (r) {
    if (!r.ok) throw new Error('board.csv HTTP ' + r.status);
    return r.text();
  }).then(function (text) {
    boot(parseCsv(text));
  }).catch(function (err) {
    var tbody = document.getElementById('tbody');
    if (tbody) tbody.innerHTML = '<tr><td colspan="11">Failed to load board.csv — ' +
      esc(err.message) + '. The raw file is at <a href="board.csv">board.csv</a>.</td></tr>';
  });

  function boot(rows) {
    var header = rows[0].map(function (h) { return h.trim(); });
    var col = {};
    header.forEach(function (h, i) { col[h] = i; });

    var data = rows.slice(1).filter(function (r) {
      return r.length > 1 && (r[col.name] || '').trim() !== '';
    }).map(function (r) {
      var row = {
        board: r[col.board] || '',
        section: r[col.section] || '',
        name: r[col.name] || '',
        qualifier: r[col.qualifier] || '',
        n: num(r[col.n]),
        acc: num(r[col.accuracy]),
        ece: num(r[col.ece]),
        p50: num(r[col.p50_ms]),
        sizeMiB: parseMiB(r[col.size] || ''),
        sizeRaw: (r[col.size] || '').trim(),
        tier: (r[col.tier] || '').trim(),
        prov: (r[col.provenance] || '').trim(),
        notes: r[col.notes] || ''
      };
      var bm = boardMeta(row.board);
      row.boardShort = bm.short;
      row.boardIdx = boardIdx(row.board);
      row.quant = quantFamily(row.name);
      derived(row, state.lambda);
      row.search = (row.name + ' ' + row.qualifier + ' ' + row.section + ' ' +
        row.tier + ' ' + row.notes).toLowerCase();
      return row;
    });

    buildControls(data);
    render(data);
  }

  /* ---------- controls ---------- */
  function buildControls(data) {
    /* board chips: every surface ON except the REPORT tuning rows */
    var counts = {};
    data.forEach(function (r) {
      counts[r.boardShort] = (counts[r.boardShort] || 0) + 1;
    });
    var boardBox = document.getElementById('f-boards');
    BOARD_ORDER.forEach(function (b) {
      if (!(b.short in counts)) return;
      var on = !/^REPORT/.test(b.short);
      if (on) state.boards.add(b.short);
      var label = document.createElement('label');
      label.className = 'chip' + (on ? ' chip-on' : '');
      var cb = document.createElement('input');
      cb.type = 'checkbox';
      cb.checked = on;
      cb.setAttribute('aria-label', b.label + ' (' + counts[b.short] + ' rows)');
      cb.addEventListener('change', function () {
        if (cb.checked) state.boards.add(b.short); else state.boards.delete(b.short);
        label.classList.toggle('chip-on', cb.checked);
        renderAll();
      });
      label.appendChild(cb);
      label.appendChild(document.createTextNode(' ' + b.short + ' (' + counts[b.short] + ')'));
      boardBox.appendChild(label);
    });

    fillSelect('f-quant', distinct(data.map(function (r) { return r.quant; })).sort());
    fillSelect('f-tier', distinct(data.map(function (r) { return r.tier || null; })).sort(), 'unlabeled');
    fillSelect('f-prov', distinct(data.map(function (r) { return r.prov || null; })).sort(), 'unlabeled');

    var bind = function (id, key, evt) {
      document.getElementById(id).addEventListener(evt || 'change', function (e) {
        state[key] = e.target.value;
        renderAll();
      });
    };
    bind('f-search', 'search', 'input');
    bind('f-quant', 'quant');
    bind('f-tier', 'tier');
    bind('f-prov', 'prov');
    bind('f-rank', 'rankBy');

    document.getElementById('f-lambda').addEventListener('input', function (e) {
      state.lambda = Number(e.target.value);
      document.getElementById('lambda-out').textContent = state.lambda.toFixed(2);
      renderAll();
    });
    document.getElementById('f-hasece').addEventListener('change', function (e) {
      state.hasEce = e.target.checked; renderAll();
    });
    document.getElementById('f-hassize').addEventListener('change', function (e) {
      state.hasSize = e.target.checked; renderAll();
    });
    document.getElementById('f-reset').addEventListener('click', function () {
      location.reload();
    });
  }

  function distinct(values) {
    var seen = new Set();
    values.forEach(function (v) { if (v) seen.add(v); });
    return Array.from(seen);
  }

  function fillSelect(id, values, emptyLabel) {
    var sel = document.getElementById(id);
    values.forEach(function (v) {
      var o = document.createElement('option');
      o.value = v === 'unlabeled' ? '__none__' : v;
      o.textContent = v === 'unlabeled' ? (emptyLabel || 'unlabeled')
        : (v.length > 34 ? v.slice(0, 33) + '…' : v);
      o.title = v;
      sel.appendChild(o);
    });
  }

  /* ---------- filter → rank → sort → render ---------- */
  function visible(data) {
    return data.filter(function (r) {
      if (!state.boards.has(r.boardShort)) return false;
      if (state.search && r.search.indexOf(state.search.toLowerCase()) === -1) return false;
      if (state.quant) {
        var q = state.quant === '__none__' ? null : state.quant;
        if ((r.quant || null) !== q) return false;
      }
      if (state.tier) {
        var t = state.tier === '__none__' ? '' : state.tier;
        if (r.tier !== t) return false;
      }
      if (state.prov) {
        var p = state.prov === '__none__' ? '' : state.prov;
        if (r.prov !== p) return false;
      }
      if (state.hasEce && r.ece === null) return false;
      if (state.hasSize && (r.sizeMiB === null || r.sizeMiB <= 0)) return false;
      return true;
    });
  }

  function assignRanks(rows) {
    /* rank WITHIN each surface, by the chosen metric — never across surfaces */
    var groups = {};
    rows.forEach(function (r) {
      (groups[r.boardShort] = groups[r.boardShort] || []).push(r);
    });
    var desc = METRICS[state.rankBy].desc;
    Object.keys(groups).forEach(function (k) {
      var ranked = groups[k]
        .filter(function (r) { return metricValue(r, state.rankBy) !== null; })
        .sort(function (a, b) {
          return desc
            ? metricValue(b, state.rankBy) - metricValue(a, state.rankBy)
            : metricValue(a, state.rankBy) - metricValue(b, state.rankBy);
        });
      ranked.forEach(function (r, i) { r.rank = i + 1; });
    });
  }

  function sortRows(rows) {
    var s = state.sort;
    if (!s.key) {
      rows.sort(function (a, b) {
        return a.boardIdx - b.boardIdx ||
          ((b.acc === null ? -Infinity : b.acc) - (a.acc === null ? -Infinity : a.acc)) ||
          a.name.localeCompare(b.name);
      });
      return;
    }
    rows.sort(function (a, b) {
      var va = s.key === 'name' ? a.name : (a[s.key] === null ? -Infinity : a[s.key]);
      var vb = s.key === 'name' ? b.name : (b[s.key] === null ? -Infinity : b[s.key]);
      if (typeof va === 'string') return s.dir * va.localeCompare(vb);
      return s.dir * (va - vb);
    });
  }

  var COLUMNS = [
    { key: 'rank', label: '#', sortable: false, num: true },
    { key: 'name', label: 'Model / run', sortable: true, num: false },
    { key: 'boardShort', label: 'Surface', sortable: false, num: false },
    { key: 'n', label: 'n', sortable: true, num: true },
    { key: 'acc', label: 'Acc %', sortable: true, num: true },
    { key: 'ece', label: 'ECE', sortable: true, num: true },
    { key: 'p50', label: 'p50 ms', sortable: true, num: true },
    { key: 'size', label: 'Size GiB', sortable: true, num: true },
    { key: 'trust', label: 'Trust %', sortable: true, num: true, derived: true },
    { key: 'ipg', label: '%/GiB', sortable: true, num: true, derived: true },
    { key: 'tier', label: 'Tier', sortable: false, num: false }
  ];

  function fmt(v, dec) {
    if (v === null) return '—';
    return v.toFixed(dec);
  }

  function cellValue(row, key) {
    switch (key) {
      case 'rank': return row.rank ? String(row.rank) : '—';
      case 'n': return row.n === null ? '—' : String(row.n);
      case 'acc': return row.acc === null ? '—' : (row.acc * 100).toFixed(1);
      case 'ece': return fmt(row.ece, 3);
      case 'p50': return row.p50 === null ? '—' : (row.p50 >= 100 ? row.p50.toFixed(0) : row.p50.toFixed(1));
      case 'size': return row.sizeMiB === null ? '—' : (row.sizeMiB / 1024).toFixed(2);
      case 'trust': return row.trust === null ? '—' : (row.trust * 100).toFixed(1);
      case 'ipg': return row.ipg === null ? '—' : (row.ipg * 100).toFixed(1);
      default: return '';
    }
  }

  function renderHead() {
    var thead = document.getElementById('thead');
    var tr = document.createElement('tr');
    COLUMNS.forEach(function (c) {
      var th = document.createElement('th');
      th.scope = 'col';
      th.setAttribute('class', (c.num ? 'num ' : '') + (c.derived ? 'derived' : ''));
      if (c.sortable) {
        var btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'sort-btn';
        btn.textContent = c.label;
        btn.setAttribute('aria-label', 'Sort by ' + c.label);
        if (state.sort.key === c.key) {
          th.setAttribute('aria-sort', state.sort.dir === -1 ? 'descending' : 'ascending');
          btn.textContent += state.sort.dir === -1 ? ' ▼' : ' ▲';
        }
        btn.addEventListener('click', function () {
          if (state.sort.key === c.key) { state.sort.dir = -state.sort.dir; }
          else { state.sort = { key: c.key, dir: c.num ? -1 : 1 }; }
          renderAll();
        });
        th.appendChild(btn);
      } else {
        th.textContent = c.label;
      }
      tr.appendChild(th);
    });
    thead.innerHTML = '';
    thead.appendChild(tr);
  }

  function renderBody(rows) {
    var tbody = document.getElementById('tbody');
    tbody.innerHTML = '';
    if (!rows.length) {
      tbody.innerHTML = '<tr><td colspan="11" class="empty">No rows match the filters.</td></tr>';
      return;
    }
    var frag = document.createDocumentFragment();
    rows.forEach(function (r) {
      var tr = document.createElement('tr');
      COLUMNS.forEach(function (c) {
        var td = document.createElement('td');
        if (c.num) td.className = 'num';
        if (c.derived) td.className += ' derived';
        if (c.key === 'rank' && r.rank === 1) td.className += ' best';
        if (c.key === 'name') {
          var strong = document.createElement('strong');
          strong.textContent = r.name;
          td.appendChild(strong);
          if (r.prov) {
            var chip = document.createElement('span');
            chip.className = 'chip chip-prov';
            chip.textContent = r.prov;
            td.appendChild(chip);
          }
          if (r.qualifier) {
            var sub = document.createElement('span');
            sub.className = 'sub';
            sub.textContent = r.qualifier;
            td.appendChild(sub);
          }
        } else if (c.key === 'boardShort') {
          td.textContent = r.boardShort;
        } else if (c.key === 'tier') {
          td.textContent = r.tier || '—';
        } else {
          td.textContent = cellValue(r, c.key);
        }
        tr.appendChild(td);
      });
      frag.appendChild(tr);
    });
    tbody.appendChild(frag);
  }

  function renderSummary(rows) {
    var count = document.getElementById('sum-count');
    count.textContent = rows.length + ' row' + (rows.length === 1 ? '' : 's') + ' shown';
    var best = document.getElementById('sum-best');
    var desc = METRICS[state.rankBy].desc;
    var groups = {};
    rows.forEach(function (r) {
      if (r.rank !== 1) return;
      groups[r.boardShort] = r;
    });
    var keys = Object.keys(groups).sort(function (a, b) { return a.localeCompare(b); });
    if (!keys.length) {
      best.textContent = 'No ranked rows in view for ' + METRICS[state.rankBy].label + '.';
      return;
    }
    best.innerHTML = '<span class="sum-best-label">Best in view — ' +
      esc(METRICS[state.rankBy].label) + ' (' + (desc ? 'higher is better' : 'lower is better') +
      ') per surface:</span> ' + keys.map(function (k) {
        var r = groups[k];
        return '<span class="sum-best-item"><strong>' + esc(k) + '</strong> ' +
          esc(r.name) + ' · ' + esc(cellValue(r, state.rankBy)) + '</span>';
      }).join('');
  }

  var currentData = [];
  function renderAll() {
    currentData.forEach(function (r) {
      r.rank = undefined;
      derived(r, state.lambda);
    });
    var rows = visible(currentData);
    assignRanks(rows);
    renderSummary(rows);
    sortRows(rows);
    renderHead();
    renderBody(rows);
    var note = document.getElementById('table-note');
    note.textContent = 'Rank is computed within each surface by ' +
      METRICS[state.rankBy].label + '; "—" means the row does not measure that metric. ' +
      'Trust % = (accuracy − λ·ECE) × 100 with λ = ' + state.lambda.toFixed(2) +
      '; %/GiB = accuracy ÷ size in GiB.';
  }

  function render(data) {
    currentData = data;
    renderAll();
  }
})();
