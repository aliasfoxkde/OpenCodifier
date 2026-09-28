#!/usr/bin/env python3
"""Render SVG charts from the decision-model benchmark results.

Reads every results/*.json produced by the runners (same collection rules
as summarize.py, including chat screens and caveats) and emits static SVG
charts into <results-dir>/charts/:

- accuracy_vs_latency.svg   accuracy vs single-decision p50 (log x), all
                            comparable arms; tier picks labeled
- quant_size_curves.svg     accuracy vs model file size for the Qwen3.5-2B
                            and -4B quant ladders (needs --models-dir)
- relational_ceiling.svg    relational_compositional per arm with the 0.50
                            ceiling line
- calibration_frontier.svg  accuracy vs ECE of the winner probability

The charts are plain SVG (no rendering dependency), deterministic (data
sorted, geometry and colors fixed), and every number is derived from the
measurement JSONs — the charts are a view of the record, never a source.

Usage:
  python3 runner/plot.py --results-dir /path/to/runs \
      --models-dir /path/to/models
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

# Fixed palette per arm family (charts must be stable across regenerations).
COLORS = {
    "engine": "#d62728",
    "embed": "#9467bd",
    "laya": "#8c564b",
    "mimo": "#b8860b",
    "q35-4b": "#1f77b4",
    "q35-2b": "#2ca02c",
    "other": "#7f7f7f",
}
FILL = {"engine": "square", "embed": "triangle", "laya": "diamond", "llama": "circle"}

W, H = 880, 520           # canvas for single-panel charts
ML, MR, MT, MB = 68, 210, 56, 58   # margins (right margin hosts labels)


def load_runs(results_dir: Path) -> list[tuple[str, dict]]:
    """Same collection rules as summarize.py (chat screens rescued, labeled)."""
    runs = []
    for p in sorted(results_dir.glob("*.json")):
        if p.name == "models.manifest.json":
            continue
        d = json.loads(p.read_text())
        if "arm" not in d:
            continue
        if "metrics" not in d:
            chat = (d.get("chat") or {}).get("metrics")
            if not chat or d["arm"] != "llama_chat_baseline_only":
                continue
            d = {
                **d,
                "metrics": {
                    "accuracy": chat["accuracy"],
                    "accuracy_by_class": chat["accuracy_by_class"],
                    "latency": {"p50_ms": chat["p50_ms"]},
                },
                "chat_only_screen": True,
            }
        else:
            # Embedding arms report mean ms/item rather than a p50 latency
            # block; normalize so they appear on the latency axis (the
            # summary table footnotes the distinction).
            lat = d["metrics"].get("latency") or {}
            if not lat.get("p50_ms") and d["metrics"].get("ms_per_item"):
                d = {
                    **d,
                    "metrics": {
                        **d["metrics"],
                        "latency": {"p50_ms": d["metrics"]["ms_per_item"]},
                    },
                }
        runs.append((p.name, d))
    return runs


def comparable(d: dict) -> bool:
    """Chat screens are sampled decode; they never enter a chart."""
    return not d.get("chat_only_screen")


def family(name: str) -> str:
    n = name.lower()
    if n.startswith("engine__"):
        return "engine"
    if n.startswith("embed__"):
        return "embed"
    if n.startswith("laya__"):
        return "laya"
    if "mimo" in n:
        return "mimo"
    if "qwen3.5-4b" in n:
        return "q35-4b"
    if "qwen3.5-2b" in n:
        return "q35-2b"
    return "other"


def short(name: str) -> str:
    s = name
    if s == "laya__en":
        return "Laya-421M"
    for stem in ("llama__", "embed__", "engine__", "laya__", "k2chat__"):
        s = s.removeprefix(stem)
    s = s.removesuffix(".json")
    for a, b in (
        ("MiMo-V2.6-Distill-Qwen-9B", "MiMo-9B"),
        ("Qwen3.5-", "Q3.5-"),
        ("Qwen3.8-", "Q3.8-"),
        ("Qwen2.5-", "Q2.5-"),
        ("-Distilled", ""),
        ("-Distill", ""),
        ("-Instruct-Q4_K_M", ""),
        ("-it-Q4_K_M", ""),
        ("-Q4_K_M", ""),
        ("-q4_k_m", ""),
        ("-q4_0", " q4_0"),
        ("UD-", "UD-"),
        ("gte-modernbert-base", "gte"),
        ("builtin-lexical", "engine lexical"),
        ("Jev-Style-0.8B-Decision-v3", "Jev-0.8B*"),
    ):
        s = s.replace(a, b)
    return s


def esc(s: str) -> str:
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def header(title: str, subtitle: str) -> list[str]:
    return [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" '
        'viewBox="0 0 {w} {h}" font-family="sans-serif">'.format(w=W, h=H),
        f"<text x=\"16\" y=\"24\" font-size=\"17\" font-weight=\"bold\">{esc(title)}</text>",
        f'<text x="16" y="42" font-size="11" fill="#555">{esc(subtitle)}</text>',
    ]


def marker(x: float, y: float, kind: str, color: str) -> str:
    if kind == "square":
        return f'<rect x="{x - 4:.1f}" y="{y - 4:.1f}" width="8" height="8" fill="{color}"/>'
    if kind == "triangle":
        return (f'<path d="M {x:.1f} {y - 5:.1f} L {x + 5:.1f} {y + 4:.1f} '
                f'L {x - 5:.1f} {y + 4:.1f} Z" fill="{color}"/>')
    if kind == "diamond":
        return (f'<path d="M {x:.1f} {y - 6:.1f} L {x + 6:.1f} {y:.1f} '
                f'L {x:.1f} {y + 6:.1f} L {x - 6:.1f} {y:.1f} Z" fill="{color}"/>')
    return f'<circle cx="{x:.1f}" cy="{y:.1f}" r="4.5" fill="{color}"/>'


def log_x(ms: float, lo: float, hi: float) -> float:
    lo_l, hi_l = math.log10(lo), math.log10(hi)
    return ML + (math.log10(max(ms, lo)) - lo_l) / (hi_l - lo_l) * (W - ML - MR)


def lin_y(v: float, lo: float, hi: float) -> float:
    return H - MB - (v - lo) / (hi - lo) * (H - MT - MB)


def axis_frame(y_lo: float, y_hi: float, y_ticks: list[float], y_fmt) -> list[str]:
    out = []
    plot_w, plot_h = W - ML - MR, H - MT - MB
    out.append(
        f'<rect x="{ML}" y="{MT}" width="{plot_w}" height="{plot_h}" '
        'fill="#fafafa" stroke="#999"/>'
    )
    for t in y_ticks:
        y = lin_y(t, y_lo, y_hi)
        if MT - 1 <= y <= H - MB + 1:
            out.append(
                f'<line x1="{ML}" y1="{y:.1f}" x2="{W - MR}" y2="{y:.1f}" stroke="#ddd"/>'
            )
            out.append(
                f'<text x="{ML - 8}" y="{y + 4:.1f}" font-size="11" '
                f'text-anchor="end" fill="#444">{y_fmt(t)}</text>'
            )
    out.append(
        f'<text x="18" y="{MT + plot_h // 2}" font-size="12" fill="#333" '
        f'text-anchor="middle" transform="rotate(-90 18 {MT + plot_h // 2})">accuracy</text>'
    )
    return out


def legend(entries: list[tuple[str, str]], y0: int) -> list[str]:
    out, y = [], y0
    for label, color in entries:
        out.append(f'<rect x="{W - MR + 12}" y="{y - 9}" width="12" height="12" fill="{color}"/>')
        out.append(
            f'<text x="{W - MR + 30}" y="{y + 2}" font-size="11" fill="#333">{esc(label)}</text>'
        )
        y += 18
    return out


def label_points(pts: list[tuple[str, float, float, float]], dy: float = 4.0) -> list[str]:
    """pts: (text, x, y, value_y_direction) — place, nudging to avoid the point."""
    return [
        f'<text x="{x + 7:.1f}" y="{y + dy:.1f}" font-size="10" fill="#222">{esc(text)}</text>'
        for text, x, y, _ in pts
    ]


def chart_accuracy_latency(runs: list[tuple[str, dict]], out_dir: Path) -> None:
    lo, hi = 1.0, 60000.0
    y_lo, y_hi = 0.15, 0.9
    pts = []
    for name, d in runs:
        if not comparable(d):
            continue
        p50 = (d["metrics"].get("latency") or {}).get("p50_ms")
        acc = d["metrics"]["accuracy"]
        if p50 is None:
            continue
        fam = family(name)
        kind = FILL["llama"] if name.startswith("llama__") else FILL[fam]
        pts.append((name, fam, kind, log_x(p50, lo, hi), lin_y(acc, y_lo, y_hi), acc, p50))

    out = header(
        "Accuracy vs single-decision latency (p50, log scale)",
        "Decision-model benchmark — candidate-conditioned tree mode, CPU-only host "
        "(-t 12, -c 8192, -ngl 0). Chat screens excluded (sampled, non-comparable).",
    )
    out += axis_frame(y_lo, y_hi, [0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9], lambda t: f"{t:.1f}")
    x_ticks = [(10, "10ms"), (100, "0.1s"), (1000, "1s"), (10000, "10s"), (60000, "60s")]
    for ms, lab in x_ticks:
        x = log_x(ms, lo, hi)
        out.append(
            f'<line x1="{x:.1f}" y1="{MT}" x2="{x:.1f}" y2="{H - MB}" stroke="#ddd"/>'
        )
        out.append(
            f'<text x="{x:.1f}" y="{H - MB + 18}" font-size="11" text-anchor="middle" '
            f'fill="#444">{lab}</text>'
        )
    out.append(
        f'<text x="{ML + (W - ML - MR) // 2}" y="{H - 16}" font-size="12" '
        'text-anchor="middle" fill="#333">single-decision p50 (log; embed arms '
        'plot mean ms/item)</text>'
    )
    # 0.50 relational-ceiling context: gridline only (accuracy axis).
    labeled = {
        "MiMo-9B-Q3_K_S", "Q3.5-4B-Q3_K_S", "Q3.5-4B-UD-Q4_K_XL", "Q3.5-2B",
        "Q3.5-0.8B q4_0", "engine lexical", "gte", "Jev-0.8B*",
    }
    labels = []
    for name, fam, kind, x, y, acc, p50 in pts:
        s = short(name)
        out.append(marker(x, y, kind, COLORS[fam]))
        if s in labeled:
            labels.append((f"{s} {acc:.3f}", x, y, acc))
    out += label_points(labels)
    out += legend(
        [
            ("decision arm (llama.cpp tree)", COLORS["q35-4b"]),
            ("MiMo-9B", COLORS["mimo"]),
            ("Qwen3.5-2B ladder", COLORS["q35-2b"]),
            ("engine lexical (5.3ms)", COLORS["engine"]),
            ("embedding zero-shot", COLORS["embed"]),
            ("Laya-421M", COLORS["laya"]),
            ("other decision arms", COLORS["other"]),
        ],
        MT + 8,
    )
    out.append("* Jev-Style-0.8B: interface mismatch (verdict-slot readout), see REPORT.md F11")
    out.append("</svg>")
    (out_dir / "accuracy_vs_latency.svg").write_text("\n".join(out) + "\n")


def chart_quant_curves(runs: list[tuple[str, dict]], models_dir: Path, out_dir: Path) -> None:
    panels = [("q35-2b", "Qwen3.5-2B ladder"), ("q35-4b", "Qwen3.5-4B ladder")]
    measured = []
    for name, d in runs:
        fam = family(name)
        if fam not in ("q35-2b", "q35-4b") or not comparable(d):
            continue
        f = models_dir / d.get("model", {}).get("file", "")
        if not f.is_file():
            continue
        measured.append((name, fam, f.stat().st_size / 1e9, d["metrics"]["accuracy"]))
    if not measured:
        return

    out = header(
        "Accuracy vs model file size — quant ladders",
        "Same decision arm and host for every point; x is measured GGUF size "
        "(GB = 10^9 bytes). The 2-bit collapse is the left edge of each ladder.",
    )
    y_lo, y_hi = 0.3, 0.85
    panel_w = (W - ML - MR - 30) // 2
    out += axis_frame(y_lo, y_hi, [0.4, 0.5, 0.6, 0.7, 0.8], lambda t: f"{t:.2f}")
    for i, (fam, title) in enumerate(panels):
        series = sorted((m for m in measured if m[1] == fam), key=lambda m: m[2])
        if not series:
            continue
        x0 = ML + i * (panel_w + 30)
        sizes = [m[2] for m in series]
        sx_lo, sx_hi = min(sizes) * 0.92, max(sizes) * 1.06

        def px(v: float, x0: int = x0, lo: float = sx_lo, hi: float = sx_hi) -> float:
            return x0 + (v - lo) / (hi - lo) * panel_w

        out.append(f'<text x="{x0 + panel_w // 2}" y="{MT - 6}" font-size="13" '
                   f'text-anchor="middle" font-weight="bold">{title}</text>')
        poly = " ".join(f"{px(s):.1f},{lin_y(a, y_lo, y_hi):.1f}" for _, _, s, a in series)
        out.append(f'<polyline points="{poly}" fill="none" stroke="{COLORS[fam]}" stroke-width="1.5"/>')
        for name, _, s, a in series:
            x, y = px(s), lin_y(a, y_lo, y_hi)
            out.append(marker(x, y, "circle", COLORS[fam]))
            out.append(
                f'<text x="{x:.1f}" y="{y - 9:.1f}" font-size="9.5" text-anchor="middle" '
                f'fill="#222">{esc(short(name))}</text>'
            )
        # x scale endpoints
        for v in (sx_lo, (sx_lo + sx_hi) / 2, sx_hi):
            out.append(
                f'<text x="{px(v):.1f}" y="{H - MB + 18}" font-size="10" '
                f'text-anchor="middle" fill="#444">{v:.1f}GB</text>'
            )
    out.append(
        f'<text x="{ML + (W - ML - MR) // 2}" y="{H - 16}" font-size="12" '
        'text-anchor="middle" fill="#333">model file size (GB) — accuracy collapses '
        'at or below 2 bits</text>'
    )
    out.append("</svg>")
    (out_dir / "quant_size_curves.svg").write_text("\n".join(out) + "\n")


def chart_relational_ceiling(runs: list[tuple[str, dict]], out_dir: Path) -> None:
    arms = []
    for name, d in runs:
        if not comparable(d):
            continue
        rel = d["metrics"]["accuracy_by_class"].get("relational_compositional")
        if rel is None:
            continue
        arms.append((name, rel))
    arms.sort(key=lambda a: (-a[1], a[0]))
    top = 14
    shown, rest = arms[:top], arms[top:]
    others = [f"{short(n)} {r:.2f}" for n, r in rest]
    chunks = [", ".join(others[i:i + 6]) for i in range(0, len(others), 6)]
    bar_h, gap = 20, 7
    plot_h = len(shown) * (bar_h + gap) + 14
    h = MT + plot_h + 30 + len(chunks) * 13
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{h}" '
        'viewBox="0 0 {w} {h}" font-family="sans-serif">'.format(w=W, h=h),
        "<text x=\"16\" y=\"24\" font-size=\"17\" font-weight=\"bold\">"
        "relational_compositional — the ceiling and who breaks it</text>",
        f'<text x="16" y="42" font-size="11" fill="#555">Top {top} arms by relational '
        'accuracy (candidate-conditioned decision arm unless noted). 0.50 held for '
        'every arm until MiMo-9B Q3_K_S at 14.3 s p50 (verifier tier, D16).</text>',
    ]
    plot_w = W - ML - MR
    x_for = lambda v: ML + v * plot_w
    for v in (0.0, 0.25, 0.5, 0.75, 1.0):
        x = x_for(v)
        out.append(f'<line x1="{x:.1f}" y1="{MT}" x2="{x:.1f}" y2="{MT + plot_h}" stroke="#ddd"/>')
        out.append(f'<text x="{x:.1f}" y="{MT + plot_h + 16}" font-size="11" '
                   f'text-anchor="middle" fill="#444">{v:.2f}</text>')
    xc = x_for(0.50)
    out.append(f'<line x1="{xc:.1f}" y1="{MT - 4}" x2="{xc:.1f}" y2="{MT + plot_h}" '
               'stroke="#d62728" stroke-dasharray="5,4" stroke-width="1.5"/>')
    out.append(f'<text x="{xc + 5:.1f}" y="{MT + 2}" font-size="10.5" fill="#d62728">'
               '0.50 ceiling (interactive tiers)</text>')
    y = MT + 10
    for name, rel in shown:
        fam = family(name)
        is_pick = "MiMo" in name and "Q3_K_S" in name
        pick_attr = ' stroke="#b8860b" stroke-width="2.5"' if is_pick else ""
        out.append(f'<rect x="{ML}" y="{y}" width="{rel * plot_w:.1f}" height="{bar_h}" '
                   f'fill="{COLORS[fam]}"{pick_attr}/>')
        out.append(f'<text x="{ML - 8}" y="{y + 14}" font-size="10.5" text-anchor="end" '
                   f'fill="#333">{esc(short(name))}</text>')
        out.append(f'<text x="{ML + rel * plot_w + 6:.1f}" y="{y + 14}" font-size="10.5" '
                   f'fill="#222">{rel:.3f}</text>')
        y += bar_h + gap
    others = [f"{short(n)} {r:.2f}" for n, r in rest]
    chunks = [", ".join(others[i:i + 6]) for i in range(0, len(others), 6)]
    for j, chunk in enumerate(chunks):
        out.append(f'<text x="{ML}" y="{y + 12 + j * 13}" font-size="10" fill="#555">'
                   f'{"Rest:" if j == 0 else ""} {esc(chunk)}</text>')
    out.append("</svg>")
    (out_dir / "relational_ceiling.svg").write_text("\n".join(out) + "\n")


def chart_calibration(runs: list[tuple[str, dict]], out_dir: Path) -> None:
    y_lo, y_hi = 0.15, 0.9
    x_lo, x_hi = 0.0, 0.65
    out = header(
        "Calibration frontier — accuracy vs ECE of the winner probability",
        "Raw (uncalibrated) winner probability, 10 equal-width bins, n=120. "
        "No arm ships calibration; D15 must fit temperatures before any "
        "confidence leaves the runtime.",
    )
    plot_w, plot_h = W - ML - MR, H - MT - MB
    out.append(f'<rect x="{ML}" y="{MT}" width="{plot_w}" height="{plot_h}" '
               'fill="#fafafa" stroke="#999"/>')

    def px(e: float) -> float:
        return ML + (e - x_lo) / (x_hi - x_lo) * plot_w

    for t in (0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9):
        y = lin_y(t, y_lo, y_hi)
        out.append(f'<line x1="{ML}" y1="{y:.1f}" x2="{W - MR}" y2="{y:.1f}" stroke="#ddd"/>')
        out.append(f'<text x="{ML - 8}" y="{y + 4:.1f}" font-size="11" text-anchor="end" '
                   f'fill="#444">{t:.1f}</text>')
    for t in (0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6):
        x = px(t)
        out.append(f'<line x1="{x:.1f}" y1="{MT}" x2="{x:.1f}" y2="{H - MB}" stroke="#ddd"/>')
        out.append(f'<text x="{x:.1f}" y="{H - MB + 18}" font-size="11" text-anchor="middle" '
                   f'fill="#444">{t:.1f}</text>')
    out.append(f'<text x="{ML + plot_w // 2}" y="{H - 16}" font-size="12" '
               'text-anchor="middle" fill="#333">ECE (lower is better)</text>')
    out.append(f'<text x="18" y="{MT + plot_h // 2}" font-size="12" fill="#333" '
               f'text-anchor="middle" transform="rotate(-90 18 {MT + plot_h // 2})">accuracy</text>')
    labels = []
    for name, d in runs:
        if not comparable(d):
            continue
        ece = d["metrics"].get("ece")
        acc = d["metrics"]["accuracy"]
        if ece is None:
            continue
        x, y = px(ece), lin_y(acc, y_lo, y_hi)
        kind = FILL["llama"] if name.startswith("llama__") else FILL[family(name)]
        out.append(marker(x, y, kind, COLORS[family(name)]))
        s = short(name)
        if s in {"MiMo-9B-Q3_K_S", "Q3.8-4B", "Q3.5-4B-Q3_K_S", "gte", "gemma-3-270m-it"}:
            labels.append((s, x, y, acc))
    out += label_points(labels)
    out += legend(
        [
            ("decision arm", COLORS["q35-4b"]),
            ("MiMo-9B", COLORS["mimo"]),
            ("Qwen3.5-2B ladder", COLORS["q35-2b"]),
            ("engine lexical", COLORS["engine"]),
            ("embedding zero-shot", COLORS["embed"]),
            ("Laya-421M", COLORS["laya"]),
            ("other decision arms", COLORS["other"]),
        ],
        MT + 8,
    )
    out.append("</svg>")
    (out_dir / "calibration_frontier.svg").write_text("\n".join(out) + "\n")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--results-dir", type=Path, required=True)
    ap.add_argument("--models-dir", type=Path, default=None,
                    help="GGUF directory; enables file-size x axis for quant curves")
    args = ap.parse_args()
    runs = load_runs(args.results_dir)
    if not runs:
        print("no result files found", file=sys.stderr)
        return 1
    out_dir = args.results_dir / "charts"
    out_dir.mkdir(parents=True, exist_ok=True)
    chart_accuracy_latency(runs, out_dir)
    if args.models_dir and args.models_dir.is_dir():
        chart_quant_curves(runs, args.models_dir, out_dir)
    chart_relational_ceiling(runs, out_dir)
    chart_calibration(runs, out_dir)
    for p in sorted(out_dir.glob("*.svg")):
        print("wrote", p)
    return 0


if __name__ == "__main__":
    sys.exit(main())
