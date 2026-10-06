#!/usr/bin/env python3
"""Emit the flat, sortable board CSV from the docs of record.

Parses every measurement table in docs/BENCHMARKS.md (the comparison page
of record) into one results/board.csv with a unified column schema, so the
board can be sorted/filtered with any tool. Raw run JSONs are gitignored by
design; where one is reachable under the local out-of-repo runs dir it
enriches the row (mean latency, item count) without ever overriding a
published number.

Usage: python3 runner/board_csv.py
  [--docs docs/BENCHMARKS.md] [--out results/board.csv]
  [--runs-dir /nas/Temp/work/oc-model-eval/runs]
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
from pathlib import Path

SUPERSCRIPTS = str.maketrans("⁰¹²³⁴⁵⁶⁷⁸⁹", "0123456789")

COLUMNS = [
    "board", "section", "name", "qualifier", "n", "n_correct", "accuracy",
    "macro", "lexical", "metadata", "relational", "ece", "brier", "auto_share_5err",
    "p50_ms", "mean_ms", "determinism", "replay_delta", "invalid_dists", "size",
    "tier", "provenance", "footnotes", "composite_a_trust", "composite_spd",
    "composite_res", "overall", "vision", "notes",
]


def clean(text: str) -> str:
    """Strip bold/italics markers and collapse whitespace."""
    return re.sub(r"\s+", " ", text.replace("**", "").replace("*", "")).strip()


def footnotes_of(text: str) -> str:
    return "".join(re.findall(r"[⁰¹²³⁴⁵⁶⁷⁸⁹]", text)).translate(SUPERSCRIPTS)


def drop_footnotes(text: str) -> str:
    return re.sub(r"[⁰¹²³⁴⁵⁶⁷⁸⁹]", "", text)


def to_float(text: str) -> str:
    m = re.search(r"-?\d+(?:\.\d+)?", text.replace(",", ""))
    return m.group(0) if m else ""


def to_ms(text: str) -> str:
    m = re.search(r"(\d+(?:\.\d+)?)\s*(ms|s)\b", text)
    if not m:
        return ""
    value = float(m.group(1))
    return str(round(value * 1000.0)) if m.group(2) == "s" else str(value)


def frac(text: str) -> str:
    """Normalize a printed accuracy (fraction or percent) to a fraction."""
    m = re.search(r"(\d+(?:\.\d+)?)\s*%", text)
    if m:
        return f"{float(m.group(1)) / 100.0:.4f}"
    return to_float(text)


def pct_cell(text: str) -> str:
    """Percent-scale cell (published boards print e.g. 88.70) -> fraction."""
    value = to_float(text)
    return f"{float(value) / 100.0:.4f}" if value else ""


def is_separator(cells: list[str]) -> bool:
    return bool(cells) and all(re.fullmatch(r"-+:?", c.strip()) for c in cells)


def split_classes(cell: str) -> dict[str, str]:
    """'0.97 / 1.00 / 0.57' -> metadata/lexical/relational (printed order)."""
    parts = [p.strip() for p in cell.split("/") if p.strip()]
    if len(parts) != 3:
        return {}
    return {"metadata": parts[0], "lexical": parts[1], "relational": parts[2]}


def parse_determinism(cell: str) -> dict[str, str]:
    out: dict[str, str] = {}
    low = cell.lower()
    if "n/a" in low or "sampled" in low:
        out["determinism"] = "sampled"
    elif re.match(r"\s*no\b", low):
        out["determinism"] = "no"
    elif "yes" in low or re.search(r"\d+/\d+", cell):
        out["determinism"] = "yes"
    else:
        out["determinism"] = clean(cell)
    m = re.search(r"Δp\s*([\d.]+)", cell)
    if m:
        out["replay_delta"] = m.group(1)
    m = re.search(r"(\d+)\s*invalid dists", cell)
    if m:
        out["invalid_dists"] = m.group(1)
    return out


def split_qualifier(name: str) -> tuple[str, str]:
    """Move a trailing '(chat screen)' style qualifier out of the name."""
    m = re.search(r"\s*\(([^)]+)\)\s*$", name)
    if not m:
        return name, ""
    return name[: m.start()].strip(), m.group(1)


def parse_tables(markdown: str) -> list[tuple[str, str, list[list[str]]]]:
    """Group consecutive pipe-lines under their headings into tables."""
    out: list[tuple[str, str, list[list[str]]]] = []
    board = section = ""
    for line in markdown.splitlines():
        if line.startswith("#"):
            level = len(line) - len(line.lstrip("#"))
            title = clean(line.lstrip("#"))
            if level <= 2:
                board, section = title, ""
            else:
                section = title
        elif line.startswith("|") and line.rstrip().endswith("|"):
            cells = [clean(c) for c in line.strip().strip("|").split("|")]
            if out and out[-1][0] == board and out[-1][1] == section:
                out[-1][2].append(cells)
            else:
                out.append((board, section, [cells]))
        elif out and out[-1][2]:
            # blank/prose line ends the current table
            out.append((out[-1][0], out[-1][1], []))
    return [t for t in out if t[2]]


def classify(header: list[str]) -> str:
    joined = " ".join(header)
    if "run" in header and "determinism" in header:
        return "suite_run"
    # Bounded-probe subsections are labeled "not a board row" in the docs
    # (r19 speed reference, r20b PTQ probe, shuffled-context probe) — no
    # CSV rows, per board discipline.
    if "args" in header and "wall" in header:
        return "probe"
    if "decision flips" in header:
        return "probe"
    if "pp400 t/s" in header:
        return "probe"
    if "system" in header and "brier" in header:
        return "jevbench_ours"
    if "public acc" in joined:
        return "jevbench_published"
    if "family-macro" in joined and "instructions" in header:
        return "fork_ab"
    if "jevbench" in joined:
        return "jevbench_published_macro"
    if "tier" in header:
        return "tier"
    if "micro-acc" in header and "macro-acc" in header:
        return "jabr"
    if "overall" in header:
        return "composite"
    if "arm" in header and "suite" in header:
        return "params_ab"
    return "other"


def map_table(board: str, section: str, rows: list[list[str]]) -> list[dict[str, str]]:
    if len(rows) < 2:
        return []
    header = [h.lower() for h in rows[0]]
    kind = classify(header)
    if kind == "probe":
        return []
    out: list[dict[str, str]] = []
    for cells in rows[1:]:
        if not any(cells) or is_separator(cells):
            continue
        r = dict(zip(header, cells))
        row = dict.fromkeys(COLUMNS, "")
        row.update(board=board, section=section, notes=kind)
        if kind == "suite_run":
            raw = r.get("run", "")
            name, qual = split_qualifier(drop_footnotes(raw).strip())
            row["name"], row["qualifier"] = name, qual
            row["footnotes"] = footnotes_of(raw)
            row |= split_classes(r.get("acc (meta/lex/rel)", ""))
            row["accuracy"] = drop_footnotes(r.get("acc", "")).strip()
            row["ece"] = r.get("ece", "").replace("—", "")
            row["p50_ms"] = to_ms(r.get("p50", ""))
            row |= parse_determinism(r.get("determinism", ""))
            low = name.lower()
            row["notes"] = ("embed" if low.startswith("embed")
                            else "engine" if low.startswith("engine")
                            else "chat-screen" if qual == "chat screen"
                            else "llm-suite")
        elif kind == "jevbench_ours":
            raw = r.get("system", "")
            name, qual = split_qualifier(drop_footnotes(raw).strip())
            row["name"], row["qualifier"] = name, qual
            row["footnotes"] = footnotes_of(raw)
            row["n"] = drop_footnotes(r.get("n", "")).strip()
            row["accuracy"] = drop_footnotes(frac(r.get("accuracy", ""))).strip()
            row["macro"] = drop_footnotes(frac(r.get("family-macro", ""))).strip()
            row["ece"] = drop_footnotes(r.get("ece", "").replace("—", "")).strip()
            row["brier"] = drop_footnotes(r.get("brier", "").replace("—", "")).strip()
            row["auto_share_5err"] = to_float(r.get("auto@5%", "").replace("—", ""))
            row["p50_ms"] = to_ms(r.get("p50 (client wall)", ""))
            row |= parse_determinism(r.get("determinism", ""))
            row["notes"] = "jevbench-our-run"
        elif kind == "jevbench_published":
            cell = r.get("public acc", "")
            name, qual = split_qualifier(r.get("system", "").strip())
            row["name"], row["qualifier"] = name.strip("* "), qual
            row["accuracy"] = frac(cell)
            m = re.search(r"\((\d+)/(\d+)\)", cell)
            if m:
                row["n_correct"], row["n"] = m.group(1), m.group(2)
            m = re.search(r"ECE\s*([\d.]+)", cell)
            if m:
                row["ece"] = m.group(1)
            m = re.search(r"Brier\s*([\d.]+)", cell)
            if m:
                row["brier"] = m.group(1)
            row["provenance"] = r.get("provenance", "")
            row["notes"] = "jevbench-published"
        elif kind == "jevbench_published_macro":
            row["name"] = r.get("system", "").strip("* ").strip()
            row["macro"] = pct_cell(r.get("jevbench (231, family-macro)", ""))
            row["accuracy"] = row["macro"]
            row["notes"] = "jevbench-published-macro"
        elif kind == "fork_ab":
            row["name"] = clean(r.get("instructions", ""))
            row["accuracy"] = frac(r.get("accuracy", ""))
            row["macro"] = frac(r.get("family-macro", ""))
            row["p50_ms"] = to_ms(r.get("p50", ""))
            row["notes"] = "fork-instruction-ab"
        elif kind == "jabr":
            row["name"] = r.get("suite", "")
            row["n"] = r.get("n", "")
            row["accuracy"] = r.get("micro-acc", "")
            row["macro"] = r.get("macro-acc", "")
            row["mean_ms"] = to_ms(r.get("latency mean", ""))
            row["qualifier"] = f"{r.get('errors', '')} errors" if r.get("errors", "") else ""
            row["provenance"] = "self-run, public harness (CC0), 2026-10-06"
            row["notes"] = "jabr-self-run"
        elif kind == "tier":
            row["tier"] = r.get("tier", "")
            row["name"] = drop_footnotes(r.get("pick", "")).strip()
            row["accuracy"] = r.get("acc", "")
            row["ece"] = r.get("ece", "")
            row["relational"] = r.get("rel", "")
            row["p50_ms"] = to_ms(r.get("p50", ""))
            row["size"] = r.get("size", "")
            row["notes"] = "tier-pick"
        elif kind == "composite":
            name, qual = split_qualifier(r.get("system", "").strip())
            row["name"], row["qualifier"] = name, qual
            row["footnotes"] = footnotes_of(
                " ".join(r.get(k, "") for k in
                         ("acc", "ece", "a_trust", "spd", "res", "overall")))
            for key in ("acc", "ece", "a_trust", "spd", "res", "overall"):
                r[key] = drop_footnotes(r.get(key, "")).replace("—", "").strip()
            row["accuracy"] = r["acc"]
            row["ece"] = r["ece"]
            row["composite_a_trust"] = r["a_trust"]
            row["composite_spd"] = r["spd"]
            row["composite_res"] = r["res"]
            row["overall"] = r["overall"]
            row["vision"] = r.get("vision", "")
            row["footnotes"] = footnotes_of(name)
            row["notes"] = "composite-score"
        elif kind == "params_ab":
            row["name"] = r.get("arm", "")
            row["qualifier"] = ", ".join(
                v for v in (r.get("threads / ctx", ""), r.get("suite", ""))
                if v and v != "—")
            row["accuracy"] = r.get("acc", "")
            row["ece"] = r.get("ece", "")
            row["p50_ms"] = to_ms(r.get("p50", ""))
            row["provenance"] = ", ".join(
                v for v in (r.get("load window", ""), r.get("status", ""))
                if v)
            row["notes"] = "fork-params-ab"
        else:
            row["name"] = clean(" | ".join(cells))
            row["notes"] = "unmapped-table"
        out.append(row)
    return out


def enrich(runs_dir: Path, csv_path: Path) -> int:
    """Fill mean_ms/n from raw run JSONs where they are reachable."""
    by_name = {p.name: p for p in runs_dir.glob("*.json")} if runs_dir.is_dir() else {}
    rows = list(csv.DictReader(csv_path.open()))
    added = 0
    for row in rows:
        src = by_name.get(row["name"])
        if not src:
            continue
        try:
            data = json.loads(src.read_text())
        except (OSError, json.JSONDecodeError):
            continue
        metrics = data.get("metrics", {})
        # Engine arms nest latency under metrics.latency (run_engine.py);
        # model arms carry a flat mean_ms.
        mean = metrics.get("mean_ms")
        if mean is None and isinstance(metrics.get("latency"), dict):
            mean = metrics["latency"].get("mean_ms")
        if mean and not row["mean_ms"]:
            row["mean_ms"] = str(round(mean))
        if metrics.get("n") and not row["n"]:
            row["n"] = str(metrics["n"])
        added += 1
    with csv_path.open("w", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=COLUMNS)
        writer.writeheader()
        writer.writerows(rows)
    return added


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    here = Path(__file__).resolve().parent
    ap.add_argument("--docs", type=Path,
                    default=here.parents[2] / "docs" / "BENCHMARKS.md")
    ap.add_argument("--report", type=Path,
                    default=here.parent / "results" / "REPORT.md")
    ap.add_argument("--out", type=Path,
                    default=here.parent / "results" / "board.csv")
    ap.add_argument("--runs-dir", type=Path,
                    default=Path("/nas/Temp/work/oc-model-eval/runs"))
    args = ap.parse_args()

    rows: list[dict[str, str]] = []
    for board, section, cells in parse_tables(args.docs.read_text()):
        rows.extend(map_table(board, section, cells))
    # REPORT.md re-tabulates the BENCHMARKS.md rows under human names; take
    # from it only the tables the comparison page does not carry.
    for _board, section, cells in parse_tables(args.report.read_text()):
        for row in map_table("REPORT.md", section, cells):
            if row["notes"] == "fork-params-ab":
                rows.append(row)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=COLUMNS)
        writer.writeheader()
        writer.writerows(rows)
    enriched = enrich(args.runs_dir, args.out)
    print(f"{len(rows)} rows -> {args.out} ({enriched} enriched from {args.runs_dir})")
    unmapped = [r for r in rows if r["notes"] == "unmapped-table"]
    for r in unmapped:
        print(f"unmapped: [{r['board']}] {r['name'][:90]}", file=sys.stderr)
    return 1 if unmapped else 0


if __name__ == "__main__":
    sys.exit(main())
