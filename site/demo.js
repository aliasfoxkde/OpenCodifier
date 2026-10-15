/* Shared engine glue for every page that runs the WASM engine in your tab
   (decide.html console, playground.html, api.html examples). No server, no network, no model: the zero-ML stack
   (rules → cache → filter → lexical) plus graph validation/execution.
   Every engine-derived string is escaped before it reaches the DOM —
   editor content is user input and is treated as hostile, exactly like
   the runtime treats it. api.html links here via sessionStorage
   "oc-preset" ({request: id, graph: id}). */
"use strict";

(function () {
  const LIMITS = {
    max_input_bytes: 1048576,
    max_questions: 32,
    max_candidates: 256,
    max_graph_nodes: 128,
    max_execution_time: { secs: 10, nanos: 0 },
    max_retrieval_results: 64,
  };

  function request(text, question, candidates, id) {
    return {
      state: { text: text, facts: {} },
      questions: [{
        type: "choice", id: id || "q1", text: question,
        candidates: candidates.map(function (c) {
          return { id: c[0], description: c[1] };
        }),
      }],
      policy: { min_confidence: 0.8, verify_below: 0.65, abstain_below: 0.5, risk: "low" },
      metadata: { request_id: "playground", limits: LIMITS },
    };
  }

  /* Preset payloads are the same fixtures the Rust wasm tests and the
     HTTP integration tests pin — numbers quoted in labels were measured
     against the real engine, not invented for the page. */
  const PRESETS = {
    proof: {
      label: "Proof — relational root cause (accepts at 1.0)",
      request: request(
        "api depends on billing. billing depends on catalog. api is down. billing is down.",
        "Which service is the root cause of the outage?",
        [["api", "The public API tier"], ["billing", "The billing service"], ["catalog", "The product catalog"]],
      ),
    },
    lexical: {
      label: "Lexical — ticket routing (winner under the gate → verify)",
      request: request(
        "Ticket: customer reports a duplicate charge on invoice #4411 and demands a refund.",
        "Which team should own this ticket?",
        [
          ["billing", "duplicate charges, invoice disputes, refunds and payment corrections"],
          ["technical", "outages, error rates and broken functionality"],
          ["sales", "pricing, quotes, renewals and new contracts"],
        ],
      ),
    },
    hard: {
      label: "Hard — no evidence (honest abstain)",
      request: request(
        "payments is degraded. search is degraded. uploads is degraded.",
        "Which service should we restart first?",
        [
          ["payments", "handles money movement for checkout"],
          ["search", "serves query traffic to the storefront"],
          ["uploads", "ingests customer files"],
        ],
      ),
    },
    boolean: {
      label: "Boolean — threshold question",
      request: {
        state: { text: "replicas must be 2. prod-03 currently runs 2 replicas.", facts: {} },
        questions: [{ type: "boolean", id: "b1", text: "Is prod-03 compliant with the replica rule?" }],
        policy: { min_confidence: 0.8, verify_below: 0.65, abstain_below: 0.5, risk: "low" },
        metadata: { request_id: "playground", limits: LIMITS },
      },
    },
    invalid: {
      label: "Invalid — zero candidates (canonical refusal)",
      request: request("the service is down.", "Which service is down?", []),
    },
  };

  const GRAPHS = {
    minimal: {
      label: "Minimal — normalize → choice → threshold → output",
      graph: {
        version: 1,
        nodes: [
          { id: "normalize", kind: "normalize" },
          { id: "choice", kind: "choice", depends_on: ["normalize"] },
          { id: "threshold", kind: "threshold", threshold: 0.5, depends_on: ["choice"] },
          { id: "output", kind: "output", depends_on: ["threshold"] },
        ],
      },
    },
    cyclic: {
      label: "Invalid — cyclic (graph.cycle refusal)",
      graph: {
        version: 1,
        nodes: [
          { id: "a", kind: "rule", depends_on: ["b"] },
          { id: "b", kind: "rule", depends_on: ["a"] },
        ],
      },
    },
  };

  /* ---------- shorthand ---------- */
  function $(id) { return document.getElementById(id); }
  function byId(id) { return document.getElementById(id); }
  function esc(s) {
    return String(s).replace(/[&<>"']/g, function (ch) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[ch];
    });
  }
  function tryParse(s) {
    try { return JSON.parse(s); } catch (e) { return null; }
  }
  function fmtMs(v) { return v >= 100 ? v.toFixed(0) : v.toFixed(2); }

  const reqEditor = $("req-editor");
  const graphEditor = $("graph-editor");
  const out = $("pg-output");
  const badge = $("pg-badge");
  const chips = {
    engine: $("chip-engine"),
    wasm: $("chip-wasm"),
    network: $("chip-network"),
  };

  function setBadge(kind, text) {
    badge.className = "pg-badge pg-" + kind;
    badge.textContent = text;
  }

  function section(title) {
    return '<h3 class="pg-h">' + esc(title) + "</h3>";
  }

  /* ---------- renderers ---------- */
  function renderError(err, ms) {
    const raw = err instanceof Error ? (err.message || String(err)) : String(err);
    setBadge("error", "refused");
    let code = "engine error";
    let message = raw;
    const asJson = raw.indexOf("{") !== -1 ? tryParse(raw) : null;
    if (asJson && asJson.code) { code = asJson.code; message = asJson.message || raw; }
    out.innerHTML =
      section("The runtime refused this input — that is the gate working") +
      '<div class="pg-error"><code class="pg-code">' + esc(code) + "</code> " +
      esc(message) + "</div>" +
      '<p class="fineprint">Abstention and refusal are outcomes, not failures — the same request ' +
      "through <code>POST /v1/decide</code> returns the same code (HTTP 400).</p>" +
      '<p class="pg-ms">measured: ' + fmtMs(ms) + " ms (in-tab WASM)</p>";
  }

  function answersTable(answers) {
    let html = section("Answers") + '<table class="pg-table"><thead><tr>' +
      "<th>question</th><th>type</th><th>decision</th><th>confidence</th><th>distribution</th>" +
      "</tr></thead><tbody>";
    answers.forEach(function (a) {
      const dist = (a.distribution && a.distribution.entries ? a.distribution.entries : [])
        .map(function (e) { return esc(e.key) + " " + (e.probability * 100).toFixed(1) + "%"; })
        .join(" · ");
      const raw = a.choice !== undefined && a.choice !== null ? a.choice : a.value;
      const choice = raw === true ? "true" : raw === false ? "false" : String(raw);
      html += "<tr><td>" + esc(a.question_id) + "</td><td>" + esc(a.type) + "</td><td><strong>" +
        esc(choice) + "</strong></td><td>" + (a.confidence * 100).toFixed(2) + "%</td><td>" +
        dist + "</td></tr>";
    });
    return html + "</tbody></table>";
  }

  function confidencePanel(c) {
    const rows = [
      ["calibrated", c.calibrated_confidence],
      ["top probability", c.top_probability],
      ["margin", c.margin],
      ["entropy", c.entropy],
      ["OOD score", c.ood_score],
      ["verifier agreement", c.verifier_agreement],
    ];
    let html = section("Confidence — multi-dimensional, never a bare softmax") + '<dl class="pg-kv">';
    rows.forEach(function (r) {
      const val = (r[1] === undefined || r[1] === null) ? "—" : r[1];
      html += "<dt>" + esc(r[0]) + "</dt><dd>" +
        (typeof val === "number" ? val.toFixed(4) : esc(val)) + "</dd>";
    });
    return html + "</dl>";
  }

  function metricsPanel(m) {
    return section("Execution metrics") + '<dl class="pg-kv">' +
      "<dt>cache</dt><dd>" + (m.cache_hit ? "HIT — this exact decision was made before" : "miss") + "</dd>" +
      "<dt>candidates</dt><dd>" + esc(m.candidates_in) + " in → " + esc(m.candidates_out) +
      " after narrowing</dd>" +
      "<dt>verifier</dt><dd>" + (m.verification_triggered
        ? "triggered"
        : "not triggered (confidence-gated)") + "</dd></dl>";
  }

  function tracePanel(trace) {
    let html = section("Execution trace — the deterministic explainability surface") + '<ol class="pg-trace">';
    (trace && trace.entries ? trace.entries : []).forEach(function (en) {
      html += "<li><code>" + esc(en.node) + "</code> <span class='pg-detail'>" +
        esc(JSON.stringify(en.detail)) + "</span></li>";
    });
    return html + "</ol>";
  }

  function renderResponse(parsed, ms) {
    const outcome = parsed.outcome;
    const klass = outcome === "accept" ? "accept" : outcome === "verify" ? "verify" : "abstain";
    const headline = {
      accept: "accepted — calibrated confidence cleared the gate",
      verify: "verify — below the accept gate, handed to the confidence-gated verifier",
      abstain: "abstained — the runtime refuses to guess (a successful outcome)",
    }[outcome] || outcome;
    setBadge(klass, outcome);
    out.innerHTML =
      '<p class="pg-headline">' + esc(headline) + "</p>" +
      answersTable(parsed.answers) +
      confidencePanel(parsed.confidence) +
      metricsPanel(parsed.metrics) +
      tracePanel(parsed.trace) +
      '<p class="pg-ms">engine ' + esc(identity.engine_semver) + ' · measured: <strong>' +
      fmtMs(ms) + " ms</strong> in this tab · zero ML · zero network</p>" +
      '<details class="pg-raw"><summary>Raw response JSON</summary><pre>' +
      esc(JSON.stringify(parsed, null, 2)) + "</pre></details>";
  }

  function renderBatch(parsed, ms) {
    setBadge("accept", "batch · " + esc(parsed.count) + " decided");
    let html = section("Batch — one boundary crossing, shared cache, per-item error isolation") +
      '<table class="pg-table"><thead><tr><th>#</th><th>result</th></tr></thead><tbody>';
    parsed.results.forEach(function (r, i) {
      if (r.response) {
        const a = r.response.answers[0];
        html += "<tr><td>" + (i + 1) + "</td><td><span class='pg-ok'>" + esc(r.response.outcome) +
          "</span> · " + esc(String(a.choice)) + " @ " + (a.confidence * 100).toFixed(1) + "%" +
          (r.response.metrics.cache_hit ? " · <em>cache hit</em>" : "") + "</td></tr>";
      } else {
        html += "<tr><td>" + (i + 1) + "</td><td><span class='pg-err'>" + esc(r.error.code) +
          "</span> — " + esc(r.error.message) + "</td></tr>";
      }
    });
    html += "</tbody></table>" +
      '<p class="pg-ms">measured: <strong>' + fmtMs(ms) + " ms</strong> for the whole batch — " +
      "one item's refusal never fails the others.</p>" +
      '<details class="pg-raw"><summary>Raw batch JSON</summary><pre>' +
      esc(JSON.stringify(parsed, null, 2)) + "</pre></details>";
    out.innerHTML = html;
  }

  function renderGraphResult(parsed, ms, what) {
    setBadge("accept", what);
    out.innerHTML = section(what) + "<pre class='pg-pre'>" +
      esc(JSON.stringify(parsed, null, 2)) + "</pre>" +
      '<p class="pg-ms">measured: ' + fmtMs(ms) + " ms</p>";
  }

  /* One dispatcher for every successful call — results are told apart by
     shape, never by which button was clicked. */
  function renderDispatch(raw, ms) {
    const parsed = tryParse(raw);
    if (!parsed) { renderError(new Error(raw), ms); return; }
    if (parsed.results && parsed.count !== undefined) { renderBatch(parsed, ms); return; }
    if (parsed.answers) { renderResponse(parsed, ms); return; }
    if (parsed.version !== undefined && parsed.nodes !== undefined) {
      renderGraphResult(parsed, ms, "Graph validated — declarative DAG accepted");
      return;
    }
    renderGraphResult(parsed, ms, "Graph run complete");
  }

  function withTiming(fn) {
    const t0 = performance.now();
    let raw;
    try { raw = fn(); } catch (err) { renderError(err, performance.now() - t0); return; }
    renderDispatch(raw, performance.now() - t0);
  }

  /* ---------- engine plumbing ---------- */
  function fail(msg) {
    chips.wasm.textContent = "WASM: failed to start";
    chips.wasm.classList.add("chip-bad");
    setBadge("error", "engine unavailable");
    out.innerHTML = "<p>The WASM engine could not start in this browser. " + esc(msg) + "</p>";
  }

  let engine = null;
  let identity = null;

  function initEngine() {
    /* The glue exports its async initializer as the DEFAULT export
       (`__wbg_init as default`), not a named `init`. */
    import("./wasm/opencodifier_wasm.js")
      .then(function (mod) { return mod.default().then(function () { return mod; }); })
      .then(function (mod) {
        engine = new mod.WasmEngine();
        identity = JSON.parse(engine.identity());
        engine.decide(JSON.stringify(PRESETS.proof.request)); /* warmup */
        chips.engine.textContent = "Engine: opencodifier " + esc(identity.engine_semver) +
          " [" + esc(identity.model_id) + "]";
        chips.wasm.textContent = "WASM: running locally";
        chips.wasm.classList.add("chip-ok");
        setBadge("abstain", "ready");
        out.innerHTML = "<p>Engine ready. Pick a preset or paste your own request JSON, then " +
          "<strong>Decide</strong>. Everything runs in this tab — the Network chip below counts " +
          "cross-origin requests, live.</p>";
        document.querySelectorAll(".pg-actions button, .bd-actions button")
          .forEach(function (b) { b.disabled = false; });

        /* Deep link: decide.html?run=<preset|batch> loads that preset and
           (legacy try.html?run=… bookmarks redirect here)
           decides immediately — same fixture ids as the select; "batch"
           fires the batch action on the current editor contents. */
        const runPreset = new URLSearchParams(location.search).get("run");
        if (runPreset && PRESETS[runPreset]) {
          reqEditor.value = JSON.stringify(PRESETS[runPreset].request, null, 2) + "\n";
          $("preset-select").value = runPreset;
          $("act-decide").click();
        } else if (runPreset === "batch") {
          $("act-batch").click();
        }
      })
      .catch(function (err) { fail(err && err.message ? err.message : String(err)); });
  }

  /* The Network chip is measured, not asserted: same-origin resources are
     expected (scripts, wasm, csv); anything cross-origin is listed. */
  function measureNetwork() {
    const origins = {};
    performance.getEntriesByType("resource").forEach(function (e) {
      const url = new URL(e.name, location.href);
      if (url.origin !== location.origin) {
        origins[url.origin] = (origins[url.origin] || 0) + 1;
      }
    });
    const keys = Object.keys(origins);
    if (!keys.length) {
      chips.network.textContent = "Network: none — 0 cross-origin requests (measured)";
      chips.network.classList.add("chip-ok");
    } else {
      chips.network.textContent = "Network: " + keys.join(", ") + " — unexpected for this page";
      chips.network.classList.add("chip-bad");
    }
  }

  /* ---------- actions ---------- */
  function bindActions() {
    $("act-decide").addEventListener("click", function () {
      withTiming(function () { return engine.decide(reqEditor.value); });
    });

    /* Batch = the editor's request twice (second one must cache-hit) plus
       one invalid item — demonstrates per-item isolation without a
       second editor. */
    $("act-batch").addEventListener("click", function () {
      withTiming(function () {
        let current;
        try { current = JSON.parse(reqEditor.value); } catch (e) { current = null; }
        return engine.decide_batch(JSON.stringify({ requests: [current, current, {}] }));
      });
    });

    $("act-validate-graph").addEventListener("click", function () {
      withTiming(function () { return engine.validate_graph(graphEditor.value); });
    });

    $("act-run-graph").addEventListener("click", function () {
      withTiming(function () { return engine.run_graph(graphEditor.value, reqEditor.value); });
    });

    $("preset-select").addEventListener("change", function (e) {
      const p = PRESETS[e.target.value];
      if (p) reqEditor.value = JSON.stringify(p.request, null, 2) + "\n";
    });
    $("graph-select").addEventListener("change", function (e) {
      const g = GRAPHS[e.target.value];
      if (g) graphEditor.value = JSON.stringify(g.graph, null, 2) + "\n";
    });
  }

  /* ---------- Try-a-Decision builder ----------
     A form that compiles into the same request JSON the console decides.
     Pure composition lives in TryDecision.buildRequest so tests can pin it
     without a DOM; this section only wires the form to that function. */
  const BD_POLICY = { min_confidence: 0.8, verify_below: 0.65, abstain_below: 0.5, risk: "low" };
  const BD_LIMITS = LIMITS;

  const TryDecision = {
    /* Mirror of NEGATORS in crates/opencodifier-engine/src/lexical.rs —
       the lexical boolean rung counts these tokens and flips polarity on
       an odd count. A test pins this list against the Rust source. */
    NEGATORS: ["not", "no", "never", "without", "cannot", "neither", "nor"],
    countNegators: function (text) {
      const tokens = String(text).toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
      return tokens.filter(function (t) { return this.NEGATORS.indexOf(t) !== -1; }, this).length;
    },
    /* criteria are composed into the question text: the wire carries a
       boolean as {id, text}, and the zero-ML rungs read text — this is the
       honest way to express "what counts as yes/no" today. The template
       itself must stay negator-free: a bare "no" would flip the rung's
       polarity read (countNegators exists so the page can say so). */
    composeBooleanText: function (question, yesWhen, noWhen) {
      let text = question.trim();
      if (yesWhen.trim()) text += " Criteria — satisfied when: " + yesWhen.trim() + ".";
      if (noWhen.trim()) text += " fail when: " + noWhen.trim() + ".";
      return text;
    },
    looksLikeJson: function (state) {
      return /^\s*[\[{]/.test(state) && tryParse(state) !== null;
    },
    buildRequest: function (input) {
      const cat = input.category;
      let q;
      if (cat === "yes-no") {
        q = {
          type: "boolean", id: "q1",
          text: this.composeBooleanText(input.question, input.yesWhen || "", input.noWhen || ""),
        };
      } else if (cat === "choice") {
        q = {
          type: "choice", id: "q1", text: input.question.trim(),
          candidates: (input.candidates || [])
            .map(function (c) { return { id: String(c.id || "").trim(), description: String(c.description || "").trim() }; })
            .filter(function (c) { return c.id; }),
        };
      } else if (cat === "score") {
        q = {
          type: "score", id: "q1", text: input.question.trim(),
          levels: String(input.levels || "").split(",")
            .map(function (l) { return { label: l.trim() }; })
            .filter(function (l) { return l.label; }),
        };
      } else {
        return null;
      }
      return {
        state: { text: input.state, facts: {} },
        questions: [q],
        policy: BD_POLICY,
        metadata: { request_id: "try-a-decision", limits: BD_LIMITS },
      };
    },
    /* best-effort reverse mapping for the JSON→Form switch; null when the
       JSON was shaped by hand and cannot populate the form */
    parseRequest: function (req) {
      if (!req || !req.state || typeof req.state.text !== "string") return null;
      if (!Array.isArray(req.questions) || req.questions.length !== 1) return null;
      const q = req.questions[0];
      if (q.type === "yes-no" || q.type === "boolean") return { category: "yes-no", state: req.state.text, question: q.text, yesWhen: "", noWhen: "" };
      if (q.type === "choice" && Array.isArray(q.candidates)) {
        return { category: "choice", state: req.state.text, question: q.text, candidates: q.candidates };
      }
      if (q.type === "score" && Array.isArray(q.levels)) {
        return { category: "score", state: req.state.text, question: q.text,
          levels: q.levels.map(function (l) { return l.label; }).join(", ") };
      }
      return null;
    },
  };
  window.TryDecision = TryDecision;

  const BD_EXAMPLES = {
    "yes-no": {
      state: "replicas must be 2. prod-03 currently runs 2 replicas.",
      question: "Is prod-03 compliant with the replica rule?",
      yesWhen: "the service runs at least the replica count the rule requires",
      noWhen: "the service runs fewer replicas than the rule requires",
    },
    choice: {
      state: "api depends on billing. billing depends on catalog. api is down. billing is down.",
      question: "Which service is the root cause of the outage?",
      candidates: [
        { id: "api", description: "The public API tier" },
        { id: "billing", description: "The billing service" },
        { id: "catalog", description: "The product catalog" },
      ],
    },
    score: {
      state: "Service latency p99 is 2400ms. The SLO says p99 must stay under 500ms.",
      question: "How severe is this SLO breach?",
      levels: "trivial, minor, major, critical",
    },
  };
  const BD_PRESETS = [
    { id: "yes-no", label: "replica rule" },
    { id: "yes-no", label: "door policy", ex: {
      state: "Visitor has a valid day pass: yes. Pass expires at 18:00. Current time is 17:20.",
      question: "Should the visitor be admitted now?",
      yesWhen: "the pass is valid and the current time is before its expiry",
      noWhen: "the pass is expired, invalid, or missing",
    } },
    { id: "choice", label: "outage root cause" },
    { id: "choice", label: "cheapest tier", ex: {
      state: "Workload: 40k decisions/day, bursty, no GPU on site, data stays local.",
      question: "Which deployment tier fits this workload best?",
      candidates: [
        { id: "local-cpu", description: "Runs on existing CPUs, data never leaves" },
        { id: "hosted-api", description: "Per-decision cost, no local hardware" },
        { id: "gpu-box", description: "Fastest tail, wasted on this volume" },
      ],
    } },
    { id: "score", label: "SLO severity" },
    { id: "score", label: "support ticket", ex: {
      state: "Customer reports every login attempt fails since this morning. Paid tier, 200 seats affected.",
      question: "How should this ticket be prioritized?",
      levels: "low, normal, high, urgent",
    } },
  ];
  function bdRenderPresets() {
    const wrap = $("bd-presets");
    BD_PRESETS.forEach(function (pr) {
      const b = document.createElement("button");
      b.type = "button";
      b.className = "bd-preset-chip";
      b.textContent = pr.label;
      b.addEventListener("click", function () {
        $("bd-category").value = pr.id;
        bdLoadExample(pr.id, pr.ex);
        wrap.querySelectorAll(".bd-preset-chip").forEach(function (c) { c.classList.remove("on"); });
        b.classList.add("on");
      });
      wrap.appendChild(b);
    });
  }
  const BD_CATNOTE = {
    "yes-no": "A boolean question carries no candidates — the engine returns true or false.",
    choice: "A choice question lists its candidates; the engine may only answer with one of them.",
    score: "A score question ranks its labels — the answer is the top level, with a distribution. In this zero-ML tab a score needs state/label word overlap or a fired rule; with neither, the runtime abstains rather than guessing.",
  };

  let bdCandSeq = 0;
  function bdCandRow(cid, cdesc) {
    const id = "bd-cand-" + (++bdCandSeq);
    const wrap = document.createElement("div");
    wrap.className = "bd-cand";
    wrap.innerHTML =
      '<input type="text" class="bd-cid" id="' + id + '" placeholder="id" value="' + esc(cid || "") + '" aria-label="candidate id">' +
      '<input type="text" class="bd-cdesc" placeholder="description (what makes it the answer)" value="' + esc(cdesc || "") + '" aria-label="candidate description">' +
      '<button type="button" class="bd-cand-remove" title="remove candidate" aria-label="remove candidate">✕</button>';
    wrap.querySelector(".bd-cand-remove").addEventListener("click", function () {
      wrap.remove(); bdSyncJson();
    });
    wrap.addEventListener("input", bdSyncJson);
    return wrap;
  }

  function bdFormInput() {
    return {
      category: $("bd-category").value,
      state: $("bd-state").value,
      question: $("bd-question").value,
      yesWhen: $("bd-yes-when").value,
      noWhen: $("bd-no-when").value,
      levels: $("bd-levels").value,
      candidates: Array.prototype.map.call(
        document.querySelectorAll("#bd-cands .bd-cand"),
        function (row) {
          return { id: row.querySelector(".bd-cid").value, description: row.querySelector(".bd-cdesc").value };
        }),
    };
  }

  function bdCurrentJson() {
    if ($("bd-jsonpane").hidden) {
      return JSON.stringify(TryDecision.buildRequest(bdFormInput()), null, 2) + "\n";
    }
    return $("bd-json").value;
  }

  function bdSyncJson() {
    const note = $("bd-state-note");
    const state = $("bd-state").value;
    if (TryDecision.looksLikeJson(state)) {
      note.textContent = "JSON detected — sent to the engine as text (state.text). Structure belongs in facts, via the console JSON below.";
      note.classList.add("bd-note-bad");
    } else {
      note.textContent = "Plain text is sent as-is. JSON pasted here is also sent as text — the engine reads text.";
      note.classList.remove("bd-note-bad");
    }
    const crit = $("bd-crit-note");
    if (crit) {
      const composed = TryDecision.composeBooleanText(
        $("bd-question").value, $("bd-yes-when").value, $("bd-no-when").value);
      const n = TryDecision.countNegators(composed);
      const odd = n % 2 === 1;
      crit.textContent = "negation words in the composed question: " + n +
        (odd ? " — an ODD count inverts how the lexical rung reads polarity (not / no / never / without / cannot / neither / nor)"
             : " (even — polarity reads as asked)");
      crit.classList.toggle("bd-note-bad", odd);
    }
    if (!$("bd-jsonpane").hidden) $("bd-json").value = bdCurrentJson();
  }

  function bdLoadExample(cat, ex) {
    if (typeof ex === "undefined") ex = BD_EXAMPLES[cat];
    $("bd-state").value = ex.state;
    $("bd-question").value = ex.question;
    $("bd-yes-when").value = ex.yesWhen || "";
    $("bd-no-when").value = ex.noWhen || "";
    $("bd-levels").value = ex.levels || "";
    const cands = $("bd-cands");
    cands.textContent = "";
    (ex.candidates && ex.candidates.length ? ex.candidates : [{ id: "", description: "" }])
      .forEach(function (c) { cands.appendChild(bdCandRow(c.id, c.description)); });
    $("bd-catnote").textContent = BD_CATNOTE[cat] || "";
    bdSyncJson();
  }

  function bdSetView(json) {
    const showJson = !!json;
    $("bd-formpane").hidden = showJson;
    $("bd-jsonpane").hidden = !showJson;
    $("bd-view-form").classList.toggle("on", !showJson);
    $("bd-view-json").classList.toggle("on", showJson);
    $("bd-view-form").setAttribute("aria-pressed", String(!showJson));
    $("bd-view-json").setAttribute("aria-pressed", String(showJson));
    if (showJson) $("bd-json").value = JSON.stringify(TryDecision.buildRequest(bdFormInput()), null, 2) + "\n";
  }

  function bdDecide() {
    reqEditor.value = bdCurrentJson();
    $("act-decide").click();          /* one dispatch path: same renderers */
    document.querySelector(".pg-results").scrollIntoView({ behavior: "smooth", block: "start" });
  }

  function bindBuilder() {
    $("bd-category").addEventListener("change", function (e) { bdLoadExample(e.target.value); });
    ["bd-state", "bd-question", "bd-yes-when", "bd-no-when", "bd-levels"].forEach(function (id) {
      byId(id).addEventListener("input", bdSyncJson);
    });
    $("bd-add-cand").addEventListener("click", function () {
      $("bd-cands").appendChild(bdCandRow("", ""));
    });
    $("bd-view-form").addEventListener("click", function () {
      /* leaving JSON: populate the form only when the JSON matches its shape */
      if (!$("bd-jsonpane").hidden) {
        const parsed = tryParse($("bd-json").value);
        const fromJson = parsed ? TryDecision.parseRequest(parsed) : null;
        if (fromJson) {
          $("bd-category").value = fromJson.category;
          $("bd-state").value = fromJson.state;
          $("bd-question").value = fromJson.question;
          if (fromJson.category === "score") $("bd-levels").value = fromJson.levels;
          if (fromJson.category === "choice" && fromJson.candidates.length) {
            const cands = $("bd-cands");
            cands.textContent = "";
            fromJson.candidates.forEach(function (c) { cands.appendChild(bdCandRow(c.id, c.description)); });
          }
        }
      }
      bdSetView(false);
      bdSyncJson();
    });
    $("bd-view-json").addEventListener("click", function () { bdSetView(true); });
    $("bd-decide").addEventListener("click", bdDecide);
    bdRenderPresets();
    bdLoadExample("yes-no");
  }

  /* ---------- boot ---------- */
  function boot() {
    bindBuilder();
    reqEditor.value = JSON.stringify(PRESETS.proof.request, null, 2) + "\n";
    graphEditor.value = JSON.stringify(GRAPHS.minimal.graph, null, 2) + "\n";

    /* api.html handoff: "run in playground" links set oc-preset */
    let handoff = null;
    try { handoff = JSON.parse(sessionStorage.getItem("oc-preset") || "null"); } catch (e) { /* ok */ }
    if (handoff && handoff.request && PRESETS[handoff.request]) {
      reqEditor.value = JSON.stringify(PRESETS[handoff.request].request, null, 2) + "\n";
      $("preset-select").value = handoff.request;
      sessionStorage.removeItem("oc-preset");
    }

    bindActions();
    initEngine();
    measureNetwork();
    window.addEventListener("load", measureNetwork);
  }

  boot();
})();
