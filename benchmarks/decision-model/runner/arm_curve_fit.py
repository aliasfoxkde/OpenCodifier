"""Val-curve extrapolator for e1_train arms (TRAINING.md §12).

Reads one or more run dirs' ``train-manifest.json`` ``val_history``
(the record of record, trainer v5+) and fits two plateau forms:

    power law:  L(s) = Linf + a * s^-b
    exp:        L(s) = Linf + a * exp(-b * s)

For each candidate ``b`` on a fixed grid the model is linear in
(Linf, a), so a closed-form 2x2 least-squares solve covers the grid;
the form+b with the lowest RMSE wins, then ``b`` is locally polished.
Outputs the projected val loss at the arm's total step count and two
flags:

- ``plateaued``: the last point improves on the best of the three
  preceding points by < 1%. A plateau means THIS recipe, on THIS
  data, with THIS schedule has converged — it does NOT mean the arm
  is good (only the held-out gates say that) and it does NOT bound
  other recipes (see TRAINING.md §12 for the named interventions
  that can move a plateaued curve).
- ``val_turning_up``: the last point is above the minimum of the
  tail — past the val minimum; adapter-best holds the right
  checkpoint and more steps are overfitting territory.

``--warn-above X`` marks ``abort_candidate`` when the projected final
sits above X. The tool flags for review; it never kills an arm.

Within-loss-mode comparability rule: hard-CE and soft-KL val losses
are different surfaces. Never compare projections ACROSS loss modes —
only within one mode, same platform.

Pure stdlib: unit-testable on hosts without torch, like the other
runner helpers. ``--self-test`` runs synthetic-curve assertions.

Usage:
    python3 runner/arm_curve_fit.py RUN_DIR [RUN_DIR...] \
        [--total-steps N] [--warn-above X] [--min-points 5]
"""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from pathlib import Path

GRID_POWER = [round(0.05 * 1.15 ** k, 6) for k in range(60)]
GRID_EXP = [round(1e-6 * 1.5 ** k, 10) for k in range(50)]


def _lin_lsq(x: list[float], y: list[float]) -> tuple[float, float, float]:
    """Least squares y ~ a*x + Linf; returns (a, Linf, rmse)."""
    n = float(len(x))
    sxx = sum(v * v for v in x)
    sx = sum(x)
    sy = sum(y)
    sxy = sum(xi * yi for xi, yi in zip(x, y))
    det = sxx * n - sx * sx
    if abs(det) < 1e-30:
        raise ZeroDivisionError("degenerate design matrix")
    a = (sxy * n - sy * sx) / det
    linf = (sxx * sy - sx * sxy) / det
    resid = math.fsum((yi - (a * xi + linf)) ** 2
                      for xi, yi in zip(x, y))
    return a, linf, math.sqrt(resid / n)


def _fit_form(xs: list[float], ys: list[float], trans,
              grid: list[float]) -> tuple[float, float, float, float]:
    """Grid b, solve linear part, polish best b; returns (b, a, linf,
    rmse). Each form gets only its own b range, and a candidate with
    Linf < 0 is rejected outright — a loss floor cannot be negative,
    and the unconstrained solve happily trades an absurd negative
    floor for a hair of RMSE on noisy tails."""
    best = None
    for b in grid:
        try:
            x = [trans(s, b) for s in xs]
            a, linf, rmse = _lin_lsq(x, ys)
        except ZeroDivisionError:
            continue
        if not all(math.isfinite(v) for v in (a, linf, rmse)):
            continue
        if linf < 0:
            continue
        if best is None or rmse < best[3]:
            best = (b, a, linf, rmse)
    if best is None:
        raise ValueError("no finite fit on the grid")
    b, a, linf, rmse = best
    for mult in (0.9, 0.95, 1.05, 1.1):
        b2 = b * mult
        try:
            x = [trans(s, b2) for s in xs]
            a2, linf2, rmse2 = _lin_lsq(x, ys)
        except ZeroDivisionError:
            continue
        if rmse2 < rmse and linf2 >= 0:
            b, a, linf, rmse = b2, a2, linf2, rmse2
    return b, a, linf, rmse


def fit_curve(points: list[tuple[int, float]]) -> dict:
    """Fit plateau forms to (step, loss) points; report projection and
    flags. Projection is returned at max(step) — the caller supplies
    the arm's true total separately if it wants a different horizon."""
    pts = sorted((float(s), l) for s, l in points)
    xs = [s for s, _ in pts]
    ys = [l for _, l in pts]
    if len(pts) < 2:
        return {"status": "insufficient", "points": len(pts)}
    if max(ys) - min(ys) < 1e-9:
        return {"status": "flat", "points": len(pts),
                "plateaued": True, "last_loss": ys[-1]}
    try:
        pow_f = _fit_form(xs, ys, lambda s, b: s ** -b, GRID_POWER)
        exp_f = _fit_form(xs, ys, lambda s, b: math.exp(-b * s),
                          GRID_EXP)
    except ValueError:
        return {"status": "unfittable", "points": len(pts),
                "last_loss": ys[-1],
                "note": "no candidate with a non-negative loss floor; "
                        "curve is likely still in its noisy descent"}
    form, fit = ("power", pow_f) if pow_f[3] <= exp_f[3] \
        else ("exp", exp_f)
    b, a, linf, rmse = fit
    horizon = xs[-1]
    proj = linf + (a * horizon ** -b if form == "power"
                   else a * math.exp(-b * horizon))
    tail = ys[-4:-1] if len(ys) >= 4 else ys[:-1]
    base = min(tail)
    plateaued = (base - ys[-1]) / base < 0.01
    tail_min = min(ys[-5:]) if len(ys) >= 5 else min(ys)
    turning_up = ys[-1] > tail_min * 1.002
    return {"status": "ok", "form": form, "b": round(b, 6),
            "linf": round(linf, 6), "rmse": round(rmse, 6),
            "points": len(pts), "last_loss": ys[-1],
            "projected_final": round(proj, 6),
            "plateaued": plateaued, "val_turning_up": turning_up}


VAL_LINE = re.compile(
    r"^val step=(\d+) loss=([0-9.]+)", re.MULTILINE)
TOTAL_LINE = re.compile(r"step=(\d+)/(\d+)")
KNOBS_LINE = re.compile(r'^knobs: (\{.*\})', re.MULTILINE)


def _read_log(run_dir: Path) -> str:
    log = run_dir / "train-log.txt"
    return log.read_text(errors="ignore") if log.is_file() else ""


def history_for(run_dir: Path) -> tuple[list[tuple[int, float]],
                                       str | None]:
    """(points, loss_mode). Manifest val_history is the record of
    record (trainer v5 writes it at run end); mid-run dirs have only
    the log, so parse ``val step=N loss=X`` lines from it."""
    mf = run_dir / "train-manifest.json"
    loss_mode: str | None = None
    pts: list[tuple[int, float]] = []
    if mf.is_file():
        m = json.loads(mf.read_text())
        loss_mode = m.get("loss_mode")
        pts = [(h["step"], h["loss"]) for h in (m.get("val_history")
                                                or [])
               if "step" in h and "loss" in h]
    if not pts:
        log = _read_log(run_dir)
        pts = [(int(s), float(l)) for s, l in VAL_LINE.findall(log)]
        if loss_mode is None and log:
            k = KNOBS_LINE.search(log)
            if k:
                try:
                    soft = json.loads(k.group(1)).get("soft_labels")
                except json.JSONDecodeError:
                    soft = None
                loss_mode = ("soft" if soft else "hard") \
                    if soft is not None else None
    return pts, loss_mode


def total_steps_for(run_dir: Path) -> int | None:
    m = TOTAL_LINE.search(_read_log(run_dir))
    return int(m.group(2)) if m else None


def self_test() -> int:
    failures: list[str] = []

    def check(name: str, got, want, tol: float) -> None:
        ok = isinstance(got, (int, float)) and abs(got - want) <= tol
        if not ok:
            failures.append(f"{name}: got {got}, want ~{want}")

    steps = [int(100 * 1.35 ** k) for k in range(18)]
    curve_p = [0.05 + 2.0 * s ** -0.7 for s in steps]
    r = fit_curve(list(zip(steps, curve_p)))
    check("power projection", r["projected_final"],
          0.05 + 2.0 * steps[-1] ** -0.7, 0.002)
    check("power rmse small", 0.0 if r["rmse"] < 0.005 else 1.0, 0.0,
          1e-9)
    curve_e = [0.1 + 3.0 * math.exp(-s / 800.0) for s in steps]
    r = fit_curve(list(zip(steps, curve_e)))
    check("exp projection", r["projected_final"],
          0.1 + 3.0 * math.exp(-steps[-1] / 800.0), 0.005)
    r = fit_curve([(100 * k, 0.05 + 0.001 / k) for k in range(1, 12)])
    if r.get("plateaued") is not True:
        failures.append(f"flat curve not flagged plateaued: {r}")
    up = [0.2, 0.15, 0.12, 0.11, 0.105, 0.103, 0.102, 0.1015, 0.115]
    r = fit_curve(list(zip(range(100, 1000, 100), up)))
    if r.get("val_turning_up") is not True:
        failures.append(f"upturn not flagged: {r}")
    r = fit_curve([(100, 0.3)])
    if r.get("status") != "insufficient":
        failures.append(f"single point not insufficient: {r}")
    if failures:
        for f in failures:
            sys.stdout.write(f"SELF-TEST FAIL {f}\n")
        return 1
    sys.stdout.write("SELF-TEST PASS\n")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("run_dirs", nargs="*", type=Path)
    ap.add_argument("--total-steps", type=int, default=None,
                    help="projection horizon; default reads it from "
                         "the run's train-log.txt")
    ap.add_argument("--warn-above", type=float, default=None,
                    help="flag abort_candidate when the projected "
                         "final val exceeds this")
    ap.add_argument("--min-points", type=int, default=5,
                    help="fewer val points than this: report but do "
                         "not project")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if not args.run_dirs:
        ap.error("at least one RUN_DIR (or --self-test)")
    rc = 0
    for rd in args.run_dirs:
        mf = rd / "train-manifest.json"
        pts, loss_mode = history_for(rd)
        if not pts:
            sys.stdout.write(
                f"{rd}: no val history (manifest or log)\n")
            rc = 1
            continue
        out: dict = {"run": str(rd)}
        if len(pts) >= args.min_points:
            out.update(fit_curve(pts))
            total = args.total_steps or total_steps_for(rd)
            if total and out.get("status") == "ok":
                f = out
                out["total_steps"] = total
                out["note"] = (
                    f"projection is at step {pts[-1][0]}; scale toward "
                    f"step {total} only via a refit with more points — "
                    f"flat-extrapolation under a power law is "
                    f"directional, not a promise")
                if args.warn_above is not None \
                        and f["projected_final"] > args.warn_above \
                        and not f["val_turning_up"]:
                    out["abort_candidate"] = True
        else:
            out["status"] = "insufficient"
            out["points"] = len(pts)
        out["loss_mode"] = loss_mode
        sys.stdout.write(json.dumps(out) + "\n")
    return rc


if __name__ == "__main__":
    sys.exit(main())
