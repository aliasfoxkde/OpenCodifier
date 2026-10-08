#!/usr/bin/env python3
"""Frozen-evidence packaging per board run (RESEARCH §15.6 item 8).

One manifest per freeze: the SHA-256 of every input the board numbers
derive from — the docs of record, the generated artifacts, the suite
file, the runner code itself, and each frozen run JSON the parity
bundle consumed — plus a repro script that regenerates the derived
artifacts from those inputs. Numbers without their evidence bytes are
a claim; this makes each board state re-verifiable:

  sha256sum --check results/evidence/EVIDENCE-<stamp>.txt
  bash results/evidence/repro-<stamp>.sh   # rebuilds board.csv et al.

Writes three files into ``results/evidence/``:
- ``EVIDENCE-<stamp>.txt``  sha256sum-format manifest (machine-checkable)
- ``EVIDENCE-<stamp>.json`` the same digests plus sizes and timestamps
- ``repro-<stamp>.sh``      the derivation chain, pinned to this freeze
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import time
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git_rev(repo: Path) -> str:
    try:
        return subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=repo, timeout=10,
            capture_output=True, text=True, check=True,
        ).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return "unknown"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    here = Path(__file__).resolve().parent
    bench = here.parent
    repo = bench.parents[1]
    ap.add_argument("--runs-dir", type=Path,
                    default=Path("/nas/Temp/work/oc-model-eval/runs"))
    ap.add_argument("--parity", type=Path, default=bench / "results" / "parity.json")
    ap.add_argument("--outdir", type=Path, default=bench / "results" / "evidence")
    args = ap.parse_args()

    inputs: list[Path] = [
        repo / "docs" / "BENCHMARKS.md",
        bench / "results" / "REPORT.md",
        bench / "results" / "board.csv",
        bench / "results" / "parity.json",
        bench / "results" / "RELIABILITY.md",
        bench / "results" / "models.manifest.json",
        bench / "suite" / "suite.json",
        # The code that produced the numbers is evidence too.
        here / "board_csv.py",
        here / "parity.py",
        here / "summarize.py",
        here / "freeze_evidence.py",
    ]
    if args.parity.is_file():
        parity = json.loads(args.parity.read_text())
        for name in sorted(parity):
            run_file = args.runs_dir / name
            if run_file.is_file():
                inputs.append(run_file)
            else:
                sys.stderr.write(f"parity source missing, skipped: {run_file}\n")
    inputs = sorted({p for p in inputs if p.is_file()})

    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    args.outdir.mkdir(parents=True, exist_ok=True)
    # Repo files key by repo-relative path; the frozen run JSONs live
    # outside the repo by design, so they key by full path — which is
    # also what makes the manifest directly `sha256sum --check`-able.
    def key_of(p: Path) -> str:
        try:
            return str(p.relative_to(repo))
        except ValueError:
            return str(p)

    digest = {key_of(p): sha256(p) for p in inputs}

    txt = args.outdir / f"EVIDENCE-{stamp}.txt"
    txt.write_text("".join(f"{v}  {k}\n" for k, v in sorted(digest.items())))
    js = args.outdir / f"EVIDENCE-{stamp}.json"
    js.write_text(json.dumps({
        "created_utc": stamp,
        "git_rev": git_rev(repo),
        "runs_dir": str(args.runs_dir),
        "inputs": {k: {"sha256": v,
                       "bytes": Path(k).stat().st_size}
                   for k, v in sorted(digest.items())},
    }, indent=1, sort_keys=True) + "\n")

    repro = args.outdir / f"repro-{stamp}.sh"
    runs = str(args.runs_dir)
    repro.write_text(f"""#!/usr/bin/env bash
# Reproduces the derived board artifacts from the frozen evidence of
# {stamp} (git {git_rev(repo)[:12]}). Run from the repository root.
# 1. verify the inputs are byte-identical to what this freeze hashed:
#      sha256sum --check results/evidence/EVIDENCE-{stamp}.txt
set -euo pipefail
RUNS={runs!r} python3 benchmarks/decision-model/runner/parity.py \\
  --runs-dir "$RUNS"
RUNS={runs!r} python3 benchmarks/decision-model/runner/board_csv.py \\
  --runs-dir "$RUNS"
python3 benchmarks/decision-model/runner/summarize.py \\
  --results-dir benchmarks/decision-model/results
""")
    repro.chmod(0o755)
    sys.stdout.write(
        f"froze {len(digest)} inputs -> {txt.name}, {js.name}, {repro.name}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
