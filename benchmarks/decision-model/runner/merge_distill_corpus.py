#!/usr/bin/env python3
"""Merge distillation corpora into one pinned, IR-shaped training artifact.

Task #88 step 1 (docs/TRAINING.md §9). Sources, in merge order:

1. ``LocalLLaMA/typed-decisions`` (Apache-2.0, parquet) — native IR shape:
   rows carry ``state`` (JSON), ``questions`` (``{name: {type, instructions,
   criteria}}`` over choice/noul/score) and ``gold`` (per-question label,
   option probabilities, confidence, scalar ``noul``). Emitted verbatim
   per row — zero-loss.
2. ``SargeDev/jev-distill-corpus`` (Apache-2.0, jsonl) — relevance noul
   decisions: ``{query, text, label_32b, label_binary, jev, row_id}``.
   Normalized into the same request shape; raw teacher values are carried
   under explicit keys (no probability transform is invented here).
3. Local suites (``suite.json``, ``suite_holdout.json``) — choice items,
   tagged as eval surfaces. They are emitted last and never dropped, and
   external sources are decontaminated against them.

Determinism: source order fixed, per-source iteration in file order,
dedupe keeps first occurrence, all hashing SHA-256 over UTF-8.

Excluded by license rule (recorded in the manifest): ``nvidia/HelpSteer2``
(CC-BY-4.0; disclosed as a jebadiah-4b-v2 training source) — the program
accepts Apache-2.0 corpora only.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from collections import Counter
from pathlib import Path

MANIFEST_VERSION = "opencodifier.distill-manifest/1"
RECORD_VERSION = "opencodifier.distill-record/1"


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def norm(text: str) -> str:
    return " ".join(text.split()).casefold()


def state_fingerprint(state) -> str:
    """Stable fingerprint of a state value (str passes through, dict canonicalized)."""
    if isinstance(state, str):
        return hashlib.sha256(norm(state).encode("utf-8")).hexdigest()
    return hashlib.sha256(
        json.dumps(state, sort_keys=True, ensure_ascii=False).encode("utf-8")
    ).hexdigest()


class MergeStats:
    def __init__(self) -> None:
        self.rows_read = 0
        self.parse_skipped = 0
        self.records_emitted = 0
        self.questions_dropped_contamination = 0
        self.records_dropped_duplicate = 0
        self.kind_counts: Counter[str] = Counter()


def eval_keys(suite_paths: list[Path]) -> set[tuple[str, str, str]]:
    keys: set[tuple[str, str, str]] = set()
    for path in suite_paths:
        doc = json.loads(path.read_text())
        for item in doc["items"]:
            q = item["question"]
            state = item.get("context", "")
            crit = {c["id"]: c.get("description", "") for c in item.get("candidates", [])}
            keys.add(("choice", norm(q), state_fingerprint(state)))
            for oid, desc in crit.items():
                keys.add(("choice", norm(f"{q} {oid} {desc}"), state_fingerprint(state)))
    return keys


def load_typed_decisions(path: Path, stats: MergeStats, seen: set, evals: set,
                         out: list[dict]) -> None:
    try:
        import pyarrow.parquet as pq  # type: ignore[import-not-found]
    except ImportError as e:
        raise SystemExit(
            f"typed-decisions source requires pyarrow (pip install pyarrow): {e}")
    tbl = pq.read_table(str(path))
    cols = {c: tbl.column(c).to_pylist() for c in
            ("id", "workflow", "split", "state", "questions", "gold", "n_questions")}
    for i in range(len(cols["id"])):
        stats.rows_read += 1
        try:
            state = json.loads(cols["state"][i])
            questions = json.loads(cols["questions"][i])
            gold = json.loads(cols["gold"][i])
        except (json.JSONDecodeError, TypeError):
            stats.parse_skipped += 1
            continue
        sf = state_fingerprint(state)
        kept_questions = {}
        contaminated = 0
        for name, q in questions.items():
            kind = q.get("type", "")
            if (kind, norm(q.get("instructions", "")), sf) in evals:
                contaminated += 1
                continue
            kept_questions[name] = q
        stats.questions_dropped_contamination += contaminated
        if not kept_questions:
            continue
        kept_gold = {n: g for n, g in gold.items() if n in kept_questions}
        rec = {
            "record_version": RECORD_VERSION,
            "record_id": f"typed-decisions:{cols['id'][i]}",
            "source": "LocalLLaMA/typed-decisions",
            "source_config": cols["workflow"][i],
            "source_split": cols["split"][i],
            "license": "apache-2.0",
            "request": {"state": state, "questions": kept_questions},
            "target": kept_gold,
            "teacher": {},
        }
        key = (f"typed-decisions:{cols['id'][i]}", sf)
        if key in seen:
            stats.records_dropped_duplicate += 1
            continue
        seen.add(key)
        for name, q in kept_questions.items():
            stats.kind_counts[q.get("type", "unknown")] += 1
        stats.records_emitted += 1
        out.append(rec)


NOUL_CRITERIA = {
    "false": "The text does not satisfy the query.",
    "true": "The text satisfies the query.",
}


def load_jev_corpus(path: Path, stats: MergeStats, seen: set, evals: set,
                    out: list[dict]) -> None:
    with path.open() as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            stats.rows_read += 1
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                stats.parse_skipped += 1
                continue
            state = {"query": row["query"], "text": row["text"]}
            sf = state_fingerprint(state)
            question = row["query"]
            if ("noul", norm(question), sf) in evals:
                stats.questions_dropped_contamination += 1
                continue
            label = "true" if row["label_binary"] else "false"
            rec = {
                "record_version": RECORD_VERSION,
                "record_id": f"jev-distill-corpus:{row['row_id']}",
                "source": "SargeDev/jev-distill-corpus",
                "source_config": "default",
                "source_split": "train",
                "license": "apache-2.0",
                "request": {
                    "state": state,
                    "questions": {
                        "relevance": {
                            "type": "noul",
                            "instructions": question,
                            "criteria": dict(NOUL_CRITERIA),
                        }
                    },
                },
                "target": {"relevance": {"label": label, "type": "noul"}},
                "teacher": {"label_32b": row.get("label_32b"),
                            "label_binary": row.get("label_binary"),
                            "jev": row.get("jev")},
            }
            key = ("jev-distill-corpus", norm(question), sf)
            if key in seen:
                stats.records_dropped_duplicate += 1
                continue
            seen.add(key)
            stats.kind_counts["noul"] += 1
            stats.records_emitted += 1
            out.append(rec)


def load_suite(path: Path, tag: str, stats: MergeStats, out: list[dict]) -> None:
    doc = json.loads(path.read_text())
    for item in doc["items"]:
        stats.rows_read += 1
        criteria = {c["id"]: c.get("description", "")
                    for c in item.get("candidates", [])}
        rec = {
            "record_version": RECORD_VERSION,
            "record_id": f"{tag}:{item['id']}",
            "source": tag,
            "source_config": item.get("class", ""),
            "source_split": tag,
            "license": "apache-2.0",
            "request": {
                "state": item.get("context", ""),
                "questions": {
                    "q": {
                        "type": "choice",
                        "instructions": item["question"],
                        "criteria": criteria,
                    }
                },
            },
            "target": {"q": {"label": item["answer"], "type": "choice"}},
            "teacher": {},
        }
        stats.kind_counts["choice"] += 1
        stats.records_emitted += 1
        out.append(rec)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--typed-decisions", type=Path, default=None,
                    help="all/train-*.parquet from LocalLLaMA/typed-decisions")
    ap.add_argument("--jev-corpus", type=Path, default=None,
                    help="dataset.jsonl from SargeDev/jev-distill-corpus")
    ap.add_argument("--suite", type=Path, action="append", default=[],
                    help="eval suite json (repeatable); emitted tagged, never dropped")
    ap.add_argument("--output", type=Path, required=True)
    ap.add_argument("--manifest", type=Path, required=True)
    args = ap.parse_args()

    if not (args.typed_decisions or args.jev_corpus or args.suite):
        raise SystemExit("no sources given")

    evals = eval_keys(args.suite)
    seen: set = set()
    out: list[dict] = []
    sources: dict[str, MergeStats] = {}

    def stats_for(name: str) -> MergeStats:
        return sources.setdefault(name, MergeStats())

    if args.typed_decisions:
        st = stats_for("LocalLLaMA/typed-decisions")
        load_typed_decisions(args.typed_decisions, st, seen, evals, out)
    if args.jev_corpus:
        st = stats_for("SargeDev/jev-distill-corpus")
        load_jev_corpus(args.jev_corpus, st, seen, evals, out)
    for suite_path in args.suite:
        tag = suite_path.stem
        st = stats_for(tag)
        load_suite(suite_path, tag, st, out)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    payload = "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in out)
    args.output.write_text(payload, encoding="utf-8")

    manifest = {
        "manifest_version": MANIFEST_VERSION,
        "record_version": RECORD_VERSION,
        "merged_sha256": sha256_file(args.output),
        "records_total": len(out),
        "sources": {},
        "eval_surfaces_tagged": [p.name for p in args.suite],
        "decontamination": "external records matching an eval-suite "
                           "(kind, normalized question, state fingerprint) "
                           "are dropped per question and counted",
        "dedup": "exact only: first-seen wins on (source-id or normalized "
                 "question, state fingerprint); near-dup detection is NOT "
                 "applied in this merge and remains open",
        "normalization_notes": [
            "typed-decisions rows emitted verbatim in {state, questions, gold} "
            "shape; contamination drops are per question",
            "jev-distill-corpus normalized to a noul question with fixed "
            "criteria; label_binary becomes the hard label; label_32b/jev "
            "carried raw under teacher (no probability transform invented)",
            "suite items mapped to single choice questions; emitted last, "
            "tagged as eval surfaces, never deduped away",
        ],
        "license_exclusions": [
            {"corpus": "nvidia/HelpSteer2", "license": "CC-BY-4.0",
             "reason": "program accepts Apache-2.0 corpora only "
                       "(docs/TRAINING.md §9); disclosed as a jebadiah-4b-v2 "
                       "training source"},
            {"corpus": "frontier-infra/jebadiah-4b-v2 unspecified corpora",
             "reason": "model card discloses only typed-decisions (included) "
                       "and HelpSteer2 (excluded); no further corpus assumed"},
        ],
    }
    for name, st in sources.items():
        src_path = (args.typed_decisions if name == "LocalLLaMA/typed-decisions"
                    else args.jev_corpus if name == "SargeDev/jev-distill-corpus"
                    else None)
        manifest["sources"][name] = {
            "path": str(src_path) if src_path else name,
            "sha256": sha256_file(src_path) if src_path else None,
            "rows_read": st.rows_read,
            "parse_skipped": st.parse_skipped,
            "records_emitted": st.records_emitted,
            "questions_dropped_contamination": st.questions_dropped_contamination,
            "records_dropped_duplicate": st.records_dropped_duplicate,
            "kinds": dict(sorted(st.kind_counts.items())),
        }
    kind_totals: Counter[str] = Counter()
    split_totals: Counter[str] = Counter()
    for r in out:
        for q in r["request"]["questions"].values():
            kind_totals[q.get("type", "unknown")] += 1
        split_totals[r["source_split"]] += 1
    manifest["kinds"] = dict(sorted(kind_totals.items()))
    manifest["splits"] = dict(sorted(split_totals.items()))
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.write_text(
        json.dumps(manifest, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")

    print(f"merged records={len(out)} -> {args.output}")
    print(f"kinds={dict(sorted(kind_totals.items()))} "
          f"splits={dict(sorted(split_totals.items()))}")
    for name, entry in manifest["sources"].items():
        print(f"  {name}: rows={entry['rows_read']} emitted={entry['records_emitted']} "
              f"contam={entry['questions_dropped_contamination']} "
              f"dup={entry['records_dropped_duplicate']}")


if __name__ == "__main__":
    sys.exit(main())
