# fork_4b kill salvage — partial-run context (NOT a number of record)

The 2026-09-30 12:07:31 chain watchdog kill (sibling load spike to 151,
<3 min) left 231 of 693 JevBench public tasks complete in
`/dev/shm/jevbench-fork4b/results.jsonl` before the re-run driver
(preserving this dir as `.killed-1207`) rebuilds the pre-d15 binary at
`ad129b0` and starts over. These rows are a **latency-biased subsample**
— tasks complete out of order under `--decision-seqs 24`, so what
survives over-represents fast tasks and under-represents `easy` (48 of
231 rows) — and the arm ran the OLD pre-d15 binary. Zero non-ok rows,
zero schema errors: the kill was clean mid-flight.

Subsample accuracy 0.667 (easy 0.979 n=48 · hard 0.396 n=111 ·
original 0.875 n=72), mean latency 38.4 s. Use: context for #39 and a
determinism cross-check once the re-run lands — never as the fork arm's
accuracy. The row of record is the re-run's
`runs/jevbench/fork_4b-v1/`.
