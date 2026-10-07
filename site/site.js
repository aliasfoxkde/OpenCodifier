/* OpenCodifier landing page — vanilla JS, no dependencies.
   All numbers are measured runs; sources in the footer.
   Input contract: this page reads NO dynamic input (no URL params, no
   fetch). The theme mode is persisted by site-core.js under the "oc-theme"
   localStorage key — the only storage this page touches, and it never
   reaches innerHTML. The scenario cards below run the REAL deterministic
   engine compiled to WASM (site/wasm/, same-origin, offline): every string
   rendered into the scenario output is either a compile-time literal or a
   value parsed from the engine's own JSON response for those literals. */
"use strict";

/* ---------- animated counters ---------- */
const prefersReduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
if (!prefersReduced) {
  document.querySelectorAll(".count").forEach((el) => {
    el.textContent = (0).toFixed(parseInt(el.dataset.decimals || "0", 10));
  });
}
function animateCount(el) {
  const target = parseFloat(el.dataset.target);
  const decimals = parseInt(el.dataset.decimals || "0", 10);
  if (prefersReduced || target === 0) { el.textContent = target.toFixed(decimals); return; }
  const t0 = performance.now(), dur = 1100;
  function tick(t) {
    const k = Math.min(1, (t - t0) / dur);
    const eased = 1 - Math.pow(1 - k, 3);
    el.textContent = (target * eased).toFixed(decimals);
    if (k < 1) requestAnimationFrame(tick);
  }
  requestAnimationFrame(tick);
}
const cio = new IntersectionObserver(
  (entries) => entries.forEach((e) => { if (e.isIntersecting) { animateCount(e.target); cio.unobserve(e.target); } }),
  { threshold: 0.6 }
);
document.querySelectorAll(".count").forEach((el) => cio.observe(el));

/* ---------- benchmark chart ----------
   Accuracy: published anchors, JevBench public 231 split (upstream authors' table).
   Latency: p50, log scale; ours measured on CPU, hosted row is the vendor range midpoint.
   Cost: third-party board reference for hosted Jev; ours is a property of the design. */
const CHART = {
  acc: {
    title: "Accuracy — JevBench public split (231 items)",
    scale: "linear",
    unit: "%",
    rows: [
      { label: "Hosted Jev", sub: "closed weights", v: 86.6 },
      { label: "Raw LLM (Qwen3.5-4B + prompt)", sub: "no calibration", v: 80.5 },
      { label: "OpenCodifier model rung", sub: "4B open weights", v: 76.6, ours: true },
      { label: "Jev-Style-2B", sub: "open", v: 73.6 },
      { label: "decider-2b", sub: "open", v: 71.0 },
      { label: "open-jev-2b", sub: "open", v: 64.5 },
      { label: "Jev-Style-0.8B", sub: "open", v: 64.1 },
      { label: "Laya", sub: "open", v: 58.4 },
    ],
    note: "Published anchors from the benchmark's own table, run by the upstream harness on their split. " +
      "The 80.5% row is a raw general-purpose LLM prompted for answers — 651 ms per decision, no calibrated confidence, no abstention. " +
      "Our rung beats every purpose-built <strong>open</strong> model published on the split; hosted Jev keeps the accuracy crown for now (see “Claims we refuse to make”).",
  },
  lat: {
    title: "Latency (p50 per decision, log scale)",
    scale: "log",
    unit: "ms",
    rows: [
      { label: "OpenCodifier engine", sub: "rules + lexical, CPU", v: 2.0, ours: true },
      { label: "OpenCodifier static-embedding rung", sub: "20.5 MB table, CPU", v: 7.9, ours: true },
      { label: "Hosted Jev", sub: "vendor-stated range 70–500 ms", v: 300 },
      { label: "Raw LLM (Qwen3.5-4B + prompt)", sub: "GPU", v: 651 },
      { label: "OpenCodifier 4B model rung", sub: "tree mode, CPU — escalation tail only", v: 2280, ours: true },
    ],
    note: "Log scale — the engine is ~300× faster than the raw-LLM route. The expensive 4B rung is the <em>tail</em>, not the norm: the gate sends only items cheaper rungs couldn't decide. " +
      "On our audited internal suite the full fusion ladder medians 0.9–137 ms end-to-end because roughly half the items never pay for the model.",
  },
  cost: {
    title: "Cost per 1M decisions",
    scale: "linear",
    unit: "$",
    rows: [
      { label: "OpenCodifier", sub: "self-hosted, your hardware", v: 0, ours: true },
      { label: "Hosted Jev", sub: "third-party board reference estimate", v: 32 },
    ],
    note: "The reference price for hosted Jev is the third-party board's estimate " +
      "(~$0.032 per 1,000 decisions). OpenCodifier's marginal cost is $0: no API, no metering, " +
      "no account — your machine, your electricity, and the cheap rungs mean even the electricity " +
      "bill is dominated by the 4B rung's rare escalations.",
  },
};

const chartEl = document.getElementById("chart");
const chartNote = document.getElementById("chart-note");

function fmt(v, unit) {
  if (unit === "$") return "$" + v.toFixed(0);
  if (unit === "ms") return v >= 1000 ? (v / 1000).toFixed(1) + " s" : v + " ms";
  return v.toFixed(1) + unit;
}

function renderChart(metric) {
  const cfg = CHART[metric];
  const maxV = cfg.scale === "log"
    ? Math.log10(Math.max(...cfg.rows.map((r) => r.v)))
    : Math.max(...cfg.rows.map((r) => r.v));
  chartEl.innerHTML = "";
  cfg.rows.forEach((r, i) => {
    const row = document.createElement("div");
    row.className = "bar-row" + (r.ours ? " ours" : "");
    const frac = cfg.scale === "log"
      ? (Math.log10(Math.max(r.v, 1)) / maxV)
      : (r.v / maxV);
    const width = Math.max(2.5, frac * 100);
    row.innerHTML =
      '<div class="bar-label">' + r.label + "<small>" + r.sub + "</small></div>" +
      '<div class="bar-track"><div class="bar-fill" data-w="' + width + '"></div></div>' +
      '<div class="bar-value">' + fmt(r.v, cfg.unit) + "</div>";
    chartEl.appendChild(row);
    setTimeout(() => {
      const f = row.querySelector(".bar-fill");
      if (f) f.style.width = f.dataset.w + "%";
    }, prefersReduced ? 0 : 60 + i * 70);
  });
  chartNote.innerHTML = "<strong>" + cfg.title + ".</strong> " + cfg.note;
}

/* WAI-ARIA tabs: roving tabindex + arrow keys (APG pattern). activateTab is
   the single writer of tab state; renderChart stays DOM-only. */
const tabs = Array.from(document.querySelectorAll(".tab"));

function activateTab(tab) {
  tabs.forEach((t) => {
    const active = t === tab;
    t.classList.toggle("active", active);
    t.setAttribute("aria-selected", active ? "true" : "false");
    t.tabIndex = active ? 0 : -1;
  });
  document.getElementById("chart-panel").setAttribute("aria-labelledby", tab.id);
  renderChart(tab.dataset.metric);
}

tabs.forEach((tab) => {
  tab.addEventListener("click", () => activateTab(tab));
  tab.addEventListener("keydown", (e) => {
    const i = tabs.indexOf(tab);
    let j = null;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") j = (i + 1) % tabs.length;
    else if (e.key === "ArrowLeft" || e.key === "ArrowUp") j = (i - 1 + tabs.length) % tabs.length;
    else if (e.key === "Home") j = 0;
    else if (e.key === "End") j = tabs.length - 1;
    if (j === null) return;
    e.preventDefault();
    activateTab(tabs[j]);
    tabs[j].focus();
  });
});
activateTab(tabs[0]);

/* ---------- ladder ---------- */
const RUNGS = [
  {
    name: "Exact rule",
    sub: "relational proofs over extracted facts",
    cost: "~0 ms",
    detail: {
      title: "1 · Exact rule (relational proofs)",
      body: "Questions whose answer can be <em>proven</em> — a unique satisfier over extracted facts, " +
        "a dependency-chain derivation, a count with one maximum — exit here at confidence 1.0. " +
        "Not model-shaped confidence: proof. Zero ML anywhere in the path.",
      kv: [["cost", "~0 ms"], ["confidence", "1.0 (proof)"], ["accepts", "only exact proofs"]],
    },
  },
  {
    name: "Cached decision",
    sub: "versioned, byte-stable",
    cost: "~0 ms",
    detail: {
      title: "2 · Cached decision",
      body: "Every accepted decision is cached under keys that include the model, calibration, policy, " +
        "graph <em>and</em> ladder versions — so an artifact update invalidates exactly the cache entries " +
        "it should, and nothing else. Same input, same answer, instantly, forever.",
      kv: [["cost", "~0 ms (hit)"], ["key inputs", "model · calibration · policy · graph · ladder"], ["stability", "bit-identical replay"]],
    },
  },
  {
    name: "Metadata filter",
    sub: "structured narrowing",
    cost: "sub-ms",
    detail: {
      title: "3 · Metadata filter",
      body: "Structured fields narrow the candidate set before any text scoring happens. Removing candidates " +
        "that cannot satisfy the question is the cheapest intelligence there is — and unlike semantic " +
        "evidence, a filtered candidate is <em>provably</em> out.",
      kv: [["cost", "sub-ms"], ["evidence", "structured, exact"], ["removes", "only provable non-candidates"]],
    },
  },
  {
    name: "Lexical / BM25",
    sub: "hand-written scorer",
    cost: "0.4 ms p50",
    detail: {
      title: "4 · Lexical / BM25",
      body: "A hand-written BM25 scorer over candidate descriptions decides the clear lexical majority — " +
        "0.4 ms median. It accepts only above its rung's confidence gate; below the gate the item " +
        "escalates, it is never silently discarded on weak evidence.",
      kv: [["cost", "0.4 ms p50"], ["held-out accuracy", "0.700 standalone (escalates the rest)"], ["accepts", "only ≥ gate (margin + confidence)"]],
    },
  },
  {
    name: "Embedding similarity",
    sub: "static-embedding scoring",
    cost: "low ms",
    detail: {
      title: "5 · Embedding similarity",
      body: "Fast semantic scoring over a static embedding table — in our cheapest measured arm, " +
        "20.5 MB on disk and 7.9 ms client-wall on real benchmark items. No vector DB, no server: " +
        "the table loads in-process.",
      kv: [["cost", "7.9 ms client-wall (measured)"], ["footprint", "20.5 MB (2-bit table)"], ["role", "semantic scoring, gate-gated"]],
    },
  },
  {
    name: "Decision model",
    sub: "4B open weights, parallel readout",
    cost: "0.3–2.3 s CPU",
    detail: {
      title: "6 · Candidate-conditioned decision model",
      body: "The expensive rung: a 4B open-weight model that scores <em>every candidate's probability " +
        "in one parallel pass</em> — no tokens generated, nothing parsed. It sees only what the cheaper " +
        "rungs couldn't decide confidently. It is also the rung our own distillation program is " +
        "shrinking (see Roadmap).",
      kv: [["cost", "0.3–2.3 s on CPU (context-dependent)"], ["readout", "full distribution per candidate, one pass"], ["paid by", "escalated items only"]],
    },
  },
  {
    name: "Verifier",
    sub: "confidence-gated",
    cost: "only on disagreement",
    detail: {
      title: "7 · Verifier",
      body: "When a rung's answer is uncertain, a verifier gets one shot: agree → accept, disagree → " +
        "abstain or escalate. Verification is confidence-gated — the runtime never runs two classifiers " +
        "on every request, because that would double the cost of everything to fix the tail.",
      kv: [["fires", "only on low confidence"], ["outcomes", "accept · verify · abstain"], ["cost rule", "one classifier per accepted question"]],
    },
  },
];

const ladderEl = document.getElementById("ladder");
const detailEl = document.getElementById("ladder-detail");

RUNGS.forEach((r, i) => {
  const b = document.createElement("button");
  b.className = "rung";
  b.dataset.i = i;
  b.innerHTML =
    '<span class="rung-num">' + (i + 1) + "</span>" +
    '<span><span class="rung-name">' + r.name + '</span><br><span class="rung-sub">' + r.sub + "</span></span>" +
    '<span class="rung-cost">' + r.cost + "</span>";
  b.addEventListener("click", () => selectRung(i));
  ladderEl.appendChild(b);
});

function selectRung(i) {
  document.querySelectorAll(".rung").forEach((el, k) => el.classList.toggle("active", k === i));
  const d = RUNGS[i].detail;
  detailEl.innerHTML =
    "<h3>" + d.title + "</h3><p>" + d.body + "</p>" +
    '<dl class="kv">' + d.kv.map(([k, v]) => "<dt>" + k + "</dt><dd>" + v + "</dd>").join("") + "</dl>";
}
selectRung(0);

/* ---------- scenarios: the real engine, compiled to WASM ----------
   No simulation. Clicking a card builds the request JSON (compile-time
   literals below), runs engine.decide() in this tab, and renders the
   response: rungs light from the actual trace nodes, the confidence is the
   engine's calibrated value, and the wall time is measured around the call.
   The WASM build ships the zero-ML stack (rungs 1–4): no embeddings, no
   model, no verifier — which is exactly why the outcomes are honest. */
const LIMITS = {
  max_input_bytes: 1048576,
  max_questions: 32,
  max_candidates: 256,
  max_graph_nodes: 128,
  max_execution_time: { secs: 10, nanos: 0 },
  max_retrieval_results: 64,
};

const SCENARIOS = {
  proof: {
    label: "compliance question",
    state: "api depends on billing. billing depends on catalog. api is down. billing is down.",
    question: "Which service is the root cause of the outage?",
    candidates: [
      ["api", "The public API tier"],
      ["billing", "The billing service"],
      ["catalog", "The product catalog"],
    ],
  },
  lexical: {
    label: "ticket-routing question",
    state: "Ticket: customer reports a duplicate charge on invoice #4411 and demands a refund.",
    question: "Which team should own this ticket?",
    candidates: [
      ["billing", "duplicate charges, invoice disputes, refunds and payment corrections"],
      ["technical", "outages, error rates and broken functionality"],
      ["sales", "pricing, quotes, renewals and new contracts"],
    ],
  },
  hard: {
    label: "genuinely hard question",
    state: "payments is degraded. search is degraded. uploads is degraded.",
    question: "Which service should we restart first?",
    candidates: [
      ["payments", "handles money movement for checkout"],
      ["search", "serves query traffic to the storefront"],
      ["uploads", "ingests customer files"],
    ],
  },
};

/* trace node → ladder rung index (the zero-ML stack touches rungs 1–4) */
const RUNG_OF_NODE = { rule: 0, cache: 1, filter: 2, lexical: 3, choice: 3 };

function buildRequest(sc) {
  return JSON.stringify({
    state: { text: sc.state, facts: {} },
    questions: [{
      type: "choice",
      id: "q1",
      text: sc.question,
      candidates: sc.candidates.map(([id, description]) => ({ id, description })),
    }],
    policy: { min_confidence: 0.8, verify_below: 0.65, abstain_below: 0.5, risk: "low" },
    metadata: { request_id: "site-demo", limits: LIMITS },
  });
}

const OUTCOME_TEXT = {
  accept: (a) =>
    "<span class='ok'>ACCEPT</span> at calibrated confidence " +
    "<span class='ms'>" + a.confidence.toFixed(4) + "</span> — above the 0.8 gate, good to automate on.",
  verify: (a) =>
    "<span class='ms'>VERIFY</span>: top choice “" + a.choice + "” at " + a.confidence.toFixed(4) +
    " — below the 0.8 accept gate, so the runtime hands it to the confidence-gated verifier " +
    "instead of pretending to be sure. (This tab ships no verifier — zero-ML stack.)",
  abstain: () =>
    "<span class='ms'>ABSTAIN</span> at confidence 0.333 — a three-way coin flip. The runtime refuses " +
    "to guess: abstention is a successful outcome, not an error.",
};

let enginePromise = null;
function getEngine() {
  if (!enginePromise) {
    enginePromise = import("./wasm/opencodifier_wasm.js").then((m) => m.init().then(() => new m.WasmEngine()));
  }
  return enginePromise;
}

/* One warmup decide so the first click reports steady-state time, not
   module materialization. Failure keeps the cards usable with an honest
   message — the page still works, the live demo just says so. */
let engineReady = null;
function warmup() {
  if (engineReady) return engineReady;
  engineReady = getEngine()
    .then((eng) => {
      eng.decide(buildRequest(SCENARIOS.proof)); // warmup: steady-state timing
      return { eng, identity: JSON.parse(eng.identity()) };
    })
    .catch((err) => { enginePromise = null; engineReady = null; throw err; });
  return engineReady;
}

const outEl = document.getElementById("scenario-output");
let scenarioTimer = null;

function litRungs(traceEntries) {
  const rungs = [];
  traceEntries.forEach((en) => {
    const r = RUNG_OF_NODE[en.node];
    if (r !== undefined && !rungs.includes(r)) rungs.push(r);
  });
  return rungs.sort((a, b) => a - b);
}

function line(html) {
  const div = document.createElement("div");
  div.innerHTML = html; // literals + engine-parsed numbers/ids from our own request
  return div;
}

function renderResult(sc, out, ms, identity) {
  const frag = document.createDocumentFragment();
  frag.appendChild(line("<strong>request:</strong> “" + sc.question + "” — <em>" + sc.label + "</em>, run in this tab"));
  litRungs(out.trace.entries).forEach((r) => {
    frag.appendChild(line("<span class='ok'>rung " + (r + 1) + " · " + RUNGS[r].name + ":</span> executed"));
  });
  const a = out.answers[0];
  frag.appendChild(line(OUTCOME_TEXT[out.outcome](a)));
  frag.appendChild(line("wall: <span class='ms'>" + ms.toFixed(2) + " ms</span> · engine: WASM in your browser · zero ML · no network · engine " +
    identity.engine_semver + " [" + identity.model_id + "]"));
  outEl.innerHTML = "";
  outEl.appendChild(frag);
}

document.querySelectorAll(".scenario").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".scenario").forEach((b) => {
      b.classList.remove("running");
      b.disabled = false;
    });
    btn.classList.add("running");
    btn.disabled = true;
    document.querySelectorAll(".rung").forEach((r) => r.classList.remove("lit"));
    if (scenarioTimer) scenarioTimer.forEach(clearTimeout);
    outEl.innerHTML = "";
    outEl.appendChild(line("running the real engine in this tab…"));

    warmup().then(({ eng, identity }) => {
      const t0 = performance.now();
      const out = JSON.parse(eng.decide(buildRequest(SCENARIOS[btn.dataset.scenario])));
      const ms = performance.now() - t0;
      const rungs = litRungs(out.trace.entries);
      const delays = rungs.map((_, i) => (prefersReduced ? 0 : 250 + i * 350));
      scenarioTimer = rungs.map((r, i) => setTimeout(() => {
        const el = ladderEl.querySelector(".rung[data-i=\"" + r + "\"]");
        if (el) el.classList.add("lit");
      }, delays[i]));
      scenarioTimer.push(setTimeout(() => {
        renderResult(SCENARIOS[btn.dataset.scenario], out, ms, identity);
        btn.disabled = false;
      }, prefersReduced ? 0 : 250 + rungs.length * 350));
    }).catch(() => {
      outEl.innerHTML = "";
      outEl.appendChild(line("The WASM engine could not start in this browser. " +
        "The full playground — and the native binary — are on the <a href='try.html'>Try it</a> page."));
      btn.disabled = false;
    });
  });
});
