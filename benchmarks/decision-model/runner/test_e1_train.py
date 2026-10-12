#!/usr/bin/env python3
"""Fixture tests for e1_train.bucketed_order (torch-free, stdlib only).

The A7 length-bucketing path was found inert on 2026-10-12 (t5500):
the original implementation shuffled the flat concatenation of
length-sorted chunks, dissolving the grouping it had just built, so
`--length-bucket` batches padded exactly like the plain shuffle and
measured wall time was identical. These tests pin the repaired
semantics: consecutive micro-batches drawn from the order are
length-homogeneous, while the epoch-level permutation still reshuffles.

Run: python3 -m pytest benchmarks/decision-model/runner/test_e1_train.py -q
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e1_train import bucketed_order  # noqa: E402


def _lengths() -> list[int]:
    # 64 rows, four distinct length bands — homogeneous batches are only
    # possible if band members stay near each other in the order.
    return [200 + 300 * (i % 4) + (i * 7) % 100 for i in range(64)]


def test_chunks_are_length_sorted() -> None:
    lengths = _lengths()
    order = bucketed_order(lengths, seed=42, epoch=0, bucket=16)
    assert sorted(order) == list(range(64))
    for start in range(0, len(order), 16):
        chunk = order[start:start + 16]
        widths = [lengths[i] for i in chunk]
        # The flat shuffle that made --length-bucket inert breaks this
        # on the first chunk: sortedness inside the mega-batch is the
        # property the grouping exists to create.
        assert widths == sorted(widths), (start, widths)


def test_micro_batches_are_mostly_length_homogeneous() -> None:
    lengths = _lengths()
    order = bucketed_order(lengths, seed=42, epoch=0, bucket=16)
    width = 4  # micro-batch of 4 indices drawn consecutively
    homogeneous = 0
    total = 0
    for start in range(0, len(order) - width + 1, width):
        span = order[start:start + width]
        widths = [lengths[i] for i in span]
        total += 1
        # Same length band (band width 300) + intra-band jitter < 100.
        # Sorted chunks of 4 band members keep boundary batches (one
        # per chunk) mixed, like HF's LengthGroupedSampler — so assert
        # the rate, not every batch.
        homogeneous += max(widths) - min(widths) < 300
    assert homogeneous / total >= 0.7, (homogeneous, total)


def test_orders_differ_across_epochs_and_are_deterministic() -> None:
    lengths = _lengths()
    e0 = bucketed_order(lengths, seed=42, epoch=0, bucket=16)
    e0_again = bucketed_order(lengths, seed=42, epoch=0, bucket=16)
    e1 = bucketed_order(lengths, seed=42, epoch=1, bucket=16)
    assert e0 == e0_again
    assert e0 != e1
    assert sorted(e0) == sorted(e1) == list(range(64))
