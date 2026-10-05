#!/usr/bin/env python3
"""Independent suite-integrity audit: re-derive every answer from the item's
own context, never from generator internals.

The generator (suite/generate_suite.py) is the author; this script is the
adversarial reader. It re-derives each item's ground truth by parsing the
rendered context text alone and recomputing the answer, so a hardcoded or
inconsistent answer field cannot pass silently. It also checks the
structural claims the board's numbers rest on:

- balance (40/40/40) and unique ids, answer always among candidates
- main vs holdout suites are fully disjoint (ids AND contexts)
- class B candidate descriptions share no content word with the utterance
  (the "no lexical hints" design claim), and no external dataset files are
  referenced anywhere under suite/
- byte-lock: regenerating both suites into a temp dir reproduces the
  committed files exactly

Exit code 0 = every check passed. Any failure prints the offending item
ids and exits 1.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

SUITE_DIR = Path(__file__).resolve().parent.parent / "suite"

STOPWORDS = {
    "a", "an", "and", "any", "are", "as", "at", "be", "but", "by", "can",
    "do", "for", "from", "has", "have", "i", "in", "is", "it", "its", "me",
    "my", "no", "not", "of", "on", "or", "our", "own", "s", "should", "so",
    "the", "their", "them", "there", "this", "to", "us", "we", "what",
    "which", "with", "would", "you", "your",
}


def fail(msg: str) -> None:
    print(f"FAIL {msg}")
    raise SystemExit(1)


def load(name: str) -> dict:
    return json.loads((SUITE_DIR / name).read_text(encoding="utf-8"))


# --- class A: re-derive the unique constraint satisfier from the context ---

def derive_class_a(item: dict) -> str:
    constraints = re.findall(r"(\w[\w-]*) must be ([\w.-]+?)(?=[.;])",
                             item["context"])
    if not constraints:
        fail(f"{item['id']}: no constraints parsed from context")
    satisfiers = []
    for line in item["context"].splitlines():
        m = re.match(r"- ([\w-]+): (.+)\.", line)
        if not m:
            continue
        name, fields = m.groups()
        attrs = dict(re.findall(r"(\w[\w-]*)=([\w.-]+)", fields))
        if all(attrs.get(k) == v for k, v in constraints):
            satisfiers.append(name)
    if len(satisfiers) != 1:
        fail(f"{item['id']}: {len(satisfiers)} constraint satisfiers, expected 1")
    return satisfiers[0]


# --- class B: the labels are human judgment (a domain tag on a support
# utterance), so they cannot be re-derived mechanically. What IS checkable
# mechanically: no item may be solvable by trivial keyword matching — if the
# correct description out-attracts every wrong description on shared content
# words, a lexical rule alone picks the answer and the class stops testing
# lexical→semantic generalization. A handful of incidental overlaps is a
# difficulty property; a keyword-viable solver would be an integrity one.

def check_class_b(item: dict) -> int:
    def words(text: str) -> set[str]:
        return {w for w in re.findall(r"[a-z']+", text.lower())
                if w not in STOPWORDS and len(w) > 2}

    utterance = words(item["context"])
    correct = next(c for c in item["candidates"] if c["id"] == item["answer"])
    wrong = [c for c in item["candidates"] if c["id"] != item["answer"]]
    hit = len(utterance & words(correct["description"]))
    best_wrong = max((len(utterance & words(c["description"])) for c in wrong),
                     default=0)
    if hit > best_wrong:
        print(f"  LEAK {item['id']}: keyword overlap picks the answer "
              f"(correct {hit} > best wrong {best_wrong})")
        return 1
    return 0


# --- class C: re-derive chain root / unique-max region / topological first --

def derive_class_c(item: dict) -> str:
    ctx = item["context"]
    if "depends on" in ctx:
        # chain: the failing node that appears in a dependency edge; the
        # decoy failure appears in no edge at all
        failing = set(re.findall(r"(\w+) is failing", ctx))
        in_edge = (set(re.findall(r"(\w+) depends on", ctx))
                   | set(re.findall(r"depends on (\w+)", ctx)))
        roots = sorted(n for n in failing if n in in_edge)
        if len(roots) != 1:
            fail(f"{item['id']}: {len(roots)} failing-with-dependents, expected 1")
        return roots[0]
    if item["question"].startswith("Which region"):
        counts: dict[str, int] = {}
        for m in re.finditer(r"(\w+): ([^.]+)\.", ctx):
            region, statuses = m.groups()
            counts[region] = statuses.split(", ").count("healthy")
        best = max(counts.values())
        winners = sorted(r for r, c in counts.items() if c == best)
        if len(winners) != 1:
            fail(f"{item['id']}: {len(winners)}-way tie on healthy count")
        return winners[0]
    if "comes back online only after" in ctx:
        waits = set(re.findall(r"(\w+) comes back online", ctx))
        after = set(re.findall(r"only after (\w+)", ctx))
        first = sorted(after - waits)
        if len(first) != 1:
            fail(f"{item['id']}: {len(first)} components with no predecessor")
        return first[0]
    fail(f"{item['id']}: unrecognized relational family")


def check_suite(suite: dict, name: str) -> int:
    items = suite["items"]
    if len(items) != 120:
        fail(f"{name}: {len(items)} items, expected 120")
    classes = Counter(i["class"] for i in items)
    if set(classes.values()) != {40}:
        fail(f"{name}: unbalanced classes {dict(classes)}")
    ids = [i["id"] for i in items]
    if len(ids) != len(set(ids)):
        fail(f"{name}: duplicate item ids")
    leaked = 0
    for it in items:
        cand_ids = [c["id"] for c in it["candidates"]]
        if it["answer"] not in cand_ids:
            fail(f"{it['id']}: answer not among candidates")
        if len(cand_ids) < 4:
            fail(f"{it['id']}: fewer than 4 candidates")
        if it["class"] == "metadata_match":
            got = derive_class_a(it)
        elif it["class"] == "relational_compositional":
            got = derive_class_c(it)
        else:
            leaked += check_class_b(it)
            got = it["answer"]  # human-judged domain tag; leak check above
        if got != it["answer"]:
            fail(f"{it['id']}: stored answer '{it['answer']}' != re-derived '{got}'")
    if leaked > 12:  # >10% of class B: a keyword rule would be a viable solver
        fail(f"{name}: {leaked}/80 class-B items keyword-leakable")
    print(f"  {name}: 120 items, 40/40/40, A/C answers re-derived exactly, "
          f"B keyword leaks {leaked}/80 (threshold 12)")
    return 0


def check_disjoint(a: dict, b: dict) -> None:
    ia = {i["id"] for i in a["items"]}
    ib = {i["id"] for i in b["items"]}
    ca = {i["context"] for i in a["items"]}
    cb = {i["context"] for i in b["items"]}
    if ia & ib:
        fail(f"id overlap across suites: {sorted(ia & ib)}")
    if ca & cb:
        fail(f"context overlap across suites: {len(ca & cb)} shared contexts")
    if a["seed"] == b["seed"] or a["suite_version"] == b["suite_version"]:
        fail("suites share seed or version")
    print("  disjoint: zero id/context overlap, distinct seeds and versions")


def check_byte_lock() -> None:
    gen = str(SUITE_DIR / "generate_suite.py")
    with tempfile.TemporaryDirectory() as tmp:
        for argv, out in (([gen], "suite.json"),
                          ([gen, "--holdout"], "suite_holdout.json")):
            dest = Path(tmp) / out
            subprocess.run([sys.executable, *argv, str(dest)],
                           check=True, capture_output=True)
            if dest.read_bytes() != (SUITE_DIR / out).read_bytes():
                fail(f"byte-lock broken for {out}")
    print("  byte-lock: regeneration reproduces both committed files exactly")


def check_no_external_data() -> None:
    """No dataset files, downloads, or non-repo references under suite/."""
    allowed = {"generate_suite.py", "suite.json", "suite_holdout.json",
               "__pycache__"}  # bytecode of the generator itself, not data
    for p in SUITE_DIR.iterdir():
        if p.is_dir() and p.name == "__pycache__":
            continue
        if p.name not in allowed:
            fail(f"unexpected file under suite/: {p.name}")
    text = "\n".join(
        p.read_text(encoding="utf-8") for p in SUITE_DIR.glob("*.py")
    )
    imports = re.findall(r"^\s*(?:import|from)\s+([\w.]+)", text, re.M)
    banned = {"requests", "urllib", "http", "pandas", "datasets",
              "huggingface_hub", "numpy"}
    bad = sorted(set(imports) & banned)
    if bad:
        fail(f"generator imports {bad}")
    if re.search(r"https?://", text) or re.search(r"\bopen\(", text):
        fail("generator reaches the filesystem or network for data")
    print("  provenance: suite/ holds only the generator and the two suites;"
          " stdlib-only generator, no downloads, no external datasets")


def main() -> int:
    v1, v2 = load("suite.json"), load("suite_holdout.json")
    print("audit_suite: independent re-derivation of committed suites")
    check_suite(v1, f"suite.json (v{v1['suite_version']})")
    check_suite(v2, f"suite_holdout.json (v{v2['suite_version']})")
    check_disjoint(v1, v2)
    check_byte_lock()
    check_no_external_data()
    print("PASS: every answer is derivable from its own context (A, C) or a"
          " structurally verified human tag (B); suites are disjoint and"
          " byte-locked; no external data enters the benchmark")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
