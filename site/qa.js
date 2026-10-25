/* QA tool shell — the playground's yes/no tool on the Decisions SDK (#196).
   qa-core.js owns the knowledge panel, the wire contract, and the three
   honest states; this file is chrome: the question box, the editable
   knowledge panel, the verdict card, the mutually-exclusive disclosures
   (knowledge / trace / escalation payload), and the session stats overlay.
   The score on screen is always the engine's real output. */
"use strict";

(function () {
  const DK = globalThis.DecisionsSDK;
  const QA = globalThis.QaCore;
  const root = document.getElementById("qa-root");
  if (!root || !DK || !QA) return;

  const h = DK.shell.h;
  const byId = (id) => document.getElementById(id);
  const fmtPct = DK.fmtPct;

  const STATE_LABEL = {
    decided: "decided",
    abstained: "abstained — no honest answer",
    escalated: "escalated — needs a second opinion",
    unavailable: "engine unavailable",
  };

  const S = {
    rules: QA.normalizeRules(QA.BUILT_IN_RULES).rules,
    question: "",
    context: "",
    last: null,           /* { response, state, fired, payload } */
    engine: DK.engineBridge({}),
    meters: DK.meters(),
    board: DK.sessionBoard(),
    stats: { decided: 0, abstained: 0, escalated: 0, unavailable: 0 },
  };

  /* ---------- ask ---------- */

  async function ask() {
    const question = byId("qa-question").value.trim();
    if (!question) return;
    const context = byId("qa-context").value;
    const composed = QA.composeRequest(question, context, S.rules);

    const btn = byId("qa-ask");
    btn.disabled = true;
    try {
      await S.engine.ensure();
      const response = S.engine.choose(composed.request);
      const state = QA.classify(response);
      S.last = {
        response,
        state,
        fired: composed.fired,
        payload: state === "escalated" ? QA.escalationPayload(question, context, S.rules, response) : null,
      };
      S.stats[state] = (S.stats[state] || 0) + 1;
      renderVerdict();
    } finally {
      btn.disabled = false;
    }
  }

  /* ---------- verdict card ---------- */

  function renderVerdict() {
    const out = byId("qa-verdict");
    out.textContent = "";
    if (!S.last) return;
    const { response, state } = S.last;
    const conf = response && response.confidence ? response.confidence.calibrated_confidence : null;

    out.appendChild(h("div", { class: "qa-verdict card", "data-state": state },
      h("span", { class: "pg-badge" + (state === "decided" ? " live" : state === "unavailable" ? "" : " soon"), role: "status" },
        STATE_LABEL[state]),
      conf !== null
        ? h("p", { class: "qa-conf" },
            h("strong", { class: "qa-conf-num" }, fmtPct(conf)),
            " calibrated confidence — the engine's real output")
        : h("p", { class: "qa-conf" }, "no score — the engine produced nothing to grade"),
      state === "decided" && response.answers && response.answers[0]
        ? h("p", null, "Answer: ",
            h("strong", null, response.answers[0].value === true ? "yes" : "no"),
            " at probability ",
            h("code", null, String(response.answers[0].probability)))
        : null,
      state === "abstained"
        ? h("p", null, "The runtime refused to guess under this evidence. That is a successful outcome, not an error — a wrong answer would be worse.")
        : null,
      state === "escalated"
        ? h("p", null, "The confidence gate did not accept, so the runtime asks for a second opinion — exactly what a host model would receive.")
        : null,
      state === "unavailable"
        ? h("p", null, "The WASM engine did not boot in this tab. Nothing was scored; nothing is pretended.")
        : null));

    if (S.last.fired.length) {
      out.appendChild(h("p", { class: "fineprint" },
        "Rules that fired: " +
        S.last.fired.map((r) => r.keyword + " → " + r.factKey).join(", ") +
        ". Their facts were attached to the state text — see the knowledge panel."));
    } else {
      out.appendChild(h("p", { class: "fineprint" },
        "No knowledge-panel rule matched, so the state carried your words only. The engine has no world knowledge — a bare question it cannot ground is exactly how an abstain happens."));
    }
    renderStats();
  }

  function renderStats() {
    const el = byId("qa-stats");
    if (el) el.textContent =
      "Session stats — decided " + S.stats.decided + " · abstained " + S.stats.abstained +
      " · escalated " + S.stats.escalated + " · unavailable " + S.stats.unavailable;
  }

  /* ---------- knowledge panel ---------- */

  function rulesEditor() {
    const box = h("div", { class: "qa-rules" });
    S.rules.forEach((rule, i) => {
      box.appendChild(h("div", { class: "qa-rule" },
        h("code", null, rule.keyword),
        " → ",
        h("code", null, rule.factKey + " = " + rule.factValue),
        " (" + rule.answer + ") ",
        h("button", { type: "button", class: "btn btn-ghost btn-small", "data-del": String(i) }, "remove")));
    });
    box.addEventListener("click", (e) => {
      const btn = e.target && e.target.closest ? e.target.closest("[data-del]") : null;
      if (!btn) return;
      S.rules.splice(Number(btn.getAttribute("data-del")), 1);
      renderRules();
    });
    return box;
  }

  function renderRules() {
    const pane = byId("qa-rules-pane");
    pane.textContent = "";
    pane.appendChild(rulesEditor());

    const form = h("div", { class: "qa-rule-form" },
      h("div", { class: "qa-rule-grid" },
        h("label", null, "when the question contains",
          h("input", { id: "qa-r-kw", type: "text", spellcheck: "false" })),
        h("label", null, "attach fact",
          h("input", { id: "qa-r-key", type: "text", spellcheck: "false" })),
        h("label", null, "with value",
          h("input", { id: "qa-r-val", type: "text", spellcheck: "false" })),
        h("label", null, "asserting",
          h("select", { id: "qa-r-ans" },
            h("option", { value: "yes" }, "yes"),
            h("option", { value: "no" }, "no")))),
      h("button", { type: "button", class: "btn btn-small", id: "qa-r-add" }, "+ add rule"),
      h("p", { class: "fineprint", id: "qa-r-error", role: "alert" }));
    form.addEventListener("click", (e) => {
      if (e.target && e.target.id !== "qa-r-add") return;
      const candidate = {
        keyword: byId("qa-r-kw").value,
        factKey: byId("qa-r-key").value,
        factValue: byId("qa-r-val").value,
        answer: byId("qa-r-ans").value,
      };
      const problem = QA.validateRule(candidate);
      byId("qa-r-error").textContent = problem || "";
      if (problem) return;
      S.rules.push({
        keyword: candidate.keyword.trim(),
        factKey: candidate.factKey.trim(),
        factValue: candidate.factValue.trim(),
        answer: candidate.answer,
        note: "",
      });
      byId("qa-r-kw").value = byId("qa-r-key").value = byId("qa-r-val").value = "";
      renderRules();
    });
    pane.appendChild(form);
    pane.appendChild(h("p", { class: "fineprint" },
      "Rules are the page's only knowledge. They bind a question keyword to a fact attached to the state text — visible, editable, and the reason any confident answer is honest. Delete them all and every bare question abstains."));
  }

  /* ---------- disclosures (mutually exclusive) ---------- */

  function renderDisclosures() {
    const host = byId("qa-disclosures");
    host.textContent = "";
    const trace = S.last && S.last.response ? S.last.response.trace : null;
    const conf = S.last && S.last.response && S.last.response.confidence
      ? S.last.response.confidence : null;

    const items = [
      {
        id: "knowledge", label: "Knowledge panel",
        kids: [h("div", { id: "qa-rules-pane" })],
      },
      {
        id: "trace", label: "Execution trace",
        kids: [trace
          ? h("pre", { class: "qa-json" }, JSON.stringify(trace, null, 2))
          : h("p", { class: "fineprint" }, "Ask a question first — the trace is the pipeline's own record of what ran.")],
      },
      {
        id: "confidence", label: "Confidence report",
        kids: [conf
          ? h("pre", { class: "qa-json" }, JSON.stringify(conf, null, 2))
          : h("p", { class: "fineprint" }, "The six-component report (top probability, margin, entropy, OOD, verifier agreement, calibrated confidence) appears with the first answer.")],
      },
      {
        id: "escalation", label: "Escalation payload",
        kids: [S.last && S.last.payload
          ? h("pre", { class: "qa-json" }, JSON.stringify(S.last.payload, null, 2))
          : h("p", { class: "fineprint" }, "The payload a host model would receive appears when the gate trips (confidence below " + QA.ESCALATE_BELOW + ").")],
      },
    ];

    host.appendChild(DK.shell.accordion(items));
    renderRules(); /* the knowledge pane is inside the accordion */
  }

  /* ---------- boot ---------- */

  function boot() {
    root.textContent = "";

    root.appendChild(h("div", { class: "card qa-askcard" },
      h("div", { class: "ctl" },
        h("label", { for: "qa-question" }, "Your yes/no question"),
        h("input", { id: "qa-question", type: "text", spellcheck: "false",
          placeholder: "e.g. is the sky blue", "aria-describedby": "qa-question-note" }),
        h("p", { class: "fineprint", id: "qa-question-note" },
          "State it affirmatively — the runtime's boolean parity inverts negators, so “isn't it blue?” asks the opposite question.")),
      h("div", { class: "ctl" },
        h("label", { for: "qa-context" }, "Context (optional)"),
        h("textarea", { id: "qa-context", rows: "2", spellcheck: "false" })),
      h("div", { class: "qa-actions" },
        h("button", { type: "button", class: "btn", id: "qa-ask", onclick: ask }, "Ask"),
        h("span", { class: "fineprint", id: "qa-status" }, "engine: " + S.engine.status)),
      h("div", { id: "qa-verdict", "aria-live": "polite" })));

    root.appendChild(h("div", { id: "qa-disclosures" }));
    renderDisclosures();

    root.appendChild(h("div", { class: "grid-2 game-about", style: "margin-top:1.4rem" },
      h("div", { class: "card" },
        h("h3", null, "What decides what"),
        h("ul", { class: "checks" },
          h("li", null, h("strong", null, "The score is real. "),
            "Every number is the WASM engine's own calibrated confidence over the composed request — the same IR, lexical rung, and gate the CLI runs."),
          h("li", null, h("strong", null, "Knowledge is on the page. "),
            "The engine has no world knowledge; the knowledge panel's rules are what ground a question. They are visible and editable — nothing up the sleeve."),
          h("li", null, h("strong", null, "Abstain is success. "),
            "When no rule grounds the question, the runtime says so — the page shows the abstain, never an invented number.")),
        h("p", { class: "fineprint", id: "qa-stats" },
          "Session stats — decided " + S.stats.decided + " · abstained " + S.stats.abstained +
          " · escalated " + S.stats.escalated + " · unavailable " + S.stats.unavailable)),
      h("div", { class: "card card-honest" },
        h("h3", null, "Honest edges"),
        h("ul", { class: "crosses" },
          h("li", null, "Delete every rule and the page cannot answer anything — that is the point, and the demo."),
          h("li", null, "“Is the sky blue” scores high ONLY because the knowledge panel records sky_color = blue. That is honesty, not weakness: the chain is visible."),
          h("li", null, "Rules live in this tab's session only — nothing is saved, nothing is sent anywhere.")))));

    const status = byId("qa-status");
    S.engine.ensure().then(() => { status.textContent = "engine: " + S.engine.status; });
  }

  boot();
})();
