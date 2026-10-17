#!/usr/bin/env python3
"""merge-v2 pre-work: near-duplicate + gap analysis over merged-v1.

Report-only — no corpus is rewritten and no record is dropped here. The
v1 merge was exact-duplicate-only (stated in the manifest), so this pass
answers the merge-v2 open item: how much near-duplicate mass does the
corpus carry, where does it live (source, question-kind), and what does
the kind imbalance look like (148k relevance-noul vs ~2k choice/score
questions is the gap the VIVERE extraction lane would fill).

Records are `opencodifier.distill-record/1`: `request.state` +
`request.questions` {name: {type, instructions, criteria}} + `target`.
The training unit is the (state, question) pair, so kinds are counted
per question; duplication is measured on canonicalized state text.

Method: unicode-aware normalize (casefold, drop punct, collapse ws), then
  pass 1 — exact-normalized duplicate clusters over (state, question);
  pass 2 — MinHash (word 5-gram shingles, 64 permutations) with LSH
           banding (16 bands x 4 rows) within source groups; candidate
           pairs verified at Jaccard >= 0.8 on the state text.
Outputs merged-v1-dupreport.json next to the corpus.
"""
from __future__ import annotations

import hashlib
import json
import re
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path

SHINGLE_K = 5
PERMS = 64
BANDS, ROWS = 16, 4
JACCARD_T = 0.8
BUCKET_CAP = 400  # pair-gen guard: boilerplate bands blow up O(n^2)

_norm_re = re.compile(r"[^\w\s]+", re.UNICODE)
_ws_re = re.compile(r"\s+", re.UNICODE)


def normalize(text: str) -> str:
    return _ws_re.sub(" ", _norm_re.sub(" ", text.casefold())).strip()


def canon_state(state) -> str:
    return json.dumps(state, sort_keys=True, ensure_ascii=False,
                      separators=(",", ":"))


def q_signature(qname: str, q: dict) -> str:
    return json.dumps({"n": qname,
                       "i": q.get("instructions", ""),
                       "c": q.get("criteria", "")},
                      sort_keys=True, ensure_ascii=False)


def shingles(text: str) -> set:
    words = normalize(text).split()
    if len(words) < SHINGLE_K:
        return {" ".join(words)} if words else set()
    return {" ".join(words[i:i + SHINGLE_K])
            for i in range(len(words) - SHINGLE_K + 1)}


def minsig(sh: set) -> list:
    sig = [1 << 64] * PERMS
    for s in sh:
        h = int.from_bytes(hashlib.blake2b(
            s.encode("utf-8"), digest_size=8).digest(), "big")
        for i in range(PERMS):
            v = ((h * (i + 0x9E3779B97F4A7C15) + i * 0xBF58476D1CE4E5B9)
                 ^ (h >> 31)) & 0xFFFFFFFFFFFFFFFF
            if v < sig[i]:
                sig[i] = v
    return sig


def jaccard(a: set, b: set) -> float:
    if not a or not b:
        return 0.0
    inter = len(a & b)
    return inter / (len(a) + len(b) - inter)


def main() -> int:
    corpus = Path(sys.argv[1])
    out_path = corpus.with_name("merged-v1-dupreport.json")
    t0 = time.time()
    records = []
    with corpus.open() as fh:
        for line in fh:
            records.append(json.loads(line))
    print(f"loaded {len(records)} records in {time.time()-t0:.1f}s",
          flush=True)

    # ---- question-kind census + gold-label census (per question) ----
    qkinds = Counter()
    qkind_by_source = defaultdict(Counter)
    choice_labels = Counter()
    score_targets = []
    n_states = 0
    for r in records:
        req = r.get("request") or {}
        tgt = r.get("target") or {}
        qkind_by_source[r.get("source")].update(
            q.get("type") for q in (req.get("questions") or {}).values())
        for qname, q in (req.get("questions") or {}).items():
            qkinds[q.get("type")] += 1
            t = tgt.get(qname) or {}
            if q.get("type") == "choice":
                choice_labels[t.get("label")] += 1
            elif q.get("type") == "score":
                sc = t.get("score", t.get("value"))
                if sc is not None:
                    score_targets.append(float(sc))
        n_states += 1
    print(f"question census: {dict(qkinds)}", flush=True)

    # ---- pass 1: exact-normalized (state, question) pairs ----
    t0 = time.time()
    pair_key_hits = Counter()
    state_exact: defaultdict = defaultdict(list)
    for idx, r in enumerate(records):
        req = r.get("request") or {}
        skey = hashlib.sha256(
            canon_state(req.get("state")).encode("utf-8")).hexdigest()
        state_exact[skey].append(idx)
        for qname, q in (req.get("questions") or {}).items():
            pk = hashlib.sha256((skey + "\x1f"
                                 + q_signature(qname, q)).encode("utf-8"))
            pair_key_hits[pk.hexdigest()] += 1
    dup_pairs = sum(c - 1 for c in pair_key_hits.values() if c > 1)
    state_clusters = [v for v in state_exact.values() if len(v) > 1]
    print(f"pass1 exact: {len(state_clusters)} dup states, "
          f"{dup_pairs} redundant (state,question) pairs "
          f"in {time.time()-t0:.1f}s", flush=True)

    # ---- pass 2: minhash lsh on state text within source groups ----
    t0 = time.time()
    src_of = [r.get("source") for r in records]
    sh_cache: dict = {}
    sig_cache: dict = {}
    for idx, r in enumerate(records):
        text = canon_state((r.get("request") or {}).get("state"))
        sh_cache[idx] = shingles(text)
        sig_cache[idx] = minsig(sh_cache[idx])
    print(f"signatures in {time.time()-t0:.1f}s", flush=True)

    t0 = time.time()
    buckets: dict = defaultdict(list)
    for idx in range(len(records)):
        sig = sig_cache[idx]
        for b in range(BANDS):
            band = (src_of[idx], b,
                    hash(tuple(sig[b * ROWS:(b + 1) * ROWS])))
            buckets[band].append(idx)
    cand: set = set()
    heavy_buckets = 0
    for members in buckets.values():
        if len(members) < 2:
            continue
        if len(members) > BUCKET_CAP:
            heavy_buckets += 1
            continue
        members = sorted(set(members))
        for i in range(len(members)):
            for j in range(i + 1, len(members)):
                cand.add((members[i], members[j]))
    print(f"lsh candidates: {len(cand)} pairs "
          f"({heavy_buckets} heavy buckets skipped >={BUCKET_CAP}) "
          f"in {time.time()-t0:.1f}s", flush=True)

    t0 = time.time()
    near_pairs = []
    union_root = list(range(len(records)))

    def find(x: int) -> int:
        while union_root[x] != x:
            union_root[x] = union_root[union_root[x]]
            x = union_root[union_root[x]]
        return x

    for i, j in sorted(cand):
        if find(i) == find(j):
            continue
        jj = jaccard(sh_cache[i], sh_cache[j])
        if jj >= JACCARD_T:
            near_pairs.append((i, j, round(jj, 4)))
            union_root[find(i)] = find(j)
    clusters: dict = defaultdict(list)
    for idx in range(len(records)):
        clusters[find(idx)].append(idx)
    near_clusters = [c for c in clusters.values() if len(c) > 1]
    print(f"near-dup: {len(near_pairs)} pairs, {len(near_clusters)} "
          f"clusters in {time.time()-t0:.1f}s", flush=True)

    def describe(idx: int) -> dict:
        r = records[idx]
        return {"source": r.get("source"),
                "source_config": r.get("source_config"),
                "n_questions": len((r.get("request") or {})
                                   .get("questions") or {})}

    near_by_source = Counter()
    for c in near_clusters:
        near_by_source.update(src_of[i] for i in c)
    redundant = sum(len(c) - 1 for c in near_clusters)
    jmin = min((p[2] for p in near_pairs), default=None)
    jmax = max((p[2] for p in near_pairs), default=None)

    # ---- gold-label agreement inside near-dup clusters ----
    def gold_labels(idx: int) -> set:
        r = records[idx]
        tgt = r.get("target") or {}
        out = set()
        for qname, t in tgt.items():
            if isinstance(t, dict):
                out.add(f"{t.get('type')}:{t.get('label')}")
        return frozenset(out)

    mixed_clusters = 0
    mixed_samples = []
    for c in near_clusters:
        label_sets = {gold_labels(i) for i in c}
        if len(label_sets) > 1:
            mixed_clusters += 1
            if len(mixed_samples) < 10:
                mixed_samples.append({
                    "members": [{"id": records[i].get("record_id"),
                                 "labels": sorted(gold_labels(i))}
                                for i in c[:6]]})
    uniform = len(near_clusters) - mixed_clusters

    score_hist = Counter(round(s) for s in score_targets)

    report = {
        "report_version": "opencodifier.dupreport/2",
        "corpus": corpus.name,
        "records": len(records),
        "question_kinds": dict(qkinds),
        "qkind_by_source": {k: dict(v)
                            for k, v in sorted(qkind_by_source.items())},
        "choice_label_distribution": dict(choice_labels.most_common()),
        "score_target_histogram": {str(k): score_hist[k]
                                   for k in sorted(score_hist)},
        "exact_normalized": {
            "unit": "(state, question) pair + duplicate states",
            "duplicate_states": len(state_clusters),
            "records_in_dup_states": sum(len(c) for c in state_clusters),
            "redundant_pairs": dup_pairs,
            "sample_dup_states": [
                [describe(i) for i in c][:4]
                for c in sorted(state_clusters, key=len,
                                reverse=True)[:10]],
        },
        "near_duplicate": {
            "scope": "canonicalized request.state text, within source",
            "jaccard_threshold": JACCARD_T,
            "shingle_k": SHINGLE_K, "perms": PERMS,
            "bands_rows": f"{BANDS}x{ROWS}",
            "bucket_cap": BUCKET_CAP,
            "heavy_buckets_skipped": heavy_buckets,
            "candidate_pairs": len(cand),
            "verified_pairs": len(near_pairs),
            "clusters": len(near_clusters),
            "redundant_records": redundant,
            "redundant_by_source": dict(near_by_source),
            "jaccard_range": [jmin, jmax],
            "label_agreement": {
                "uniform_clusters": uniform,
                "mixed_label_clusters": mixed_clusters,
                "mixed_samples": mixed_samples,
            },
            "largest_clusters": [
                {"size": len(c),
                 "members": [describe(i) for i in c][:6]}
                for c in sorted(near_clusters, key=len,
                                reverse=True)[:10]],
        },
        "gap_summary": {
            "relevance_noul_questions":
                qkinds.get("noul", 0),
            "choice_questions": qkinds.get("choice", 0),
            "score_questions": qkinds.get("score", 0),
            "imbalance_ratio": round(
                qkinds.get("noul", 0)
                / max(qkinds.get("choice", 0) + qkinds.get("score", 0), 1),
                1),
        },
        "runtime_s": round(time.time() - t0, 1),
    }
    out_path.write_text(json.dumps(report, indent=1))
    print(json.dumps({k: report[k] for k in
                      ("question_kinds", "exact_normalized",
                       "near_duplicate", "gap_summary", "runtime_s")},
                     indent=1)[:4000])
    print(f"wrote {out_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
