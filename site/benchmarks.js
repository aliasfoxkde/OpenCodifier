/* The measured board — renders site/board.csv (synced from
   benchmarks/decision-model/results/board.csv) with a full data-page
   surface: unique stable row IDs, per-field tooltips, column visibility,
   size + numeric filters, sticky header, and disclosed derived scores.
   No dependencies, no external requests. */

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
    { re: /^Board A/i, short: 'A · JevBench', letter: 'A', label: 'Board A — JevBench public split' },
    { re: /^Board B/i, short: 'B · suite', letter: 'B', label: 'Board B — our locked suite' },
    { re: /^Board C/i, short: 'C · jabr', letter: 'C', label: 'Board C — jabr classifier gate' },
    { re: /^Cost per 1,000/i, short: 'Cost', letter: 'E', label: 'Cost per 1,000 decisions (modeled)' },
    { re: /^Composite/i, short: 'Composite', letter: 'D', label: 'Composite deployment score' },
    { re: /^The target bar/i, short: 'Target bar', letter: 'T', label: 'The target bar' },
    { re: /^REPORT\.md/i, short: 'REPORT', letter: 'R', label: 'REPORT.md — tuning A/B' }
  ];

  function boardMeta(b) {
    for (var i = 0; i < BOARD_ORDER.length; i++) {
      if (BOARD_ORDER[i].re.test(b)) return BOARD_ORDER[i];
    }
    return { re: null, short: b, letter: '?', label: b, idx: BOARD_ORDER.length };
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
    size: '',               // size-bucket filter
    minAcc: null,           // numeric advanced filters (null = off)
    maxEce: null,
    maxP50: null,
    minTrust: null,
    hidden: new Set(),      // column keys toggled off
    rankBy: 'trust',
    lambda: 0.5,
    hasEce: false,
    hasSize: false,
    sort: { key: null, dir: -1 }  // null = default board order + acc desc
  };

  var METRICS = {
    trust: { label: 'Trust', desc: true },
    acc: { label: 'Accuracy', desc: true },
    ipg: { label: 'Acc/GiB', desc: true },
    ece: { label: 'ECE', desc: false },
    p50: { label: 'p50 latency', desc: false }
  };

  /* Field dictionary — every header's title attribute comes from here,
     so the table explains itself without leaving the page. */
  var TIPS = {
    rank: 'Rank within the row\'s surface by the current "best by" metric, shown as position/surface-size. The same position appears once per surface — it is scoped to the surface, never global; the ID column is the globally unique one.',
    id: 'Stable row ID: surface letter (A JevBench · B locked suite · C jabr · D composite · T target bar · R tuning) + a sequence number fixed at load. Unique per row and stable under filtering and sorting.',
    name: 'The measured system or run. The small line under it is the run qualifier — harness, mode, or hardware note. The chip is provenance: who ran it and where the number is published.',
    surface: 'Which benchmark surface the row measures. Scores are never compared across surfaces — a public-split score and a locked-suite score measure different things.',
    n: 'Items scored on that run. Different surfaces have different n (231 for the JevBench public split).',
    acc: 'Micro accuracy: correctly answered items ÷ items scored, on that row\'s surface. Higher is better.',
    macro: 'Family-macro accuracy: unweighted mean over answer families, so large families cannot dominate. Shown where the harness reports it.',
    lexical: 'Accuracy on the lexical-match slice of our locked suite — the rung-4 family.',
    metadata: 'Accuracy on the metadata-filter slice of our locked suite — the rung-3 family.',
    relational: 'Accuracy on the relational/dependency slice of our locked suite — the exact-proof family. This is the slice the deterministic engine is built for.',
    ece: 'Expected calibration error: how far stated confidence sits from observed correctness (0 = perfect, lower is better). Rows without a measured ECE show no Trust score either.',
    brier: 'Brier score over the outcome distribution — mean squared error of the probability assigned to the truth (lower is better).',
    auroc: 'AUROC of the winner probability as the score for "answered correctly" — selective-classification discrimination: does higher confidence mean more often right? 0.5 = no discrimination, 1 = perfect. From runner/parity.py over the frozen run JSONs.',
    p50: 'Median decision latency in milliseconds on the row\'s hardware (CPU-only consumer desktop unless the qualifier says otherwise). Lower is better.',
    mean: 'Mean decision latency in milliseconds — the arithmetic average, which heavy-tail runs pull above the median.',
    p95: '95th-percentile decision latency in milliseconds (nearest-index convention, same as p50). The tail a user actually feels.',
    p99: '99th-percentile decision latency in milliseconds. Shown only where the frozen run recorded per-item wall clocks — nulls are honest, not missing.',
    reliability: 'Compact 10-bin reliability curve, "mean-p/acc/n" per occupied bin — where stated confidence lands and how often it is right there. Full curves: results/RELIABILITY.md.',
    tokens: 'Measured prompt tokens per decision plus the single verdict token (model rungs only). The input to the billed-equivalent cost columns.',
    cost1k: 'Modeled cost per 1,000 decisions in USD: the benchmarkheaven $0.0125/hour CPU rate applied to the row\'s measured p50 (self-hosted inference is amortized hardware, not invoices). Modeled, never invoiced — the docs of record carry the caveats.',
    cost1m: 'The same modeled cost scaled to one million decisions, the per-million unit the third-party board publishes for hosted arms.',
    billed1k: 'Billed-equivalent per 1,000 decisions: the row\'s measured tokens priced at the parametric API rate ($0.10/1M in + $0.40/1M out) — a parameter, not a vendor quote. What the same workload bills for through an API.',
    billed1m: 'The same billed-equivalent scaled to one million decisions.',
    size: 'Measured artifact size for the row: model file size for model rows; the static engine binary for engine rows (the engine ships no model weights). This is the denominator of Acc/GiB.',
    trust: 'Derived, not measured: accuracy − λ·ECE, λ yours to set above. Rows without ECE get no Trust score — never a guessed one.',
    ipg: 'Derived, not measured: accuracy ÷ artifact size in GiB — intelligence per gigabyte. Engine rows divide by their multi-MiB static binary, which is why they dominate this column.',
    det: 'Deterministic replay: "yes (delta 0.000)" means re-running the artifact reproduced every prediction bit-for-bit. The number is the worst replay divergence observed.',
    tier: 'The deployment tier the row belongs to on the composite board (engine / small encoder / local model / hosted).',
    prov: 'Provenance: who produced the number — our committed run, or the external authors\' published figure on the named surface.',
    notes: 'Row provenance key linking the row to its source table in the docs of record, plus tuning notes where present.'
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
    if (tbody) tbody.innerHTML = '<tr><td colspan="' + visibleColumns().length + '">Failed to load board.csv — ' +
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
        macro: num(r[col.macro]),
        lexical: num(r[col.lexical]),
        metadata: num(r[col.metadata]),
        relational: num(r[col.relational]),
        ece: num(r[col.ece]),
        brier: num(r[col.brier]),
        auroc: num(r[col.auroc]),
        p50: num(r[col.p50_ms]),
        mean: num(r[col.mean_ms]),
        p95: num(r[col.p95_ms]),
        p99: num(r[col.p99_ms]),
        reliability: (r[col.reliability] || '').trim(),
        tokens: num(r[col.tokens_per_decision]),
        cost1k: num(r[col.cost_per_1k_usd]),
        cost1m: num(r[col.cost_per_1m_usd]),
        billed1k: num(r[col.cost_billed_per_1k_usd]),
        billed1m: num(r[col.cost_billed_per_1m_usd]),
        sizeMiB: parseMiB(r[col.size] || ''),
        sizeRaw: (r[col.size] || '').trim(),
        det: (r[col.determinism] || '').trim(),
        replayDelta: num(r[col.replay_delta]),
        tier: (r[col.tier] || '').trim(),
        prov: (r[col.provenance] || '').trim(),
        notes: r[col.notes] || ''
      };
      var bm = boardMeta(row.board);
      row.boardShort = bm.short;
      row.boardLetter = bm.letter;
      row.boardIdx = boardIdx(row.board);
      row.quant = quantFamily(row.name);
      derived(row, state.lambda);
      row.search = (row.name + ' ' + row.qualifier + ' ' + row.section + ' ' +
        row.tier + ' ' + row.notes).toLowerCase();
      return row;
    });

    /* Stable unique IDs: surface letter + sequence in CSV order, assigned
       once at boot — filtering and sorting never renumber a row. The
       extended columns start off so the default table stays readable. */
    COLUMNS.forEach(function (c) {
      if (c.group === 'more') state.hidden.add(c.key);
    });
    var seq = {};
    data.forEach(function (r) {
      seq[r.boardLetter] = (seq[r.boardLetter] || 0) + 1;
      r.id = r.boardLetter + '-' + String(seq[r.boardLetter]).padStart(2, '0');
      r.idKey = r.boardIdx * 1000 + seq[r.boardLetter];
    });
    /* per-surface row counts for the "1/23" rank display */
    var perBoard = {};
    data.forEach(function (r) { perBoard[r.boardShort] = (perBoard[r.boardShort] || 0) + 1; });
    data.forEach(function (r) { r.boardCount = perBoard[r.boardShort]; });

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
    fillSelect('f-size', ['engine / no weights', 'under 0.5 GiB', '0.5 – 2 GiB', '2 – 4 GiB', '4 – 8 GiB', '8 GiB and up']);

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
    bind('f-size', 'size');
    bind('f-rank', 'rankBy');

    var bindNum = function (id, key, scale) {
      document.getElementById(id).addEventListener('input', function (e) {
        var v = parseFloat(e.target.value);
        state[key] = Number.isFinite(v) ? v / (scale || 1) : null;
        renderAll();
      });
    };
    bindNum('f-minacc', 'minAcc', 100);   // typed in %, stored as fraction
    bindNum('f-maxece', 'maxEce');
    bindNum('f-maxp50', 'maxP50');
    bindNum('f-mintrust', 'minTrust', 100);

    /* column visibility chips (core columns are not toggleable) */
    var colBox = document.getElementById('f-cols');
    COLUMNS.forEach(function (c) {
      if (c.group === 'core') return;
      var label = document.createElement('label');
      label.className = 'chip chip-on';
      var cb = document.createElement('input');
      cb.type = 'checkbox';
      cb.checked = true;
      cb.setAttribute('aria-label', 'Show column ' + c.label);
      cb.addEventListener('change', function () {
        if (cb.checked) state.hidden.delete(c.key); else state.hidden.add(c.key);
        label.classList.toggle('chip-on', cb.checked);
        renderAll();
      });
      label.appendChild(cb);
      label.appendChild(document.createTextNode(' ' + c.label));
      colBox.appendChild(label);
    });

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

  function sizeBucket(row) {
    if (row.sizeMiB === null) return 'engine / no weights';
    var gib = row.sizeMiB / 1024;
    if (gib < 0.5) return 'under 0.5 GiB';
    if (gib < 2) return '0.5 – 2 GiB';
    if (gib < 4) return '2 – 4 GiB';
    if (gib < 8) return '4 – 8 GiB';
    return '8 GiB and up';
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
      if (state.size && sizeBucket(r) !== state.size) return false;
      if (state.minAcc !== null && (r.acc === null || r.acc < state.minAcc)) return false;
      if (state.maxEce !== null && (r.ece === null || r.ece > state.maxEce)) return false;
      if (state.maxP50 !== null && (r.p50 === null || r.p50 > state.maxP50)) return false;
      if (state.minTrust !== null && (r.trust === null || r.trust < state.minTrust)) return false;
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
      groups[k].forEach(function (r) { if (r.rank === undefined) r.rank = null; });
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
      var va, vb;
      if (s.key === 'name') { va = a.name; vb = b.name; }
      else if (s.key === 'id') { va = a.idKey; vb = b.idKey; }
      else if (s.key === 'surface') { va = a.boardIdx; vb = b.boardIdx; }
      else { va = (a[s.key] === null || a[s.key] === undefined) ? -Infinity : a[s.key];
             vb = (b[s.key] === null || b[s.key] === undefined) ? -Infinity : b[s.key]; }
      if (typeof va === 'string') return s.dir * va.localeCompare(vb);
      return s.dir * (va - vb);
    });
  }

  var COLUMNS = [
    { key: 'id',    label: 'ID',        group: 'core', num: true, sortable: true },
    { key: 'name',  label: 'System / run', group: 'core', sortable: true },
    { key: 'surface', label: 'Surface', group: 'core', sortable: true },
    { key: 'n',     label: 'n',         group: 'core', num: true, sortable: true },
    { key: 'acc',   label: 'Acc %',     group: 'core', num: true, sortable: true },
    { key: 'macro', label: 'Macro %',   group: 'core', num: true, sortable: true },
    { key: 'ece',   label: 'ECE',       group: 'core', num: true, sortable: true },
    { key: 'p50',   label: 'p50 ms',    group: 'core', num: true, sortable: true },
    { key: 'size',  label: 'Size',      group: 'core', num: true, sortable: true },
    { key: 'trust', label: 'Trust %',   group: 'core', num: true, sortable: true, derived: true },
    { key: 'ipg',   label: '%/GiB',     group: 'core', num: true, sortable: true, derived: true },
    { key: 'rank',  label: 'Rank',      group: 'more', num: true, sortable: false },
    { key: 'brier', label: 'Brier',     group: 'more', num: true, sortable: true },
    { key: 'auroc', label: 'AUROC',     group: 'more', num: true, sortable: true },
    { key: 'mean',  label: 'mean ms',   group: 'more', num: true, sortable: true },
    { key: 'p95',   label: 'p95 ms',    group: 'more', num: true, sortable: true },
    { key: 'p99',   label: 'p99 ms',    group: 'more', num: true, sortable: true },
    { key: 'reliability', label: 'Reliability', group: 'more', sortable: true },
    { key: 'cost1k', label: '$/1k',     group: 'more', num: true, sortable: true },
    { key: 'cost1m', label: '$/1M',     group: 'more', num: true, sortable: true },
    { key: 'billed1k', label: '$/1k billed', group: 'more', num: true, sortable: true },
    { key: 'billed1m', label: '$/1M billed', group: 'more', num: true, sortable: true },
    { key: 'tokens', label: 'tokens/dec', group: 'more', num: true, sortable: true },
    { key: 'lexical', label: 'Lex %',   group: 'more', num: true, sortable: true },
    { key: 'metadata', label: 'Meta %', group: 'more', num: true, sortable: true },
    { key: 'relational', label: 'Rel %', group: 'more', num: true, sortable: true },
    { key: 'det',   label: 'Replay',    group: 'more', sortable: false },
    { key: 'tier',  label: 'Tier',      group: 'more', sortable: true },
    { key: 'prov',  label: 'Provenance', group: 'more', sortable: true },
    { key: 'notes', label: 'Notes',     group: 'more', sortable: true }
  ];

  function visibleColumns() {
    return COLUMNS.filter(function (c) { return !state.hidden.has(c.key); });
  }

  function fmt(v, dec) {
    if (v === null || v === undefined) return '—';
    return v.toFixed(dec);
  }

  function cellValue(row, key) {
    switch (key) {
      case 'id': return row.id;
      case 'n': return row.n === null ? '—' : String(row.n);
      case 'acc': return fmt(row.acc === null ? null : row.acc * 100, 1);
      case 'macro': return fmt(row.macro === null ? null : row.macro * 100, 1);
      case 'lexical': return fmt(row.lexical === null ? null : row.lexical * 100, 1);
      case 'metadata': return fmt(row.metadata === null ? null : row.metadata * 100, 1);
      case 'relational': return fmt(row.relational === null ? null : row.relational * 100, 1);
      case 'ece': return fmt(row.ece, 3);
      case 'brier': return fmt(row.brier, 3);
      case 'auroc': return fmt(row.auroc, 3);
      case 'p50': return row.p50 === null ? '—' : (row.p50 >= 100 ? row.p50.toFixed(0) : row.p50.toFixed(1));
      case 'mean': return row.mean === null ? '—' : (row.mean >= 100 ? row.mean.toFixed(0) : row.mean.toFixed(1));
      case 'p95': return row.p95 === null ? '—' : (row.p95 >= 100 ? row.p95.toFixed(0) : row.p95.toFixed(1));
      case 'p99': return row.p99 === null ? '—' : (row.p99 >= 100 ? row.p99.toFixed(0) : row.p99.toFixed(1));
      case 'reliability': return row.reliability || '—';
      /* cost keeps the docs' own significant figures — rounding $0.0000070 to
         $0.00 would erase the entire value */
      case 'cost1k': return row.cost1k === null ? '—' : '$' + row.cost1k;
      case 'cost1m': return row.cost1m === null ? '—' : '$' + row.cost1m;
      case 'billed1k': return row.billed1k === null ? '—' : '$' + row.billed1k;
      case 'billed1m': return row.billed1m === null ? '—' : '$' + row.billed1m;
      case 'tokens': return row.tokens === null ? '—' : (row.tokens >= 100 ? row.tokens.toFixed(0) : row.tokens.toFixed(1));
      case 'size':
        if (row.sizeMiB === null) return '—';
        return row.sizeMiB >= 1024 ? (row.sizeMiB / 1024).toFixed(2) + ' GiB' : row.sizeMiB.toFixed(1) + ' MiB';
      case 'trust': return row.trust === null ? '—' : (row.trust * 100).toFixed(1);
      case 'ipg': return row.ipg === null ? '—' : row.ipg >= 100 ? row.ipg.toFixed(0) : row.ipg.toFixed(1);
      case 'det': return row.det || '—';
      case 'tier': return row.tier || '—';
      case 'prov': return row.prov || '—';
      case 'notes': return row.notes || '—';
      default: return '';
    }
  }

  /* sort keys for the string-backed numeric columns */
  function sortValue(row, key) {
    switch (key) {
      case 'size': return row.sizeMiB;
      case 'det': return row.replayDelta;
      case 'tier': return row.tier;
      case 'prov': return row.prov;
      case 'notes': return row.notes;
      default: return row[key];
    }
  }

  function renderHead() {
    var thead = document.getElementById('thead');
    var tr = document.createElement('tr');
    visibleColumns().forEach(function (c) {
      var th = document.createElement('th');
      th.scope = 'col';
      th.title = TIPS[c.key] || c.label;
      th.setAttribute('class', (c.num ? 'num ' : '') + (c.derived ? 'derived' : ''));
      if (c.sortable) {
        var btn = document.createElement('button');
        btn.type = 'button';
        btn.className = 'sort-btn';
        btn.textContent = c.label;
        btn.title = TIPS[c.key] || c.label;
        btn.setAttribute('aria-label', 'Sort by ' + c.label + '. ' + (TIPS[c.key] || ''));
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
    var cols = visibleColumns();
    if (!rows.length) {
      tbody.innerHTML = '<tr><td colspan="' + cols.length + '" class="empty">No rows match the filters.</td></tr>';
      return;
    }
    var frag = document.createDocumentFragment();
    rows.forEach(function (r) {
      var tr = document.createElement('tr');
      cols.forEach(function (c) {
        var td = document.createElement('td');
        if (c.num) td.className = 'num';
        if (c.derived) td.className += ' derived';
        if (c.key === 'id' && r.rank === 1) td.className += ' best';
        if (c.key === 'name') {
          var strong = document.createElement('strong');
          strong.textContent = r.name;
          td.appendChild(strong);
          if (r.quant) {
            var q = document.createElement('span');
            q.className = 'chip chip-prov';
            q.textContent = r.quant;
            q.title = 'Quantization family parsed from the model name';
            td.appendChild(q);
          }
          if (r.qualifier) {
            var sub = document.createElement('span');
            sub.className = 'sub';
            sub.textContent = r.qualifier;
            sub.title = r.qualifier;
            td.appendChild(sub);
          }
        } else if (c.key === 'surface') {
          td.textContent = r.boardShort;
          td.title = r.board;
        } else if (c.key === 'rank') {
          td.textContent = rankCell(r);
          td.title = TIPS.rank;
        } else if (c.key === 'notes' || c.key === 'prov') {
          var v = cellValue(r, c.key);
          td.textContent = v;
          td.title = v;
          if (c.key === 'notes' && v.length > 42) td.textContent = v.slice(0, 41) + '…';
        } else {
          td.textContent = cellValue(r, c.key);
          if (c.key === 'size' && r.sizeRaw && r.sizeRaw !== cellValue(r, 'size')) {
            td.title = r.sizeRaw;
          }
        }
        tr.appendChild(td);
      });
      frag.appendChild(tr);
    });
    tbody.appendChild(frag);
  }

  /* the surface-scoped rank, rendered as "1/23" so per-surface numbering
     can never read as a global duplicate */
  function rankCell(row) {
    if (row.rank === null || row.rank === undefined) return '—';
    return row.rank + '/' + row.boardCount;
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
    note.textContent = 'ID is a stable per-surface key (A JevBench · B locked suite · C jabr · D composite · T target bar · R tuning); ' +
      'Rank reads "position/surface size" within each surface by ' + METRICS[state.rankBy].label +
      ' — it repeats across surfaces by design, the ID never does. "—" means the row does not measure that metric. ' +
      'Trust % = (accuracy − λ·ECE) × 100 with λ = ' + state.lambda.toFixed(2) +
      '; %/GiB = accuracy ÷ artifact size in GiB (engine rows divide by their static binary). ' +
      'The $/1k and $/1M figures on the Cost surface are modeled on the benchmarkheaven CPU basis — amortized hardware, never invoiced. ' +
      'Hover any column header for its definition.';
  }

  function render(data) {
    currentData = data;
    renderAll();
  }
})();
