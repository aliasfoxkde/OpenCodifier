/* OpenCodifier — QA tool core (DOM-free).
   #196 of the escalation-lanes plan: ask a yes/no question, get a real
   calibrated confidence score — never a fabricated number.

   Honesty constraint (ESCALATION_LANES_PLAN §8): the engine has no world
   knowledge. "Is the sky blue?" over an empty state cannot be answered, and
   a tool that returned 0.97 for it would be lying. So this module owns a
   VISIBLE, EDITABLE knowledge panel: a small list of rules binding a fact
   to a question keyword. The score on screen is always the engine's real
   output over the composed state — and when no rule matches, the page shows
   the honest abstain (or the escalation payload), not an invented answer.

   This module never touches the DOM; qa.js renders. */

(function () {
  "use strict";
  const DK = globalThis.DecisionsSDK;

  const POLICY = { min_confidence: 0.55, verify_below: 0.55, abstain_below: 0.55, risk: "low" };
  const ESCALATE_BELOW = 0.55; /* the gate that trips the escalation demo */

  /* ---------- the knowledge panel ---------- */

  /* A rule: { keyword, factKey, factValue, answer } — when the question
     contains `keyword` (case-insensitive, word boundary), the fact
     factKey = factValue is attached to the state text, which is what gives
     the lexical rung something to agree with. `answer` is the side the rule
     asserts ("yes" | "no"); it composes into the question text. */

  const BUILT_IN_RULES = [
    { keyword: "sky", factKey: "sky_color", factValue: "blue", answer: "yes",
      note: "the knowledge panel records sky_color = blue" },
    { keyword: "compile", factKey: "build_status", factValue: "cargo check passes on the recorded tree", answer: "yes",
      note: "the knowledge panel records build_status = cargo check passes" },
    { keyword: "deploy", factKey: "deploy_policy", factValue: "production deploys require operator approval", answer: "no",
      note: "the knowledge panel records the deploy policy" },
    { keyword: "local", factKey: "posture", factValue: "the runtime is local-first with no telemetry", answer: "yes",
      note: "the knowledge panel records the local-first posture" },
  ];

  /* Rules are data, validated before use: a malformed rule from the editor
     is refused with a named reason, never silently dropped. */
  function validateRule(rule) {
    if (!rule || typeof rule !== "object") return "a rule must be an object";
    if (typeof rule.keyword !== "string" || !rule.keyword.trim()) return "keyword is empty";
    if (typeof rule.factKey !== "string" || !rule.factKey.trim()) return "fact key is empty";
    if (typeof rule.factValue !== "string" || !rule.factValue.trim()) return "fact value is empty";
    if (rule.answer !== "yes" && rule.answer !== "no") return "answer must be yes or no";
    return null;
  }

  function normalizeRules(raw) {
    if (!Array.isArray(raw)) return { ok: false, errors: ["rules must be a JSON array"], rules: [] };
    const rules = [];
    const errors = [];
    raw.forEach((rule, i) => {
      const problem = validateRule(rule);
      if (problem) errors.push("rule " + (i + 1) + ": " + problem);
      else rules.push({
        keyword: rule.keyword.trim(),
        factKey: rule.factKey.trim(),
        factValue: rule.factValue.trim(),
        answer: rule.answer,
        note: typeof rule.note === "string" && rule.note.trim() ? rule.note.trim() : "",
      });
    });
    return { ok: errors.length === 0, errors, rules };
  }

  /* The word-boundary keyword match decides which rules fire for a question.
     Returns the FIRING rules in panel order — the page shows exactly these,
     so the visitor can see why the state says what it says. */
  function matchingRules(question, rules) {
    const text = String(question || "").toLowerCase();
    const out = [];
    for (const rule of rules) {
      const kw = rule.keyword.toLowerCase();
      const re = new RegExp("(^|[^a-z0-9])" + kw.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "([^a-z0-9]|$)", "i");
      if (re.test(text)) out.push(rule);
    }
    return out;
  }

  /* ---------- the wire contract ---------- */

  /* The composed request: state text = the firing rules' facts (and any
     context) in panel order — NEVER the question itself. The lexical rung
     scores state-vs-question; putting the question in the state would
     self-match and hand every bare question an inflated confidence. With a
     fact-free state the rung has nothing to agree with, which is what makes
     the bare question abstain. facts = {} (tagged-enum contract). The
     question is negator-free by contract (parity flips booleans). */
  function composeRequest(question, context, rules) {
    const fired = matchingRules(question, rules);
    const parts = [];
    if (context && context.trim()) parts.push(context.trim());
    for (const rule of fired) parts.push(rule.factValue);
    return {
      request: {
        state: { text: parts.join(". "), facts: {} },
        questions: [{
          type: "boolean",
          id: "qa",
          text: question.trim(),
        }],
        policy: POLICY,
      },
      fired,
    };
  }

  /* ---------- the three states ---------- */

  /* Classify an engine response into the page's three honest states:
     decided (accept), abstained (abstain), escalated (below the escalation
     gate — with the payload a host would receive). verify counts as
     escalated here: the runtime asked for a second opinion, which is
     exactly what escalation is. */
  function classify(response) {
    if (!response || !response.outcome) return "unavailable";
    const conf = response.confidence ? response.confidence.calibrated_confidence : 0;
    if (response.outcome === "accept") return "decided";
    if (response.outcome === "abstain") return "abstained";
    return "escalated"; /* verify + escalate both mean "not decisive alone" */
  }

  /* The escalation payload: what a host model would receive — the request
     plus the confidence report that tripped the gate. Deterministic. */
  function escalationPayload(question, context, rules, response) {
    const composed = composeRequest(question, context, rules);
    return {
      reason: "confidence " +
        (response && response.confidence ? response.confidence.calibrated_confidence : 0) +
        " below the escalation gate " + ESCALATE_BELOW,
      request: composed.request,
      confidence: response ? response.confidence : null,
    };
  }

  globalThis.QaCore = {
    POLICY,
    ESCALATE_BELOW,
    BUILT_IN_RULES,
    validateRule,
    normalizeRules,
    matchingRules,
    composeRequest,
    classify,
    escalationPayload,
  };
})();
