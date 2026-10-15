#!/usr/bin/env python3
"""T1.5: build merged-v5 — the consensus-label corpus (campaign doc
docs/planning/08b-gate-closure-campaign.md).

Three signals per question, all in one label space by construction:
  G = merged-v4 target label (the incumbent gold)
  T = teacher-facts-v1 argmax_id (E1-E second opinion, 187,822 questions)
  S = strands full-pass prediction (independent gemma-line judge, 188,061)

Policy, fixed BEFORE any flip count was observed:
  - suite / suite_holdout rows: byte-identical copy (eval fixtures).
  - flip target to T only when T == S and T != G (two independent
    models against the incumbent). No other rule flips a label.
  - otherwise the row keeps G byte-identical; the ledger records why
    (agree / keep_2v1_teacher_gold / keep_2v1_strands_gold / split3 /
    missing_teacher / missing_strands / strands_cov_gap).
  - a flipped distribution is replaced by strands' native probabilities
    (same option space, verified) with confidence = mean(S, T tops);
    the original values survive in the ledger.
  - score adjacency is ANALYSIS ONLY (recorded in the manifest); the
    flip rule is uniform across kinds.

Streaming, single pass, deterministic (file order, no threads).
Outputs: merged-v5.jsonl, merged-v5.disputes.jsonl, manifest.json.
"""

import argparse
import hashlib
import json
import sys
from collections import Counter
from pathlib import Path

BASE = "/nas/Temp/work/oc-model-eval"
SUITE_PREFIX = "suite"


def sha256_file(path, buf=1 << 20):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            chunk = f.read(buf)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def strands_prediction(q):
    """(label, probs_dict|None) in gold space, or (None, None)."""
    if q.get("error") is not None:
        return None, None
    ans = q.get("answer")
    if not isinstance(ans, dict) or ans.get("type") != q["type"]:
        return None, None
    if q["type"] == "noul":
        p = ans.get("noul")
        if not isinstance(p, (int, float)) or not 0.0 <= float(p) <= 1.0:
            return None, None
        return ("true" if float(p) > 0.5 else "false"), None
    probs = ans.get("probabilities")
    if not isinstance(probs, dict) or not probs:
        return None, None
    return max(probs.items(), key=lambda kv: float(kv[1]))[0], probs


def load_teacher(facts_dirs):
    """item_id -> fact; item_id -> (record_id, question)."""
    id2recq = {}
    with open(facts_dirs["items"]) as f:
        for line in f:
            if line.strip():
                d = json.loads(line)
                id2recq[d["id"]] = (d["record_id"], d["question"])
    facts = {}
    for shard in facts_dirs["shards"]:
        with open(shard) as f:
            for line in f:
                if line.strip():
                    d = json.loads(line)
                    facts[d["id"]] = d
    return facts, id2recq


def load_strands(path):
    out = {}
    with open(path) as f:
        for line in f:
            if line.strip():
                d = json.loads(line)
                out[d["record_id"]] = d
    return out


def rebuild_target(tgt, qtype, label, probs, confidence):
    """Mutate a flipped target preserving v4 key order; 6dp rounding.

    probs is strands' p (float) for noul or a {option: p} dict otherwise;
    noul probabilities are rebuilt from p so label/noul/probabilities stay
    mutually consistent, keeping the original key order.
    """
    out = {}
    for k in tgt:
        v = tgt[k]
        if k == "label":
            v = label
        elif k == "confidence":
            v = round(confidence, 6)
        elif k == "noul" and qtype == "noul":
            v = round(float(probs), 6)
        elif k == "probabilities" and qtype == "noul":
            p = round(float(probs), 6)
            pair = {"true": p, "false": round(1.0 - p, 6)}
            if isinstance(v, dict) and set(v.keys()) == {"true", "false"}:
                v = {key: pair[key] for key in v}
            else:
                v = pair
        elif k == "probabilities" and probs is not None:
            new = {str(x): round(float(p), 6) for x, p in probs.items()}
            if isinstance(v, dict) and set(v.keys()) == set(new.keys()):
                v = {key: new[key] for key in v}
            else:
                v = new
        out[k] = v
    if "score" in out and out.get("probabilities"):
        # v4's `score` field is exactly EV over the probability keys
        # (verified 2400/2400 on unflipped rows); keep it consistent
        out["score"] = round(sum(int(j) * float(p)
                                 for j, p in out["probabilities"].items()), 6)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--corpus", default=f"{BASE}/corpora/merged-v4/merged-v4.jsonl")
    ap.add_argument("--items", default=f"{BASE}/corpora/teacher-items-v1/items.jsonl")
    ap.add_argument("--facts", nargs="+",
                    default=[f"{BASE}/corpora/teacher-facts-v1/shard-0/facts.jsonl",
                             f"{BASE}/corpora/teacher-facts-v1/shard-1/facts.jsonl"])
    ap.add_argument("--strands", default=f"{BASE}/runs/strands2b-full-v1/results.jsonl")
    ap.add_argument("--out-dir", default=f"{BASE}/corpora/merged-v5")
    ap.add_argument("--limit", type=int, default=0, help="smoke: first N rows only")
    args = ap.parse_args()

    sys.stdout.write("loading teacher facts/items...\n")
    sys.stdout.flush()
    facts, id2recq = load_teacher({"items": args.items, "shards": args.facts})
    # record_id+question -> fact
    teach = {id2recq[i]: f for i, f in facts.items() if i in id2recq}
    sys.stdout.write(f"teacher questions={len(teach)}\n")
    sys.stdout.write("loading strands pass...\n")
    sys.stdout.flush()
    strands = load_strands(args.strands)
    sys.stdout.write(f"strands records={len(strands)}\n")
    sys.stdout.flush()

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    n_rows = n_q = n_flips = 0
    action = Counter()
    flips_by_kind = Counter()
    flips_by_source = Counter()
    score_adj = Counter()  # |S-G| bucket distance on disagreements
    keys = ("type", "label", "confidence")
    with open(args.corpus) as f, \
            open(out_dir / "merged-v5.jsonl", "w") as w, \
            open(out_dir / "merged-v5.disputes.jsonl", "w") as d:
        for line in f:
            if not line.strip():
                continue
            rec = json.loads(line)
            if args.limit and n_rows >= args.limit:
                break
            n_rows += 1
            rid = rec["record_id"]
            is_suite = str(rec.get("source", "")).startswith(SUITE_PREFIX)
            sq = (strands.get(rid) or {}).get("questions") or {}
            new_target = rec["target"]
            for name, q in rec["request"]["questions"].items():
                n_q += 1
                qtype = q["type"]
                gold = rec["target"].get(name, {}).get("label")
                teach_fact = teach.get((rid, name))
                teacher = teach_fact.get("argmax_id") if teach_fact else None
                pred, sprobs = strands_prediction(sq.get(name) or {})
                if is_suite:
                    act = "holdout"
                elif teacher is None:
                    act = "missing_teacher"
                elif pred is None:
                    act = "missing_strands"
                elif not (teacher == pred) and not (teacher == gold) and not (pred == gold):
                    act = "split3"
                elif teacher == pred and teacher != gold:
                    # the only flipping rule
                    gp = rec["target"][name].get("probabilities") or {}
                    space_ok = (sprobs is not None and gp
                                and set(gp.keys()) <= set(sprobs.keys())) \
                        if qtype != "noul" else isinstance(sq[name]["answer"].get("noul"), (int, float))
                    if not space_ok:
                        act = "strands_cov_gap"
                    else:
                        act = "flip"
                        old_target = dict(rec["target"][name])
                        conf_t = float(teach_fact.get("top") or 0.0)
                        if qtype == "noul":
                            dist = sq[name]["answer"]["noul"]
                            # strands noul answers carry no confidence field;
                            # the binary margin is the equivalent signal
                            conf_s = abs(2.0 * float(dist) - 1.0)
                        else:
                            dist = {str(k): float(v) for k, v in sprobs.items()}
                            conf_s = float(sq[name]["answer"].get("confidence") or 0.0)
                            if qtype == "score" and gold is not None and gold.lstrip("-").isdigit() \
                                    and pred.lstrip("-").isdigit():
                                score_adj[abs(int(pred) - int(gold))] += 1
                        new_target[name] = rebuild_target(
                            rec["target"][name], qtype, teacher, dist,
                            (conf_s + conf_t) / 2.0)
                        n_flips += 1
                        flips_by_kind[qtype] += 1
                        flips_by_source[str(rec.get("source"))] += 1
                elif teacher == gold and pred != gold:
                    act = "keep_2v1_teacher_gold"
                    if qtype == "score" and pred is not None \
                            and gold.lstrip("-").isdigit() and pred.lstrip("-").isdigit():
                        score_adj[abs(int(pred) - int(gold))] += 1
                elif pred == gold and teacher != gold:
                    act = "keep_2v1_strands_gold"
                    if qtype == "score" and teacher is not None \
                            and gold.lstrip("-").isdigit() and teacher.lstrip("-").isdigit():
                        score_adj[abs(int(teacher) - int(gold))] += 1
                else:
                    act = "agree"
                    if qtype == "score" and pred is not None \
                            and gold.lstrip("-").isdigit() and pred.lstrip("-").isdigit() and pred != gold:
                        score_adj[abs(int(pred) - int(gold))] += 1
                action[act] += 1
                if act == "flip":
                    old = {k: old_target.get(k) for k in keys if k in old_target}
                    old["probabilities"] = old_target.get("probabilities")
                    new = {k: new_target[name].get(k) for k in keys if k in new_target[name]}
                    new["probabilities"] = new_target[name].get("probabilities")
                    d.write(json.dumps({"record_id": rid, "question": name,
                                        "qtype": qtype, "action": act,
                                        "gold": gold, "teacher": teacher,
                                        "strands": pred, "old": old, "new": new}) + "\n")
                else:
                    d.write(json.dumps({"record_id": rid, "question": name,
                                        "qtype": qtype, "action": act,
                                        "gold": gold, "teacher": teacher,
                                        "strands": pred}) + "\n")
            rec["target"] = new_target
            w.write(json.dumps(rec) + "\n")
            if n_rows % 25000 == 0:
                sys.stdout.write(f"rows={n_rows} flips={n_flips}\n")
                sys.stdout.flush()

    manifest = {
        "policy": "flip iff teacher==strands!=gold (pre-registered before "
                  "any flip count); suite rows byte-identical; flipped "
                  "distribution = strands native probs, confidence = "
                  "mean(strands, teacher tops)",
        "inputs": {
            "corpus": {"path": args.corpus, "sha256": sha256_file(args.corpus)},
            "items": {"path": args.items, "sha256": sha256_file(args.items)},
            "facts": [{"path": p, "sha256": sha256_file(p)} for p in args.facts],
            "strands": {"path": args.strands, "sha256": sha256_file(args.strands)},
        },
        "outputs": {
            "corpus": str(out_dir / "merged-v5.jsonl"),
            "disputes": str(out_dir / "merged-v5.disputes.jsonl"),
        },
        "rows": n_rows, "questions": n_q, "flips": n_flips,
        "actions": dict(action),
        "flips_by_kind": dict(flips_by_kind),
        "flips_by_source": dict(flips_by_source),
        "score_disagreement_adjacency": dict(sorted(score_adj.items())),
        "corpus_sha256": None,
        "disputes_sha256": None,
    }
    manifest["corpus_sha256"] = sha256_file(out_dir / "merged-v5.jsonl")
    manifest["disputes_sha256"] = sha256_file(out_dir / "merged-v5.disputes.jsonl")
    with open(out_dir / "manifest.json", "w") as w:
        json.dump(manifest, w, indent=2, sort_keys=True)
    sys.stdout.write(json.dumps({k: manifest[k] for k in
                                 ("rows", "questions", "flips", "actions",
                                  "flips_by_kind", "flips_by_source",
                                  "score_disagreement_adjacency")}, indent=1) + "\n")
    sys.stdout.write(f"corpus sha256 {manifest['corpus_sha256']}\n")


if __name__ == "__main__":
    main()
