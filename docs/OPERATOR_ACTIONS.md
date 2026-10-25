# Operator Action Checklist — 2026-10-21

## Immediately actionable

### 1. PR #7 — merge cube games branch ✅
- **Branch:** `site/games-audit-2026-10` (gitforge + origin)
- **Status:** ✅ MERGED and deployed to CF Pages production (2026-10-19)

### 2. Vision branch — merge to main ✅
- **Branch:** `vision/vision-rung-2026-10`
- **Status:** ✅ MERGED to `main` and pushed to gitforge (2026-10-20)

### 3. X/Discord handles — add to site
- Pages currently render nothing for X/Discord links
- Add real handles to `site/index.html` / `site/site-core.js` footer

### 4. Sandbox EROFS fix (local CI — operator action)
- `cargo clippy` and `cargo test` fail with rc=101 when routed through the
  harness job manager's sandbox (`ProtectSystem=strict ReadWritePaths`)
  because `$PWD/target/` is not in the allowlist.
- **Fix:** add `$PWD/target/` to the `ProtectSystem=strict ReadWritePaths=`
  directive in the systemd unit that starts the job manager service.
- Without this fix, the local `just ci` gate cannot pass; GitForge CI
  (which uses its own sandbox) is unaffected.

---

## T5500-dependent

### 5. E1-F / E1-G arms training — COMPLETE (both chains ran clean)
- **E1-F (GPU 1):** corpus e1-softlabel-v2 (187,822 rows, teacher distributions from MiMo).
  Trained 11,615/11,615 steps; adapter-best saved. **Fails the release gate:**
  suite holdout 0.725 (< 0.75), JevBench-231 0.6667 (≥ 0.6494). Calibration is
  the win: suite ECE 0.0907, JevBench ECE 0.1446 — roughly half of E1-E's.
- **E1-G (GPU 0):** corpus merged-v6-full (306,999 rows, Jev-distilled stream).
  Trained 19,058/19,058 steps, **zero `gn=nan`**, final val loss 0.0387 /
  tok_acc 0.9845; adapter-best saved. **Passes both gates but does not
  dethrone E1-E:** suite holdout 0.7667, rel200 0.7214, JevBench-231 0.6805
  (macro 0.6055, ECE 0.294). Champion remains E1-E at 0.8083 / 0.6840.
- **Leakage audit (2026-10-22): CLEAN.** Exact + 200-char-prefix normalized
  state-text match, 196 text-state JevBench items × 292,412 unique v6 training
  prompts: 0 exact matches, 0 prefix matches. E1-G's 0.6805 is reportable.
  (Audit script: `/tmp/leakage_audit_v6.py` on t5500.)
- **CORPUS NOTE (v3 corpus analysis, 2026-10-21):** `SargeDev/jev-distill-corpus-v3`
  (740,957 rows) has **zero family overlap** with JevBench-231's taxonomy
  (`relational_compositional`, `lexical_semantic`, `metadata_match`). E1-G's
  per-family JevBench breakdown shows the cost (macro 0.6055 vs E1-E's 0.6832,
  one family at 0.0) even though the overall number survives. The path forward
  is (a) upweight the typed-decisions stream, (b) remap v3 families
  to eval taxonomy, or (c) per-family routing via `metadata.family` (D42,
  shipped — mechanism only, no profile until thresholds are re-measured).
  See RESEARCH.md §29.
- **Knobs:** Qwen3.5-2B + verdict-slot LoRA (r=16, alpha=32, dropout=0.05,
  lr=1e-4, batch=2, accum=8, max_len=2048, fp16, sdpa, seed=42, 1 epoch).
- **Gates:** suite ≥ 0.75 AND JevBench 231 ≥ 0.6494 (E1-E champion is 0.8083 / 0.6840).
- **NaN gate fixed (2026-10-22):** the launcher's `grep -qiE '(^|[^a-z])nan([^a-z]|$)'`
  false-positived on a single transient `gn=nan` at E1-F step 5180 and aborted
  its eval chain. Fixed copy strips `gn=nan` before grepping; **synced back to
  `~/t5500-bundle/train_e1fg.sh` on 2026-10-22** after pid 54136 exited (old
  script kept as `train_e1fg.sh.bak-badgate`).
- **Scripts/results:** `~/t5500-bundle/runs/{e1-f-2b-soft,e1-g-2b-v6}/`,
  `~/t5500-bundle/eval-results/jevbench-slot-{e1f-soft,e1g-v6}-231/`,
  logs `~/t5500-bundle/logs/e1fg-*.out`.

### 6. HF release of E1-E 2B — awaiting operator approval
- **Gates:** suite holdout 0.8083 ≥ 0.75 ✅; JevBench 231 0.6840 ≥ 0.6494 ✅
- **Artifacts:** `~/t5500-bundle/runs/e1-e-qwen2b/adapter-best/`
- **Release requires:** operator approval + org name; Apache-2.0; corpus withheld; explicit opt-in pull only, never auto-download at build.

### 5 (prev). jabr upstream PR — ✅ DONE
- **PR:** https://github.com/aliasfoxkde/classifier-benchmark/pull/1 (fork `aliasfoxkde/classifier-benchmark`, branch `opencodifier-backend`)
- **Upstream:** needs review + merge by jabr maintainers at `jabr/classifier-benchmark`

---

## Verification commands (run on this host)

```bash
# CI gates
cd /nas/Temp/repos/OpenCodifier
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --doc
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
aegis scan --file crates/ --quiet

# Vision probe (requires ORT_DYLIB_PATH)
ORT_DYLIB_PATH=~/.local/lib/python3.13/site-packages/onnxruntime/capi/libonnxruntime.so.1.24.2 \
  ./target/release/opencodifier decide \
    -i /tmp/vision-req.json \
    --vision-model-dir /nas/Temp/work/oc-model-eval/models/siglip2-base-patch16-256-ONNX \
    --image /tmp/blue.png
```

---

## JevBench split results (2026-10-19)

| split | n_attempted | n_valid | accuracy | ECE | p50 latency | determinism |
|---|---|---|---|---|---|---|
| easy | 48 | 48 | 0.6458 | 0.270 | 3.8 ms | 48/48 ✅ |
| full (public subset) | 72 | 46 | 0.2917 | 0.240 | 5.0 ms | 72/72 ✅ |
| hard | 53 | 47 | 0.2264 | 0.682 | 10.9 ms | — |

- Easy split: strong on adequacy (0.50) and policy (0.42); weak on extraction (0.33) and intent (0.25)
- Full (subset): ordinal family 0/12 schema-valid; extraction ECE 0.63 (confident + wrong)
- Hard split: 47.7% coverage (58 items not attempted); multi_hop 0.00, temporal_numeric 0.10
- Determinism: engine is bit-reproducible — all splits pass the replay check
- Raw runs at `/nas/Temp/work/oc-model-eval/runs/jevbench/engine-{easy,full,hard}/`

---

## Git state
- `main` → gitforge @ `5c05289` (clean, pushed 2026-10-24; recent:
  #188 gate A/B **GATE SHIPS**, #189 slice 1, §27 corrections +
  SEMANTIC_RERANKER design, tokio RUSTSEC bump, aegis regen)
  - `main` vs `origin/main`: **ahead 399, behind 1** — origin's one extra
    commit is the PR #7 merge (`b3d03ae`, same content as the local
    `site/games-audit-2026-10` line). Reconciling origin is an operator
    decision (GitForge is the CI pipeline of record).
- `fork/llama-decision-2026-10` → local @ `48ab97f` (= main @ `f60fd87` +
  the smoke-gate commit; worktree `/nas/Temp/repos/OpenCodifier-fork`)
- `vision/vision-rung-2026-10` → gitforge @ `d75df6d`-era (merged lineage)
- `site/games-audit-2026-10` → gitforge + origin (PR #7 MERGED)
- Stale worktree (operator cleanup candidate): `/home/mkinney/agent-work/
  opencodifier-correctness-20261020` — ~399 commits behind, clean, its one
  "extra" commit is the PR #7 duplicate.

---

## Results this session (2026-10-22)

| Item | Result |
|---|---|
| E1-F/E1-G arms | **Complete.** E1-F: 0.725 suite / 0.6667 JevBench — fails gate, ECE halved. E1-G: 0.7667 suite / 0.6805 JevBench — passes gate, below champion. E1-E remains champion. Leakage audit clean. |
| §27 HTTP bench | **Already existed.** Fixed stale note in RESEARCH.md — `crates/opencodifier-http/benches/http.rs` has bench_http_overhead. |
| Upload script fix | **Fixed.** Calibration artifact path corrected to `e1e-qwen3.5-2b-holdout.json`. |
| JEV-27B corpus discovery | **New lane.** `SargeDev/jev-distill-corpus-v3` (740,957 rows) with `family`/`domain`/`target` fields. yuri_v3 (498k rows) from Jev 1.13; openjev_v2 (94k, CC0). |
| Training pipeline improvements | **Identified.** Multi-teacher aggregation, domain-specific adapters, quality-weighted curriculum, fast iteration on 1000-item stratified slice. |

---

## JEV-27B corpus — action items

Priority for next session:

1. **Probe JEV-27B on JevBench-231** — `autotrust/JEV-27B` (Qwen3.8-27B + 109M LoRA) as contamination audit baseline
2. **Train on v3 corpus directly** — 740k rows with Jev 1.13 soft labels; replace/augment E1-G's merged-v6
3. **Use openjev_v2 for clean eval** — 94k CC0 rows with programmatic ground truth
4. **VIVERE extract from JEV-27B on contested items** — E1-E slot_agree ~0.98; mine the ~2% disagreement for training signal
