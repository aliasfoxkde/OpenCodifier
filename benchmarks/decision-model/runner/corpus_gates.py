#!/usr/bin/env python3
"""Corpus-prep gates for merged-v4 ingestion (RESEARCH §15.6 item 9).

Reads one corpus JSONL split and routes every row to kept or
quarantined — never silently deleted. Four gate families, each with
its reason recorded on the quarantined row and counted in the
manifest:

- ``schema`` — unparseable line, missing fields, unknown kind,
  ``len(target) != len(options)``, NaN/negative mass, sum off 1.0 by
  more than the tolerance.
- ``noise`` — teacher-distribution quality: top probability below
  ``--min-top-prob`` or top-2 margin below ``--min-margin``. Defaults
  are the permissive floor (0.5 inclusive, margin gate off) — the
  manifest's decile histograms exist so thresholds get *measured*,
  not guessed (the #124 A/B exists precisely because a
  down-weight-vs-drop call was made without measurement).
- ``cap`` — per-class (family+kind+argmax label) and per-family
  shares: rows past the cap are quarantined in corpus order
  (first-come keep), so the cut is deterministic and reproducible.
- ``fight`` — identical (state, question, options) rows whose argmax
  labels disagree: the minority labels are quarantined; on an exact
  tie every row of the group is. Canon is byte-exact, so — as
  merge-v2 learned — passage-QA structure does not false-positive
  here.

Rows are emitted verbatim plus a ``_gates`` sidecar (family, kind,
argmax, top, margin) on kept rows and ``_gate`` reasons on
quarantined ones. Outputs per split: ``<stem>.kept.jsonl``,
``<stem>.quarantined.jsonl``, plus one shared ``manifest.json``
(SHA-256 of every input, census before/after, per-gate counts, noise
histograms, config echo).

  python3 runner/corpus_gates.py --split train.jsonl \
      --out-dir results/corpus-v4-gates
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from collections import Counter
from pathlib import Path

KINDS = ("noul", "choice", "score")
SUM_TOLERANCE = 1e-3


def sha256_of(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def canon(state: str, question: str, options: list) -> str:
    """Byte-exact group key for label-fight detection."""
    return json.dumps({"s": state, "q": question, "o": options},
                      ensure_ascii=False, separators=(",", ":"))


def schema_reasons(row: dict) -> list:
    """Hard-schema violations for one parsed row (empty list = clean)."""
    out = []
    for field in ("id", "kind", "state", "question", "target"):
        if row.get(field) in (None, ""):
            out.append(f"missing:{field}")
    kind = row.get("kind")
    if kind in KINDS:
        options = row.get("options")
        target = row.get("target")
        if not isinstance(options, list) or not options:
            out.append("options:not-a-list")
        elif not isinstance(target, list) or len(target) != len(options):
            out.append("target:length-mismatch")
        else:
            if any(not isinstance(p, (int, float)) or p != p or p < 0
                   for p in target):
                out.append("target:nan-or-negative")
            total = sum(target)
            if abs(total - 1.0) > SUM_TOLERANCE:
                out.append(f"target:sum={total:.4f}")
    elif kind is not None:
        out.append(f"kind:{kind}")
    return out


def label_facts(row: dict) -> tuple:
    """(family, kind, argmax_label, top_prob, top2_margin) for a clean row."""
    target = row["target"]
    options = row["options"]
    ordered = sorted(range(len(target)), key=lambda i: -target[i])
    top_i = ordered[0]
    margin = (target[ordered[0]] - target[ordered[1]]) if len(ordered) > 1 \
        else target[ordered[0]]
    return (row.get("family") or "none", row["kind"],
            str(options[top_i]), float(target[top_i]), float(margin))


def decile(v: float) -> str:
    return f"{min(int(v * 10), 9)}"


def noise_reasons(top: float, margin: float, min_top: float,
                  min_margin: float) -> list:
    out = []
    if top < min_top:
        out.append(f"noise:top={top:.3f}<{min_top}")
    if min_margin > 0 and margin < min_margin:
        out.append(f"noise:margin={margin:.3f}<{min_margin}")
    return out


def pass_one(path: Path, min_top: float, min_margin: float) -> dict:
    """Census: class/family counts, label-fight groups, noise histograms."""
    class_counts: Counter = Counter()
    family_counts: Counter = Counter()
    fights: dict = {}
    top_hist: Counter = Counter()
    margin_hist: Counter = Counter()
    total = 0
    bad = 0
    with path.open() as fh:
        for line in fh:
            total += 1
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                bad += 1
                continue
            if schema_reasons(row):
                bad += 1
                continue
            family, kind, label, top, margin = label_facts(row)
            class_counts[(family, kind, label)] += 1
            family_counts[family] += 1
            top_hist[decile(top)] += 1
            margin_hist[decile(margin)] += 1
            key = canon(row["state"], row["question"], row["options"])
            fights.setdefault(key, Counter())[label] += 1
    return {"total": total, "bad": bad, "class_counts": class_counts,
            "family_counts": family_counts, "fights": fights,
            "top_hist": top_hist, "margin_hist": margin_hist}


def majority_label(counts: Counter):
    """(winning_label | None, is_tie) for one label-fight group."""
    ordered = counts.most_common()
    if len(ordered) > 1 and ordered[0][1] == ordered[1][1]:
        return None, True
    return ordered[0][0], False


def pass_two(path: Path, args, census: dict, kept_out, quar_out) -> dict:
    """Emit rows with per-row gate decisions; return the split's counts."""
    total = census["total"]
    # Caps are ceilings on the FILE, not on the class: a class may keep
    # at most floor(share * clean rows) rows. share >= 1.0 disables.
    clean_total = total - census["bad"]
    class_allowed = (None if args.max_class_share >= 1.0
                     else int(args.max_class_share * clean_total))
    family_allowed = (None if args.max_family_share >= 1.0
                      else int(args.max_family_share * clean_total))
    class_running: Counter = Counter()
    family_running: Counter = Counter()

    group_majority: dict = {}
    for key, counts in census["fights"].items():
        group_majority[key] = majority_label(counts)

    gate_counts: Counter = Counter()
    kept_n = 0

    for line in path.open():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            quar_out.write(json.dumps(
                {"_gate": {"gates": ["schema"], "detail": ["unparseable"]}}
            ).rstrip() + "\n")
            gate_counts["schema"] += 1
            continue
        reasons = schema_reasons(row)
        if reasons:
            quar_out.write(json.dumps(
                {"_gate": {"gates": ["schema"], "detail": reasons}}
            ).rstrip() + "\n")
            gate_counts["schema"] += 1
            continue
        family, kind, label, top, margin = label_facts(row)
        facts = {"family": family, "kind": kind, "argmax": label,
                 "top": round(top, 4), "margin": round(margin, 4)}
        reasons = noise_reasons(top, margin, args.min_top_prob,
                                args.min_margin)
        key = canon(row["state"], row["question"], row["options"])
        winner, tie = group_majority[key]
        if tie:
            reasons.append("fight:tie")
        elif winner != label:
            reasons.append(f"fight:minority-of-{winner}")
        ckey = (family, kind, label)
        class_running[ckey] += 1
        if class_allowed is not None and class_running[ckey] > class_allowed:
            reasons.append("cap:class")
        family_running[family] += 1
        if family_allowed is not None and family_running[family] > family_allowed:
            reasons.append("cap:family")
        if reasons:
            quar_out.write(json.dumps({"_gate": {"gates": reasons, **facts},
                                       **row}).rstrip() + "\n")
            for r in reasons:
                gate_counts[r.split(":")[0]] += 1
        else:
            kept_out.write(json.dumps({"_gates": facts, **row}
                                      ).rstrip() + "\n")
            kept_n += 1
    return {"kept": kept_n,
            "quarantined": total - kept_n,
            "gate_counts": dict(gate_counts)}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--split", action="append", required=True, type=Path,
                    help="corpus JSONL to gate (repeatable)")
    ap.add_argument("--out-dir", type=Path, required=True)
    ap.add_argument("--min-top-prob", type=float, default=0.5,
                    help="quarantine rows whose top prob is below this "
                         "(a row at exactly the floor passes)")
    ap.add_argument("--min-margin", type=float, default=0.0,
                    help="quarantine rows whose top-2 margin is below "
                         "this (0 disables the margin gate)")
    ap.add_argument("--max-class-share", type=float, default=1.0,
                    help="max share of one (family,kind,argmax) class "
                         "(1.0 disables)")
    ap.add_argument("--max-family-share", type=float, default=1.0,
                    help="max share of one family (1.0 disables)")
    args = ap.parse_args()
    for p in args.split:
        if not p.is_file():
            sys.stderr.write(f"split not found: {p}\n")
            return 1
    args.out_dir.mkdir(parents=True, exist_ok=True)

    manifest = {
        "created_utc": time.strftime("%Y%m%dT%H%M%SZ", time.gmtime()),
        "tool": "runner/corpus_gates.py",
        "config": {"min_top_prob": args.min_top_prob,
                   "min_margin": args.min_margin,
                   "max_class_share": args.max_class_share,
                   "max_family_share": args.max_family_share},
        "splits": {},
    }
    for path in args.split:
        census = pass_one(path, args.min_top_prob, args.min_margin)
        with (args.out_dir / f"{path.stem}.kept.jsonl").open("w") as kept_out, \
                (args.out_dir / f"{path.stem}.quarantined.jsonl").open("w") as quar_out:
            counts = pass_two(path, args, census, kept_out, quar_out)
        manifest["splits"][path.name] = {
            "sha256": sha256_of(path),
            "rows": census["total"],
            "schema_bad_at_census": census["bad"],
            **counts,
        }
        manifest.setdefault("noise_histograms", {})[path.name] = {
            "top_prob_deciles": dict(sorted(census["top_hist"].items())),
            "margin_deciles": dict(sorted(census["margin_hist"].items())),
        }
        sys.stdout.write(f"{path.name}: {census['total']} rows -> "
                         f"{counts['kept']} kept, "
                         f"{counts['quarantined']} quarantined "
                         f"{counts['gate_counts']}\n")
    (args.out_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=1, sort_keys=True) + "\n")
    sys.stdout.write(f"manifest -> {args.out_dir / 'manifest.json'}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
