#!/usr/bin/env python3
"""Generate the real-world validation workloads (docs/VALIDATION.md §3, §5).

Emits, into --out-dir:

* ``session_plan.json`` — 24 sessions × 50 turns of incident-triage loop
  (class A). Every turn's gold answer is provable from the rendered state
  by construction: root-cause turns have exactly one failing entity with
  dependents (the `RelationalSolver::root_cause` precondition), and
  restore-first turns always face the unique source of a linear
  restores-chain (the `first_restored` precondition). Repeat probes on
  turns 20 and 40 re-issue turn t−5's exact request body.
* ``longctx_tiers.json`` — the committed 120-item suite padded to three
  estimated-token tiers plus the 1 MiB socket probe (class C). Padding is
  the `make_long_suite.py` distractor prose: vocabulary-disjoint from
  every item (asserted) and grammar-inert (asserted by `audit_workloads.py`).

Deterministic: fixed seed, fixed source order, `--check` re-derives and
byte-compares both artifacts. Gold derivation mirrors
`crates/opencodifier-engine/src/relational.rs` exactly; the sibling
auditor re-derives it from rendered text alone.

Fact-grammar sentences follow `crates/opencodifier-engine/src/facts.rs`:
sentences end in `.`, entity names are runs of `[A-Za-z0-9_-]`, and the
only patterns emitted here are `X depends on Y.`, `X is
healthy|degraded|down.`, and `X comes back online only after Y.`
"""
from __future__ import annotations

import argparse
import json
import random
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
SUITE = REPO / "benchmarks" / "decision-model" / "suite" / "suite.json"

SEED = 20261005
SESSIONS = 24
TURNS = 50
PROBE_EVERY = 20
PROBE_LOOKBACK = 5

# The fixed entity set: identical across sessions so a cross-session cache
# bleed would surface as a wrong answer on a repeated entity/question pair.
ENTITIES = [
    "auth", "billing", "cache", "catalog", "dispatch", "gateway",
    "ingest", "ledger", "media", "notify", "queue", "search",
]

# Fixed dependency DAG (dependent depends on dependency). Acyclic; every
# entity gains at least one dependent by the time turn 13 closes.
DEP_EDGES = [
    ("auth", "gateway"), ("billing", "gateway"), ("billing", "auth"),
    ("catalog", "gateway"), ("search", "catalog"), ("queue", "gateway"),
    ("dispatch", "queue"), ("dispatch", "catalog"), ("notify", "dispatch"),
    ("media", "cache"), ("ingest", "queue"), ("ledger", "billing"),
]

# Fixed restores-chain, added in order: later comes back online only
# after earlier. A chain keeps `gates - waits` a single entity at all
# times, which is exactly the first_restored uniqueness precondition.
RESTORES_CHAIN = ["auth", "gateway", "queue", "dispatch", "notify"]

# Health carousel over entities that have dependents once the DAG lands:
# healthy -> degraded -> down (held two turns) -> healthy -> next entity.
CAROUSEL = ["gateway", "queue", "catalog", "billing", "auth", "search"]

HEALTHS = ["healthy", "degraded", "down", "down", "healthy"]

TIERS = [
    ("L1", 4096),
    ("L2", 32768),
    ("L3", 131072),
]
PROBE_TOKENS = 262144

POLICY = {
    "min_confidence": 0.8,
    "verify_below": 0.65,
    "abstain_below": 0.5,
    "risk": "low",
}
LIMITS = {
    "max_input_bytes": 1_048_576,
    "max_questions": 32,
    "max_candidates": 256,
    "max_graph_nodes": 128,
    "max_execution_time": {"secs": 120, "nanos": 0},
    "max_retrieval_results": 64,
}

# Distractor prose, verbatim from make_long_suite.py (same templates, same
# fillers): administrative vocabulary, no candidate ids, no fact grammar.
TEMPLATES = [
    "Ledger note {n}: the quarterly archive lists correspondence, {a} and {b}.",
    "Appendix {n}: procedural paperwork, {a}, {b} and filing queues were reviewed.",
    "Minute {n}: committee attendance was recorded and the {a} discussion adjourned.",
    "Register {n}: the custodian catalogued {a} alongside {b} without further action.",
    "Bulletin {n}: administrative circulars covered {a} and {b} for the quarter.",
    "Summary {n}: clerical routines processed {a}, {b} and routine correspondence.",
]
FILLER_A = [
    "inventories", "reimbursements", "archived memoranda", "filing queues",
    "stationery totals", "committee rosters", "custodial schedules",
    "circulation lists", "retention schedules", "registry updates",
    "notarised copies", "draft circulars", "postal manifests",
    "shelf audits", "binding orders",
]
FILLER_B = [
    "scheduling minutiae", "unrelated correspondence", "administrative overhead",
    "procedural minutiae", "logistics paperwork", "archival minutiae",
    "vendor contracts", "travel reimbursements", "quarterly circulars",
    "registry errata", "filing backlogs", "clerical rotations",
    "attendance rosters", "room bookings", "photocopy quotas",
]
STOP_WORDS = frozenset(
    "a an the and or but if then else when of to in on for with is are was "
    "were be been this that these those it its as at by from into do does "
    "did which what how should".split()
)


def sentence(entity: str, status: str) -> str:
    return f"{entity} is {status}."


def depends_sentence(dependent: str, dependency: str) -> str:
    return f"{dependent} depends on {dependency}."


def restores_sentence(later: str, earlier: str) -> str:
    return f"{later} comes back online only after {earlier}."


def content_words(text: str) -> set[str]:
    return {w for w in
            "".join(c if c.isalnum() else " " for c in text.lower()).split()
            if w not in STOP_WORDS and not w.isdigit()}


def candidate_ids(item: dict) -> list[str]:
    return [c["id"] for c in item.get("candidates", [])]


class Session:
    """One deterministic incident script: mutations + provable questions."""

    def __init__(self, session_id: int, rng: random.Random) -> None:
        self.session_id = session_id
        self.rng = rng
        self.dep_added: list[tuple[str, str]] = []
        self.restores_added: list[tuple[str, str]] = []
        self.health: dict[str, str] = {}
        self.carousel_index = 0
        self.carousel_step = 0
        self.turns: list[dict] = []
        self.request_bodies: dict[int, str] = {}

    # -- mutation engine --------------------------------------------------

    def next_mutation(self, turn: int) -> dict:
        """Pick the turn's mutation deterministically.

        Order of preference keeps every question provable: the restores
        chain starts on turn 1; dependency edges fill in from turn 2; the
        health carousel runs once at least four dependency edges exist and
        holds exactly one `down` entity at a time.
        """
        if turn == 1:
            later, earlier = RESTORES_CHAIN[0], RESTORES_CHAIN[1]
            return {"kind": "restores", "later": later, "earlier": earlier}
        if len(self.dep_added) < len(DEP_EDGES) and turn % 3 != 0:
            edge = DEP_EDGES[len(self.dep_added)]
            return {"kind": "depends", "dependent": edge[0], "dependency": edge[1]}
        if len(self.dep_added) >= 4:
            entity = CAROUSEL[self.carousel_index]
            status = HEALTHS[self.carousel_step]
            self.carousel_step += 1
            if self.carousel_step >= len(HEALTHS):
                self.carousel_step = 0
                self.carousel_index = (self.carousel_index + 1) % len(CAROUSEL)
            return {"kind": "health", "entity": entity, "status": status}
        # Few edges and not a restores turn: grow the restores chain.
        idx = len(self.restores_added) + 1
        idx = min(idx, len(RESTORES_CHAIN) - 1)
        return {"kind": "restores",
                "later": RESTORES_CHAIN[idx], "earlier": RESTORES_CHAIN[idx - 1]}

    # -- gold derivation (mirrors relational.rs) --------------------------

    def root_cause(self) -> str | None:
        failing = {e for e, s in self.health.items() if s in ("down", "failing",
                                                              "offline")}
        dependents: dict[str, list[str]] = {}
        dependencies: dict[str, list[str]] = {}
        for dep, on in self.dep_added:
            dependents.setdefault(on, []).append(dep)
            dependencies.setdefault(dep, []).append(on)

        def transitive(start: str) -> set[str]:
            seen: set[str] = set()
            stack = list(dependencies.get(start, []))
            while stack:
                node = stack.pop()
                if node not in seen:
                    seen.add(node)
                    stack.extend(dependencies.get(node, []))
            return seen

        roots = [n for n in sorted(failing)
                 if dependents.get(n)
                 and not any(d != n and d in failing for d in transitive(n))]
        return roots[0] if len(roots) == 1 else None

    def first_restored(self) -> str | None:
        waits = {later for later, _ in self.restores_added}
        gates = {earlier for _, earlier in self.restores_added}
        first = sorted(gates - waits)
        return first[0] if len(first) == 1 else None

    # -- rendering --------------------------------------------------------

    def render_text(self, turn: int) -> str:
        # Sentences split on '.' only (facts.rs) — the header is
        # period-terminated so it can never merge with the first fact
        # sentence, which would silently drop that fact from extraction.
        lines = [f"Session S-{self.session_id:03d} turn {turn:03d} incident board."]
        lines.extend(depends_sentence(d, o) for d, o in self.dep_added)
        lines.extend(restores_sentence(l, e) for l, e in self.restores_added)
        lines.extend(sentence(e, s) for e, s in sorted(self.health.items()))
        return "\n".join(lines) + "\n"

    def state_facts(self) -> dict:
        facts: dict[str, dict] = {}
        for dep, on in self.dep_added:
            facts[f"dependency.{dep}"] = {"kind": "text", "value": on}
        for later, earlier in self.restores_added:
            facts[f"restores.{later}"] = {"kind": "text", "value": earlier}
        for entity, status in self.health.items():
            facts[f"health.{entity}"] = {"kind": "text", "value": status}
        return {k: facts[k] for k in sorted(facts)}

    def candidates_for(self, gold: str) -> list[dict]:
        rest = [e for e in ENTITIES if e != gold]
        start = ENTITIES.index(gold)
        picks = [rest[(start + i) % len(rest)] for i in range(3)]
        ordered = sorted([gold] + picks)
        return [{"id": e, "description": f"component {e} of the incident board"}
                for e in ordered]

    # -- script -----------------------------------------------------------

    def build(self) -> None:
        for turn in range(1, TURNS + 1):
            if turn in (PROBE_EVERY, 2 * PROBE_EVERY):
                source = self.turns[turn - PROBE_LOOKBACK - 1]
                probe = {
                    "turn": turn, "kind": "repeat_probe",
                    "probe_of_turn": source["turn"],
                    "question": source["question"],
                    "gold": source["gold"],
                }
                self.turns.append(probe)
                self.request_bodies[turn] = self.request_bodies[source["turn"]]
                continue

            mut = self.next_mutation(turn)
            if mut["kind"] == "depends":
                self.dep_added.append((mut["dependent"], mut["dependency"]))
            elif mut["kind"] == "restores":
                self.restores_added.append((mut["later"], mut["earlier"]))
            else:
                self.health[mut["entity"]] = mut["status"]

            root = self.root_cause()
            if root is not None:
                gold, question_text = root, (
                    "Which single entity is the root cause of the current outage?")
            else:
                first = self.first_restored()
                if first is None:
                    raise SystemExit(
                        f"session {self.session_id} turn {turn}: no provable gold")
                gold, question_text = first, (
                    "Which single entity must be restored first?")
            self.turns.append({
                "turn": turn, "kind": "decision",
                "mutation": mut,
                "question": {
                    "type": "choice", "id": f"q-{self.session_id:03d}-{turn:03d}",
                    "text": question_text,
                    "candidates": self.candidates_for(gold),
                },
                "gold": gold,
            })
            body = {
                "state": {"text": self.render_text(turn),
                          "facts": self.state_facts()},
                "questions": [self.turns[-1]["question"]],
                "policy": POLICY,
                "metadata": {"request_id": f"S-{self.session_id:03d}-T-{turn:03d}",
                             "limits": LIMITS},
            }
            self.request_bodies[turn] = json.dumps(body, sort_keys=True,
                                                   ensure_ascii=False)

    def to_json(self) -> dict:
        return {
            "session_id": self.session_id,
            "turns": self.turns,
            "request_bodies": self.request_bodies,
        }


def build_sessions(seed: int) -> list[dict]:
    sessions = []
    for i in range(SESSIONS):
        session = Session(i, random.Random(seed + i))
        session.build()
        sessions.append(session.to_json())
    return sessions


def pad_context(text: str, target_bytes: int, rng: random.Random,
                forbidden_words: set[str], item_id: str) -> str:
    """Wrap `text` in grammar-inert distractor prose up to target bytes.

    Per-item disjointness is the invariant (asserted, not assumed): a
    template or filler sharing any content word with THIS item is excluded
    from this item's pools; an emptied pool fails the generator loudly.
    """
    templates = [t for t in TEMPLATES if not content_words(t) & forbidden_words]
    fillers_a = [f for f in FILLER_A if not content_words(f) & forbidden_words]
    fillers_b = [f for f in FILLER_B if not content_words(f) & forbidden_words]
    for name, pool in (("templates", templates), ("filler_a", fillers_a),
                       ("filler_b", fillers_b)):
        if not pool:
            raise SystemExit(f"{item_id}: distractor pool {name} emptied by "
                             f"disjointness filter (forbidden: {sorted(forbidden_words)})")
    # The original context rides as ONE unbroken block (a substring of the
    # padded text); distractor sentences are appended, then the block list
    # is shuffled. Shuffling the context's own characters would destroy it.
    parts = [text]
    n = 0
    while sum(len(s) + 1 for s in parts) < target_bytes:
        n += 1
        template = templates[n % len(templates)]
        a = fillers_a[rng.randrange(len(fillers_a))]
        b = fillers_b[rng.randrange(len(fillers_b))]
        sentence_text = template.format(n=n, a=a, b=b)
        words = content_words(sentence_text)
        if words & forbidden_words:
            raise SystemExit(
                f"{item_id}: distractor vocabulary collides: {words & forbidden_words}")
        parts.append(sentence_text)
    rng.shuffle(parts)
    return "\n".join(parts) + "\n"


def build_tiers() -> dict:
    suite = json.loads(SUITE.read_text())
    tiers = {}
    for name, est_tokens in TIERS:
        target = est_tokens * 4
        items = []
        for item in suite["items"]:
            forbidden = content_words(item["question"])
            for cand in item.get("candidates", []):
                forbidden |= content_words(cand["description"])
                forbidden.add(cand["id"].lower())
            padded = pad_context(str(item["context"]), target,
                                 random.Random(SEED + est_tokens),
                                 forbidden, f"{name}/{item['id']}")
            items.append({"id": item["id"], "class": item["class"],
                          "question": item["question"],
                          "candidates": item["candidates"],
                          "answer": item["answer"],
                          "context": padded,
                          "context_bytes": len(padded.encode("utf-8"))})
        tiers[name] = {"est_tokens": est_tokens, "items": items}
    probe_item = suite["items"][0]
    forbidden = content_words(probe_item["question"])
    for cand in probe_item.get("candidates", []):
        forbidden |= content_words(cand["description"])
        forbidden.add(cand["id"].lower())
    padded = pad_context(str(probe_item["context"]), PROBE_TOKENS * 4,
                         random.Random(SEED + PROBE_TOKENS), forbidden, "probe")
    tiers["probe"] = {"est_tokens": PROBE_TOKENS,
                      "items": [{"id": probe_item["id"],
                                 "class": probe_item["class"],
                                 "question": probe_item["question"],
                                 "candidates": probe_item["candidates"],
                                 "answer": probe_item["answer"],
                                 "context": padded,
                                 "context_bytes": len(padded.encode("utf-8"))}]}
    return tiers


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--seed", type=int, default=SEED)
    ap.add_argument("--out-dir", type=Path, default=None)
    ap.add_argument("--check", type=Path, default=None,
                    help="regenerate and byte-compare against this dir")
    args = ap.parse_args()

    doc = {
        "generator": "generate_workloads.py",
        "seed": args.seed,
        "sessions": build_sessions(args.seed),
        "longctx_tiers": build_tiers(),
    }
    session_plan = json.dumps({"generator": doc["generator"],
                               "seed": doc["seed"],
                               "sessions": doc["sessions"]},
                              sort_keys=True, ensure_ascii=False) + "\n"
    tiers_doc = {"generator": doc["generator"], "seed": doc["seed"],
                 "tiers": {name: {"est_tokens": tier["est_tokens"],
                                  "items": tier["items"]}
                           for name, tier in doc["longctx_tiers"].items()}}
    tiers_json = json.dumps(tiers_doc, sort_keys=True, ensure_ascii=False) + "\n"

    if args.check is not None:
        for name, payload in (("session_plan.json", session_plan),
                              ("longctx_tiers.json", tiers_json)):
            existing = (args.check / name).read_text()
            if existing != payload:
                raise SystemExit(f"--check FAILED: {name} bytes differ")
        print("check ok: session_plan.json + longctx_tiers.json byte-identical")
        return

    if args.out_dir is None:
        raise SystemExit("one of --out-dir / --check is required")
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "session_plan.json").write_text(session_plan)
    (args.out_dir / "longctx_tiers.json").write_text(tiers_json)
    n_decisions = sum(1 for s in doc["sessions"] for t in s["turns"]
                      if t["kind"] == "decision")
    n_probes = sum(1 for s in doc["sessions"] for t in s["turns"]
                   if t["kind"] == "repeat_probe")
    tier_summary = ", ".join(
        "%s:%dt/%di" % (name, tier["est_tokens"], len(tier["items"]))
        for name, tier in doc["longctx_tiers"].items())
    print("wrote session_plan.json (%d sessions, %d decisions, %d probes)"
          " and longctx_tiers.json (%s) to %s"
          % (SESSIONS, n_decisions, n_probes, tier_summary, args.out_dir))


if __name__ == "__main__":
    sys.exit(main())
