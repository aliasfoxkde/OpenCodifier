#!/usr/bin/env python3
"""Audit the generated validation workloads (docs/VALIDATION.md §3.1, §5.1).

Independent re-derivation, deliberately not sharing code paths with the
generator's gold logic:

* Class A — every decision turn's gold answer is recomputed from the
  RENDERED state text alone (fact-grammar extraction mirrors
  `crates/opencodifier-engine/src/facts.rs`; root-cause and first-to-
  restore mirror `relational.rs`), and must equal the recorded gold and
  appear among the turn's candidates. Repeat probes must be byte-identical
  to their source turn's request body.
* Class C — every padded item keeps the original context, question,
  candidates and answer; padding is grammar-inert (zero facts extracted
  from the padding lines) and vocabulary-disjoint from the item's question
  and candidate text; tier byte targets are met.

Exits nonzero on the first class of violations; prints counts either way.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
SUITE = REPO / "benchmarks" / "decision-model" / "suite" / "suite.json"
STATUSES = {"healthy": "healthy", "ok": "healthy", "degraded": "degraded",
            "down": "down", "failing": "down", "offline": "down"}


def entity_ok(word: str) -> bool:
    return 0 < len(word) <= 64 and all(
        c.isalnum() or c in "_-" for c in word)


def extract_facts(text: str) -> list[tuple]:
    """Mirror of facts.rs: sentence split on '.', exact patterns only."""
    facts: list[tuple] = []
    for raw in text.split("."):
        sentence = raw.strip()
        if not sentence:
            continue
        tokens = sentence.split()
        words = [t.strip(",") for t in tokens]
        lower = [w.lower() for w in words]
        if len(lower) == 4 and lower[1] == "depends" and lower[2] == "on" \
                and entity_ok(words[0]) and entity_ok(words[3]):
            facts.append(("depends_on", words[0], words[3]))
            continue
        if len(lower) == 3 and lower[1] == "is" and lower[2] in STATUSES \
                and entity_ok(words[0]):
            facts.append(("health", words[0], STATUSES[lower[2]]))
            continue
        if len(lower) >= 7 and lower[1] == "comes" and lower[2] == "back" \
                and lower[3] == "online" and lower[4] == "only" \
                and lower[5] == "after" and len(lower) == 7 \
                and entity_ok(words[0]) and entity_ok(words[6]):
            facts.append(("restores_after", words[0], words[6]))
            continue
        # Colon form: "X: S, S, ..." with every remaining token a status.
        if (len(words) >= 2 and words[0].endswith(":")
                and entity_ok(words[0][:-1])
                and all(w.lower() in STATUSES for w in words[1:])):
            facts.append(("health_list", words[0][:-1],
                          [STATUSES[w.lower()] for w in words[1:]]))
    return facts


def root_cause(facts: list[tuple], candidates: list[str]) -> str | None:
    failing = {e for kind, e, *rest in facts
               if (kind == "health" and rest[0] == "down")
               or (kind == "health_list" and "down" in rest[0])}
    dependents: dict[str, list[str]] = {}
    dependencies: dict[str, list[str]] = {}
    for kind, a, b in facts:
        if kind == "depends_on":
            dependents.setdefault(b, []).append(a)
            dependencies.setdefault(a, []).append(b)

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
    return roots[0] if len(roots) == 1 and roots[0] in candidates else None


def first_restored(facts: list[tuple], candidates: list[str]) -> str | None:
    waits = {a for kind, a, _ in facts if kind == "restores_after"}
    gates = {b for kind, _, b in facts if kind == "restores_after"}
    first = sorted(gates - waits)
    return first[0] if len(first) == 1 and first[0] in candidates else None


def content_words(text: str) -> set[str]:
    stop = frozenset(
        "a an the and or but if then else when of to in on for with is are "
        "was were be been this that these those it its as at by from into "
        "do does did which what how should".split())
    return {w for w in
            "".join(c if c.isalnum() else " " for c in text.lower()).split()
            if w not in stop and not w.isdigit()}


def audit_sessions(plan: dict) -> list[str]:
    problems: list[str] = []
    n_decisions = 0
    n_probes = 0
    for session in plan["sessions"]:
        bodies = session["request_bodies"]
        for turn in session["turns"]:
            tid = turn["turn"]
            if turn["kind"] == "repeat_probe":
                n_probes += 1
                src = turn["probe_of_turn"]
                if bodies[str(tid)] != bodies[str(src)]:
                    problems.append(
                        f"s{session['session_id']} t{tid}: probe body differs "
                        f"from source turn {src}")
                continue
            n_decisions += 1
            body = json.loads(bodies[str(tid)])
            text = body["state"]["text"]
            facts = extract_facts(text)
            cands = [c["id"] for c in turn["question"]["candidates"]]
            root = root_cause(facts, cands)
            first = first_restored(facts, cands)
            derived = root if root is not None else first
            if derived != turn["gold"]:
                problems.append(
                    f"s{session['session_id']} t{tid}: text-derived gold "
                    f"{derived!r} != recorded {turn['gold']!r}")
            if turn["gold"] not in cands:
                problems.append(
                    f"s{session['session_id']} t{tid}: gold not among "
                    f"candidates")
    print(f"class A: {n_decisions} decisions, {n_probes} probes, "
          f"{len(problems)} problems")
    return problems


def audit_tiers(tiers: dict, original: dict) -> list[str]:
    problems: list[str] = []
    orig = {i["id"]: i for i in original["items"]}
    for name, tier in tiers.items():
        expected = 1 if name == "probe" else 120
        if len(tier["items"]) != expected:
            problems.append(f"{name}: {len(tier['items'])} items, "
                            f"expected {expected}")
        for item in tier["items"]:
            src = orig[item["id"]]
            if src["context"] not in item["context"]:
                problems.append(f"{name}/{item['id']}: original context lost")
            if (item["question"] != src["question"]
                    or item["answer"] != src["answer"]
                    or item["candidates"] != src["candidates"]):
                problems.append(f"{name}/{item['id']}: question/answer drift")
            padding_lines = [ln for ln in item["context"].splitlines()
                             if ln and ln not in src["context"].splitlines()]
            if extract_facts("\n".join(padding_lines)):
                problems.append(
                    f"{name}/{item['id']}: padding matches fact grammar")
            forbidden = content_words(src["question"])
            for c in src["candidates"]:
                forbidden |= content_words(c["description"])
                forbidden.add(c["id"].lower())
            pad_words = content_words("\n".join(padding_lines))
            if pad_words & forbidden:
                problems.append(
                    f"{name}/{item['id']}: padding vocabulary collides: "
                    f"{sorted(pad_words & forbidden)[:5]}")
            target = tier["est_tokens"] * 4
            if item["context_bytes"] < target:
                problems.append(
                    f"{name}/{item['id']}: {item['context_bytes']} bytes < "
                    f"target {target}")
    print(f"class C: {sum(len(t['items']) for t in tiers.values())} items "
          f"across {len(tiers)} tiers, {len(problems)} problems")
    return problems


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dir", type=Path, required=True,
                    help="directory holding session_plan.json + "
                         "longctx_tiers.json")
    ap.add_argument("--skip-tiers", action="store_true",
                    help="audit class A only (tier files are large)")
    args = ap.parse_args()

    plan = json.loads((args.dir / "session_plan.json").read_text())
    problems = audit_sessions(plan)
    if not args.skip_tiers:
        tiers = json.loads((args.dir / "longctx_tiers.json").read_text())
        problems += audit_tiers(tiers["tiers"],
                                json.loads(SUITE.read_text()))
    if problems:
        for p in problems[:20]:
            print("PROBLEM:", p)
        raise SystemExit(f"audit FAILED with {len(problems)} problems")
    print("audit ok")


if __name__ == "__main__":
    sys.exit(main())
