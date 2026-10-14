#!/usr/bin/env python3
"""T3.2 pre-test: does per-kind temperature beat a single global T?

Two modes, both fit by NLL and evaluated in-sample (disclosed — this is
a sizing probe for the campaign's calibration leg, not a shipped
calibration artifact):

- logits mode (default): a verdict-slot holdout eval (rows.jsonl +
  suite_holdout.json, sha-verifiable join). Kind inferred from the suite
  item's candidate list (fork-lane readout convention).
- probs mode (--probs RUN_DIR REF_DIR [global_T]): a JevBench harness
  run's results.jsonl joined against the jevbench-ref public split for
  gold labels + question types.

argmax is T-invariant, so accuracy cannot move; the gate is ECE-only,
per D15. auto@5% = largest prefix ranked by confidence with accuracy
>= 95% (the board's auto-accept column definition).

Defaults are the 0.8B v2-hard arm (the board-leg-failing arm where
calibration matters most; e1-e's 2B artifacts are stranded on the
T5500 until its PSU lands).
"""
import json
import math
import sys

HOLDOUT = ("/nas/Temp/work/oc-model-eval/runs/v2-08b-hard-fed/"
           "eval-holdout-best/rows.jsonl")
SUITE = "/nas/Temp/repos/OpenCodifier/benchmarks/decision-model/suite/suite_holdout.json"
BOARD_RUN = ("/nas/Temp/work/oc-model-eval/runs/"
             "fork-08b-v2-hard-fed")
REF = "/var/tmp/jevbench-ref"
GLOBAL_T = 3.362
NOUL_MAP = {"true": "yes", "false": "no"}


def kind_of_candidates(ids):
    if len(ids) == 2 and set(ids) <= {"yes", "no", "true", "false"}:
        return "bool"
    if all(i.lstrip("-").isdigit() for i in ids):
        return "score"
    return "choice"


# ---------- logits mode ----------

def softmax_conf(logits, t):
    scaled = [x / t for x in logits]
    m = max(scaled)
    ex = [math.exp(x - m) for x in scaled]
    z = sum(ex)
    probs = [e / z for e in ex]
    top = max(range(len(probs)), key=probs.__getitem__)
    return top, probs[top]


def nll_logits(rows, temps):
    tot = 0.0
    for r in rows:
        t = temps[r["kind"]]
        m = max(r["slot_scores"])
        ex = [math.exp((x - m) / t) for x in r["slot_scores"]]
        z = sum(ex)
        tot -= (r["slot_scores"][r["gold"]] - m) / t - math.log(z)
    return tot / len(rows)


def ece_logits(rows, temps, n_bins=10):
    bins = [[0, 0.0, 0.0] for _ in range(n_bins)]
    for r in rows:
        _, conf = softmax_conf(r["slot_scores"], temps[r["kind"]])
        b = min(int(conf * n_bins), n_bins - 1)
        bins[b][0] += 1
        bins[b][1] += conf
        bins[b][2] += 1.0 if r["pred"] == r["gold"] else 0.0
    n = sum(b[0] for b in bins)
    return sum(b[0] * abs(b[2] / b[0] - b[1] / b[0]) for b in bins if b[0]) / n


JSON_OUT = None


def run_logits_mode(rows_path=None, suite_path=None):
    d = json.load(open(suite_path or SUITE))
    items = d if isinstance(d, list) else d.get("items") or []
    suite = {it["id"]: it for it in items}
    rows = []
    for line in open(rows_path or HOLDOUT):
        if not line.strip():
            continue
        r = json.loads(line)
        item = suite.get(r["id"]) or suite.get(r["id"][1:])  # holdout ids carry an 'h' prefix
        if item is None:
            continue
        ids = [str(c["id"]) for c in item["candidates"]]
        if r["answer"] not in ids or len(r["slot_scores"]) != len(ids):
            continue
        rows.append({"id": r["id"], "kind": kind_of_candidates(ids),
                     "slot_scores": r["slot_scores"],
                     "gold": ids.index(r["answer"]),
                     "pred": ids.index(r["pred"]) if r["pred"] in ids else None,
                     "correct": bool(r["correct"])})
    if not rows:
        sys.exit("no joinable rows")

    def nll(rows_, temps):
        return nll_logits(rows_, temps)

    def ece(rows_, temps):
        return ece_logits(rows_, temps)

    report(rows, nll, ece, one=lambda k: 1.0, glob=lambda k: GLOBAL_T,
           label=f"logits mode: {(rows_path or HOLDOUT).rsplit('/', 2)[-2]}",
           json_out=JSON_OUT)


# ---------- probs mode ----------

def calibrated_probs(probs, t):
    lp = {x: math.log(max(float(v), 1e-12)) / t for x, v in probs.items()}
    m = max(lp.values())
    z = sum(math.exp(lp[x] - m) for x in lp)
    return {x: math.exp(lp[x] - m) / z for x in lp}


def nll_probs(rows, temps):
    tot = 0.0
    for r in rows:
        q = calibrated_probs(r["probs"], temps[r["kind"]])
        tot -= math.log(max(q[r["gold"]], 1e-12))
    return tot / len(rows)


def ece_probs(rows, temps, n_bins=10):
    bins = [[0, 0.0, 0.0] for _ in range(n_bins)]
    for r in rows:
        q = calibrated_probs(r["probs"], temps[r["kind"]])
        conf = q[r["pred"]]
        b = min(int(conf * n_bins), n_bins - 1)
        bins[b][0] += 1
        bins[b][1] += conf
        bins[b][2] += 1.0 if r["pred"] == r["gold"] else 0.0
    n = sum(b[0] for b in bins)
    return sum(b[0] * abs(b[2] / b[0] - b[1] / b[0]) for b in bins if b[0]) / n


def auto5(rows, temps):
    ranked = sorted(rows, key=lambda r: -calibrated_probs(r["probs"], temps[r["kind"]])[r["pred"]])
    best = 0
    for n in range(1, len(ranked) + 1):
        acc = sum(r["pred"] == r["gold"] for r in ranked[:n]) / n
        if acc >= 0.95:
            best = n
    return best, len(ranked)


def run_probs_mode(run_dir, ref_dir, glob_t):
    tasks = {}
    for f in ("easy", "hard", "original"):
        for line in open(f"{ref_dir}/{f}.jsonl"):
            if line.strip():
                t = json.loads(line)
                tasks[t["id"]] = (str(t["expected"]), t["question"]["type"])
    rows = []
    for line in open(f"{run_dir}/results.jsonl"):
        if not line.strip():
            continue
        r = json.loads(line)
        if r.get("status") != "ok" or not r.get("probs") or not r.get("predicted"):
            continue
        gold, qtype = tasks.get(r["task_id"], (None, None))
        if gold is None:
            continue
        gold = NOUL_MAP.get(gold, gold)
        kind = {"noul": "bool", "choice": "choice", "score": "score"}[qtype]
        if gold not in r["probs"] or r["predicted"] not in r["probs"]:
            continue
        rows.append({"id": r["task_id"], "kind": kind,
                     "probs": {str(k): float(v) for k, v in r["probs"].items()},
                     "gold": gold, "pred": r["predicted"],
                     "correct": bool(r.get("correct"))})
    if not rows:
        sys.exit("no joinable rows")

    report(rows, nll_probs, ece_probs, one=lambda k: 1.0, glob=lambda k: glob_t,
           label=f"probs mode: {run_dir.rsplit('/', 1)[-1]}",
           with_auto5=True, json_out=JSON_OUT)


# ---------- shared report ----------

def report(rows, nll, ece, one, glob, label, with_auto5=False, json_out=None):
    acc = sum(r["correct"] for r in rows) / len(rows)
    kinds = sorted({r["kind"] for r in rows})
    grid = [0.2 * (20.0 / 0.2) ** (i / 199) for i in range(200)]
    per = {}
    for k in kinds:
        sub = [r for r in rows if r["kind"] == k]
        best_t, best_v = None, None
        for t in grid:
            v = nll(sub, {k: t})
            if best_v is None or v < best_v:
                best_t, best_v = t, v
        per[k] = best_t

    sys.stdout.write(f"{label}  rows={len(rows)} acc={acc:.4f}  (in-sample, disclosed)\n")
    sys.stdout.write(f"{'kind':<7} {'n':>4} {'acc':>6} {'T*':>7}  {'ECE@T=1':>8} {'ECE@glob':>9} {'ECE@perk':>8}\n")
    for k in kinds + ["ALL"]:
        sub = rows if k == "ALL" else [r for r in rows if r["kind"] == k]
        if not sub:
            continue
        kacc = sum(r["correct"] for r in sub) / len(sub)
        t_star = "—" if k == "ALL" else f"{per[k]:.3f}"
        sys.stdout.write(f"{k:<7} {len(sub):>4} {kacc:>6.3f} {t_star:>7}  "
                         f"{ece(sub, {x: one(x) for x in kinds}):>8.4f} "
                         f"{ece(sub, {x: glob(x) for x in kinds}):>9.4f} "
                         f"{ece(sub, per):>8.4f}\n")
    if with_auto5:
        for name, temps in (("T=1", {x: 1.0 for x in kinds}),
                            (f"glob", {x: glob(x) for x in kinds}),
                            ("perk", per)):
            n, tot = auto5(rows, temps)
            sys.stdout.write(f"auto@5% @{name}: {n}/{tot} = {n / tot:.3f}\n")
    sys.stdout.write(f"per-kind temps: {per}\n")
    if json_out:
        with open(json_out, "w") as f:
            json.dump({"per_kind_T": {k: per[k] for k in per},
                       "ece_per_kind": {k: ece([r for r in rows if r["kind"] == k], per)
                                        for k in per},
                       "ece_all_perk": ece(rows, per),
                       "rows": len(rows), "acc": acc}, f, indent=2, sort_keys=True)


def main():
    global JSON_OUT
    argv = sys.argv[1:]
    if "--json" in argv:
        i = argv.index("--json")
        JSON_OUT = argv[i + 1]
        del argv[i:i + 2]
    if argv and argv[0] == "--probs":
        run_dir = argv[1] if len(argv) > 1 else BOARD_RUN
        ref_dir = argv[2] if len(argv) > 2 else REF
        gt = float(argv[3]) if len(argv) > 3 else GLOBAL_T
        run_probs_mode(run_dir, ref_dir, gt)
    else:
        rows_f = argv[0] if argv else HOLDOUT
        suite_f = argv[1] if len(argv) > 1 else SUITE
        run_logits_mode(rows_f, suite_f)


if __name__ == "__main__":
    main()
