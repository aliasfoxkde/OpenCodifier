"""Per-arm resource accounting for the decision-model benchmark runners.

A :class:`ResourceMonitor` samples ``/proc`` for the runner's process tree —
spawned servers (llama-server, the opencodifier binary) are descendants of
the runner process, so one monitor rooted at ``os.getpid()`` covers the
whole arm. Peak RSS comes from the kernel-maintained ``VmHWM`` (not from
sampling), CPU seconds from ``utime + stime`` ticks, and IO bytes from
``/proc/<pid>/io``. Counters are cumulative-maxima per pid, so processes
that exit mid-run keep their contribution.

No third-party dependency: stdlib ``/proc`` reads only. The sampler thread
runs at 4 Hz; its own CPU cost is included in the arm's numbers, which is
the honest measurement (well under 0.1 % in practice).

Result shape (attached to run JSONs under ``"resources"``)::

    {
      "sampling": {"interval_s": 0.25, "samples": 612, "pids_seen": 2},
      "peak_rss_mib": 1222.4,      # max single-process VmHWM in the tree
      "tree_rss_mib_final": 1240.1, # sum of VmRSS when stopped
      "cpu_seconds": 731.2,         # whole tree, utime+stime
      "io_read_mib": 1301.9,        # whole tree, bytes actually read
      "io_write_mib": 12.0,         # whole tree
      "wall_s": 153.1,
    }

An arm whose counters were unreadable (permission, non-Linux) reports
``"error"`` instead of fabricated zeros.
"""

from __future__ import annotations

import os
import threading
import time
from pathlib import Path

_CLK_TCK = os.sysconf("SC_CLK_TCK")
_PAGE_SIZE = os.sysconf("SC_PAGE_SIZE")
_MIB = 1024.0 * 1024.0


def _read_status(pid: int) -> dict[str, int]:
    """Parse /proc/<pid>/status Vm* lines (kB) into a dict."""
    out: dict[str, int] = {}
    try:
        text = Path(f"/proc/{pid}/status").read_text()
    except OSError:
        return out
    for line in text.splitlines():
        key, _, value = line.partition(":")
        if key in ("VmRSS", "VmHWM") and value.strip().endswith("kB"):
            out[key] = int(value.strip().split()[0]) * 1024
    return out


def _read_cpu_ticks(pid: int) -> int | None:
    """utime + stime in clock ticks from /proc/<pid>/stat."""
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
    except OSError:
        return None
    # comm may contain spaces and parentheses: split after the last ')'.
    fields = stat.rpartition(")")[2].split()
    try:
        return int(fields[11]) + int(fields[12])
    except (IndexError, ValueError):
        return None


def _read_io(pid: int) -> tuple[int, int] | None:
    """(read_bytes, write_bytes) from /proc/<pid>/io."""
    try:
        text = Path(f"/proc/{pid}/io").read_text()
    except OSError:
        return None
    values: dict[str, int] = {}
    for line in text.splitlines():
        key, _, value = line.partition(":")
        if key in ("read_bytes", "write_bytes"):
            values[key] = int(value.strip())
    if "read_bytes" in values and "write_bytes" in values:
        return values["read_bytes"], values["write_bytes"]
    return None


def _process_tree(root: int) -> list[int]:
    """Return [root] + all descendants, walking PPID links over one /proc scan."""
    ppid_of: dict[int, int] = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / "stat").read_text()
        except OSError:
            continue
        fields = stat.rpartition(")")[2].split()
        try:
            ppid_of[int(entry.name)] = int(fields[1])
        except (IndexError, ValueError):
            continue
    tree: list[int] = []
    stack = [root]
    while stack:
        pid = stack.pop()
        tree.append(pid)
        stack.extend(p for p, pp in ppid_of.items() if pp == pid and p not in tree)
    return tree


class ResourceMonitor:
    """Sample RSS/CPU/IO of the runner's process tree until :meth:`stop`."""

    def __init__(self, root_pid: int | None = None, interval_s: float = 0.25):
        self._root = os.getpid() if root_pid is None else root_pid
        self._interval = interval_s
        self._stop_evt = threading.Event()
        self._thread: threading.Thread | None = None
        self._pids_seen: set[int] = set()
        self._hwm: dict[int, int] = {}
        self._cpu_ticks: dict[int, int] = {}
        self._io_bytes: dict[int, tuple[int, int]] = {}
        self._samples = 0
        self.error: str | None = None
        self._t0 = 0.0
        self._wall_s = 0.0

    # -- lifecycle ----------------------------------------------------------

    def start(self) -> None:
        self._t0 = time.monotonic()
        self._thread = threading.Thread(target=self._loop, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        """Stop sampling (idempotent) and freeze the wall-clock total."""
        if self._thread is not None:
            self._stop_evt.set()
            self._thread.join(timeout=self._interval * 4 + 2.0)
            self._thread = None
        self._wall_s = time.monotonic() - self._t0

    def _loop(self) -> None:
        while not self._stop_evt.wait(self._interval):
            self._sample()
            self._samples += 1

    def _sample(self) -> None:
        for pid in _process_tree(self._root):
            self._pids_seen.add(pid)
            # VmHWM must be captured while the process lives: its /proc
            # entry disappears on exit, taking the kernel peak with it.
            status = _read_status(pid)
            if hwm := status.get("VmHWM"):
                self._hwm[pid] = max(self._hwm.get(pid, 0), hwm)
            if (ticks := _read_cpu_ticks(pid)) is not None:
                self._cpu_ticks[pid] = max(self._cpu_ticks.get(pid, 0), ticks)
            if (io := _read_io(pid)) is not None:
                prev = self._io_bytes.get(pid, (0, 0))
                self._io_bytes[pid] = (max(prev[0], io[0]), max(prev[1], io[1]))

    # -- reporting ----------------------------------------------------------

    def report(self) -> dict:
        """Fold the sampled counters into the run-JSON ``resources`` block."""
        if self._thread is not None:
            self.stop()
        peak_rss = max(self._hwm.values()) if self._hwm else 0
        final_tree_rss = 0
        for pid in self._pids_seen:
            final_tree_rss += _read_status(pid).get("VmRSS", 0)
        if not self._pids_seen:
            self.error = "no /proc samples collected"
        report: dict = {
            "sampling": {
                "interval_s": self._interval,
                "samples": self._samples,
                "pids_seen": len(self._pids_seen),
            },
            "peak_rss_mib": round(peak_rss / _MIB, 1),
            "tree_rss_mib_final": round(final_tree_rss / _MIB, 1),
            "cpu_seconds": round(
                sum(self._cpu_ticks.values()) / _CLK_TCK, 2
            ),
            "io_read_mib": round(
                sum(v[0] for v in self._io_bytes.values()) / _MIB, 1
            ),
            "io_write_mib": round(
                sum(v[1] for v in self._io_bytes.values()) / _MIB, 1
            ),
            "wall_s": round(self._wall_s, 2),
        }
        if self.error is not None:
            report["error"] = self.error
        return report

    def __enter__(self) -> "ResourceMonitor":
        self.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        self.stop()
