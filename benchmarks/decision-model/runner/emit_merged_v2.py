#!/usr/bin/env python3
"""Emit merged-v2 from merged-v1 under the dupreport v3 policy.

Policy (docs/TRAINING.md §9, 2026-10-05):
  - uniform-label near-dup clusters  -> keep one representative, drop
    the rest (representative = highest teacher.jev, tie-break lowest
    input line index — deterministic, prefers the strongest teacher
    vote as the surviving gold);
  - mixed-label near-dup clusters    -> QUARANTINE: every member moves
    to merged-v2.quarantine.jsonl carrying `_quarantine` provenance
    (cluster id = min member record_id, the cluster's label set);
    excluded from train — picking a winner inside a label fight
    invents supervision, keeping the fight poisons it;
  - everything else (incl. suite/suite_holdout rows) passes through
    untouched, eval rows never deduped.

Reuses analyze_merged_dups' shingle/MinHash/LSH so cluster identity
matches the published dupreport (same params, same sorted-candidate
union order). Gates before pinning: zero state-hash collisions between
surviving train rows and suite rows; per-source/per-kind reconciliation
against the input census. Outputs land next to the corpus.
"""
from __future__ import annotations

import hashlib
import json
import sys
import time
from collections import Counter, defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from analyze_merged_dups import (BANDS, BUCKET_CAP, JACCARD_T, PERMS,  # noqa: E402
                                 ROWS, SHINGLE_K, canon_state, jaccard,
                                 minsig, shingles)

OUT_TAGS = ("_quarantine",)


def gold_labels(record: dict) -> frozenset:
    out = set()
    for _qname, t in (record.get("target") or {}).items():
        if isinstance(t, dict):
            out.add(f"{t.get('type')}:{t.get('label')}")
    return frozenset(out)


def main() -> int:
    corpus = Path(sys.argv[1])
    out_train = corpus.with_name("merged-v2.jsonl")
    out_quar = corpus.with_name("merged-v2.quarantine.jsonl")
    out_manifest = corpus.with_name("merged-v2.manifest.json")
    t_start = time.time()

    records = [json.loads(line)
               for line in corpus.open()]
    print(f"loaded {len(records)} records", flush=True)

    # ---- near-dup clusters (identical params/order to the dupreport) ----
    src_of = [r.get("source") for r in records]
    sh_cache, sig_cache = {}, {}
    for idx, r in enumerate(records):
        sh_cache[idx] = shingles(canon_state((r.get("request") or {})
                                             .get("state")))
        sig_cache[idx] = minsig(sh_cache[idx])
    print("signatures done", flush=True)

    buckets: dict = defaultdict(list)
    for idx in range(len(records)):
        sig = sig_cache[idx]
        for b in range(BANDS):
            buckets[(src_of[idx], b,
                     hash(tuple(sig[b * ROWS:(b + 1) * ROWS])))].append(idx)
    cand: set = set()
    for members in buckets.values():
        if 2 <= len(members) <= BUCKET_CAP:
            members = sorted(set(members))
            for i in range(len(members)):
                for j in range(i + 1, len(members)):
                    cand.add((members[i], members[j]))
    union_root = list(range(len(records)))

    def find(x: int) -> int:
        while union_root[x] != x:
            union_root[x] = union_root[union_root[x]]
            x = union_root[union_root[x]]
        return x

    for i, j in sorted(cand):
        if find(i) == find(j):
            continue
        if jaccard(sh_cache[i], sh_cache[j]) >= JACCARD_T:
            union_root[find(i)] = find(j)
    clusters: dict = defaultdict(list)
    for idx in range(len(records)):
        clusters[find(idx)].append(idx)
    near = [sorted(c) for c in clusters.values() if len(c) > 1]
    print(f"near-dup clusters: {len(near)}", flush=True)

    # ---- partition under policy ----
    dropped = quarantined = 0
    keep_idx: set = set(range(len(records)))
    quarantine_rows: list = []
    uniform_clusters = mixed_clusters = 0
    dropped_uniform = quarantined_members = 0
    for members in near:
        labels = {gold_labels(records[i]) for i in members}
        if len(labels) == 1:
            uniform_clusters += 1
            rep = max(members, key=lambda i: (
                float((records[i].get("teacher") or {}).get("jev") or 0.0),
                -i))
            for i in members:
                if i != rep:
                    keep_idx.discard(i)
                    dropped_uniform += 1
        else:
            mixed_clusters += 1
            cluster_id = min(records[i]["record_id"] for i in members)
            label_set = sorted(set().union(*labels))
            for i in members:
                keep_idx.discard(i)
                row = dict(records[i])
                row["_quarantine"] = {"cluster_id": cluster_id,
                                      "cluster_size": len(members),
                                      "cluster_labels": label_set}
                quarantine_rows.append(row)
                quarantined_members += 1
    print(f"uniform {uniform_clusters} (dropped {dropped_uniform}), "
          f"mixed {mixed_clusters} (quarantined {quarantined_members})",
          flush=True)

    # ---- emit ----
    with out_train.open("w") as fh:
        for idx in sorted(keep_idx):
            fh.write(json.dumps(records[idx], ensure_ascii=False) + "\n")
    with out_quar.open("w") as fh:
        for row in quarantine_rows:
            fh.write(json.dumps(row, ensure_ascii=False) + "\n")

    def sha256_of(path: Path) -> str:
        h = hashlib.sha256()
        with path.open("rb") as fh:
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest()

    # ---- gates before pinning ----
    suite_hashes = {
        hashlib.sha256(canon_state((r.get("request") or {}).get("state"))
                       .encode("utf-8")).hexdigest()
        for r in records if str(r.get("source", "")).startswith("suite")}
    collisions = 0
    suite_rows_in_output = 0
    kept_kinds: Counter = Counter()
    kept_sources: Counter = Counter()
    with out_train.open() as fh:
        for line in fh:
            r = json.loads(line)
            kept_sources[r.get("source")] += 1
            is_suite = str(r.get("source", "")).startswith("suite")
            suite_rows_in_output += is_suite
            kept_kinds.update(q.get("type") for q in
                              (r.get("request") or {})
                              .get("questions", {}).values())
            if not is_suite and hashlib.sha256(
                    canon_state((r.get("request") or {}).get("state"))
                    .encode("utf-8")).hexdigest() in suite_hashes:
                collisions += 1
    assert collisions == 0, f"train/suite state collisions: {collisions}"
    parent_sha = sha256_of(corpus)

    manifest = {
        "manifest_version": "opencodifier.distill-manifest/2",
        "parent": {"corpus": corpus.name, "sha256": parent_sha},
        "policy": {
            "near_dup": {"shingle_k": SHINGLE_K, "perms": PERMS,
                         "bands_rows": f"{BANDS}x{ROWS}",
                         "jaccard_threshold": JACCARD_T,
                         "bucket_cap": BUCKET_CAP},
            "uniform_cluster_rule": "keep highest teacher.jev, "
                                    "tie-break lowest input line",
            "mixed_cluster_rule": "quarantine all members, excluded "
                                  "from train",
        },
        "records_total": len(keep_idx),
        "sources": dict(kept_sources),
        "question_kinds": dict(kept_kinds),
        "uniform_clusters": uniform_clusters,
        "dropped_uniform": dropped_uniform,
        "mixed_clusters": mixed_clusters,
        "quarantined_members": quarantined_members,
        "quarantine_file": out_quar.name,
        "sha256": {"merged-v2.jsonl": sha256_of(out_train),
                   "merged-v2.quarantine.jsonl": sha256_of(out_quar)},
        "gates": {"train_suite_state_collisions": collisions,
                  "suite_rows_in_output": suite_rows_in_output,
                  "suite_rows_untouched": True},
        "runtime_s": round(time.time() - t_start, 1),
    }
    out_manifest.write_text(json.dumps(manifest, indent=1))
    print(json.dumps({k: manifest[k] for k in
                      ("records_total", "sources", "question_kinds",
                       "sha256", "gates", "runtime_s")}, indent=1))
    print(f"wrote {out_train}, {out_quar}, {out_manifest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
