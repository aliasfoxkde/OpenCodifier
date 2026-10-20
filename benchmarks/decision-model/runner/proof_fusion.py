#!/usr/bin/env python3
"""Measure a proof-first fusion on the JevBench public split.

The engine arm (model id ``relational-v1|builtin-lexical-v2``) produces
exact proofs (p = 1.0) when its relational solver resolves a question over
extracted facts.  The E1-E 2B arm produces a learned distribution.  A
proof-first fusion prefers the engine's answer ONLY where a proof actually
fired, and defers to the learned model everywhere else.

This avoids the F24 failure mode: the engine's out-of-domain ECE 0.402
comes from its *learned* lexical rung, which never gets to gate.

Usage:
    python3 runner/proof_fusion.py [--jevbench-root DIR]
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
from collections import defaultdict

DEFAULT_ROOT = pathlib.Path("/nas/Temp/work/oc-model-eval/runs/jevbench")


def out(msg: str = "") -> None:
    sys.stdout.write(msg + "\n")


def load_jsonl(path: pathlib.Path) -> list[dict]:
    rows: list[dict] = []
    with path.open() as fh:
        for line in fh:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def is_proof(resp: object) -> bool:
    """True when the engine answered with a self-identifying p=1.0 proof."""
    if not isinstance(resp, dict):
        return False
    trace = resp.get("trace") or {}
    entries = trace.get("entries") or []
    for entry in entries:
        detail = entry.get("detail") or {}
        node = entry.get("node") or ""
        if node in ("relational", "proof", "relational-proof"):
            return True
        for key in ("proof", "proved", "is_proof"):
            if detail.get(key):
                return True
    ans = resp.get("answer") or {}
    return abs(float(ans.get("probability", 0.0)) - 1.0) < 1e-12


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--jevbench-root", type=pathlib.Path, default=DEFAULT_ROOT)
    ap.add_argument("--engine-run", default="engine-v4")
    ap.add_argument("--model-run", default="e1e-slot-231")
    args = ap.parse_args()

    engine_dir = args.jevbench_root / args.engine_run
    model_dir = args.jevbench_root / args.model_run

    engine: dict[str, dict] = {}
    for row in load_jsonl(engine_dir / "results.jsonl"):
        raw_path = engine_dir / "raw" / f"{row.get('raw_sha256', '')}.json"
        proof = False
        model_id = ""
        if raw_path.exists():
            raw = json.loads(raw_path.read_text())
            resp = raw.get("response") or {}
            proof = is_proof(resp)
            for entry in (resp.get("trace") or {}).get("entries") or []:
                detail = entry.get("detail") or {}
                if "model" in detail:
                    model_id = str(detail["model"])
                    break
        engine[row["task_id"]] = {
            "family": row.get("family"),
            "correct": bool(row.get("correct")),
            "proof": proof,
            "model": model_id,
        }

    model: dict[str, bool] = {}
    for row in load_jsonl(model_dir / "results.jsonl"):
        tid = row.get("task_id")
        if tid is not None and "correct" in row:
            model[tid] = bool(row["correct"])

    out(f"engine rows: {len(engine)}  model rows: {len(model)}")
    common = sorted(set(engine) & set(model))
    out(f"common task_ids: {len(common)}")
    if not common:
        out("no common task_ids — nothing to fuse")
        return 1

    eng_corr = sum(1 for t in common if engine[t]["correct"])
    mod_corr = sum(1 for t in common if model[t])
    n = len(common)
    out("")
    out(f"engine accuracy (common): {eng_corr}/{n} = {eng_corr/n:.4f}")
    out(f"E1-E accuracy (common):   {mod_corr}/{n} = {mod_corr/n:.4f}")

    proved = [t for t in common if engine[t]["proof"]]
    out("")
    out(f"engine proof fired on {len(proved)}/{n} common items")
    if proved:
        pc = sum(1 for t in proved if engine[t]["correct"])
        pe = sum(1 for t in proved if model[t])
        out(f"  proof correctness:      {pc}/{len(proved)} = {pc/len(proved):.4f}")
        out(f"  E1-E on the same items: {pe}/{len(proved)} = {pe/len(proved):.4f}")
        byfam: dict[str, list[int]] = defaultdict(lambda: [0, 0])
        for t in proved:
            byfam[str(engine[t]["family"])][0] += 1
            byfam[str(engine[t]["family"])][1] += int(engine[t]["correct"])
        out("  proof by family (n, correct):")
        for fam, (fn, fc) in sorted(byfam.items()):
            out(f"    {fam:22s} {fn:3d}  {fc:3d}  {fc/fn:.3f}")

    fused = sum(
        1 if engine[t]["proof"] else int(model[t])
        for t in common
    )
    out("")
    out(f"proof-first fusion: {fused}/{n} = {fused/n:.4f}")
    out(
        f"  vs E1-E alone:     {mod_corr}/{n} = {mod_corr/n:.4f}"
        f"  (delta {fused - mod_corr:+d} items, {(fused-mod_corr)/n*100:+.2f} pp)"
    )

    out("")
    out("E1-E per family (common set):")
    fam_stat: dict[str, list[int]] = defaultdict(lambda: [0, 0, 0])
    for t in common:
        f = str(engine[t]["family"])
        fam_stat[f][0] += 1
        fam_stat[f][1] += int(model[t])
        fam_stat[f][2] += int(engine[t]["correct"])
    for f, (fn, fe, fg) in sorted(fam_stat.items()):
        out(f"  {f:22s} n={fn:3d}  E1-E {fe/fn:.3f}  engine {fg/fn:.3f}")

    out("")
    out("items where engine proof and E1-E disagree:")
    for t in common:
        if engine[t]["proof"] and model[t] != engine[t]["correct"]:
            out(
                f"  {t:28s} fam={str(engine[t]['family']):18s} "
                f"engine={engine[t]['correct']} e1e={model[t]}"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
